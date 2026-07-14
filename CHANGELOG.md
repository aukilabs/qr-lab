# Changelog

## Unreleased — QRKit modularization

- Added the project contribution guide and MIT license, refreshed the root
  documentation, and aligned package metadata with the repository license.
- Moved the publishable Python/Maturin project to the top-level `python/`
  directory, separate from the Rust library crates.
- Added focused `qrkit-image`, `qrkit-geometry`, `qrkit-imgproc`, and
  `qrkit-qr` packages plus the `qrkit` umbrella facade.
- Preserved `qrk-core` as a compatibility facade.
- Added reusable image views, geometry, thresholding, morphology,
  illumination, sharpening, blur analysis, and deblurring APIs.
- Added caller-owned/workspace native operator APIs and `WasmImageProcessor`.
- Added the `aukilabs-qrkit` Python/NumPy package with stateless and temporal
  scanning, reusable image-processing contexts, type stubs, and Maturin wheel
  generation.
- Preserved the existing scanner C/WASM/mobile surfaces.
