#!/usr/bin/env bash
# Build libqrk_ffi.so for Android (arm64-v8a + x86_64) with 16 KB page-size
# ELF flags, and install into bindings/expo-cpu-scanner jniLibs.
#
# Requires: cargo, cargo-ndk, Android NDK (ANDROID_NDK_HOME or SDK ndk/).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OUT_JNI="$ROOT/bindings/expo-cpu-scanner/android/src/main/jniLibs"
# cargo-ndk installs as <out>/<abi>/lib*.so
TMP_OUT="$(mktemp -d "${TMPDIR:-/tmp}/qrk-android.XXXXXX")"
trap 'rm -rf "$TMP_OUT"' EXIT

# Resolve NDK for cargo-ndk / clang.
if [[ -z "${ANDROID_NDK_HOME:-}" && -z "${ANDROID_NDK_ROOT:-}" ]]; then
  SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
  if [[ -d "$SDK/ndk" ]]; then
    # Prefer newest installed NDK.
    NDK_VER="$(ls -1 "$SDK/ndk" | sort -V | tail -1)"
    export ANDROID_NDK_HOME="$SDK/ndk/$NDK_VER"
  fi
fi
if [[ -z "${ANDROID_NDK_HOME:-}" || ! -d "${ANDROID_NDK_HOME}" ]]; then
  echo "error: Android NDK not found. Set ANDROID_NDK_HOME or install via Android Studio SDK Manager." >&2
  exit 1
fi
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-$ANDROID_NDK_HOME}"

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "error: cargo-ndk not on PATH. Install with: cargo install cargo-ndk" >&2
  exit 1
fi

API_LEVEL="${ANDROID_API:-24}"
# Workspace profile: LTO + strip (see root Cargo.toml [profile.release-mobile]).
PROFILE="${QRK_MOBILE_PROFILE:-release-mobile}"

# Google Play 16 KB page-size requirement. Also set in .cargo/config.toml for
# the android targets; export here too so cargo-ndk's env cannot drop them.
PAGE16="-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS:-} $PAGE16"
export CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS:-} $PAGE16"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_RUSTFLAGS="${CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_RUSTFLAGS:-} $PAGE16"
export CARGO_TARGET_I686_LINUX_ANDROID_RUSTFLAGS="${CARGO_TARGET_I686_LINUX_ANDROID_RUSTFLAGS:-} $PAGE16"

echo "==> Building qrk-ffi for Android (API $API_LEVEL, profile $PROFILE)"
echo "    NDK: $ANDROID_NDK_HOME"
echo "    16 KB page flags: -Wl,-z,max-page-size=16384 -Wl,-z,common-page-size=16384"

# cargo-ndk sets the linker; rustflags above force 16 KB LOAD alignment.
cargo ndk \
  -t arm64-v8a \
  -t x86_64 \
  -o "$TMP_OUT" \
  -P "$API_LEVEL" \
  build \
  -p qrk-ffi \
  --profile "$PROFILE"

for abi in arm64-v8a x86_64; do
  src="$TMP_OUT/$abi/libqrk_ffi.so"
  if [[ ! -f "$src" ]]; then
    echo "error: missing $src" >&2
    exit 1
  fi
  dest_dir="$OUT_JNI/$abi"
  mkdir -p "$dest_dir"
  cp -f "$src" "$dest_dir/libqrk_ffi.so"
  echo "    installed $dest_dir/libqrk_ffi.so ($(wc -c < "$dest_dir/libqrk_ffi.so") bytes)"
done

echo "==> Verifying 16 KB ELF LOAD alignment"
"$ROOT/scripts/check-android-16kb.sh"

echo "OK: Android natives ready under bindings/expo-cpu-scanner/android/src/main/jniLibs/"
