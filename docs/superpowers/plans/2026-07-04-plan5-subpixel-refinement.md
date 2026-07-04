# Plan 5: Subpixel Corner Refinement + Source-Resolution Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The pose-estimation payoff: each decoded code's four module-region corners refined to ≤0.1px mean error against fixture ground truth via full-resolution edge-line fitting; the pipeline restructured around decimate-detect/full-res-sample so far codes (IMG_4832) decode at production working resolution; a 3D orbit debug mode showing live corner error.

**Architecture:** A new `scan()` entry point owns both views: it downscales the source luma internally (production NN formula moves into Rust), detects at working resolution, but samples modules and refines corners against the **source-resolution** luma through scale-composed transforms. Refinement (stage 8): for each decoded code, the bit matrix identifies dark border modules; perpendicular gradient profiles at their edge crossings are localized by quadratic peak interpolation (Devernay), each edge gets a gradient-weighted total-least-squares line fit with one outlier-rejection pass, and adjacent lines intersect into `refined_corners` (source px). The debug UI gains mode 1: an orbitable react-three-fiber scene rendering a known QR with a known camera, projecting analytic ground-truth corners and displaying live refined-corner error while you orbit.

**Tech Stack:** Rust (no new required deps); debug-ui adds `three` + `@react-three/fiber` (+ `@react-three/drei` for OrbitControls); `qrcode` becomes a (dev→) feature dep of qrk-wasm for in-browser QR generation.

## Global Constraints

- **Corner definition (user-confirmed):** the four outer corners of the module region (dark-square boundary, quiet zone excluded), TL,TR,BR,BL, reported in **source-image pixels**. Fixture `corners_px` is the ground truth. `DecodedCode` keeps `corners` (working px, sampling-transform-derived) and gains `refined_corners: Option<[[f64; 2]; 4]>` plus `source_scale: f64` (working = source × scale) so consumers can convert.
- **No overfitting (standing directive)** + gate-failure protocol, verbatim from Plans 2–4.
- Pinned refinement constants (consts.rs, provenance required):
  - Edge sampling: probe at 2 sub-positions per dark border module (0.35, 0.70 module fractions — GPU-scanner precedent), middle ~80% of each edge (skip 1.5 modules at each end — corner rounding under blur), cap 64 points/edge.
  - Profile: 7 bilinear samples at 0.5·source-module spacing along the edge normal, centered on the coarse edge position; central-difference gradient; 3-point quadratic peak interpolation (Devernay ~0.05 px/point at high SNR).
  - Fit: gradient-magnitude-weighted TLS (closed-form 2×2 eigenvector); one refit dropping residuals > max(0.15 source-module, 2× median residual); require ≥6 surviving points/edge else keep the unrefined corner pair for that intersection (refined_corners still produced from whatever lines exist — <2 valid lines → None).
- **Accuracy gates:**
  1. Synthetic unit gate: on clean synthetic renders (known homography, no noise), refined corners ≤0.05 px from analytic truth.
  2. Fixture gate (`tests/refine_gate.rs`): per-prefix mean refined-corner error vs `corners_px` — **≤0.10 px mean on near_/rot_/ver_ (nominal)**; far_/tilt45_/combo_/trans_/inv_ thresholds measured on first green run, reviewed by the controller, then **locked in the test with the measured values + a comment** (spec §6 measured-then-locked protocol).
  3. Source-res sampling gate: `IMG_4832.png` at working max-dim 1280 decodes ≥1 code with payload `HTTPS://R8.HR/OU3QBPE14BY` (currently fails — detects 3 triplets, can't sample at ~2px/module; source-res sampling is the fix). All existing decode gates stay green.
- **API restructure is recorded:** `pub fn scan(source: &LumaView, opts: &ScanOptions {max_working_dim: u32 /*0=full*/, refine: bool}) -> Detections` becomes the primary entry; `detect()`/`detect_with` remain (working-res-only, no refinement) for existing tests. `scan_rgba` signature changes to accept SOURCE rgba + max_dim + refine and downscales in Rust — the TS worker stops downscaling for the scan path (display path keeps downscale.ts). Envelope snapshot regenerates; debug-ui updates.
- Perf cleanups folded in (carried items): decode attempt cap becomes a **round budget** (`MAX_DECODE_ROUNDS = 72` = 24 attempts × 3 rounds, same provenance chain); dead timing-adopt branch removed (debug_assert); `BitMatrix::get/set` hoist `words_per_row`. Host perf re-baselined in the wrap task.
- Commits end with the repo's Claude co-author trailer.

## File Structure

```
crates/qrk-core/src/
  scan.rs          scan() orchestration: owned downscale, dual-view plumbing, refine call
  downscale.rs     NN downscale (production formula, moved from TS scan path; tested vs the TS vectors)
  refine.rs        edge probing, Devernay profiles, weighted TLS, intersections
  decode.rs        round budget; sample_module_ink reads SOURCE view via scale-composed transform
  consts.rs        new pinned constants
crates/qrk-core/tests/refine_gate.rs
crates/qrk-wasm/   scan_rgba(source, max_dim, with_trace, refine); feature "qr-gen" (qrcode) for generate_qr
debug-ui/src/
  scanner/{worker,client,types}.ts   source-through plumbing; refined corners
  overlays/layers/refined.ts         refined vs coarse corner overlay (+ per-corner error vs ground truth when available)
  scene3d/                           mode 1: r3f scene, QR plane, orbit, camera-sim knobs, error panel
```

---

### Task 1: `scan()` + Rust-owned downscale + dual-view plumbing

- `downscale.rs`: `pub fn downscale_luma(src: &LumaView, max_dim: u32) -> Option<(Vec<u8>, usize, usize)>` — EXACT production formula (`round(dim·max/maxside)`, floor source indexing, clamp, min 1) — port the tested TS `downscale.ts` semantics; unit tests transcribe the TS test vectors (4×2→2, identity, rounding) so the two implementations are pinned to each other; plus one cross-check test embedding expected bytes for a small asymmetric case.
- `scan.rs`: `ScanOptions`, `scan()` — downscale (or borrow source when no downscale), TileGrid/finders/triplets/decode on working view, refinement (Task 3) on source view when `refine`. `Detections` gains `source_scale: f64` (1.0 when working == source). Trace unchanged this task.
- `scan_rgba(rgba, width, height, max_dim, with_trace, refine)` — Rust-side luma + downscale; wasm shape test updated; worker.ts sends SOURCE rgba + maxDim (scan path TS downscale removed; display path untouched — document that worker traffic grows to source size: acceptable for a dev tool, and the copy-once ownership semantics unchanged). scanWidth/scanHeight in the response now come from Rust.
- Gates: all existing suites green; envelope regenerated; debug-ui updated + 158+ green.

### Task 2: Source-resolution module sampling

- `sample_module_ink` (and the BWB/edge walks that feed dimension estimation? NO — scope: sampling only; detection stays working-res) gains a source-view variant: the module→working transform composes with the working→source scale (a diagonal matrix — add `PerspectiveTransform::scaled(sx, sy)` helper with tests). Thresholds: source pixel → working tile via coordinate division (document the approximation: tile thresholds computed at working res apply to the co-located source pixels).
- decode.rs samples through the source view whenever `source_scale != 1`. OOB accounting in source space.
- **Gate 3** (IMG_4832 @1280 decodes OU3QBPE14BY) lands here, added to decode_gate.rs real-capture section. All prior gates stay green (near-res fixtures: source == working, zero behavior change — assert bit-identical Detections on 3 fixtures as a regression pin).

### Task 3: `refine.rs` — the subpixel stage

- `pub(crate) fn refine_corners(source: &LumaView, code: &DecodedCode, bits: &BitMatrix, working_to_source: f64) -> Option<[[f64; 2]; 4]>` per the pinned constants: border-module dark-selection from `bits` (row 0 / row dim−1 / col 0 / col dim−1; for inverted codes the ink test flips), coarse edge geometry from the code's corners scaled to source px, perpendicular Devernay profiles, weighted TLS + outlier refit, 4 intersections.
- Synthetic unit gate (≤0.05 px, clean renders at 3 poses × 2 versions, supersampled rasterizer from testpaint — needs an antialiased variant: render at 8× and box-reduce inside the test helper, mirroring the Python generator, so edges have real gradients).
- Wire into scan(); `refined_corners` on DecodedCode; trace records per-edge point counts + dropped-outlier counts (compact).

### Task 4: Fixture accuracy gate + refined overlay

- `tests/refine_gate.rs` per Global Constraints gate 2 (measured-then-locked flow: first run prints the per-prefix table; implementer reports; controller locks values by recorded decision).
- Envelope + TS types + `refined.ts` overlay: refined corners as crosshairs, coarse as hollow squares, connecting whiskers; when ground truth present, per-corner error labels (px, source-scale-aware). TimingsPanel row `refine_ns`.

### Task 5: Debug UI mode 1 — 3D orbit scene with live corner error

- deps: three/@react-three/fiber/@react-three/drei (recorded; dev tool only). qrk-wasm feature `qr-gen`: `generate_qr(payload, version?, ecc?) -> {dim, words}` via the qrcode crate (compiled into the debug wasm build only — keep the mobile-relevant default build free of it; verify via a feature-gated cargo check).
- `scene3d/`: mode tab "3D Scene"; a plane textured with a generated QR (canvas-rendered from the bit matrix, configurable payload/version + physical size), OrbitControls camera; per-frame: renderer canvas → rgba → ScannerClient.scan (latest-wins absorbs); overlays reuse the existing registry on a 2D layer over the WebGL canvas.
- **Ground truth**: project the plane's module-region corners through the three.js camera (Vector3.project → px) each frame; error panel shows per-corner |refined − truth| in px with rolling mean/p95 sparkline — the live accuracy meter.
- Camera-sim knobs: resolution (render target size), Gaussian blur (2-pass shader or ctx.filter), sensor noise (post-process), exposure offset — each a slider; document that these are approximations.
- Logic tests: projection math (three camera → px, vs hand values), error aggregation model; scene itself = QA.

### Task 6: Perf cleanups + round budget

- MAX_DECODE_ROUNDS (replaces attempt counting; provenance carried), dead timing-adopt branch → removed with the derivation comment kept, BitMatrix words_per_row hoist, re-baseline host timings (scan_fixture + decode_photo on near_00/multi_07/ver_12_v40/real_1) recorded in the report + README table.

### Task 7: QA + docs + wrap

- Scripted-Chrome QA: mode 1 orbit (error stays sub-px while orbiting at nominal range; grows gracefully at grazing angles), refined overlay on real_2 + tilt45_04, IMG_4832 @1280 decoded badge, knobs sweep (blur ↑ → error ↑ smoothly, no crash), mode switching, regressions on 3 Plan-4 QA items.
- READMEs; plan follow-ups section for anything deferred; ledger wrap.

## Self-review notes

- **Spec coverage:** stage 8 refinement ✓ (Task 3, spec §3.4 constants honored: 0.35/0.70 sub-positions, dark-border-module selection — now exact via the decoded bits, middle-of-edge sampling, weighted fit + intersection), corner-accuracy harness ✓ (Task 4, spec §6 gate 2 incl. measured-then-locked), debug UI mode 1 with live corner error ✓ (Task 5, the spec's headline debug-UI feature), decimate-detect/full-res-sample ✓ (Tasks 1–2, the recorded IMG_4832 follow-up; also implements the spec §3.8 note "detection may run decimated; refinement never does" — and extends it to sampling).
- **Deliberately out:** NEON/threading/device budget (next plan with FFI/Expo — needs hardware), curved/damaged codes, ECI transcoding.
- **Known risks:** worker traffic at source size (24MP photos → 98MB transfers per scan — acceptable dev-tool cost, documented; mobile path unaffected since Rust receives a borrowed Y-plane); refine on real captures has no corner truth (jitter metrics deferred until video captures exist); r3f readback (WebGL canvas → rgba) needs preserveDrawingBuffer or a readPixels-in-frame-loop pattern — implementer investigates, both are standard.
