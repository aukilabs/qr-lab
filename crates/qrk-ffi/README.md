# qrk-ffi

C ABI (+ Android JNI) bindings for `qrk-core`, consumed by `expo-cpu-scanner`.

## C API

See [`include/qrk.h`](include/qrk.h):

| Symbol | Purpose |
|---|---|
| `qrk_version` | static version string |
| `qrk_scan_luma` | scan grayscale frame → heap JSON |
| `qrk_free_string` | free that JSON |

## JSON envelope

```json
{
  "scanWidth": 1280,
  "scanHeight": 720,
  "sourceScale": 1.0,
  "codes": [
    {
      "payload": "hello",
      "payloadBytesB64": "aGVsbG8=",
      "version": 1,
      "ecc": "M",
      "mirrored": false,
      "inverted": false,
      "dimension": 21,
      "corners": [[x,y],[x,y],[x,y],[x,y]],
      "refinedCorners": null
    }
  ],
  "timings": {
    "tilesNs": 0,
    "findersNs": 0,
    "tripletsNs": 0,
    "versionNs": 0,
    "alignmentNs": 0,
    "sampleDecodeNs": 0,
    "refineNs": 0
  }
}
```

- `corners` are TL/TR/BR/BL in **working** pixels.
- `refinedCorners` (when `refine != 0`) are in **source** pixels.
- `sourceScale` is `working / source` (width axis); convert working → source with `working / sourceScale`.

## Android 16 KB pages

Android `.so` builds link with:

```
-Wl,-z,max-page-size=16384
-Wl,-z,common-page-size=16384
```

Configured in the repo `.cargo/config.toml` for `*-linux-android` targets and
enforced by `scripts/check-android-16kb.sh` after `just expo-android`.

## Build

```bash
just expo-android   # → expo-cpu-scanner/android/src/main/jniLibs/{arm64-v8a,x86_64}/libqrk_ffi.so
just expo-ios       # → expo-cpu-scanner/ios/Qrk.xcframework
just expo-native    # both
```
