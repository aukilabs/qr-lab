#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

cargo build -p qr-lab-ffi --release

case "$(uname -s)" in
  Darwin) library="$ROOT/target/release/libqrk_ffi.dylib" ;;
  Linux) library="$ROOT/target/release/libqrk_ffi.so" ;;
  *) echo "error: unsupported host for ABI check" >&2; exit 1 ;;
esac

symbols="$(nm -g "$library")"
required=(
  qrk_version
  qrk_scan_luma
  qrk_free_string
  qrk_operator_context_create
  qrk_operator_context_destroy
  qrk_operator_last_error
  qrk_background_divide_luma_v1
  qrk_estimate_line_blur_luma_v1
  qrk_van_cittert_luma_v1
)

for symbol in "${required[@]}"; do
  if ! grep -Eq "[ _]${symbol}$" <<<"$symbols"; then
    echo "error: missing exported ABI symbol $symbol" >&2
    exit 1
  fi
done

cc -fsyntax-only -I "$ROOT/crates/qr-lab-ffi/include" \
  "$ROOT/crates/qr-lab-ffi/examples/operator.c"

echo "OK: legacy and QR Lab operator ABI symbols exported"
