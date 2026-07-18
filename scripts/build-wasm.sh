#!/usr/bin/env bash
set -euo pipefail

# Builds crates/qr-lab-wasm into a `--target web` package that the debug UI's
# worker imports directly (see debug-ui/src/scanner/worker.ts). Run from
# anywhere; paths below are resolved relative to the repo root.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$REPO_ROOT/crates/qr-lab-wasm"
OUT_DIR="$REPO_ROOT/debug-ui/src/wasm"

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "wasm-pack not found on PATH — installing it now (cargo install wasm-pack)." >&2
  echo "This is a one-time build-tool install; it can take a few minutes." >&2
  if ! cargo install wasm-pack; then
    echo "error: 'cargo install wasm-pack' failed. Install it manually:" >&2
    echo "  cargo install wasm-pack" >&2
    echo "See https://rustwasm.github.io/wasm-pack/installer/ for alternatives." >&2
    exit 1
  fi
fi

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

# IMPORTANT: wasm-pack resolves --out-dir relative to the crate directory
# being built (crates/qr-lab-wasm), NOT relative to the cwd this script runs
# from. `../../debug-ui/src/wasm` (two levels up from crates/qr-lab-wasm, back
# to the repo root) is what actually lands the package at
# debug-ui/src/wasm — verified empirically: an absolute --out-dir also
# works and is less fragile to crate-path changes, so we use that instead.
#
# --features qr-gen (Plan 5 Task 5): the debug UI's 3D-scene mode calls
# `generate_qr` (gated behind this cargo feature so the mobile-relevant
# default build stays free of the `qrcode` crate — see qr-lab-wasm/Cargo.toml
# and this repo's `scripts/check-wasm.sh`, which checks both feature
# configurations). The debug tool always wants it, so this script — the
# ONLY thing that produces the package the debug UI actually imports —
# always enables it.
wasm-pack build "$CRATE_DIR" --target web --release --out-dir "$OUT_DIR" -- --features qr-gen

echo "wasm build complete: $OUT_DIR"
