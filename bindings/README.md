# QRKit bindings

This directory contains the publishable language and framework integrations
built on the QRKit Rust crates:

- [`python/`](python/) — the `aukilabs-qrkit` Maturin/PyO3 project for PyPI.
- [`expo-cpu-scanner/`](expo-cpu-scanner/) — the Expo module, prebuilt Android
  libraries, iOS XCFramework, and example app.

Build artifacts are written into their corresponding binding package:

| Command | Output |
|---|---|
| `just python-build` | `bindings/python/dist/` |
| `just expo-android` | `bindings/expo-cpu-scanner/android/src/main/jniLibs/` |
| `just expo-ios` | `bindings/expo-cpu-scanner/ios/Qrk.xcframework/` |

Run `just python-test`, `just expo-android-check`, and `just ffi-test` for the
binding-specific checks that do not launch an example application.
