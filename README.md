# QR Lab

[![CI](https://github.com/aukilabs/qr-lab/actions/workflows/ci.yml/badge.svg)](https://github.com/aukilabs/qr-lab/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/MSRV-1.87+-orange.svg)](rust-toolchain.toml)
[![Status](https://img.shields.io/badge/status-pre--1.0-yellow.svg)](docs/qr-lab/stability.md)

**QR Lab** is a modular, CPU-only computer-vision toolkit centered on a complete
QR code scanner. It targets real camera frames: versions 1–40, multiple and
mirrored codes, robust recovery for difficult lighting and blur, temporal video
scanning, and subpixel corner refinement for pose estimation.

The core is pure Rust. The same scanner is available from Rust, Python/NumPy, C
(and Android/iOS via that ABI), WebAssembly, and Expo. The image, geometry, and
image-processing crates can be used on their own — no QR decoder required.

> **Status:** pre-1.0. Public APIs may still change. See the
> [API stability policy](docs/qr-lab/stability.md) before depending on QR Lab in
> a public library. Crates.io, PyPI, and npm packages are not published yet;
> install from this repository (below).

## Why QR Lab

| | |
|---|---|
| **CPU-only** | No GPU, no platform Vision frameworks, no camera stack |
| **Camera-first** | Strided Y planes, RGB8 frames, working-resolution caps, multi-code frames |
| **Robust** | Recovery ladder for blur, low resolution, uneven light, polarity flips |
| **Composable** | Image / geometry / imgproc crates usable without the QR pipeline |
| **Portable** | One core, many bindings (Rust, Python, C, WASM, mobile) |

## Features

- Pure-CPU detection and decoding with no GPU or platform vision dependency
- Standard QR versions 1–40, multi-code frames, mirrored codes, inverted polarity
- Robust recovery ladder for blur, low resolution, uneven illumination, and difficult thresholds
- Stateful temporal scanning for video streams
- Source-resolution, subpixel-refined corners in TL/TR/BR/BL order
- Reusable grayscale image views, projective geometry, thresholding, morphology, illumination correction, blur estimation, and restoration
- Native, Python, WebAssembly, and mobile bindings built from the same scanner

## Install

QR Lab is source-first until the first crates.io / PyPI / npm release.

### Rust

```toml
[dependencies]
qr-lab = { git = "https://github.com/aukilabs/qr-lab" }
```

Or depend only on the pieces you need:

```toml
[dependencies]
qr-lab-image = { git = "https://github.com/aukilabs/qr-lab" }
qr-lab-geometry = { git = "https://github.com/aukilabs/qr-lab" }
qr-lab-imgproc = { git = "https://github.com/aukilabs/qr-lab" }
qr-lab-qr = { git = "https://github.com/aukilabs/qr-lab" }
```

A path checkout works the same way: clone the repo and point Cargo at
`crates/qr-lab`.

### Python

Build a wheel from a clone (Maturin, Python 3.9+):

```bash
git clone https://github.com/aukilabs/qr-lab.git
cd qr-lab
just python-build          # → bindings/python/dist/
pip install bindings/python/dist/qr_lab-*.whl
```

```python
import numpy as np
import qr_lab

gray = np.fromfile("frame.luma", dtype=np.uint8).reshape(720, 1280)
result = qr_lab.scan(gray, preset="robust_fast", refine=True)
for code in result["codes"]:
    print(code["payload"], code["corners_source"])
```

See the [Python guide](bindings/python/README.md) for temporal scanning and
image operators.

### C / mobile

Link against `qr-lab-ffi` (`libqrk_ffi`) and include
[`crates/qr-lab-ffi/include/qrk.h`](crates/qr-lab-ffi/include/qrk.h). See the
[C and native guide](crates/qr-lab-ffi/README.md). Exported symbols keep the
short `qrk_*` ABI prefix for binary stability.

## Quick start (Rust)

Requires **Rust 1.87+** (see `rust-toolchain.toml`). [`just`](https://just.systems/)
is optional but recommended for multi-target workflows.

```bash
git clone https://github.com/aukilabs/qr-lab.git
cd qr-lab
cargo test --workspace --release
cargo run --release -p qr-lab --example full_scanner
```

Scan a borrowed 8-bit grayscale frame (including a camera Y plane with row padding):

```rust
use qr_lab::image::Gray8View;
use qr_lab::{Scanner, ScannerConfig};

fn scan_frame(
    pixels: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let frame = Gray8View::new(pixels, width, height, stride)?;
    let mut scanner = Scanner::new(ScannerConfig::robust_fast());
    let result = scanner.scan(&frame);

    for detected in result.codes {
        println!("{}", detected.code.payload);
        println!("source corners: {:?}", detected.corners_source);
    }

    Ok(())
}
```

Packed RGB8 frames use `Scanner::scan_rgb8` with `qr_lab::qr::Rgb8View` (BT.601
luma, allocation reused across frames).

### Crate layout

```text
qr-lab                    umbrella facade and complete scanner
└── qr-lab-qr             QR detection, decoding, robust recovery, sessions
    ├── qr-lab-imgproc    reusable enhancement and restoration
    ├── qr-lab-geometry   transforms, sampling, and line fitting
    └── qr-lab-image      grayscale views, ROIs, and owned buffers

qr-lab-core               compatibility facade (older import path)
qr-lab-ffi / qr-lab-wasm  native/mobile and WebAssembly bindings
bindings/python           PyPI / Maturin project (`import qr_lab`)
```

## Bindings and tools

| Target | Location | Documentation |
|---|---|---|
| Rust | `crates/qr-lab*` | [Developer docs](docs/qr-lab/README.md) |
| Python / NumPy | `bindings/python` | [Python guide](bindings/python/README.md) |
| C, Android JNI, iOS | `crates/qr-lab-ffi` | [C and native guide](crates/qr-lab-ffi/README.md) |
| WebAssembly | `crates/qr-lab-wasm` | Used by the debug UI |
| Expo | `bindings/expo-cpu-scanner` | [Expo module guide](bindings/expo-cpu-scanner/README.md) |
| Browser debug UI | `debug-ui` | [Debug UI guide](debug-ui/README.md) |

Publishable integrations are summarized in the [bindings guide](bindings/README.md).

## Limitations

- **Pre-1.0.** Minor versions may include breaking changes. See
  [stability](docs/qr-lab/stability.md).
- **Reed-Solomon / bitstream decode** is delegated to
  [`rqrr`](https://crates.io/crates/rqrr) (MIT OR Apache-2.0). Finder detection,
  sampling, robust recovery, temporal scanning, and corner refinement are QR Lab.
- **No FNC1 or Structured Append.** ECI payloads are returned as bytes for the
  caller to interpret.
- **No GPU, no `no_std`, no camera capture.** Feed it a grayscale or RGB8 buffer.
- **Packages are not on crates.io / PyPI / npm yet.** Install from git or a
  local checkout.

## Development

```bash
just test                 # Rust workspace tests (release)
just ci                   # format check + workspace tests (what GitHub Actions runs)
just ui                   # build WASM and start the Vite debug UI
just ui-test              # debug UI unit tests
just python-build         # build a wheel into bindings/python/dist
just python-test          # isolated Python/NumPy integration tests
just qr-lab-deps          # crate boundary and feature checks
just qr-lab-abi           # C ABI export verification
just expo-native          # Android and iOS native artifacts
```

Platform notes:

- Debug UI: Node.js 22.12+, npm, and `wasm-pack`
- Python: 3.9+ and Maturin or `uv` / `uvx`
- Android: NDK and `cargo-ndk`
- iOS: macOS with Xcode and the iOS Rust targets

### Fixtures

Golden fixtures under `fixtures/` are **not** committed (large binaries). Generate
them deterministically after clone:

```bash
cd tools/fixtures
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
.venv/bin/python generate.py --out ../../fixtures --seed 7
```

Same seed + pinned `requirements.txt` ⇒ byte-identical `.png` / `.luma` / `.json`.
See the [fixture guide](tools/fixtures/README.md). Real photos under
`fixtures/real/` are local-only. Benchmark methodology lives in
[docs/qr-lab/benchmarks.md](docs/qr-lab/benchmarks.md).

### Documentation

| Document | Contents |
|---|---|
| [docs/qr-lab/architecture-decisions.md](docs/qr-lab/architecture-decisions.md) | Package, coordinates, allocation, bindings |
| [docs/qr-lab/stability.md](docs/qr-lab/stability.md) | What is stable vs internal |
| [docs/qr-lab/migration.md](docs/qr-lab/migration.md) | Migrating from older package names |
| [docs/qr-lab/benchmarks.md](docs/qr-lab/benchmarks.md) | Quality and latency reference results |
| [CHANGELOG.md](CHANGELOG.md) | User-visible changes |

Generate Rust API docs with `cargo doc --workspace --no-deps --open`.

## Repository layout

```text
crates/                 Rust libraries, native/WASM bindings, and benchmarks
bindings/               Publishable Python and Expo bindings
debug-ui/               React/Vite scanner inspection tool
docs/                   architecture, stability, migration, design notes
fixtures/               generated golden fixtures (gitignored)
scripts/                cross-target build and verification scripts
tools/fixtures/         deterministic fixture generator and tests
```

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) for
setup, testing, fixtures, and pull-request guidance.

Security issues should be reported privately — see [SECURITY.md](SECURITY.md).

## Acknowledgments

QR bitstream decoding uses [`rqrr`](https://crates.io/crates/rqrr) by
[WanzenBug](https://github.com/WanzenBug/rqrr) and contributors.

## License

QR Lab is licensed under the [MIT License](LICENSE).

Copyright (c) 2026 [Auki Labs](https://auki.com).
