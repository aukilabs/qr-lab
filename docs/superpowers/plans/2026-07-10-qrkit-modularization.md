# QR Lab modularization plan

Status: implemented on `codex/qr-lab-modularization` (2026-07-10); final
verification evidence is recorded in the repository's QR Lab documentation.

Goal: evolve the repository into QR Lab: a layered Rust computer-vision toolkit
that preserves the complete cross-compiled QR scanner while exposing useful
image-processing components to other pipelines. A barcode scanner, for example,
should be able to use QR Lab's illumination correction or deblurring without
depending on QR detection, decoding, session handling, WASM bindings, or mobile
wrappers.

## 1. Recommended architecture

Use focused library crates beneath a QR-specific scanner and an umbrella facade:

```text
qr-lab                    Umbrella API and complete-scanner entry point
└── qr-lab-qr             QR detection, decoding, robust ladder, and sessions
    ├── qr-lab-imgproc    Enhancement, restoration, and thresholding
    ├── qr-lab-geometry   Transforms, sampling, and geometric fitting
    └── qr-lab-image      Grayscale image views, owned images, ROIs, and buffers

qr-lab-imgproc
├── qr-lab-geometry
└── qr-lab-image

qr-lab-geometry
└── qr-lab-image

qr-lab-ffi  ─────────────> qr-lab
qr-lab-wasm ─────────────> qr-lab
Expo / Android / iOS ───> qr-lab-ffi
qr-lab-core compatibility ─> qr-lab
```

The dependency direction is the key constraint. `qr-lab-imgproc` must not pull
in `rqrr`, QR finder logic, scanner sessions, JSON serialization, WASM, JNI, or
Expo code.

This structure follows the umbrella-plus-focused-crates direction used by
[Kornia-rs](https://github.com/kornia/kornia-rs) and described in its
[2025 architecture paper](https://arxiv.org/abs/2505.12425). QR Lab should keep
the practical, bounded scope seen in Rust's
[`image`](https://docs.rs/image/latest/image/) and
[`imageproc`](https://docs.rs/imageproc/latest/imageproc/) crates rather than
attempting to become a generic tensor or graph-execution framework.

## 2. Proposed crates and responsibilities

### 2.1 `qr-lab-image`

Provide the minimal data foundation shared by every other crate:

- `Gray8View<'a>` for zero-copy, strided grayscale input.
- `Gray8ViewMut<'a>` for caller-owned output.
- `Gray8Image` for convenient owned storage.
- `Size`, `Rect`, and checked ROI/subview operations.
- RGBA/RGB-to-luma adapters.
- Checked constructors returning `Result` rather than panicking.
- An explicit coordinate convention: integer coordinates represent pixel
  centers.

The first public image model should remain grayscale-first. This covers QR and
barcode processing, camera Y planes, documents, thresholding, and deblurring
without prematurely creating a general tensor abstraction.

The current [`LumaView`](../../../crates/qr-lab-core/src/luma.rs) is the starting
point. Its internal subview functionality should become a checked public API.

### 2.2 `qr-lab-geometry`

Expose reusable image geometry and sampling:

- Points, lines, quads, rectangles, and sizes.
- Homography and perspective transforms.
- Bilinear sampling with explicit border behavior.
- Coordinate transforms associated with resizing and cropping.
- Subpixel edge localization.
- Weighted total-least-squares line fitting.
- Line intersection and geometric validation.

The current
[`PerspectiveTransform`](../../../crates/qr-lab-core/src/homography.rs) moves here.
Duplicate bilinear sampling in refinement and QR version sampling should be
replaced by one shared implementation.

Generic edge localization and line fitting should be separated from the
QR-specific logic that chooses which QR edges to probe.

### 2.3 `qr-lab-imgproc`

This is the main reusable computer-vision package.

#### Resize

- Box/area downsampling.
- Bilinear 2x, 3x, and 4x upsampling.
- General destination-size resize.
- Returned or queryable source-to-destination coordinate transforms.

#### Thresholding

- Tile statistics.
- Configurable adaptive and Sauvola-style thresholding.
- Configurable tile size and neighborhood.
- Contrast-floor and threshold-offset controls.

#### Morphology

- Erosion and dilation.
- Opening and closing.
- The existing fast Van Herk/Gil-Werman implementation as a private backend.

#### Illumination

- Background estimation.
- Background division and shadow normalization.
- Explicit denominator floor and output target controls.

#### Sharpening

- A precisely named fixed-binomial unsharp operator matching current behavior.
- A configurable unsharp-mask operation once independently validated.

#### Blur analysis

- Structure-tensor blur direction and confidence estimation.
- Edge-rise blur-length estimation.
- Explicit reporting of requested and rasterized directions.

#### Deblurring

- Directional unsharp masking.
- Line-PSF Van Cittert restoration.
- Configurable blur length, iterations, relaxation, and border behavior.

Most of this functionality already exists privately in
[`enhance.rs`](../../../crates/qr-lab-core/src/enhance.rs). The first extraction
must preserve the algorithms and integer behavior exactly. Algorithm upgrades
belong in later, independently measured work.

Every substantial operator should eventually provide two APIs:

```rust
// Convenient allocating API.
let restored = deblur_line(src, &config)?;

// Pipeline API with caller-owned storage and reusable scratch memory.
deblur_line_into(src, dst, &config, &mut workspace)?;
```

Blur estimation and restoration must remain separate. Consumers may need the
estimate without using QR Lab's restoration, or may want to select a different
restoration method.

### 2.4 `qr-lab-qr`

Keep functionality here when its semantics depend on QR structure:

- 1:1:3:1:1 finder-pattern detection.
- Finder triplet formation.
- QR version and alignment-pattern handling.
- Module-grid sampling.
- QR `BitMatrix` and decoding.
- Reed-Solomon/checksum-driven recovery.
- QR-specific corner refinement and edge-probe selection.
- Robust scan ladder and retry policy.
- Temporal scan session and QR finder pooling.
- QR trace and diagnostic results.

The robust scanner in [`ladder.rs`](../../../crates/qr-lab-core/src/ladder.rs)
should consume general operators but remain QR-specific. Its evidence model,
module-pitch thresholds, ROI policy, checksum oracle, and retry decisions are
not general image-processing APIs.

### 2.5 `qr-lab`

Provide the umbrella package for users who want the complete scanner:

```rust
let mut scanner = qr_lab::qr::Scanner::new(
    qr_lab::qr::ScannerConfig::robust_fast(),
);

let result = scanner.scan(frame)?;
```

It may re-export commonly used modules for convenience:

```rust
use qr_lab::image::Gray8View;
use qr_lab::imgproc::deblur;
```

Consumers concerned with dependency size can depend directly on
`qr-lab-imgproc` or another focused crate.

## 3. Current functionality mapping

| Current area | Destination | Classification |
|---|---|---|
| `luma.rs` | `qr-lab-image` | General |
| `downscale.rs` | `qr-lab-imgproc::resize` | General |
| `homography.rs` | `qr-lab-geometry` | General |
| Tile grid and adaptive binarization | `qr-lab-imgproc::threshold` | General after configuration |
| Morphology and background division | `qr-lab-imgproc` | General |
| Sharpening and blur analysis | `qr-lab-imgproc` | General |
| Directional deblurring | `qr-lab-imgproc` | High-value public functionality |
| Generic edge localization and TLS fitting | `qr-lab-geometry` or `qr-lab-imgproc` | General |
| QR edge-probe selection | `qr-lab-qr` | QR-specific |
| Finder, triplet, version, and alignment logic | `qr-lab-qr` | QR-specific |
| Module sampling, `BitMatrix`, and decoding | `qr-lab-qr` | QR-specific |
| Robust ladder and temporal session | `qr-lab-qr` | QR-specific |
| Scanner trace and timings | `qr-lab-qr` | QR-specific initially |
| NEON implementation details | Private backend modules | Internal |

Experimental Catmull-Rom code and architecture-specific SIMD functions should
remain private until their portability and measurable value are established.

## 4. Public API design rules

All reusable operators should follow the same conventions:

- Offer an ergonomic allocating wrapper and an allocation-free `*_into` form.
- Accept reusable typed workspaces for scratch memory where needed.
- Validate dimensions, strides, ROIs, and configuration values.
- Return typed errors rather than asserting or silently accepting invalid data.
- Make border behavior, output dimensions, and coordinate mappings explicit.
- Preserve deterministic integer/fixed-point behavior where currently promised.
- Avoid hidden angle snapping; expose the direction actually used.
- Avoid describing a fixed implementation as a fully generic operation.
- Keep CPU/SIMD selection internal to the implementation.

Cargo features must be additive, following the
[official Cargo feature guidance](https://doc.rust-lang.org/cargo/reference/features.html).
Mutually exclusive architectures or backends should not be modeled as features.

## 5. Compatibility strategy

The modularization must not require existing scanner consumers to rewrite their
applications.

For at least one compatibility release:

- Preserve `qr_lab_core::scan` and related Rust entry points.
- Preserve C symbols such as `qrk_scan_luma`.
- Preserve WASM `scan_rgba`, `scan_rgba_robust`, and `WasmScanSession`.
- Preserve the Expo package API.
- Preserve Android shared libraries, the iOS XCFramework, and the WASM build.
- Make `qr-lab-core` a compatibility facade over the new implementation.
- Deprecate compatibility names only after repository consumers and bindings
  have migrated.

The new modules should first be introduced beneath the working scanner. Package
renaming and facade cleanup happen after behavior and performance are stable.

## 6. Cross-language module exposure

Use a tiered rollout:

1. Stabilize the modular Rust APIs.
2. Continue exposing the complete scanner on every existing target.
3. Expose selected high-value operators through C and WASM once the Rust API is
   proven: resize, illumination normalization, adaptive thresholding, blur
   estimation, and deblurring.
4. Do not mirror every private Rust function into every language binding.

A future C operator API should use:

- Opaque reusable context or workspace handles.
- Versioned configuration structures.
- Caller-owned input and output buffers.
- Status codes and retrievable error messages.
- No JSON serialization for per-pixel operations.

The existing JSON scanner API remains available for compatibility.

## 7. Migration phases

### Phase 0 — Freeze behavior and decisions

Estimated effort: 1–2 days.

- Record current public Rust, C, WASM, Expo, Android, and iOS interfaces.
- Capture dependency, binary-size, recall, and latency baselines.
- Decide MSRV, publication policy, crate names, and support tiers.
- Check crates.io/npm availability and unrelated uses of the QR Lab name.
- Record coordinate, border, allocation, and error-handling decisions.

No behavior changes are allowed in this phase.

### Phase 1 — Extract image and geometry foundations

Estimated effort: 3–5 days.

- Add `qr-lab-image` and `qr-lab-geometry`.
- Move `LumaView`, homography, geometric primitives, and bilinear sampling.
- Add checked buffer construction and public ROI views.
- Remove duplicate samplers.
- Make the existing scanner consume the new crates.
- Require bit-identical scanner results.

### Phase 2 — Extract current image-processing algorithms

Estimated effort: 5–8 days.

- Add `qr-lab-imgproc`.
- Move resize, morphology, illumination normalization, sharpening, blur
  estimation, and restoration.
- Preserve current integer arithmetic and deterministic results.
- Keep scanner policies and constants that express QR evidence in `qr-lab-core`.
- Initially expose current behavior through narrow, honest operation names.

This is a structural phase, not a deblur redesign.

### Phase 3 — Stabilize public operator APIs

Estimated effort: 5–8 days.

- Add validated configuration types.
- Add allocation-free `*_into` APIs and reusable workspaces.
- Make border modes and output geometry explicit.
- Replace appropriate hidden constants with configuration.
- Add operator-level documentation and examples.
- Benchmark allocating and workspace-based calls.
- Promote only independently validated operations to stable APIs.

Example use from an unrelated barcode pipeline:

```rust
let src = Gray8View::new(bytes, width, height, stride)?;

let estimate =
    qr_lab_imgproc::blur::estimate_line_blur(src, &estimate_config)?;

let mut output = Gray8Image::new(src.size());
let mut workspace = DeblurWorkspace::default();

qr_lab_imgproc::deblur::van_cittert_line_into(
    src,
    output.view_mut(),
    &DeblurConfig::from_estimate(estimate),
    &mut workspace,
)?;

barcode_scanner.decode(output.view());
```

### Phase 4 — Separate QR functionality and add the facade

Estimated effort: 4–6 days.

- Move QR-specific code into `qr-lab-qr`.
- Add a stateful `Scanner` API while preserving stateless calls.
- Add the `qr-lab` umbrella crate.
- Convert `qr-lab-core` into a compatibility facade.
- Verify complete scanner behavior and diagnostics remain compatible.

### Phase 5 — Bindings and cross-compilation

Estimated effort: 5–8 days.

- Point C, WASM, Android, iOS, and Expo scanner bindings at `qr-lab`.
- Add a versioned native operator API for the first selected modules.
- Start with normalization/deblur only if operator benchmarks justify them.
- Verify ARM64, x86-64, WASM, Android, and iOS artifacts.
- Add ABI compatibility checks for existing C symbols.

### Phase 6 — Documentation and release hardening

Estimated effort: 3–5 days.

Provide examples for:

- The complete robust QR scanner.
- Temporal video scanning.
- Zero-copy camera Y-plane input.
- Standalone deblurring.
- A barcode preprocessor using QR Lab enhancement.
- C operator use with a reusable workspace.
- WASM scanner integration.

Add crate documentation, stability classifications, changelogs, and migration
guidance.

The complete effort is approximately 4–6 weeks for one engineer when performed
incrementally with full cross-platform verification.

## 8. Validation gates

Every phase must satisfy the relevant gates:

- Existing scanner outputs remain correct.
- Extraction phases produce bit-identical operator results.
- `qr-lab-imgproc` has no dependency on `rqrr`, QR decoding, WASM, JNI, or Expo.
- A standalone barcode/deblur example compiles without `qr-lab-qr`.
- Scanner recall does not regress on the evaluation corpus.
- Mean and p95 latency remain within an agreed extraction tolerance; start with
  3% until target-specific budgets are established.
- Workspace tests, Clippy, formatting, docs, WASM, Android, and iOS builds pass.
- Existing C symbols pass an ABI compatibility check.
- Public APIs are checked using
  [`cargo-semver-checks`](https://docs.rs/crate/cargo-semver-checks/latest).
- Binary size and dependency budgets detect accidental coupling.

Operator-specific tests should cover:

- Strided buffers and non-zero-origin ROIs.
- 1xN, Nx1, empty, invalid, and overflow-prone dimensions.
- Every public border mode.
- Determinism across supported targets.
- Agreement with a straightforward reference implementation.
- Allocation counts for workspace-based APIs.

Deblur validation must be independent of QR decoding:

- Procedurally generated line-PSF blur cases.
- Direction and blur-length estimation error.
- Edge-rise restoration.
- Image-quality or gradient-restoration measurements.
- Downstream tests using QR, conventional barcodes, and another structured
  marker type.

Use small procedural fixtures in Git and keep large evaluation packs external
and optional. Do not add the previously excluded large degraded fixture pack to
`main`.

## 9. Explicit non-goals for the first QR Lab release

- No general tensor abstraction.
- No GPU compute framework.
- No OpenCV-style operation graph.
- No public SIMD backend API.
- No arbitrary pixel format support in every operation.
- No neural deblur model bundled into the foundational package.
- No simultaneous rewrite of the working QR scanner.

OpenCV's [G-API](https://docs.opencv.org/4.x/d0/d1e/gapi.html) demonstrates the
potential value of graph execution, but also its complexity and API volatility.
Composable functions, explicit buffers, and reusable workspaces are the more
appropriate first target for QR Lab.

## 10. Decisions to approve before implementation

Recommended defaults:

1. Adopt the layered crate structure in this document.
2. Stabilize reusable modules in Rust first.
3. Preserve every current scanner surface throughout migration.
4. Expose only selected high-value operators through C/WASM after validation.
5. Keep the initial public image representation grayscale and strided.
6. Separate blur estimation from restoration.
7. Extract existing algorithms without changing them, then improve techniques
   through isolated and measured follow-up work.
8. Keep large benchmark assets outside the repository.

Before Phase 0 completes, confirm:

- Final crate and package names.
- Whether the modular crates will be published immediately or remain workspace
  packages for one stabilization cycle.
- The MSRV and whether any foundational crate needs `no_std` support.
- Which operators, if any, must be available through C/WASM in the first QR Lab
  release.
- Target-specific performance and binary-size budgets.
