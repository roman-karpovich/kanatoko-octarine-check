# Octarine on a Stellar testnet fork

An independent test of the [Octarine](https://github.com/mystic-finance/Stellar-RFQ)
settlement contract against real Stellar testnet state. It is not affiliated with or
endorsed by Octarine or Blend.

## What it runs

`tests/testnet_blend.rs` deploys two Octarine contracts, `rfq` and `feed_oracle`,
into a fork of testnet. Both are built unmodified from upstream commit
`9afddd0acf08df16aaf404e35c79b4dc5bea97ce`. Prices come from the SEP-40 oracle that
the testnet Blend v2 pool `CCEBVDYM32YNYCVNRXQKDFFPISJJCV557CDZEIRBEE4NCV4KHPQ44HGF`
is configured with, read through `feed_oracle`. The tokens are testnet XLM and the
pool's USDC reserve.

1. **Rolling RFQ fill.** 500 XLM sells at 0.42 USDC, less 2.50 bps/day over a
   30-day rolling horizon: 208.425 USDC, with a 10 bps fee.
2. **Dutch ask decay.** A listing of 100 XLM decays from 50 to 40 USDC over a week.
   After 3.5 days the ask is 45 USDC, and it fills at that price.
3. **Stale feed.** The second half of the RFQ order fills 3.5 days later, well past
   the 900-second feed age limit. It fills at the same price, because this oracle
   stamps every read with the current ledger time, so the staleness check never
   fires against it.

The fork is provided by [Kanatoko](https://github.com/roman-karpovich/kanatoko)
30.0.0-rc.1, which records the network entries a Rust scenario touches and then
replays them offline.

## Captured state

`fixtures/testnet-blend.json` holds testnet ledger 5648538 (2026-11-09 23:18:52 UTC),
Protocol 30, executed on the Protocol 30 host. It has nine entries: the Blend pool
instance and WASM, the oracle instance, WASM and two price entries, the XLM and USDC
asset contracts, and the USDC issuer account. The scenario discovered them, so
nothing was listed or patched by hand.

The test relies on these test-only steps:

- authorization is mocked;
- local accounts are created and given XLM by writing them into the fork;
- USDC is minted under the issuer's mocked authorization;
- time moves with `set_timestamp`, and the ledger sequence stays the same.

No captured storage is overridden.

## Run

Requirements: Rust 1.91.0 for the test, Rust 1.92.0 with the `wasm32v1-none` target
for the contracts, and `stellar-cli` 27.0.0. The CLI refuses Rust 1.91.0 for contract
builds, and installing it with `cargo install` needs Rust 1.93.0 or newer.

```sh
git clone https://github.com/mystic-finance/Stellar-RFQ upstream
git -C upstream checkout 9afddd0acf08df16aaf404e35c79b4dc5bea97ce
./build-upstream.sh upstream
cargo test --locked
```

`cargo test` replays the capture offline, without RPC calls.

## Rebuild

The Octarine WASM is not committed. `build-upstream.sh` builds it from the checkout
with `stellar contract build`, as the upstream Makefile does. Upstream does not commit
a `Cargo.lock`, so the script copies `upstream-deps.lock` into the checkout as its
`Cargo.lock`. That file holds the crates.io versions used here, and cargo adds the
workspace's own crates to it. The script then checks the result against `wasm.sha256`:

| File | SHA-256 |
| --- | --- |
| `rfq.wasm` | `c2c4a97d5c4166fe3b2d739703de94da41e28d890fa09456cc43ab2b73201d72` |
| `feed_oracle.wasm` | `eabb698b03fee3bd8cae6cba8648472a70640ab761cdc24468f8ecf470498060` |

## Re-capture

```sh
RECAPTURE_RPC=https://soroban-testnet.stellar.org cargo test --locked
```

This takes a fresh capture from any public testnet RPC and rewrites the fixture. The
committed fixture was taken through `https://soroban-rpc.testnet.stellar.gateway.fm`.
The test asserts exact prices, so it fails if the oracle's prices change.

## License

This repository's own files are licensed under MIT OR Apache-2.0. It contains no
Octarine source code or WASM. See [NOTICE](NOTICE).
