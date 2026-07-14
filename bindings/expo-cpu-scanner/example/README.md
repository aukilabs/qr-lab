# expo-cpu-scanner example — live camera

Dev-client app that runs **qrk** on real camera frames.

```
┌─────────────────────────────┐
│  frame-camera (local module)│  CameraX / AVFoundation
│  PreviewView + Y-plane      │
└─────────────┬───────────────┘
              │ onFrame { luma, width, height, stride }
              ▼
┌─────────────────────────────┐
│  expo-cpu-scanner           │  scanLuma → libqrk_ffi
└─────────────┬───────────────┘
              │ QrScanResult
              ▼
┌─────────────────────────────┐
│  JS overlay + stats panel   │  refined corners, payloads, timings
└─────────────────────────────┘
```

No `expo-camera` and no Vision Camera — frame acquisition is a **local Expo
module** under `modules/frame-camera` so you control the buffer path and can
feed any native scanner (here: `expo-cpu-scanner`).

## Prerequisites

```bash
# repo root — prebuilt scanner natives
just expo-native
```

## Run (iOS Simulator + serve-sim camera)

The iOS Simulator has no hardware camera. [serve-sim](https://github.com/EvanBacon/serve-sim)
injects a synthetic AVFoundation feed (host **webcam** or a **file**) into the
app process so `frame-camera` sees real frames.

```bash
# repo root
just expo-native
just expo-example-ios                 # build + install on booted sim
cd bindings/expo-cpu-scanner/example && npx expo start --port 8082 --dev-client

# LIVE host webcam (default) — what you want for “see the camera”
just expo-example-sim-camera

# Optional: pick a webcam by name substring
just expo-example-sim-camera webcam="FaceTime"

# Optional: static QR image for decode checks (NOT a live camera)
just expo-example-sim-camera-file     # fixtures/near_00.png, mirror off
```

| Recipe | Feed | Use when |
|---|---|---|
| `expo-example-sim-camera` | Host webcam | Live camera through serve-sim |
| `expo-example-sim-camera-file` | PNG/MP4 file | Verify QR decode on a known fixture |

`--mirror off` keeps QR modules unflipped. After inject, the preview should
show the webcam (or file), a floating stats panel, and cyan corner overlays
when a code decodes.

### Android (device / emulator)

```bash
just expo-example-android
```

Physical devices use the real back camera. Emulators need a virtual scene /
webcam configured in the AVD.

**Not Expo Go** — both local modules ship native code.

## `frame-camera` API

```ts
import { FrameCameraView } from "frame-camera";

<FrameCameraView
  style={StyleSheet.absoluteFill}
  targetFps={12}
  active
  onFrame={({ nativeEvent }) => {
    const { luma, width, height, stride } = nativeEvent;
    scanLuma(luma, width, height, { maxDim: 1280, refine: true, stride });
  }}
  onError={({ nativeEvent }) => console.warn(nativeEvent.message)}
/>
```

Native side throttles to `targetFps` and uses `STRATEGY_KEEP_ONLY_LATEST` /
`alwaysDiscardsLateVideoFrames` so the analysis pipeline never queues up.

## Permissions

- iOS: `NSCameraUsageDescription` in `app.json`
- Android: `CAMERA` permission + runtime request in `App.tsx`

## After changing native code

Rebuild the app (`npx expo run:ios` / `run:android`). Metro alone is enough
for pure JS/TS edits.
