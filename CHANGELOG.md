# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
for crates that are published (pre-1.0: minor versions may include breaking
changes; see [docs/qr-lab/stability.md](docs/qr-lab/stability.md)).

## Unreleased

### Changed

- Renamed the project from **QRKit** / `qrkit` to **QR Lab** / `qr-lab`.
  - Rust crates: `qr-lab`, `qr-lab-image`, `qr-lab-geometry`, `qr-lab-imgproc`,
    `qr-lab-qr`, `qr-lab-core`, `qr-lab-ffi`, `qr-lab-wasm`, `qr-lab-bench`
  - Python package/import: `qr-lab` / `qr_lab` (was `aukilabs-qrkit` / `auki_qrkit`)
  - Repository and docs paths updated to match
  - C ABI symbols remain `qrk_*` / `libqrk_ffi` for binary compatibility
- Expanded open-source documentation: root README, CONTRIBUTING, package
  metadata, and rustdoc on public library exports (`#![warn(missing_docs)]` on
  library crates).

### Added

- Project contribution guide and MIT license; package metadata aligned with the
  repository license.
- Publishable Python/Maturin and Expo integrations under `bindings/`.
- Focused `qr-lab-image`, `qr-lab-geometry`, `qr-lab-imgproc`, and `qr-lab-qr`
  packages plus the `qr-lab` umbrella facade.
- Compatibility facade `qr-lab-core` for existing consumers.
- Reusable image views, geometry, thresholding, morphology, illumination,
  sharpening, blur analysis, and deblurring APIs.
- Caller-owned/workspace native operator APIs and `WasmImageProcessor`.
- Python/NumPy package with stateless and temporal scanning, reusable
  image-processing contexts, type stubs, and Maturin wheel generation.

### Preserved

- Existing scanner C / WASM / mobile surfaces (`qrk_*` ABI, Expo package path).
