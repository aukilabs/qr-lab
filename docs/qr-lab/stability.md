# QR Lab API stability

## Compatibility surfaces

The following remain compatible for the initial QR Lab cycle:

- Public `qr-lab-core` scanner functions and data types.
- C symbols `qrk_version`, `qrk_scan_luma`, and `qrk_free_string`.
- WASM scanner exports and their serialized scanner envelopes.
- Android JNI scanner entry point, iOS static library, Expo package API, and
  existing artifact names.
- The initial `qr_lab` scanner and operator names once the first wheel is
  published; the package remains alpha before that release.

## Stable modular surface

- `qr-lab-image`: grayscale views, mutable views, owned images, sizes, ROIs,
  checked RGBA conversion, and the original luma compatibility names.
- `qr-lab-geometry`: perspective transform, bilinear sampler, border modes,
  points, TLS line fitting, and intersections.
- `qr-lab-imgproc`: documented modules and their public configurations,
  allocating functions, `*_into` functions, and workspaces.
- `qr-lab-qr`: the scanner facade plus scanner types re-exported by `qr-lab`.

## Unstable implementation details

- `qr_lab_imgproc::internal` exists only to preserve the scanner's historical
  byte-exact kernels during migration. It is doc-hidden and not a supported
  consumer API.
- Architecture-specific SIMD modules are private implementation backends.
- Experimental Catmull-Rom upscaling remains internal.

Public changes are expected to follow Cargo semantic-versioning rules and be
checked with `cargo-semver-checks` before publication.

## Semver-checker note for the compatibility facade

`cargo-semver-checks 0.48.0` was run against `bbac779`. It reports externally
defined re-exports from the new facade as “missing,” even with explicit
`#[doc(inline)]` re-exports, although those exact paths compile. Until the tool
can follow dependency re-exports, `crates/qr-lab-core/tests/public_api_compat.rs`
pins every legacy root import and the exhaustive `LumaError` variants as the
authoritative Rust source-compatibility gate. The C compatibility surface is
independently checked from the built library by `scripts/check-qr-lab-abi.sh`.
