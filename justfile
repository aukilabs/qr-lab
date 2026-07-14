# QRKit — developer recipes
#
# Common:
#   just ui          # build WASM (release) + install deps + start Vite
#   just wasm        # rebuild WASM only (after Rust changes)
#   just dev         # start Vite only (assumes WASM already built)

set dotenv-load := false
set shell := ["bash", "-euo", "pipefail", "-c"]

# Default: list recipes
default:
    @just --list

# Build crates/qrk-wasm → debug-ui/src/wasm (release, qr-gen for 3D scene)
wasm:
    ./scripts/build-wasm.sh

# Install debug-ui npm dependencies (no-op when node_modules is fresh enough)
ui-install:
    cd debug-ui && npm install

# Start the Vite debug UI (predev checks WASM + regenerates fixture manifest)
dev: ui-install
    cd debug-ui && npm run dev

# Full loop: compile WASM then run the debug UI frontend
# Usage: just ui
#        just ui -- --host   # extra args after `--` go to Vite
ui *args: wasm ui-install
    cd debug-ui && npm run dev -- {{args}}

# Typecheck + production Vite build of the debug UI (needs WASM)
ui-build: wasm ui-install
    cd debug-ui && npm run build

# Unit tests for the debug UI (vitest)
ui-test: ui-install
    cd debug-ui && npm test

# Rust workspace tests (release)
test:
    cargo test --workspace --release

# Domain gold-standard eval (needs /tmp/domain_gold frames; see domain-extract)
domain-eval max_dim="1280":
    cargo run --release -p qrk-bench --bin domain_eval -- \
        --config baseline --config robust-fast --max-dim {{max_dim}} --obs-only

# Fixture matrix bench
bench max_dim="1280":
    cargo run --release -p qrk-bench --bin qrk-bench -- \
        --config baseline --config robust-fast --max-dim {{max_dim}} --quiet

# ── Expo module (expo-cpu-scanner) native binaries ───────────────────
# Prebuilt artifacts land inside the package so app consumers need no Rust.

# Build Android libqrk_ffi.so → bindings/expo-cpu-scanner/android/src/main/jniLibs/
# (arm64-v8a + x86_64, 16 KB page-size link flags, post-check)
expo-android:
    ./scripts/build-native-android.sh

# Verify jniLibs .so ELF LOAD alignment ≥ 16 KB
expo-android-check:
    ./scripts/check-android-16kb.sh

# Build iOS Qrk.xcframework → bindings/expo-cpu-scanner/ios/
expo-ios:
    ./scripts/build-native-ios.sh

# Both platforms
expo-native: expo-android expo-ios

# Host unit tests for the FFI crate (no NDK/Xcode required)
ffi-test:
    cargo test -p qrk-ffi --release

# Check reusable-crate dependency boundaries and supported feature combinations.
qrkit-deps:
    ./scripts/check-qrkit-deps.sh

# Check legacy scanner and reusable operator C ABI exports.
qrkit-abi:
    ./scripts/check-qrkit-abi.sh

# Run the standalone 1280x720 restoration microbenchmark.
qrkit-operator-bench:
    cargo run -p qrkit-imgproc --release --example operator_bench

# Build the aukilabs-qrkit Python wheel into bindings/python/dist/.
python-build:
    ./scripts/build-python.sh

# Build an isolated wheel and run the Python/NumPy integration suite.
python-test:
    ./scripts/check-python.sh

# ── Example Expo app (bindings/expo-cpu-scanner/example) ─────────────
# Needs natives first (`just expo-native`) and a dev build (not Expo Go).

expo-example-install:
    cd bindings/expo-cpu-scanner/example && npm install

expo-example: expo-example-install
    cd bindings/expo-cpu-scanner/example && npm start

expo-example-ios: expo-example-install
    cd bindings/expo-cpu-scanner/example && npx expo run:ios

expo-example-android: expo-example-install
    cd bindings/expo-cpu-scanner/example && npx expo run:android

expo-example-prebuild: expo-example-install
    cd bindings/expo-cpu-scanner/example && npx expo prebuild

# Live host webcam → iOS Simulator camera via serve-sim (macOS 14+).
# Requires: example installed (just expo-example-ios), booted sim, Metro running.
# Default is LIVE webcam. For a static QR fixture instead:
#   just expo-example-sim-camera-file
# Optional webcam name (substring match): just expo-example-sim-camera webcam="FaceTime"
expo-example-sim-camera webcam="":
    #!/usr/bin/env bash
    set -euo pipefail
    BUNDLE_ID="com.aukilabs.cpuscannerexample"
    METRO_URL="exp+expo-cpu-scanner-example://expo-development-client/?url=http%3A%2F%2F127.0.0.1%3A8082"
    npx --yes serve-sim permissions grant camera "$BUNDLE_ID" || true
    # Prefer hot-swap if a helper is already alive (no relaunch).
    if npx --yes serve-sim camera status -q 2>/dev/null | grep -q '"alive":true'; then
      if [[ -n "{{webcam}}" ]]; then
        npx --yes serve-sim camera switch webcam "{{webcam}}"
      else
        npx --yes serve-sim camera switch webcam
      fi
      npx --yes serve-sim camera mirror off
      echo "Hot-swapped to live webcam (mirror off)."
    else
      if [[ -n "{{webcam}}" ]]; then
        npx --yes serve-sim camera "$BUNDLE_ID" --webcam "{{webcam}}" --mirror off
      else
        npx --yes serve-sim camera "$BUNDLE_ID" --webcam --mirror off
      fi
      # Dev-client needs Metro URL after serve-sim relaunches the process.
      sleep 1
      xcrun simctl openurl booted "$METRO_URL" 2>/dev/null || true
      echo "Live webcam injected + app relaunched with dylib."
    fi
    npx --yes serve-sim camera status -q || true

# Static QR image as simulator camera (decode verification), not a live feed.
expo-example-sim-camera-file file="fixtures/near_00.png":
    #!/usr/bin/env bash
    set -euo pipefail
    BUNDLE_ID="com.aukilabs.cpuscannerexample"
    FILE="{{file}}"
    METRO_URL="exp+expo-cpu-scanner-example://expo-development-client/?url=http%3A%2F%2F127.0.0.1%3A8082"
    if [[ ! -f "$FILE" ]]; then
      echo "error: fixture not found: $FILE (run from repo root)" >&2
      exit 1
    fi
    npx --yes serve-sim permissions grant camera "$BUNDLE_ID" || true
    if npx --yes serve-sim camera status -q 2>/dev/null | grep -q '"alive":true'; then
      npx --yes serve-sim camera switch "$FILE"
      npx --yes serve-sim camera mirror off
    else
      npx --yes serve-sim camera "$BUNDLE_ID" --file "$FILE" --mirror off
      sleep 1
      xcrun simctl openurl booted "$METRO_URL" 2>/dev/null || true
    fi
    echo "Static file feed: $FILE (use just expo-example-sim-camera for live webcam)"
    npx --yes serve-sim camera status -q || true
