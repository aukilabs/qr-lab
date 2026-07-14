#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

tree="$(cargo tree -p qrkit-imgproc --edges normal --prefix none)"
for forbidden in rqrr serde wasm-bindgen jni js-sys; do
  if grep -Eq "^${forbidden}( |$)" <<<"$tree"; then
    echo "error: qrkit-imgproc unexpectedly depends on $forbidden" >&2
    exit 1
  fi
done

cargo check -p qrkit-image --no-default-features
cargo check -p qrkit-geometry --no-default-features
cargo check -p qrkit-imgproc --no-default-features
cargo check -p qrkit-qr --no-default-features
cargo check -p qrkit-qr --no-default-features --features serde,debug-trace
cargo check -p qrkit --no-default-features

echo "OK: QRKit dependency boundaries and feature combinations"
