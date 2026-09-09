# qr-lab

Umbrella Rust crate for the complete [QR Lab](https://github.com/aukilabs/qr-lab)
QR scanner and reusable computer-vision types.

## Install

Not on crates.io yet. From a git checkout:

```toml
[dependencies]
qr-lab = { git = "https://github.com/aukilabs/qr-lab" }
```

## Example

```rust
use qr_lab::image::Gray8View;
use qr_lab::{Scanner, ScannerConfig};

let pixels = vec![255u8; 64 * 64];
let frame = Gray8View::new(&pixels, 64, 64, 64)?;
let mut scanner = Scanner::new(ScannerConfig::robust_fast());
let result = scanner.scan(&frame);
for detected in result.codes {
    println!("{}", detected.code.payload);
}
```

## Modules

| Path | Crate | Role |
|---|---|---|
| `qr_lab::image` | `qr-lab-image` | Grayscale views and buffers |
| `qr_lab::geometry` | `qr-lab-geometry` | Homographies, lines, sampling |
| `qr_lab::imgproc` | `qr-lab-imgproc` | Threshold, morphology, restoration |
| `qr_lab::qr` | `qr-lab-qr` | Detection, decode, robust ladder |

Scanner types such as `Scanner` and `ScannerConfig` are also re-exported at the
crate root.

Use a focused crate directly when you do not need the QR decoder.

## License

MIT — see the repository [LICENSE](../../LICENSE).
