#!/usr/bin/env bash
# Builds the guest init (crates/iii-init) as a static musl binary into
# ${CARGO_TARGET_DIR:-target}/<musl triple>/release/iii-init, where
# crates/iii-filesystem/build.rs picks it up for `--features embed-init`.
#
# Usage: BUILD_TARGET=<host target triple> scripts/build-init.sh
# The guest always runs Linux, so only the arch of BUILD_TARGET matters
# (x86_64-unknown-linux-gnu and x86_64-apple-darwin both give x86_64 init).
set -euo pipefail

# Run from the crate root so .cargo/config.toml (rust-lld linker) applies.
cd "$(dirname "$0")/.."

case "${BUILD_TARGET:?set BUILD_TARGET to the host target triple}" in
  x86_64-*) triple=x86_64-unknown-linux-musl ;;
  aarch64-*) triple=aarch64-unknown-linux-musl ;;
  *) echo "unsupported BUILD_TARGET: $BUILD_TARGET" >&2; exit 1 ;;
esac

rustup target add "$triple"
cargo build --locked -p iii-init --release --target "$triple"
init="${CARGO_TARGET_DIR:-target}/$triple/release/iii-init"
test -s "$init"
echo "built $init"
