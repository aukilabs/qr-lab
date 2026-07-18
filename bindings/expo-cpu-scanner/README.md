# expo-cpu-scanner

Expo module for the QR Lab pure-CPU QR scanner.

Native binaries are **prebuilt** into this package so app consumers never need
a Rust toolchain:

| Platform | Artifact | Built by |
|---|---|---|
| Android | `android/src/main/jniLibs/{arm64-v8a,x86_64}/libqrk_ffi.so` | `just expo-android` |
| iOS | `ios/Qrk.xcframework` | `just expo-ios` |

Android `.so` files are linked with **16 KB page-size** ELF flags
(`-Wl,-z,max-page-size=16384`) and checked by `just expo-android-check`.

## JS API

```ts
import { scanLuma, destroyScanner } from "expo-cpu-scanner";

const result = scanLuma(lumaBytes, width, height, {
  maxDim: 1280,
  refine: true,
});
// result.codes[].payload, corners (working px), refinedCorners (source px)

destroyScanner(); // no-op today; reserved for session API
```

## Build natives (from repo root)

```bash
just expo-android   # cargo-ndk → jniLibs + 16 KB check
just expo-ios       # staticlibs → Qrk.xcframework
just expo-native    # both
```

Requires: Rust toolchain, Android NDK (`ANDROID_NDK_HOME` or SDK `ndk/`),
`cargo-ndk` for Android, Xcode for iOS.

## Example app

Live camera → local `frame-camera` module (Y plane) → `scanLuma` → corner /
payload overlay + floating stats. **Not Expo Go.**

```bash
just expo-native              # prebuilt .so / xcframework
just expo-example-ios         # npx expo run:ios  (dev client)
# or
just expo-example-android
```

See [`example/README.md`](example/README.md).
## Layout

```
expo-cpu-scanner/
  src/                 TS public API
  android/             Expo module + jniLibs (prebuilt .so)
  ios/                 Expo module + Qrk.xcframework (prebuilt)
  example/             Expo dev-client app that links this package
crates/qr-lab-ffi/        Rust cdylib/staticlib + include/qrk.h
```
