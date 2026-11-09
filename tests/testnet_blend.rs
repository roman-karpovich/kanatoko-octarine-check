//! Octarine settlement, built from the pinned upstream commit, on a fork of Stellar testnet,
//! priced by the oracle of the testnet Blend v2 pool through Octarine's own SEP-40 adapter.

use ed25519_dalek::{Signer as _, SigningKey};
use kanatoko::soroban_sdk::testutils::Ledger as _;
use kanatoko::soroban_sdk::token::{StellarAssetClient, TokenClient};
use kanatoko::soroban_sdk::{self, Address, Bytes, BytesN, Env, FromVal, Map, Symbol, Val};
use kanatoko::{testnet, ScenarioFork};

mod rfq {
    use super::soroban_sdk;
    soroban_sdk::contractimport!(file = "wasm/rfq.wasm");
}

mod feed {
    use super::soroban_sdk;
    soroban_sdk::contractimport!(file = "wasm/feed_oracle.wasm");
}

const BLEND_POOL: &str = "CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF";
const XLM: &str = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC";
const USDC: &str = "CAQCFVLOBK5GIULPNZRGATJJMIZL5BSP7X5YJVMGCPTUEPFM4AVSRCJU"; // the pool's reserve
const CAPTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/testnet-blend.json");
const DAY: u64 = 86_400;
const ONE: i128 = 10_000_000; // one XLM or USDC: both have 7 decimals
const MAX_AGE: u64 = 900; // the feed age the upstream oracle script defaults to

/// Replays the committed capture offline; `RECAPTURE_RPC=<url>` takes a new one.
#[test]
fn rfq_settles_on_testnet_blend_prices() {
    let runner = testnet().cache(CAPTURE);
    let runner = match std::env::var("RECAPTURE_RPC") {
        Ok(url) => runner.rpc_url(url).refresh(),
        Err(_) => runner.offline(),
    };
    runner.run(scenario).expect("scenario runs on the capture");
}

fn scenario(fork: &ScenarioFork<'_>) {
    let env = fork.env();
    fork.mock_all_auths();
    let (xlm, usdc) = (fork.contract(XLM), fork.contract(USDC));
    let pool: Map<Symbol, Val> = fork.invoke(&fork.contract(BLEND_POOL), "get_config", ());
    let blend_oracle = Address::from_val(env, &pool.get_unchecked(Symbol::new(env, "oracle")));
    let (admin, maker, taker) =
        (account(fork, "admin"), account(fork, "maker"), account(fork, "taker"));

    let feed = feed::Client::new(env, &fork.deploy(feed::WASM, ()));
    feed.initialize(
        &admin,
        &feed::Config {
            source: blend_oracle,
            base: xlm.clone(),
            quote: usdc.clone(),
            base_asset: feed::Asset::Stellar(xlm.clone()),
            quote_asset: feed::Asset::Stellar(usdc.clone()),
            cross: true,
            base_decimals: 7,
            quote_decimals: 7,
            max_age: MAX_AGE,
            invert: false,
        },
    );
    let rfq = rfq::Client::new(env, &fork.deploy(rfq::WASM, ()));
    rfq.initialize(&admin, &usdc);
    rfq.set_config(&rfq::Config {
        min_fee_bps: 0,
        max_fee_bps: 1_000,
        fallback_max_age: 0,
        max_deviation_bps: 2_000,
        min_push_interval: 300,
        max_shift_seconds: 0,
        decay_seconds: (7 * DAY) as u32,
    });
    let oracle = rfq::OracleCfg { oracle: feed.address.clone(), max_age: MAX_AGE };
    rfq.set_oracle(&xlm, &usdc, &Some(oracle));
    let rolling = rfq::Schedule {
        mode: rfq::ScheduleMode::Rolling,
        rolling_seconds: (30 * DAY) as u32,
        next_redemption_at: 0,
        cycle_seconds: 0,
        max_bps_per_day: 1_000,
    };
    rfq.set_schedule(&admin, &xlm, &rolling);
    // Blend's testnet oracle quotes XLM at 0.42 USD and USDC at 1.00 USD.
    assert_eq!(rfq.price_of(&xlm, &usdc), 420_000_000_000_000_000);

    StellarAssetClient::new(env, &usdc).mint(&maker, &(1_000 * ONE));
    let until = env.ledger().sequence() + 1_000;
    TokenClient::new(env, &usdc).approve(&maker, &rfq.address, &(1_000 * ONE), &until);
    TokenClient::new(env, &xlm).approve(&taker, &rfq.address, &(2_000 * ONE), &until);
    let order = rfq::RfqOrder {
        maker_token: usdc.clone(),
        taker_token: xlm.clone(),
        taker_amount: 1_000 * ONE,
        min_received_amount: 400 * ONE,
        fee_bps: 10,
        taker: Some(taker.clone()),
        sender: None,
        fee_recipient: admin.clone(),
        expiry: env.ledger().timestamp() + 30 * DAY,
        salt: 1,
        taker_max_bps_per_day: 1_000,
        maker_bps_per_day: 250,
        max_maker_amount: 420 * ONE,
        maker: maker.clone(),
    };
    let signature = sign(env, &SigningKey::from_bytes(&[7; 32]), &rfq.hash_rfq_order(&order));
    rfq.register_order_signer(&maker, &signature.signer, &true);
    // 500 XLM at 0.42, less 2.50 bps/day over the 30-day rolling horizon (0.75%).
    let first = rfq.fill_rfq_order(&order, &signature, &None, &taker, &(500 * ONE));
    assert_eq!((first.maker_filled, first.fee), (2_084_250_000, 2_084_250));

    // A Dutch listing of 100 XLM decays from 50 to 40 USDC over a week.
    let dutch = rfq::DutchOrder {
        maker_token: usdc.clone(),
        taker_token: xlm.clone(),
        taker_amount: 100 * ONE,
        start_maker_amount: 50 * ONE,
        min_maker_amount: 40 * ONE,
        fee_bps: 10,
        fee_recipient: admin.clone(),
        expiry: 0,
    };
    let listing = rfq.create_dutch_order(&taker, &dutch);
    assert_eq!(rfq.current_ask(&listing), 50 * ONE);
    env.ledger().set_timestamp(env.ledger().timestamp() + 7 * DAY / 2);
    assert_eq!(rfq.current_ask(&listing), 45 * ONE);
    let bought = rfq.fill_dutch_order(&listing, &maker, &(45 * ONE));
    assert_eq!((bought.maker_filled, bought.fee), (45 * ONE, 450_000));

    // 3.5 days on, far past MAX_AGE, the feed is still fresh: this oracle stamps every read with
    // the current ledger time, so the stale-feed check never fires: the rest fills at 0.42 too.
    assert_eq!(feed.get_price(&xlm, &usdc).timestamp, env.ledger().timestamp());
    let second = rfq.fill_rfq_order(&order, &signature, &None, &taker, &(500 * ONE));
    assert_eq!(second, first);

    assert_eq!(TokenClient::new(env, &usdc).balance(&taker), 2 * 2_082_165_750 + 449_550_000);
    assert_eq!(TokenClient::new(env, &usdc).balance(&admin), 2 * 2_084_250 + 450_000);
    assert_eq!(TokenClient::new(env, &xlm).balance(&rfq.address), 0);
}

/// A local account with 10,000 XLM and a USDC trustline.
fn account(fork: &ScenarioFork<'_>, label: &str) -> Address {
    let account = fork.local_account(label);
    fork.fund_local_account(&account, 100_000_000_000);
    StellarAssetClient::new(fork.env(), &fork.contract(USDC)).trust(&account);
    account
}

/// SEP-53: the key signs sha256("Stellar Signed Message:\n" || order hash).
fn sign(env: &Env, key: &SigningKey, order_hash: &BytesN<32>) -> rfq::Signature {
    let mut message = Bytes::from_slice(env, b"Stellar Signed Message:\n");
    message.append(&Bytes::from_array(env, &order_hash.to_array()));
    let digest = env.crypto().sha256(&message).to_array();
    let signer = BytesN::from_array(env, &key.verifying_key().to_bytes());
    rfq::Signature { signer, signature: BytesN::from_array(env, &key.sign(&digest).to_bytes()) }
}
