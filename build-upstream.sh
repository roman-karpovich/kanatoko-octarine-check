#!/usr/bin/env bash
# Builds the two Octarine contracts the test deploys from a checkout of the
# pinned upstream commit, with `stellar contract build` as the upstream
# Makefile does, then checks them against the recorded hashes for this OS:
# wasm.macos.sha256 or wasm.linux.sha256.
#
#   ./build-upstream.sh path/to/Stellar-RFQ
set -euo pipefail

COMMIT=9afddd0acf08df16aaf404e35c79b4dc5bea97ce
# stellar-cli 27 refuses Rust 1.91.0 for contracts, so the WASM uses 1.92.0.
export RUSTUP_TOOLCHAIN=1.92.0

here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "${1:?usage: build-upstream.sh path/to/Stellar-RFQ}" && pwd)"

[ "$(git -C "$src" rev-parse HEAD)" = "$COMMIT" ] || { echo "checkout is not $COMMIT" >&2; exit 1; }
[ -z "$(git -C "$src" status --porcelain)" ] || { echo "checkout has local changes" >&2; exit 1; }
stellar --version | grep -q '^stellar 27\.0\.0 ' || { echo "needs stellar-cli 27.0.0" >&2; exit 1; }

# The optimised rfq.wasm lists four functions in a different order on macOS
# and on Linux; the modules are otherwise identical. Each OS has its own list.
case "$(uname -s)" in
  Darwin) sums=wasm.macos.sha256 ;;
  Linux) sums=wasm.linux.sha256 ;;
  *) echo "no recorded hashes for $(uname -s)" >&2; exit 1 ;;
esac

# Upstream does not commit a Cargo.lock. Pin the crates.io versions this WASM
# was built with; cargo adds the workspace's own crates to the lock.
cp "$here/upstream-deps.lock" "$src/Cargo.lock"
(cd "$src" && cargo metadata --format-version 1 > /dev/null)

mkdir -p "$here/wasm"
for package in rfq feed_oracle; do
  (cd "$src" && stellar contract build --locked --package "$package" --out-dir "$here/wasm")
done
(cd "$here/wasm" && shasum -a 256 -c "../$sums")
