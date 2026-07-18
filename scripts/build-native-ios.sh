#!/usr/bin/env bash
# Build libqrk_ffi.a for device + simulator and package Qrk.xcframework
# into bindings/expo-cpu-scanner/ios/.
#
# Requires: Xcode (xcodebuild), Rust targets:
#   aarch64-apple-ios
#   aarch64-apple-ios-sim
# Optional x86_64-apple-ios for Intel simulators (merged into sim slice if present).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Workspace profile: LTO + strip (see root Cargo.toml [profile.release-mobile]).
PROFILE="${QRK_MOBILE_PROFILE:-release-mobile}"
HEADER="$ROOT/crates/qr-lab-ffi/include/qrk.h"
OUT_XCFW="$ROOT/bindings/expo-cpu-scanner/ios/Qrk.xcframework"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/qrk-ios.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

if [[ ! -f "$HEADER" ]]; then
  echo "error: missing $HEADER" >&2
  exit 1
fi
if ! command -v xcodebuild >/dev/null 2>&1; then
  echo "error: xcodebuild not found (need Xcode)." >&2
  exit 1
fi

DEVICE_TARGET=aarch64-apple-ios
SIM_TARGET=aarch64-apple-ios-sim

for t in "$DEVICE_TARGET" "$SIM_TARGET"; do
  if ! rustup target list --installed | grep -qx "$t"; then
    echo "==> Installing Rust target $t"
    rustup target add "$t"
  fi
done

echo "==> Building qr-lab-ffi staticlib (device, profile $PROFILE)"
cargo build -p qr-lab-ffi --target "$DEVICE_TARGET" --profile "$PROFILE"

echo "==> Building qr-lab-ffi staticlib (simulator)"
cargo build -p qr-lab-ffi --target "$SIM_TARGET" --profile "$PROFILE"

DEVICE_LIB="$ROOT/target/$DEVICE_TARGET/$PROFILE/libqrk_ffi.a"
SIM_LIB="$ROOT/target/$SIM_TARGET/$PROFILE/libqrk_ffi.a"
for f in "$DEVICE_LIB" "$SIM_LIB"; do
  if [[ ! -f "$f" ]]; then
    echo "error: missing $f" >&2
    exit 1
  fi
done

# Headers + module map for each slice (xcodebuild -create-xcframework wants them).
mk_headers() {
  local dir="$1"
  mkdir -p "$dir"
  cp "$HEADER" "$dir/qrk.h"
  cat >"$dir/module.modulemap" <<'EOF'
module Qrk {
  header "qrk.h"
  export *
}
EOF
}

DEVICE_HEADERS="$STAGE/device/Headers"
SIM_HEADERS="$STAGE/sim/Headers"
mk_headers "$DEVICE_HEADERS"
mk_headers "$SIM_HEADERS"

# Optional Intel simulator slice — fat with arm64-sim if both exist.
# CocoaPods requires every xcframework slice to use the SAME library basename
# (libqrk_ffi.a), so we always stage copies under that name.
DEVICE_STAGED="$STAGE/device/libqrk_ffi.a"
SIM_STAGED="$STAGE/sim/libqrk_ffi.a"
cp "$DEVICE_LIB" "$DEVICE_STAGED"

if rustup target list --installed | grep -qx "x86_64-apple-ios"; then
  echo "==> Building qr-lab-ffi staticlib (x86_64 simulator)"
  cargo build -p qr-lab-ffi --target x86_64-apple-ios --profile "$PROFILE"
  X86_LIB="$ROOT/target/x86_64-apple-ios/$PROFILE/libqrk_ffi.a"
  if [[ -f "$X86_LIB" ]]; then
    lipo -create "$SIM_LIB" "$X86_LIB" -output "$SIM_STAGED"
  else
    cp "$SIM_LIB" "$SIM_STAGED"
  fi
else
  cp "$SIM_LIB" "$SIM_STAGED"
fi

rm -rf "$OUT_XCFW"
echo "==> Creating Qrk.xcframework"
xcodebuild -create-xcframework \
  -library "$DEVICE_STAGED" -headers "$DEVICE_HEADERS" \
  -library "$SIM_STAGED" -headers "$SIM_HEADERS" \
  -output "$OUT_XCFW"
echo "OK: iOS xcframework ready at bindings/expo-cpu-scanner/ios/Qrk.xcframework"
