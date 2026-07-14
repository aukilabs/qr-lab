# Changelog

## Unreleased — QRKit modularization

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
