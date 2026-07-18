# Rust CPU QR Scanner — Design

**Date:** 2026-07-03
**Status:** Draft — awaiting user review
**Supersedes:** the earlier C++ Ark-marker port design (deleted); scope was redirected by the user: pure CPU, **Rust**, **standard QR codes all versions (1–40), detection + decoding**, standalone crate + new Expo module, WASM-powered debug UI.

## 1. Goals

- Detect and decode **standard QR codes, versions 1–40**, multiple codes per frame, mirrored codes included.
- **Subpixel-accurate corner output** (0.03–0.1 px class) for pose estimation — the reason this scanner exists; decoded payload is necessary but corners are the product.
- **Hard budget: ≤5 ms/frame** at ~720p scan resolution on a Pixel 7 big core, pure CPU, while an AR renderer runs.
- Standalone Rust crate, Android-first via a new Expo module, iOS-ready core, **WASM as a first-class target**.
- A web **debug UI** (React + react-three-fiber) with per-step visualization overlays and per-stage benchmark timings.

**Non-goals:** Ark-marker compatibility, the `frameId` out-of-band row (dropped), the GPU path (untouched, later deleted by the app), FNC1/Structured-Append (escape hatch noted), curved-surface/extreme-blur robustness (CNN territory), desktop SIMD (aarch64 NEON + WASM only).

## 2. Repository layout

```
cpuscanner2/
  crates/
    qr-lab-core/        pure Rust, no platform deps: detection, sampling, refinement, trace
    qr-lab-ffi/         cdylib+staticlib, C ABI + JNI (jni crate), AHardwareBuffer lock (cfg android)
    qr-lab-wasm/        wasm-bindgen bindings, debug-trace enabled, serde trace export
  debug-ui/          Vite + React + react-three-fiber app (two modes, overlays, timings panel)
  expo-qr-scanner/   new Expo module (Kotlin + Swift shells, prebuilt .so/.xcframework)
  expo-ark-scanner/  existing GPU module — untouched reference
  tools/fixtures/    Python golden-fixture generator (ground-truth renders)
  fixtures/          generated golden set: images + ground-truth JSON (committed)
  docs/superpowers/specs/
```

(Names `qrk-*`/`expo-qr-scanner` are placeholders — bikeshed at review.)

Workspace-level decisions: Rust ≥1.87 (safe NEON intrinsics), `#![forbid(unsafe_code)]` in `qr-lab-core` (unsafe lives only in `qr-lab-ffi`), no linear-algebra dependency (hand-rolled 60-line adjugate perspective transform and 20-line weighted total-least-squares line fit, per zxing-cpp/AprilTag), `glam` optional for ergonomics only.

## 3. Core pipeline (`qr-lab-core`)

Input: `LumaView { data, width, height, stride }` — 8-bit luma, stride-aware, zero-copy. RGBA→luma helper for WASM/tests. Output: `ScanOutcome { detections: Vec<Detection>, diagnostics, trace: Option<Trace> }`.

Stages (each independently traceable):

1. **Tile statistics (dense, SIMD).** One fused pass: 16×16-tile min/max (`vminq_u8`/`vmaxq_u8` or autovectorized `chunks_exact`), 3×3 tile dilation, per-tile threshold `(min+max)/2`, low-contrast tiles marked *skip*. This is the only unconditional full-frame pass.
2. **Finder-pattern search (sparse).** Scan every 2nd–3rd row (a finder is ≥7 modules tall; skip rows inside skip-tiles), binarizing on the fly against tile thresholds; run-length match `1:1:3:1:1` with standard tolerance **in both polarities** (inverted codes — light modules on dark — are first-class, matched per-row at no extra pass cost; polarity is carried through sampling/decode); vertical + diagonal cross-checks; **concentric-ring verification** in multiple directions (zxing-cpp style) to kill false positives early. No stage may assume a white quiet-zone plate — the fixture suite includes transparent-background codes where the quiet zone is scene background. Merge candidates within a module-size-scaled radius; estimate module size per finder from run lengths.
3. **Triplet grouping.** Size-sorted, spatially binned candidate triples with module-size-ratio, leg-ratio, and right-angle filters (zxing-cpp `GenerateNoStartPattern`-style); emit multiple candidates per region and let Reed-Solomon success arbitrate — cheap because decode failures are fast.
4. **Version estimation.** `dimension = round(centerDist/moduleSize) + 7` averaged over both legs, snapped to ≡ 1 (mod 4), snap error kept as confidence. Cross-checks: **v1–6** — count timing-pattern transitions along row/col 6 (none of the incumbent libraries do this; kills ±1-version errors at shallow angles); **v≥7** — read version-info bits (BCH(18,6), ≤3 bit errors) via the provisional transform in both mirror orientations, and **rebuild the transform if the decoded dimension disagrees** (zxing-cpp trick).
5. **Alignment patterns (v≥2).** ISO 18004 Annex E lookup table (not the naive formula — it breaks at v32). Predict each pattern with the parallelogram rule from found neighbors `AP(x−1,y)+AP(x,y−1)−AP(x−1,y−1)`, re-center with a ±2.25-module concentric probe.
6. **Grid sampling.** **Piecewise homography: one perspective transform per alignment-grid tile** (zxing-cpp `GridSampler` ROI approach); module centers sampled at +0.5 against tile thresholds. Single global homography as the v1/fallback path with quirc-style jiggle refinement scored on timing/alignment cells. Produces the bit matrix + a boolean confidence per functional-pattern check.
7. **Decode.** `rqrr` with `default-features = false` (deps: `g2p`, `lru`; license MIT/Apache-2.0 + ISC): implement rqrr's public `BitGrid` trait over our sampled matrix and call its decode — full v1–40, Numeric/Alphanumeric/Byte/Kanji/ECI, Berlekamp–Massey RS. Mirrored codes: retry via rqrr's `MirroredGrid`. Escape hatches, in order: vendor rqrr's `decode.rs` + `version_db.rs` (~2k permissive lines) for zero deps; `rxing::qrcode::decoder::decode_bitmatrix` if FNC1/Structured-Append is ever needed.
8. **Subpixel corner refinement (the pose-critical stage).** AprilTag `refine_edges` upgraded with Devernay localization, applied to the code's four outer boundary lines **at full input resolution** (detection may run decimated; refinement never does):
   - From the sampled geometry, walk N points along each outer edge (middle ~80%, avoiding corner rounding), only where the adjacent module is dark against the quiet zone.
   - At each point: 5–7 bilinear luma samples along the edge normal (±2 px), central-difference gradient, **3-point quadratic peak interpolation** → subpixel crossing (~0.05 px/point).
   - **Weighted total-least-squares line fit** per edge (gradient-magnitude weights, one outlier-rejection refit) — closed-form 2×2 eigenvector, no iteration.
   - Intersect adjacent lines → 4 corners; expected 0.03–0.08 px, ~0.1 ms/code. Degenerate fits (near-parallel, <4 points) fall back to the coarse corners and flag it.
9. **Trace (behind `debug-trace` cargo feature).** When enabled, each stage records artifacts (tile thresholds + skip mask, finder candidates/verified centers with module sizes, triplets, version estimates + confidence, alignment-pattern predicted/found positions, per-tile homographies, sampled bit matrix, edge sample points + fitted lines, coarse/refined corners) and per-stage `ns` timings into a `Trace` struct. Zero code and zero cost when the feature is off (mobile release builds).

### Threading & SIMD policy

Single scan thread, **pinned to the big cluster** via `libc::sched_setaffinity` (bionic has no `pthread_setaffinity_np`); no rayon (no affinity control, steals onto A55 cores, latency-hostile). Stage 1 NEON-vectorized (safe `#[target_feature(enable = "neon")]` fns, no runtime dispatch — NEON is mandatory on aarch64); everything after stage 1 is sparse and scalar. If profiling ever exceeds budget: one persistent pinned helper thread with condvar handoff splitting stage 1–2 by row bands (AprilTag workerpool pattern) — not in v1.

### Budget (720p, Pixel 7 X1 core)

| Stage | Estimate |
|---|---|
| Tile stats (NEON) | 0.2–0.3 ms |
| Finder search (row-skipped, skip-tiled) | 0.5–1.0 ms |
| Verify + group | <0.3 ms |
| Sample + decode | ~0.3 ms/code |
| Subpixel refine | ~0.1 ms/code |
| **Total (1–3 codes)** | **~1.5–3 ms** |

Calibration task: measure zxing-cpp on the same device/frames first — no published ARM numbers exist for any baseline; this validates the budget instead of assuming it.

## 4. FFI & Expo module

- **API (JS):** same shape as today's module: `scanFrame(pointer) → result`, `destroyScanner()`. Result per detection: `corners[4]` (coarse), `improvedCorners[4]` (subpixel), `size` (modules), `version`, `eccLevel`, `payload` (string) + `payloadBytes` (base64), `bits` (row-major), `mirrored`. Envelope: `scanWidth/Height`, `toBufferUv[6]`, `diagnostics { finderCount, earlyOut, markerCount, msByStage? }`. No `id`/`frameId`.
- **`qr-lab-ffi`:** `crate-type = ["cdylib", "staticlib"]`. Core C ABI: `qrk_scan_luma(ptr, w, h, stride, opts) → owned JSON/flatbuffer` + `qrk_free`. Android extras (cfg android): JNI `extern "system"` entry accepting either an `AHardwareBuffer*` (locked via `AHardwareBuffer_lockPlanes`, Y plane only, released before return) or a direct `ByteBuffer` for the ARCore `Image` path. Built with `-Wl,-z,max-page-size=16384` (Google Play 16 KB page requirement).
- **`expo-qr-scanner`:** Expo Modules API Kotlin/Swift shells calling the C ABI. **Prebuilt binaries shipped in the npm package** (cargo-ndk → `jniLibs/arm64-v8a` in CI; iOS later: `aarch64-apple-ios{,-sim}` staticlibs → `xcodebuild -create-xcframework`, cbindgen header). Consumers never need a Rust toolchain. UniFFI/uniffi-bindgen-react-native rejected: pre-production, and the API surface is 2 functions.
- **iOS v1 scope:** core + xcframework build script exist; Swift binding is a fast-follow, not in v1 acceptance.

## 5. Debug UI (`debug-ui/`)

React + react-three-fiber + Vite, running `qr-lab-wasm` (debug-trace build, `serde`-serialized trace via wasm-bindgen).

**Mode 1 — 3D scene:** a QR code (any version/payload, generated in-app) textured on a plane in an orbitable scene (OrbitControls); optional camera-simulation knobs (blur, noise, exposure, resolution). Every frame the canvas is read back (`readPixels` → luma) and scanned; overlays render on a 2D layer registered to the canvas. **Because the QR's world transform and camera are known, the UI projects ground-truth corner positions and displays live subpixel corner error** — real-time accuracy measurement while orbiting, the core dev loop for stage 8.

**Mode 2 — media:** drag-drop image or video; videos step frame-by-frame (`requestVideoFrameCallback` + seek, play/pause/prev/next); same scan + overlays per frame.

**Shared:** every trace stage is an independently **togglable overlay layer** — tile threshold heatmap + skip mask, binarization preview, finder candidates (rejected vs verified), triplets, version estimate + confidence, alignment predicted-vs-found, sampling grid / per-tile homography boundaries, bit matrix, edge sample points + fitted lines, coarse vs refined corners, decoded payload. Side panel: per-stage timings (from the Rust trace, plus wall-clock JS-side), detection count, corner-error stats (mode 1), history sparkline. Note WASM timings are directional, not device-representative — devices get the `msByStage` diagnostics + a bench harness.

## 6. Testing & validation

**Golden fixtures are regenerated from scratch** — the old ark-scanner fixture (single GPU-captured Ark marker, 35 px tolerance) is not trusted and not reused.

0. **Fixture generator (`tools/fixtures/generate.py`):** a simple Python script (segno/qrcode for symbols, numpy + OpenCV `warpPerspective` for rendering) that renders QR codes through a **pinhole camera model** (default: 1280×720, ~65° hFOV → f ≈ 1005 px; configurable) with the code as a physically sized planar object (default 15 cm; configurable). Renders at 8× supersampling then downsamples, with optional Gaussian blur/sensor noise/exposure offsets. Because corners are computed analytically from the camera pose + homography, **ground truth is exact to float precision**. Each fixture = `PNG (+ raw .luma)` + JSON: camera intrinsics, and per code: payload, version, ECC level, mirrored flag, physical size, pose, the 4 outer corner positions (subpixel image px), and module size in px. Deterministic via seed; suite committed under `fixtures/`.
   **Scenario matrix (user-specified):**
   - **far**: 1.5–2 m (≈2.5–4 px/module for a 15 cm v1 code — the small-module stress case)
   - **near**: <1.5 m
   - **arbitrary rotation**: in-plane rotations across the full circle
   - **45° perspective angle**: out-of-plane tilt at ~45°
   - **multi-code**: 1–4 QR codes per frame (mixed versions/sizes/poses)
   - **inverted**: light modules on a dark plate (real-world inverted prints; rqrr fails these today)
   - **transparent background**: no quiet-zone plate — only the modules are painted over the scene background, so module↔background contrast is imperfect and there is no white reference
   - **inverted + transparent** combinations
   plus combinations (e.g. far + 45° + rotated), a version sweep (1–40), and mirrored variants.
1. **Round-trip decode gate:** every fixture's codes must decode to their exact payloads.
2. **Corner-accuracy gate:** detected `improvedCorners` vs ground-truth corners on the fixture suite; assert mean error ≤0.1 px on near/nominal scenarios, with per-scenario thresholds for far/45° (calibrated once measured, then locked); distributions tracked over time (this is the pose-quality gate; runs in CI on host).
3. **Robustness set:** the BoofCV/Abeles public QR dataset as a scored (not gating) benchmark — the published detection-rate yardstick (BoofCV 60.7%, zxing-cpp fastest classical). Target: ≥zxing-cpp detection rate on nominal/rotation/perspective categories.
4. **Property/unit tests:** run-length matcher, BCH version decode, Annex E table, transition counting, transform math, TLS line fit vs synthetic noisy edges, `BitGrid` glue vs rqrr on crafted matrices.
5. **Benchmarks:** criterion on host per stage; on-device bench APK printing `msByStage`; zxing-cpp measured on the same frames as the calibration baseline. Budget gate: ≤5 ms @720p on Pixel 7.
6. **Determinism:** identical input → identical output bytes (single-threaded, fixed iteration order, no fast-math).

## 7. Risks

- **No ARM baselines exist** for any QR library — the 5 ms claim is extrapolated; the zxing-cpp on-device measurement comes first and resizes expectations if needed.
- **Candidate-search cost dominates**, not decode — if finder search blows budget on noisy scenes, the knobs are: stronger skip-tiling, row-skip 3→4, detection at half-res with full-res verification (refinement is always full-res).
- **Blur / sub-2px modules** defeat all classical detectors; out of scope, stated.
- **rqrr gaps:** no FNC1/Structured-Append; ECI transcoding left to caller (we pass bytes + ECI through). rxing escape hatch documented.
- **Version off-by-one at shallow angles** — mitigated by timing-transition counting (v1–6) and version-bit transform rebuild (v≥7).

## 8. Milestones (implementation-plan granularity comes next)

1. Workspace + `qr-lab-core` skeleton + **Python fixture generator + committed golden suite** — tests first.
2. Stages 1–3 (tile stats, finder search, grouping) + trace + WASM build.
3. Debug UI shell + mode 2 (image) + overlay framework — from here on, every stage lands with its overlay.
4. Stages 4–7 (version, alignment, sampling, rqrr decode) — round-trip matrix green.
5. Stage 8 refinement + corner-accuracy harness + debug UI mode 1 (3D scene, live corner error).
6. `qr-lab-ffi` + `expo-qr-scanner` Android + on-device bench vs zxing-cpp baseline.
7. NEON stage 1, affinity, budget gate; polish; iOS build scripts.
