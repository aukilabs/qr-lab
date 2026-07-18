#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

tree="$(cargo tree -p qr-lab-imgproc --edges normal --prefix none)"
for forbidden in rqrr serde wasm-bindgen jni js-sys; do
  if grep -Eq "^${forbidden}( |$)" <<<"$tree"; then
    echo "error: qr-lab-imgproc unexpectedly depends on $forbidden" >&2
    exit 1
  fi
done

cargo check -p qr-lab-image --no-default-features
cargo check -p qr-lab-geometry --no-default-features
cargo check -p qr-lab-imgproc --no-default-features
cargo check -p qr-lab-qr --no-default-features
cargo check -p qr-lab-qr --no-default-features --features serde,debug-trace
cargo check -p qr-lab --no-default-features

echo "OK: QR Lab dependency boundaries and feature combinations"
