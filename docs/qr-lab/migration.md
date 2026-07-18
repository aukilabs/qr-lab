# Migrating to QR Lab

## Project rename (QRKit → QR Lab)

If you depended on the previous package names, update as follows:

| Old | New |
|---|---|
| `qrkit` | `qr-lab` |
| `qrkit-image` / `qrkit_image` | `qr-lab-image` / `qr_lab_image` |
| `qrkit-geometry` / `qrkit_geometry` | `qr-lab-geometry` / `qr_lab_geometry` |
| `qrkit-imgproc` / `qrkit_imgproc` | `qr-lab-imgproc` / `qr_lab_imgproc` |
| `qrkit-qr` / `qrkit_qr` | `qr-lab-qr` / `qr_lab_qr` |
| `qrk-core` / `qrk_core` | `qr-lab-core` / `qr_lab_core` |
| Python `aukilabs-qrkit` / `auki_qrkit` | `qr-lab` / `qr_lab` |

C ABI symbols (`qrk_*`) and the native library name (`libqrk_ffi`) are unchanged.

## Existing Rust scanner applications

Existing imports continue to work during the compatibility cycle:

```rust
use qr_lab_core::{scan, LumaView, ScanOptions};
```

New code should use the umbrella package:

```rust
use qr_lab::{scan, LumaView, ScanOptions};
```

The scanner data types and functions are re-exported unchanged. The old
`qr-lab-core` package is now a facade over `qr-lab`.

For a configuration-owning scanner:

```rust
use qr_lab::{Scanner, ScannerConfig};

let mut scanner = Scanner::new(ScannerConfig::robust_fast());
let result = scanner.scan(&frame);
```

Enable video state using
`ScannerConfig::robust_fast().temporal(SessionConfig::default())`. Call
`Scanner::reset` after a seek, scene cut, or camera-source change.

## Reusable computer-vision consumers

Depend on the smallest crate that provides the required functionality:

```toml
[dependencies]
qr-lab-image = "0.1"
qr-lab-imgproc = "0.1"
```

`qr-lab-imgproc` does not depend on `rqrr`, `serde`, WASM, JNI, or Expo.

Operators generally have an allocating convenience form and an `*_into` form
using caller-owned output and typed reusable scratch storage. Camera Y planes
can be wrapped directly with `Gray8View::new(data, width, height, stride)`.

## C

Existing `qrk_scan_luma`, `qrk_version`, and `qrk_free_string` symbols are
unchanged. Reusable image operators use versioned configuration structs,
caller-owned output, status codes, and an opaque `QrkOperatorContext`. See
[`../../crates/qr-lab-ffi/include/qrk.h`](../../crates/qr-lab-ffi/include/qrk.h).

## WebAssembly

Existing `scan_rgba`, `scan_rgba_robust`, and `WasmScanSession` exports remain.
`WasmImageProcessor` exposes blur estimation, background division, and Van
Cittert restoration with workspace reuse across calls.
