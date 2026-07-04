#!/usr/bin/env bash
set -euo pipefail

# Plan 5 Task 5: checks BOTH feature configurations. The default
# (feature-less) build is what matters for "does this compile the way the
# mobile-relevant surface would" — it must stay free of the `qrcode` crate
# (see qrk-wasm/Cargo.toml's `qr-gen` feature doc). The `qr-gen` build is
# what `scripts/build-wasm.sh` actually ships to the debug UI, so it needs
# checking too — a feature that only ever gets exercised by the release
# wasm-pack build (never by CI) would be an easy place for bit rot.
rustup target add wasm32-unknown-unknown 2>/dev/null
cargo check -p qrk-wasm --target wasm32-unknown-unknown
cargo check -p qrk-wasm --target wasm32-unknown-unknown --features qr-gen
