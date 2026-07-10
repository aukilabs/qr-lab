# cpuscanner2

A pure-CPU, Rust QR code scanner (standard QR, versions 1–40, multiple
codes per frame, mirrored codes included) targeting subpixel-accurate
corner output for AR pose estimation, with WASM as a first-class target.
See `docs/superpowers/specs/2026-07-03-rust-qr-scanner-design.md` for the
full design.

## Pipeline status

- **Detection (Plans 1–3): complete.** Tiling/binarization, finder-pattern
  and triplet detection, homography, and the debug UI (image/video sources,
  per-stage overlays, timings panel) all land and gate on the fixture suite
  + real captures.
- **Decode (Plan 4): complete.** Version cross-checks (timing pattern +
  BCH version-info bits, both orientations), Annex E alignment-pattern
  location, piecewise perspective sampling with a single-transform
  fallback, rqrr-based bit-matrix decoding (mirrored-orientation retry
  included), and per-triplet arbitration (dimension/version cross-checks,
  candidate cap) are all wired into `detect()`'s output (`codes`) and
  traced end-to-end in the debug UI (8 overlay layers; timings panel with
  6 stage rows: tiles/finders/triplets/version/alignment/sample+decode).
  Gate: 81/81 synthetic fixtures + 93/93 codes decode correctly, plus both
  real-photo captures' payloads (OpenCV-confirmed).
- **Corner refinement / subpixel accuracy (Plan 5): complete.** Each
  decoded code's four module-region corners are refined against
  full-SOURCE-resolution luma (Devernay edge localization + gradient-
  weighted TLS line fit + intersection) and exposed as
  `DecodedCode::refined_corners` (source px) alongside `source_scale`.
  The pipeline is restructured around `scan()`: detection still runs at a
  capped working resolution, but sampling and refinement read the source
  view through a scale-composed transform, so far/small codes that
  couldn't be sampled at working resolution now decode (`IMG_4832.png`
  @1280 working: 3 triplets detect but 0 decode without this; 1 decodes
  with it). Gates: fixture accuracy gate (`tests/refine_gate.rs`) locked
  at **≤0.10px mean corner error, uniformly across every fixture prefix**
  (near/rot/ver/far/tilt45/combo/trans/inv/invtrans/mirror/multi — the
  controller extended the nominal-prefix bar to all of them once the
  first green run measured every prefix at 0.008-0.039px, well inside
  it); e.g. the `near_00` fixture (a rendered, blurred/noised image, not a
  literal camera photo) goes from a 1.445px coarse mean to a **0.028px**
  refined mean (98% error reduction). A debug-UI "3D Scene" mode (an
  orbitable react-three-fiber scene with a live per-corner error panel)
  is the plan's headline dev-tool feature — see `debug-ui/README.md`'s
  "Mode 1" section for its architecture and QA-measured behavior.

  **Perf** (release build, M-series host; `scan()` incl. refinement —
  see `.superpowers/sdd/task-6-report.md` for the full re-baseline):

  | Target | codes | tiles | finders | sample+decode | refine |
  |---|---|---|---|---|---|
  | `near_00` | 1 | 3.2ms | 6.9ms | 253us | 227us |
  | `multi_07` | 4 | 3.2ms | 6.6ms | 775us | 369us |
  | `ver_12_v40` (v40, worst case) | 1 | 3.1ms | 7.0ms | 19.6ms | 158us |
  | `real_1.png` @1280 | 2 | 1.5ms | 1.7ms | 456us | 153us |
  | `IMG_4832.png` @1280 | 1 | 1.5ms | 2.0ms | 1.2ms | 74us |

  Refinement itself is consistently sub-millisecond (74-370us) even on
  the largest legal QR (v40); `sample+decode` dominates, with v40's dense
  177×177-module grid the clear outlier (19.6ms) — flagged as the
  biggest target for the device/NEON plan, not addressed here.

- **3D-scene controls (Plan 5d): complete, debug-UI only, no Rust
  changes.** The "3D Scene" mode gained a scene-background image (a real
  scene plane, visible in the readback — not a CSS background), QR
  ink/background colors + a background-alpha slider (transparent QR paper
  showing the scene through it, the `trans_`-fixture look), an
  emergent "reads as: normal/inverted" + low-contrast indicator, an
  on-canvas HUD (camSim knobs + live camera distance/incidence/roll,
  overlay-canvas-only — never touches the scanner's own readback buffer),
  and a "Save as fixture" button producing a real `.json`/`.png`/`.luma`
  triple in the `tools/fixtures/generate.py` schema. Verified end-to-end
  in headless Chrome: a captured scene frame's saved `.png` decodes
  correctly via `cargo run --example decode_photo` (correct payload/
  version/ecc, refined corners within ~1px of the exported ground truth),
  and the `.luma` byte-matches a Python-derived luma plane of the same PNG
  pixel-for-pixel. See `debug-ui/README.md`'s "Scene controls" section.

## Layout

- `crates/qrk-core` — the scanner core: tiling/binarization, finder-pattern
  and triplet detection, homography, and decoding (version cross-checks,
  alignment location, grid sampling, rqrr bit-matrix decode, arbitration).
  `rqrr` (plus its small transitive tail) is a required dependency for the
  bit-matrix decode step; `serde` is optional (only pulled in behind the
  `serde` feature) and `js-sys` is only pulled in on the `wasm32` target.
- `crates/qrk-wasm` — `wasm-pack`-built bindings exposing `scan_rgba` to
  the debug UI's Web Worker; built via `scripts/build-wasm.sh` /
  `npm run build:wasm` (from `debug-ui/`).
- `debug-ui/` — a React + Vite web app for visually driving the scanner
  against fixtures, real photos, and dropped images/video, with
  per-stage overlays and a timings panel. See `debug-ui/README.md` for
  setup, architecture, and how to add an overlay layer for a new
  detection stage.
- `fixtures/` — golden QR fixtures (rendered PNG/`.luma` + ground-truth
  JSON triples) plus `fixtures/real/` real-photo captures. Generated by
  `tools/fixtures/generate.py`; regeneration is deterministic (same seed →
  byte-identical output) with the pinned versions in
  `tools/fixtures/requirements.txt`.
- `tools/fixtures/` — the Python fixture generator (pinhole-camera
  rendering with exact ground-truth corners) and its tests. See
  `tools/fixtures/README.md`.
- `crates/qrk-ffi/` — C ABI + Android JNI (`libqrk_ffi`), built into the
  Expo package via `just expo-android` / `just expo-ios`.
- `expo-cpu-scanner/` — Expo module shipping prebuilt Android `.so` and
  iOS `Qrk.xcframework` (consumers do not need Rust). See its README.
- `expo-ark-scanner/` — legacy GPU/WebGPU Expo module kept only as a
  structural reference; not part of the CPU scanner product path.
- `scripts/` — repo-wide build scripts (`build-wasm.sh`, `build-native-*.sh`).
- `justfile` — developer recipes (`just ui`, `just expo-native`, …).
- `docs/superpowers/` — design spec and implementation plans for this
  project, executed plan-by-plan via the superpowers SDD workflow.

## Building

```bash
cargo build --workspace       # crates/qrk-core, qrk-wasm, qrk-ffi, qrk-bench
cargo test --workspace

just ui                       # WASM + debug UI
just expo-android             # libqrk_ffi.so → expo-cpu-scanner jniLibs (16 KB)
just expo-ios                 # Qrk.xcframework → expo-cpu-scanner/ios
just expo-native              # both platforms
just expo-example-ios         # example app (dev client) on iOS
just expo-example-android     # example app on Android
```

For the debug UI, see `debug-ui/README.md`. For the Expo module and its
example app, see `expo-cpu-scanner/README.md` and
`expo-cpu-scanner/example/README.md`.
