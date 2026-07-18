# QR Lab for Python

NumPy-first Python bindings for [QR Lab](https://github.com/aukilabs/qr-lab)'s
complete QR scanner and selected reusable computer-vision operators.

| | |
|---|---|
| **PyPI name** | `qr-lab` |
| **Import** | `import qr_lab` |
| **Python** | 3.9+ |
| **License** | MIT |

## Install

```bash
pip install qr-lab
```

If the package is not yet published, build a wheel from the monorepo root:

```bash
just python-build   # → bindings/python/dist/
# or, from this directory:
maturin build --release --out dist
```

## Quick start

```python
import numpy as np
import qr_lab

# uint8 grayscale, shape (H, W)
gray = np.fromfile("frame.luma", dtype=np.uint8).reshape(720, 1280)

result = qr_lab.scan(gray, preset="robust_fast", refine=True)
for code in result["codes"]:
    print(code["payload"])
    print(code["corners_source"])  # TL, TR, BR, BL in source pixels

# Temporal video scanning (rung rotation + cross-frame finder pool)
scanner = qr_lab.Scanner(preset="robust_fast", temporal=True, refine=True)
for frame in frames:
    detections = scanner.scan(frame)
scanner.reset()  # after a scene cut or source change
```

### Image operators

```python
processor = qr_lab.ImageProcessor()
estimate = processor.estimate_line_blur(gray)
if estimate["blur_length"] is not None and estimate["confidence"] >= 0.3:
    restored = processor.van_cittert(
        gray,
        estimate["theta_radians"],
        max(3, round(estimate["blur_length"]) | 1),
    )
```

Module-level helpers `background_divide`, `estimate_line_blur`, and `van_cittert`
are also available without a retained processor context.

## API notes

- Inputs must be two-dimensional `uint8` grayscale arrays.
- Non-contiguous arrays are copied to C order by the Python facade.
- Native work runs without holding the GIL; the binding copies the input before
  detaching so another thread cannot race NumPy storage.
- `preset` is `"robust_fast"` (default) or `"robust_full"`.
- Type stubs ship with the wheel (`py.typed` + `*.pyi`).

## Development

From the repository root:

```bash
just python-build   # wheel into bindings/python/dist
just python-test    # isolated install + pytest
```

Or from this directory:

```bash
maturin develop --release
pytest
maturin build --release --out dist
maturin sdist --out dist
```

## License

MIT — see [LICENSE](LICENSE).
