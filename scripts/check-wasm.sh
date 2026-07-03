#!/usr/bin/env bash
set -euo pipefail

rustup target add wasm32-unknown-unknown 2>/dev/null
cargo check -p qrk-wasm --target wasm32-unknown-unknown
