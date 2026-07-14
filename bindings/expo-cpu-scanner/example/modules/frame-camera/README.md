# frame-camera

Local Expo module used only by the example app. Captures **real-time camera
frames** (Y / luma plane) so they can be fed into other native modules such as
`expo-cpu-scanner`.

| Platform | Stack |
|---|---|
| Android | CameraX Preview + ImageAnalysis (YUV_420_888, Y plane) |
| iOS | AVFoundation preview + `AVCaptureVideoDataOutput` (420f, plane 0) |

## Why this exists

Third-party camera packages (Vision Camera, expo-camera) either own the full
pipeline or don't expose a clean Y-plane stream for *our* scanner. This module
is the thin capture side of the example:

```
Camera hardware → frame-camera → onFrame(luma) → scanLuma (expo-cpu-scanner)
```

## JS API

See the package `src/` and the example `App.tsx`.
