# Migrating to QRKit

## Existing Rust scanner applications

Existing imports continue to work during the compatibility cycle:

```rust
use qrk_core::{scan, LumaView, ScanOptions};
```

New code should use the umbrella package:

```rust
use qrkit::{scan, LumaView, ScanOptions};
```

The scanner data types and functions are re-exported unchanged. The old
`qrk-core` package is now a facade over `qrkit`.

For a configuration-owning scanner:

```rust
use qrkit::{Scanner, ScannerConfig};

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
qrkit-image = "0.1"
qrkit-imgproc = "0.1"
```

`qrkit-imgproc` does not depend on `rqrr`, `serde`, WASM, JNI, or Expo.

Operators generally have an allocating convenience form and an `*_into` form
using caller-owned output and typed reusable scratch storage. Camera Y planes
can be wrapped directly with `Gray8View::new(data, width, height, stride)`.

## C

Existing `qrk_scan_luma`, `qrk_version`, and `qrk_free_string` symbols are
unchanged. Reusable image operators use versioned configuration structs,
caller-owned output, status codes, and an opaque `QrkOperatorContext`. See
[`../../crates/qrk-ffi/include/qrk.h`](../../crates/qrk-ffi/include/qrk.h).

## WebAssembly

Existing `scan_rgba`, `scan_rgba_robust`, and `WasmScanSession` exports remain.
`WasmImageProcessor` exposes blur estimation, background division, and Van
Cittert restoration with workspace reuse across calls.
