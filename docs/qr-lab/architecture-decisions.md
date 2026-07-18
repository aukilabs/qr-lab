# QR Lab architecture decisions

Status: accepted for the initial workspace implementation (2026-07-10).

## Compatibility and publication

- New packages use the `qr-lab-*` names and remain unpublished workspace crates
  for one stabilization cycle.
- `cargo search qr-lab` and npm package lookups returned no package collision on
  2026-07-10. The unrelated “QRkit” sparse-QR-decomposition research name is a
  naming-discovery consideration before public trademark/package launch, not a
  Rust/npm package conflict.
- `qr-lab-core`, its Rust API, current C symbols, WASM exports, and Expo API remain
  compatibility surfaces during that cycle.
- The existing workspace MSRV of Rust 1.87 is retained.
- Foundational crates require `std` for the first release; `no_std` is deferred
  until a concrete consumer and allocator policy exist.
- The Python distribution is `qr-lab` and imports as `qr_lab`.
  The shorter `qr-lab` distribution is an unrelated package on PyPI, so neither
  the distribution nor import name will shadow it.
- Python wheels use Maturin, PyO3, and rust-numpy. NumPy integration makes
  version-specific CPython wheels preferable to `abi3`: it preserves the
  standard ndarray C API and avoids a second buffer abstraction.

## Image and coordinate model

- The initial reusable image model is 8-bit grayscale.
- Views support padded row strides and zero-copy ROIs.
- Integer coordinates identify pixel centers.
- Public constructors and ROI operations are checked.
- Operators must document their border and output-size behavior.

## Allocation and determinism

- Public operators provide an allocating convenience form and, where useful,
  a caller-owned `*_into` form.
- Operators needing scratch memory expose reusable typed workspaces.
- Extraction work preserves existing deterministic integer behavior before any
  algorithm is changed.

## Binding support tiers

1. Rust exposes all stable reusable modules.
2. The complete scanner remains available through Rust, C, WASM, Android, iOS,
   and Expo.
3. C/WASM initially expose only selected high-value image operators after the
   Rust forms stabilize.
4. Python exposes the same selected operators through two-dimensional `uint8`
   NumPy arrays, plus the complete single-frame and temporal scanners.

## Initial gates

- Scanner recall must not regress on the committed fixture suite.
- Structural extraction should remain within 3% of the baseline mean and p95
  latency on the same host and build profile.
- Large generated evaluation packs remain external and optional.
