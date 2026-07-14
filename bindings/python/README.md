# Auki QRKit for Python

NumPy-first Python bindings for QRKit's complete QR scanner and selected
reusable computer-vision operators. The distribution is named
`aukilabs-qrkit` and imports as `auki_qrkit` because the unrelated `qrkit`
name is already occupied on PyPI.

> The package is not yet published to PyPI. After its first release, it will
> be installable with:

```bash
pip install aukilabs-qrkit
```

```python
import numpy as np
import auki_qrkit

gray = np.fromfile("frame.luma", dtype=np.uint8).reshape(720, 1280)
result = auki_qrkit.scan(gray, preset="robust_fast", refine=True)

processor = auki_qrkit.ImageProcessor()
estimate = processor.estimate_line_blur(gray)
if estimate["blur_length"] is not None and estimate["confidence"] >= 0.3:
    restored = processor.van_cittert(
        gray,
        estimate["theta_radians"],
        max(3, round(estimate["blur_length"]) | 1),
    )
```

`Scanner(temporal=True)` retains QRKit's rung-rotation and cross-frame finder
pool for a continuous video stream. Call `reset()` after a scene cut or source
change.

Inputs must be two-dimensional `uint8` grayscale arrays. Non-contiguous arrays
are normalized to C order by the Python facade. Native work executes without
holding Python's GIL; the binding copies the input before detaching so another
Python thread cannot race the NumPy storage.

Build a local wheel from the repository root with `just python-build`, or run
the Python integration suite with `just python-test`.

To inspect the artifacts intended for PyPI, run the following from this
directory:

```bash
maturin build --release --out dist
maturin sdist --out dist
```

The Python distribution is licensed under the [MIT License](LICENSE).
