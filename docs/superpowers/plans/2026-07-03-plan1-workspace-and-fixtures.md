# Plan 1: Workspace + Golden Fixture Generator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Rust workspace with the `qrk-core` crate skeleton, and build the Python golden-fixture generator producing QR renders with analytically exact ground-truth corners, committed as the test suite all later plans gate on.

**Architecture:** A Python script (`tools/fixtures/generate.py`) renders QR symbols (segno) on a physically sized plane through a pinhole camera (numpy + OpenCV `warpPerspective` at 8× supersampling → INTER_AREA downsample → blur/noise), writing PNG + raw `.luma` + ground-truth JSON per fixture. A Cargo workspace hosts `qrk-core` with a `LumaView` input type and a test-side fixture loader, proving the Rust↔fixture contract before any detector code exists.

**Tech Stack:** Rust (workspace, `qrk-core`, serde_json as dev-dependency), Python 3.11+ (segno, numpy, opencv-python-headless, pytest).

## Global Constraints

- Rust MSRV **1.87** (`rust-version = "1.87"`); crate names `qrk-core` (later: `qrk-ffi`, `qrk-wasm`).
- `qrk-core` starts with `#![forbid(unsafe_code)]` and **no required runtime dependencies**.
- Fixture generation is **deterministic**: one master `--seed` (default 7) derives per-fixture seeds; regenerating produces byte-identical outputs.
- Camera default: **1280×720, hFOV 65°** → `fx = fy = 640 / tan(32.5°) ≈ 1004.71`, `cx = 639.5`, `cy = 359.5`. Physical code size default **0.15 m** (module region, excluding quiet zone).
- Ground-truth corners = the 4 corners of the **module region** (excluding quiet zone), order **TL, TR, BR, BL in symbol space**, subpixel image coordinates, x right / y down, pixel centers at integer coordinates.
- Scenario matrix (from spec §6): far 1.5–2 m, near <1.5 m, arbitrary in-plane rotation, ~45° perspective tilt, multi-code 1–4/frame, version sweep 1–40, mirrored variants.
- Fixtures are committed under `fixtures/`. PNG for humans, `.luma` (raw w×h bytes, stride = width) for tests.
- Commit after every green task; messages `feat:`/`test:`/`chore:` style, ending with the Claude co-author trailer used in this repo.

---

### Task 1: Cargo workspace + `qrk-core` skeleton with `LumaView`

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `rust-toolchain.toml`
- Create: `crates/qrk-core/Cargo.toml`
- Create: `crates/qrk-core/src/lib.rs`
- Create: `crates/qrk-core/src/luma.rs`

**Interfaces:**
- Produces: `qrk_core::LumaView<'a>` — `LumaView::new(data: &'a [u8], width: usize, height: usize, stride: usize) -> Result<LumaView<'a>, LumaError>`, `fn get(&self, x: usize, y: usize) -> u8`, `fn width(&self) -> usize`, `fn height(&self) -> usize`. All later plans consume this as the scanner input type.

- [ ] **Step 1: Create workspace scaffolding**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/qrk-core"]
```

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
```

`crates/qrk-core/Cargo.toml`:
```toml
[package]
name = "qrk-core"
version = "0.1.0"
edition = "2021"
rust-version = "1.87"
license = "MIT OR Apache-2.0"

[dependencies]

[dev-dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

- [ ] **Step 2: Write the failing tests**

`crates/qrk-core/src/luma.rs` (tests only for now, at the bottom of the file; declare the module in `lib.rs` in step 4):
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_tight_buffer() {
        let data = vec![7u8; 4 * 3];
        let v = LumaView::new(&data, 4, 3, 4).unwrap();
        assert_eq!(v.width(), 4);
        assert_eq!(v.height(), 3);
        assert_eq!(v.get(3, 2), 7);
    }

    #[test]
    fn accepts_padded_stride_and_indexes_through_it() {
        // 3 rows, width 4, stride 6; mark (0, row) with the row index.
        let mut data = vec![0u8; 6 * 2 + 4];
        data[0] = 10;
        data[6] = 11;
        data[12] = 12;
        let v = LumaView::new(&data, 4, 3, 6).unwrap();
        assert_eq!(v.get(0, 0), 10);
        assert_eq!(v.get(0, 1), 11);
        assert_eq!(v.get(0, 2), 12);
    }

    #[test]
    fn rejects_stride_smaller_than_width() {
        assert!(matches!(
            LumaView::new(&[0; 100], 8, 4, 6),
            Err(LumaError::StrideTooSmall)
        ));
    }

    #[test]
    fn rejects_short_buffer() {
        // Needs stride*(h-1)+width = 6*2+4 = 16 bytes; give 15.
        assert!(matches!(
            LumaView::new(&[0; 15], 4, 3, 6),
            Err(LumaError::BufferTooSmall)
        ));
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert!(matches!(
            LumaView::new(&[0; 16], 0, 3, 4),
            Err(LumaError::EmptyDimensions)
        ));
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p qrk-core`
Expected: compile error — `LumaView`/`LumaError` not defined.

- [ ] **Step 4: Implement `LumaView`**

Top of `crates/qrk-core/src/luma.rs`:
```rust
/// Borrowed view over an 8-bit luma (grayscale) image with row stride.
/// The scanner's only input type: zero-copy over camera Y planes.
#[derive(Clone, Copy)]
pub struct LumaView<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LumaError {
    EmptyDimensions,
    StrideTooSmall,
    BufferTooSmall,
}

impl<'a> LumaView<'a> {
    pub fn new(
        data: &'a [u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, LumaError> {
        if width == 0 || height == 0 {
            return Err(LumaError::EmptyDimensions);
        }
        if stride < width {
            return Err(LumaError::StrideTooSmall);
        }
        let needed = stride
            .checked_mul(height - 1)
            .and_then(|n| n.checked_add(width))
            .ok_or(LumaError::BufferTooSmall)?;
        if data.len() < needed {
            return Err(LumaError::BufferTooSmall);
        }
        Ok(Self { data, width, height, stride })
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        debug_assert!(x < self.width && y < self.height);
        self.data[y * self.stride + x]
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn row(&self, y: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + self.width]
    }
}
```

`crates/qrk-core/src/lib.rs`:
```rust
#![forbid(unsafe_code)]

mod luma;

pub use luma::{LumaError, LumaView};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p qrk-core`
Expected: 5 passed.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml rust-toolchain.toml crates/
git commit -m "feat: workspace + qrk-core LumaView input type"
```

---

### Task 2: Fixture tool scaffolding — camera model + pose math (pure numpy, TDD)

**Files:**
- Create: `tools/fixtures/requirements.txt`
- Create: `tools/fixtures/camera.py`
- Create: `tools/fixtures/test_camera.py`
- Create: `tools/fixtures/README.md`

**Interfaces:**
- Produces (consumed by Tasks 3–4):
  - `camera.Intrinsics` dataclass: `fx, fy, cx, cy, width, height`; classmethod `Intrinsics.default() -> Intrinsics` (1280×720, hFOV 65°).
  - `camera.make_pose(distance_m, tilt_deg, tilt_azimuth_deg, inplane_deg, image_point) -> (R: 3x3, t: 3)` — plane pose in camera frame; `image_point` is the pixel the plane center projects to.
  - `camera.project(intr, R, t, pts_plane: (N,2) meters) -> (N,2) pixels` — plane points (X,Y,0) → image.

- [ ] **Step 1: Write environment files**

`tools/fixtures/requirements.txt`:
```
numpy>=1.26
opencv-python-headless>=4.9
segno>=1.6
pytest>=8
```

`tools/fixtures/README.md`:
```markdown
# Golden fixture generator

Renders QR codes through a pinhole camera with exact ground-truth corners.

    cd tools/fixtures
    python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
    .venv/bin/pytest            # unit tests
    .venv/bin/python generate.py --out ../../fixtures --seed 7

Regeneration is deterministic: same seed → byte-identical fixtures.
Ground truth per code: payload, version, ECC, mirrored, pose, and the 4
module-region corners (TL,TR,BR,BL in symbol space) in subpixel image px.
```

- [ ] **Step 2: Write the failing tests**

`tools/fixtures/test_camera.py`:
```python
import numpy as np
import camera


def test_default_intrinsics_matches_65deg_hfov():
    intr = camera.Intrinsics.default()
    assert intr.width == 1280 and intr.height == 720
    # fx = (w/2) / tan(hfov/2)
    assert abs(intr.fx - 640.0 / np.tan(np.radians(32.5))) < 1e-6
    assert intr.fx == intr.fy
    assert intr.cx == 639.5 and intr.cy == 359.5


def test_frontal_pose_projects_center_to_requested_pixel():
    intr = camera.Intrinsics.default()
    R, t = camera.make_pose(
        distance_m=1.0, tilt_deg=0, tilt_azimuth_deg=0,
        inplane_deg=0, image_point=(640.0, 360.0), intr=intr,
    )
    px = camera.project(intr, R, t, np.zeros((1, 2)))
    assert np.allclose(px[0], [640.0, 360.0], atol=1e-9)


def test_frontal_pose_has_expected_scale():
    # At 1m frontal, a 0.15m-wide square centered on axis spans fx*0.15/1.0 px.
    intr = camera.Intrinsics.default()
    R, t = camera.make_pose(1.0, 0, 0, 0, (intr.cx, intr.cy), intr)
    half = 0.075
    pts = np.array([[-half, 0.0], [half, 0.0]])
    px = camera.project(intr, R, t, pts)
    width_px = px[1, 0] - px[0, 0]
    assert abs(width_px - intr.fx * 0.15) < 1e-6


def test_inplane_rotation_rotates_projection():
    intr = camera.Intrinsics.default()
    R, t = camera.make_pose(1.0, 0, 0, 90.0, (intr.cx, intr.cy), intr)
    # +X in plane space should project (approximately) along image -Y or +Y,
    # not along X.
    px = camera.project(intr, R, t, np.array([[0.075, 0.0], [0.0, 0.0]]))
    d = px[0] - px[1]
    assert abs(d[0]) < 1e-6 and abs(d[1]) > 10


def test_tilt_45_foreshortens_one_axis():
    intr = camera.Intrinsics.default()
    R, t = camera.make_pose(1.0, 45.0, 0.0, 0.0, (intr.cx, intr.cy), intr)
    half = 0.075
    x = camera.project(intr, R, t, np.array([[-half, 0], [half, 0]]))
    y = camera.project(intr, R, t, np.array([[0, -half], [0, half]]))
    span_x = np.linalg.norm(x[1] - x[0])
    span_y = np.linalg.norm(y[1] - y[0])
    # Tilt about azimuth 0 = rotation about the plane's X axis: Y foreshortens.
    assert span_y < span_x * 0.85


def test_projected_points_are_in_front_of_camera():
    intr = camera.Intrinsics.default()
    R, t = camera.make_pose(1.5, 45.0, 30.0, 120.0, (400.0, 500.0), intr)
    pts = np.array([[-0.075, -0.075], [0.075, 0.075]])
    _, depths = camera.project(intr, R, t, pts, return_depth=True)
    assert (depths > 0.5).all()
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd tools/fixtures && python3 -m venv .venv && .venv/bin/pip -q install -r requirements.txt && .venv/bin/pytest test_camera.py -q`
Expected: FAIL / collection error — `camera` module missing.

- [ ] **Step 4: Implement `camera.py`**

```python
"""Pinhole camera model and planar pose math for fixture generation.

Coordinate conventions:
- Camera frame: x right, y down, z forward (into the scene).
- Plane (code) frame: x right, y down within the symbol, z = plane normal
  pointing toward the camera side; points on the code are (X, Y, 0).
- Image: pixel centers at integer coordinates; x right, y down.
"""
from dataclasses import dataclass

import numpy as np


@dataclass(frozen=True)
class Intrinsics:
    fx: float
    fy: float
    cx: float
    cy: float
    width: int
    height: int

    @classmethod
    def default(cls) -> "Intrinsics":
        w, h, hfov_deg = 1280, 720, 65.0
        fx = (w / 2.0) / np.tan(np.radians(hfov_deg / 2.0))
        return cls(fx=fx, fy=fx, cx=(w - 1) / 2.0, cy=(h - 1) / 2.0,
                   width=w, height=h)

    def k(self) -> np.ndarray:
        return np.array([
            [self.fx, 0.0, self.cx],
            [0.0, self.fy, self.cy],
            [0.0, 0.0, 1.0],
        ])


def _rot(axis: np.ndarray, deg: float) -> np.ndarray:
    axis = axis / np.linalg.norm(axis)
    a = np.radians(deg)
    c, s = np.cos(a), np.sin(a)
    x, y, z = axis
    return np.array([
        [c + x * x * (1 - c), x * y * (1 - c) - z * s, x * z * (1 - c) + y * s],
        [y * x * (1 - c) + z * s, c + y * y * (1 - c), y * z * (1 - c) - x * s],
        [z * x * (1 - c) - y * s, z * y * (1 - c) + x * s, c + z * z * (1 - c)],
    ])


def make_pose(distance_m, tilt_deg, tilt_azimuth_deg, inplane_deg,
              image_point, intr: Intrinsics):
    """Pose (R, t) of the code plane in the camera frame.

    The plane center sits at `distance_m` along the camera ray through
    `image_point`. Orientation = in-plane spin about the plane normal,
    then an out-of-plane tilt of `tilt_deg` about an in-plane axis chosen
    by `tilt_azimuth_deg` (0 = plane X axis).
    """
    u, v = image_point
    ray = np.array([(u - intr.cx) / intr.fx, (v - intr.cy) / intr.fy, 1.0])
    ray /= np.linalg.norm(ray)
    t = ray * distance_m

    r_inplane = _rot(np.array([0.0, 0.0, 1.0]), inplane_deg)
    tilt_axis = _rot(np.array([0.0, 0.0, 1.0]), tilt_azimuth_deg) @ np.array(
        [1.0, 0.0, 0.0])
    r_tilt = _rot(tilt_axis, tilt_deg)
    # Frontal orientation: plane axes aligned with camera axes (z toward
    # camera is -z of camera; a frontal code has R = I under our convention
    # because plane x/y match camera x/y and content sits at z = t_z).
    r = r_tilt @ r_inplane
    return r, t


def project(intr: Intrinsics, r: np.ndarray, t: np.ndarray,
            pts_plane: np.ndarray, return_depth: bool = False):
    """Project plane points (N,2) in meters to image pixels (N,2)."""
    pts = np.asarray(pts_plane, dtype=np.float64)
    p3 = np.concatenate([pts, np.zeros((len(pts), 1))], axis=1)
    cam = (r @ p3.T).T + t
    depths = cam[:, 2]
    px = (cam[:, :2] / depths[:, None]) * np.array([intr.fx, intr.fy]) + \
        np.array([intr.cx, intr.cy])
    if return_depth:
        return px, depths
    return px
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd tools/fixtures && .venv/bin/pytest test_camera.py -q`
Expected: 6 passed.

- [ ] **Step 6: Commit**

```bash
git add tools/fixtures/requirements.txt tools/fixtures/camera.py tools/fixtures/test_camera.py tools/fixtures/README.md
git commit -m "feat: fixture camera model with tested pose/projection math"
```

---

### Task 3: QR plane rendering with supersampled warp

**Files:**
- Create: `tools/fixtures/render.py`
- Create: `tools/fixtures/test_render.py`

**Interfaces:**
- Consumes: `camera.Intrinsics`, `camera.make_pose`, `camera.project` (Task 2).
- Produces (consumed by Task 4):
  - `render.make_symbol(payload: str, version: int|None, ecc: str, mirrored: bool) -> np.ndarray` — bool module matrix (True = dark), no quiet zone; segno picks version if None.
  - `render.render_code(img: np.ndarray, intr, R, t, modules, physical_size_m, levels) -> np.ndarray` — draws one code into `img` (uint8 h×w, modified in place and returned), plane = modules + 4-module quiet zone.
  - `render.corners_px(intr, R, t, physical_size_m) -> np.ndarray` — (4,2) ground-truth module-region corners TL,TR,BR,BL.
  - `render.Levels` dataclass: `black: int = 25, white: int = 235`.
  - Constants: `render.QUIET_MODULES = 4`, `render.SS = 8` (supersampling factor).

- [ ] **Step 1: Write the failing tests**

`tools/fixtures/test_render.py`:
```python
import numpy as np
import camera
import render


def _frontal(distance=0.8, size=0.15):
    intr = camera.Intrinsics.default()
    R, t = camera.make_pose(distance, 0, 0, 0, (intr.cx, intr.cy), intr)
    return intr, R, t, size


def test_make_symbol_versions_and_mirror():
    m1 = render.make_symbol("hello", version=1, ecc="m", mirrored=False)
    assert m1.shape == (21, 21) and m1.dtype == bool
    m5 = render.make_symbol("hello", version=5, ecc="q", mirrored=False)
    assert m5.shape == (37, 37)
    mm = render.make_symbol("hello", version=1, ecc="m", mirrored=True)
    assert np.array_equal(mm, m1.T)


def test_corners_px_frontal_geometry():
    intr, R, t, size = _frontal(distance=1.0)
    c = render.corners_px(intr, R, t, size)
    assert c.shape == (4, 2)
    # TL/TR share y; TL/BL share x; width = fx * size / distance.
    assert abs(c[0, 1] - c[1, 1]) < 1e-9
    assert abs(c[0, 0] - c[3, 0]) < 1e-9
    assert abs((c[1, 0] - c[0, 0]) - intr.fx * size) < 1e-6


def test_render_code_paints_dark_finder_and_light_quiet_zone():
    intr, R, t, size = _frontal(distance=0.8)
    img = np.full((intr.height, intr.width), 128, np.uint8)
    modules = render.make_symbol("fixture-test", version=2, ecc="m",
                                 mirrored=False)
    render.render_code(img, intr, R, t, modules, size, render.Levels())
    c = render.corners_px(intr, R, t, size)
    n = modules.shape[0]
    module_px = (c[1, 0] - c[0, 0]) / n
    # Center of the TL finder (3.5 modules in from TL corner): dark.
    fx = int(round(c[0, 0] + 3.5 * module_px))
    fy = int(round(c[0, 1] + 3.5 * module_px))
    assert img[fy, fx] < 80
    # 2 modules outside the TL corner (quiet zone): light.
    qx = int(round(c[0, 0] - 2.0 * module_px))
    qy = int(round(c[0, 1] - 2.0 * module_px))
    assert img[qy, qx] > 180
    # Far from the code: untouched background.
    assert img[5, 5] == 128


def test_render_is_antialiased_at_edges():
    # A tilted render must produce intermediate gray values along the
    # module-region border (supersampling evidence).
    intr = camera.Intrinsics.default()
    R, t = camera.make_pose(0.8, 30, 45, 10, (intr.cx, intr.cy), intr)
    img = np.full((intr.height, intr.width), 128, np.uint8)
    modules = render.make_symbol("edge-aa", version=1, ecc="m", mirrored=False)
    render.render_code(img, intr, R, t, modules, 0.15, render.Levels())
    c = render.corners_px(intr, R, t, 0.15)
    x0, x1 = int(c[:, 0].min()) - 4, int(c[:, 0].max()) + 5
    y0, y1 = int(c[:, 1].min()) - 4, int(c[:, 1].max()) + 5
    region = img[y0:y1, x0:x1]
    mid = (region > 60) & (region < 200) & (region != 128)
    assert mid.sum() > 50
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd tools/fixtures && .venv/bin/pytest test_render.py -q`
Expected: FAIL — `render` module missing.

- [ ] **Step 3: Implement `render.py`**

```python
"""Rasterize QR symbols onto the camera image via a supersampled warp."""
from dataclasses import dataclass

import cv2
import numpy as np
import segno

import camera

QUIET_MODULES = 4
SS = 8            # supersampling factor for the warp target
MODULE_SRC_PX = 16  # source bitmap resolution per module


@dataclass(frozen=True)
class Levels:
    black: int = 25
    white: int = 235


def make_symbol(payload, version, ecc, mirrored):
    qr = segno.make_qr(payload, version=version, error=ecc, boost_error=False)
    m = np.array([[bool(b) for b in row] for row in qr.matrix], dtype=bool)
    if mirrored:
        m = m.T
    return m


def _plane_corners_m(physical_size_m, with_quiet, n_modules):
    """Corners TL,TR,BR,BL of module region (or incl. quiet zone) in meters,
    centered on the module region's center."""
    half = physical_size_m / 2.0
    if with_quiet:
        half += QUIET_MODULES * physical_size_m / n_modules
    return np.array([
        [-half, -half], [half, -half], [half, half], [-half, half],
    ])


def corners_px(intr, r, t, physical_size_m):
    """Ground-truth module-region corners TL,TR,BR,BL in image px."""
    # n_modules irrelevant when with_quiet is False.
    pts = _plane_corners_m(physical_size_m, with_quiet=False, n_modules=1)
    return camera.project(intr, r, t, pts)


def render_code(img, intr, r, t, modules, physical_size_m, levels):
    """Draw one code (with quiet zone) into img (uint8 h×w), in place."""
    n = modules.shape[0]
    # Source bitmap: quiet zone + modules at MODULE_SRC_PX per module.
    total = n + 2 * QUIET_MODULES
    src = np.full((total * MODULE_SRC_PX, total * MODULE_SRC_PX),
                  levels.white, np.uint8)
    dark = np.kron(modules, np.ones((MODULE_SRC_PX, MODULE_SRC_PX), bool))
    q = QUIET_MODULES * MODULE_SRC_PX
    block = src[q:q + n * MODULE_SRC_PX, q:q + n * MODULE_SRC_PX]
    block[dark] = levels.black

    # Homography: source bitmap px -> supersampled image px, exact for a
    # plane, from the 4 quiet-zone corner correspondences.
    src_corners = np.array([
        [-0.5, -0.5],
        [src.shape[1] - 0.5, -0.5],
        [src.shape[1] - 0.5, src.shape[0] - 0.5],
        [-0.5, src.shape[0] - 0.5],
    ], dtype=np.float32)
    plane = _plane_corners_m(physical_size_m, with_quiet=True, n_modules=n)
    dst = camera.project(intr, r, t, plane)
    # Image px -> supersampled px: x_ss = (x + 0.5) * SS - 0.5.
    dst_ss = ((dst + 0.5) * SS - 0.5).astype(np.float32)
    h_mat = cv2.getPerspectiveTransform(src_corners, dst_ss)

    ss_size = (img.shape[1] * SS, img.shape[0] * SS)
    warped = cv2.warpPerspective(
        src, h_mat, ss_size, flags=cv2.INTER_LINEAR,
        borderMode=cv2.BORDER_CONSTANT, borderValue=255)
    mask = cv2.warpPerspective(
        np.full(src.shape, 255, np.uint8), h_mat, ss_size,
        flags=cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT, borderValue=0)

    small = cv2.resize(warped, (img.shape[1], img.shape[0]),
                       interpolation=cv2.INTER_AREA)
    alpha = cv2.resize(mask, (img.shape[1], img.shape[0]),
                       interpolation=cv2.INTER_AREA).astype(np.float32) / 255.0
    out = img.astype(np.float32) * (1 - alpha) + small.astype(np.float32) * alpha
    img[:] = np.clip(np.rint(out), 0, 255).astype(np.uint8)
    return img
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd tools/fixtures && .venv/bin/pytest test_render.py -q`
Expected: 4 passed.

- [ ] **Step 5: Run the full fixture test suite (camera + render together)**

Run: `cd tools/fixtures && .venv/bin/pytest -q`
Expected: 10 passed.

- [ ] **Step 6: Commit**

```bash
git add tools/fixtures/render.py tools/fixtures/test_render.py
git commit -m "feat: supersampled QR plane renderer with exact homography"
```

---

### Task 4: Scenario matrix, ground-truth JSON, and `generate.py` CLI

**Files:**
- Create: `tools/fixtures/scenarios.py`
- Create: `tools/fixtures/test_scenarios.py`
- Create: `tools/fixtures/generate.py`

**Interfaces:**
- Consumes: `camera.*`, `render.*` (Tasks 2–3).
- Produces:
  - `scenarios.build_all(seed: int) -> list[FixtureSpec]` — the full deterministic scenario matrix.
  - `scenarios.FixtureSpec` dataclass: `name: str`, `codes: list[CodeSpec]`, `blur_sigma: float`, `noise_sigma: float`, `seed: int`.
  - `scenarios.CodeSpec` dataclass: `payload, version, ecc, mirrored, physical_size_m, distance_m, tilt_deg, tilt_azimuth_deg, inplane_deg, image_point`.
  - `generate.py` CLI: `--out DIR --seed N [--only PREFIX]`; writes `<name>.png`, `<name>.luma`, `<name>.json` per fixture.
  - **Ground-truth JSON schema** (consumed by Task 5's Rust loader — field names are a contract):
    ```json
    {
      "name": "far_00",
      "width": 1280, "height": 720,
      "camera": {"fx": 1004.71, "fy": 1004.71, "cx": 639.5, "cy": 359.5},
      "blur_sigma": 0.8, "noise_sigma": 2.0, "seed": 12345,
      "codes": [{
        "payload": "Q:far_00:0", "version": 1, "ecc": "m",
        "mirrored": false, "physical_size_m": 0.15, "distance_m": 1.8,
        "tilt_deg": 10.0, "tilt_azimuth_deg": 200.0, "inplane_deg": 74.0,
        "module_size_px": 3.9,
        "corners_px": [[x,y],[x,y],[x,y],[x,y]]
      }]
    }
    ```

- [ ] **Step 1: Write the failing tests**

`tools/fixtures/test_scenarios.py`:
```python
import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import scenarios


def test_matrix_covers_spec_scenarios():
    specs = scenarios.build_all(seed=7)
    names = [s.name for s in specs]
    for prefix, minimum in [("far_", 8), ("near_", 8), ("rot_", 12),
                            ("tilt45_", 8), ("multi_", 8), ("ver_", 13),
                            ("mirror_", 4)]:
        assert sum(n.startswith(prefix) for n in names) >= minimum, prefix


def test_matrix_is_deterministic():
    a = scenarios.build_all(seed=7)
    b = scenarios.build_all(seed=7)
    assert a == b
    c = scenarios.build_all(seed=8)
    assert a != c


def test_far_and_near_distances_respect_spec():
    specs = scenarios.build_all(seed=7)
    for s in specs:
        for code in s.codes:
            if s.name.startswith("far_"):
                assert 1.5 <= code.distance_m <= 2.0
            if s.name.startswith("near_"):
                assert code.distance_m < 1.5


def test_multi_has_1_to_4_codes_and_unique_payloads():
    specs = [s for s in scenarios.build_all(seed=7)
             if s.name.startswith("multi_")]
    counts = {len(s.codes) for s in specs}
    assert counts == {1, 2, 3, 4}
    for s in specs:
        payloads = [c.payload for c in s.codes]
        assert len(set(payloads)) == len(payloads)


def test_version_sweep_scales_physical_size_for_module_px():
    specs = [s for s in scenarios.build_all(seed=7)
             if s.name.startswith("ver_")]
    versions = sorted(c.version for s in specs for c in s.codes)
    assert versions[0] == 1 and versions[-1] == 40


def test_generate_cli_writes_fixture_triplet(tmp_path):
    subprocess.run(
        [sys.executable, "generate.py", "--out", str(tmp_path),
         "--seed", "7", "--only", "near_00"],
        check=True, cwd=Path(__file__).parent)
    meta = json.loads((tmp_path / "near_00.json").read_text())
    assert meta["width"] == 1280 and meta["height"] == 720
    luma = (tmp_path / "near_00.luma").read_bytes()
    assert len(luma) == 1280 * 720
    img = np.frombuffer(luma, np.uint8).reshape(720, 1280)
    for code in meta["codes"]:
        c = np.array(code["corners_px"])
        assert (c[:, 0] > 0).all() and (c[:, 0] < 1279).all()
        assert (c[:, 1] > 0).all() and (c[:, 1] < 719).all()
        # Dark ink just inside the TL corner (0.5*module Euclidean along the
        # diagonal = 0.35 modules per axis, inside the finder's dark outer
        # ring), light quiet zone just outside.
        n = {1: 21}.get(code["version"], code["version"] * 4 + 17)
        mod = code["module_size_px"]
        tl = c[0]
        inward = (c[2] - c[0]) / np.linalg.norm(c[2] - c[0])
        pin = (tl + inward * mod * 0.5).astype(int)
        pout = (tl - inward * mod * 0.5).astype(int)
        assert img[pin[1], pin[0]] < 110
        assert img[pout[1], pout[0]] > 150


def test_generate_cli_is_byte_deterministic(tmp_path):
    for d in ("a", "b"):
        subprocess.run(
            [sys.executable, "generate.py", "--out", str(tmp_path / d),
             "--seed", "7", "--only", "rot_00"],
            check=True, cwd=Path(__file__).parent)
    for ext in (".png", ".luma", ".json"):
        fa = (tmp_path / "a" / f"rot_00{ext}").read_bytes()
        fb = (tmp_path / "b" / f"rot_00{ext}").read_bytes()
        assert fa == fb, ext
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd tools/fixtures && .venv/bin/pytest test_scenarios.py -q`
Expected: FAIL — `scenarios` module missing.

- [ ] **Step 3: Implement `scenarios.py`**

```python
"""Deterministic scenario matrix for the golden fixture suite (spec §6)."""
import zlib
from dataclasses import dataclass

import numpy as np

import camera

MARGIN_PX = 40  # min distance of any corner from the image border


@dataclass(frozen=True)
class CodeSpec:
    payload: str
    version: int
    ecc: str
    mirrored: bool
    physical_size_m: float
    distance_m: float
    tilt_deg: float
    tilt_azimuth_deg: float
    inplane_deg: float
    image_point: tuple


@dataclass(frozen=True)
class FixtureSpec:
    name: str
    codes: tuple
    blur_sigma: float
    noise_sigma: float
    seed: int


def _rng_for(master_seed, name):
    # crc32, not hash(): Python str hashing is salted per process and would
    # break cross-run determinism.
    return np.random.default_rng([master_seed, zlib.crc32(name.encode())])


def _fits(intr, code: CodeSpec) -> bool:
    import render
    r, t = camera.make_pose(code.distance_m, code.tilt_deg,
                            code.tilt_azimuth_deg, code.inplane_deg,
                            code.image_point, intr)
    quiet = render._plane_corners_m(
        code.physical_size_m, with_quiet=True,
        n_modules=code.version * 4 + 17)
    px, depth = camera.project(intr, r, t, quiet, return_depth=True)
    return bool(
        (depth > 0.1).all()
        and (px[:, 0] > MARGIN_PX).all()
        and (px[:, 0] < intr.width - MARGIN_PX).all()
        and (px[:, 1] > MARGIN_PX).all()
        and (px[:, 1] < intr.height - MARGIN_PX).all()
    )


def _sample_code(rng, intr, name, idx, *, version=1, ecc="m", mirrored=False,
                 size=0.15, dist_range=(0.5, 1.4), tilt_range=(0.0, 20.0),
                 inplane=None, tilt=None):
    for _ in range(200):
        code = CodeSpec(
            # "Q:" not "QRK:": tilt45_XX/mirror_XX names must keep the
            # payload within version-1-M byte capacity (14).
            payload=f"Q:{name}:{idx}",
            version=version, ecc=ecc, mirrored=mirrored,
            physical_size_m=size,
            distance_m=float(rng.uniform(*dist_range)),
            tilt_deg=float(rng.uniform(*tilt_range)) if tilt is None else tilt,
            tilt_azimuth_deg=float(rng.uniform(0, 360)),
            inplane_deg=(float(rng.uniform(0, 360))
                         if inplane is None else inplane),
            image_point=(float(rng.uniform(250, intr.width - 250)),
                         float(rng.uniform(180, intr.height - 180))),
        )
        if _fits(intr, code):
            return code
    raise RuntimeError(f"could not place code for {name}")


def _overlaps(intr, a: CodeSpec, b: CodeSpec) -> bool:
    import render

    def bbox(c):
        r, t = camera.make_pose(c.distance_m, c.tilt_deg, c.tilt_azimuth_deg,
                                c.inplane_deg, c.image_point, intr)
        quiet = render._plane_corners_m(
            c.physical_size_m, True, c.version * 4 + 17)
        px = camera.project(intr, r, t, quiet)
        return px[:, 0].min(), px[:, 1].min(), px[:, 0].max(), px[:, 1].max()

    ax0, ay0, ax1, ay1 = bbox(a)
    bx0, by0, bx1, by1 = bbox(b)
    return not (ax1 < bx0 or bx1 < ax0 or ay1 < by0 or by1 < ay0)


def build_all(seed: int):
    intr = camera.Intrinsics.default()
    specs = []

    def add(name, codes, blur, noise):
        specs.append(FixtureSpec(
            name=name, codes=tuple(codes), blur_sigma=blur,
            noise_sigma=noise,
            seed=int(_rng_for(seed, name).integers(0, 2**31))))

    for i in range(8):  # far: 1.5-2.0 m
        rng = _rng_for(seed, f"far_{i:02d}")
        add(f"far_{i:02d}",
            [_sample_code(rng, intr, f"far_{i:02d}", 0,
                          dist_range=(1.5, 2.0))], 0.8, 2.0)

    for i in range(8):  # near: < 1.5 m
        rng = _rng_for(seed, f"near_{i:02d}")
        add(f"near_{i:02d}",
            [_sample_code(rng, intr, f"near_{i:02d}", 0,
                          dist_range=(0.4, 1.45))], 0.6, 2.0)

    for i in range(12):  # in-plane rotation sweep, 30° steps
        rng = _rng_for(seed, f"rot_{i:02d}")
        add(f"rot_{i:02d}",
            [_sample_code(rng, intr, f"rot_{i:02d}", 0,
                          inplane=i * 30.0, tilt=0.0,
                          dist_range=(0.6, 1.2))], 0.6, 2.0)

    for i in range(8):  # 45° perspective tilt, azimuth swept
        rng = _rng_for(seed, f"tilt45_{i:02d}")
        add(f"tilt45_{i:02d}",
            [_sample_code(rng, intr, f"tilt45_{i:02d}", 0, tilt=45.0,
                          dist_range=(0.5, 1.1))], 0.6, 2.0)

    for i in range(8):  # multi: 1-4 codes/frame, cycling count
        name = f"multi_{i:02d}"
        rng = _rng_for(seed, name)
        want = (i % 4) + 1
        codes = []
        for k in range(want):
            for _ in range(200):
                c = _sample_code(rng, intr, name, k, size=0.10,
                                 dist_range=(0.7, 1.4),
                                 version=int(rng.choice([1, 2, 3])))
                if not any(_overlaps(intr, c, o) for o in codes):
                    codes.append(c)
                    break
            else:
                raise RuntimeError(f"could not place {want} codes in {name}")
        add(name, codes, 0.6, 2.0)

    versions = [1, 2, 3, 4, 5, 7, 10, 15, 20, 25, 30, 35, 40]
    for i, v in enumerate(versions):  # version sweep, ~5 px/module frontal
        name = f"ver_{i:02d}_v{v}"
        rng = _rng_for(seed, name)
        n = v * 4 + 17
        dist = 0.9
        # 5 px/module target, shrunk via _fits until the quiet-zone corners
        # fit in-frame — v30+ cannot reach 5 px/module in 720p, especially
        # under random in-plane rotation (see the shrink loop in the code:
        # while not _fits(intr, code): size *= 0.9, capped at 100 iters).
        size = 5.0 * n * dist / intr.fx  # target ~5 px/module
        payload = f"QRK:{name}:" + "x" * max(0, (v * v) // 2)
        add(name, [CodeSpec(
            payload=payload, version=v,
            ecc=["l", "m", "q", "h"][i % 4], mirrored=False,
            physical_size_m=size, distance_m=dist,
            tilt_deg=float(rng.uniform(0, 10)),
            tilt_azimuth_deg=float(rng.uniform(0, 360)),
            inplane_deg=float(rng.uniform(0, 360)),
            image_point=(intr.cx, intr.cy))], 0.6, 2.0)

    for i in range(4):  # mirrored
        rng = _rng_for(seed, f"mirror_{i:02d}")
        add(f"mirror_{i:02d}",
            [_sample_code(rng, intr, f"mirror_{i:02d}", 0, mirrored=True,
                          dist_range=(0.6, 1.2))], 0.6, 2.0)

    # combo: far + 45° + rotated
    for i in range(4):
        rng = _rng_for(seed, f"combo_{i:02d}")
        add(f"combo_{i:02d}",
            [_sample_code(rng, intr, f"combo_{i:02d}", 0, tilt=45.0,
                          dist_range=(1.5, 1.8), size=0.18)], 0.8, 2.5)

    return specs
```

- [ ] **Step 4: Implement `generate.py`**

```python
"""Generate the golden fixture suite. See README.md."""
import argparse
import json
from pathlib import Path

import cv2
import numpy as np

import camera
import render
import scenarios


def render_fixture(spec, intr):
    rng = np.random.default_rng(spec.seed)
    img = np.full((intr.height, intr.width), 128, np.uint8)
    truths = []
    for code in spec.codes:
        r, t = camera.make_pose(code.distance_m, code.tilt_deg,
                                code.tilt_azimuth_deg, code.inplane_deg,
                                code.image_point, intr)
        modules = render.make_symbol(code.payload, code.version, code.ecc,
                                     code.mirrored)
        render.render_code(img, intr, r, t, modules, code.physical_size_m,
                           render.Levels())
        corners = render.corners_px(intr, r, t, code.physical_size_m)
        n = modules.shape[0]
        module_px = float(np.linalg.norm(corners[1] - corners[0]) / n)
        truths.append({
            "payload": code.payload, "version": code.version,
            "ecc": code.ecc, "mirrored": code.mirrored,
            "physical_size_m": code.physical_size_m,
            "distance_m": code.distance_m, "tilt_deg": code.tilt_deg,
            "tilt_azimuth_deg": code.tilt_azimuth_deg,
            "inplane_deg": code.inplane_deg,
            "module_size_px": module_px,
            "corners_px": [[float(x), float(y)] for x, y in corners],
        })

    if spec.blur_sigma > 0:
        img = cv2.GaussianBlur(img, (0, 0), spec.blur_sigma)
    if spec.noise_sigma > 0:
        noise = rng.normal(0, spec.noise_sigma, img.shape)
        img = np.clip(img.astype(np.float32) + noise, 0, 255).astype(np.uint8)

    meta = {
        "name": spec.name, "width": intr.width, "height": intr.height,
        "camera": {"fx": intr.fx, "fy": intr.fy, "cx": intr.cx, "cy": intr.cy},
        "blur_sigma": spec.blur_sigma, "noise_sigma": spec.noise_sigma,
        "seed": spec.seed, "codes": truths,
    }
    return img, meta


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--only", default=None,
                    help="only fixtures whose name starts with this prefix")
    args = ap.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    intr = camera.Intrinsics.default()
    specs = scenarios.build_all(args.seed)
    if args.only:
        specs = [s for s in specs if s.name.startswith(args.only)]
    for spec in specs:
        img, meta = render_fixture(spec, intr)
        ok, png = cv2.imencode(".png", img)
        assert ok
        (out / f"{spec.name}.png").write_bytes(png.tobytes())
        (out / f"{spec.name}.luma").write_bytes(img.tobytes())
        (out / f"{spec.name}.json").write_text(
            json.dumps(meta, indent=1, sort_keys=True) + "\n")
        print(f"wrote {spec.name} ({len(meta['codes'])} codes)")


if __name__ == "__main__":
    main()
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd tools/fixtures && .venv/bin/pytest -q`
Expected: all passed (camera 6, render 4, scenarios 7).

- [ ] **Step 6: Commit**

```bash
git add tools/fixtures/scenarios.py tools/fixtures/test_scenarios.py tools/fixtures/generate.py
git commit -m "feat: scenario matrix and fixture generator CLI"
```

---

### Task 5: Generate and commit the golden suite; decode sanity-check

**Files:**
- Create: `fixtures/*.png`, `fixtures/*.luma`, `fixtures/*.json` (generated)
- Create: `tools/fixtures/test_suite_sanity.py`

**Interfaces:**
- Consumes: `generate.py` CLI (Task 4).
- Produces: the committed golden suite under `fixtures/` (≥65 fixtures) — the contract for every later plan's gates.

- [ ] **Step 1: Write the failing sanity test**

An independent decoder must read our renders — this catches generator bugs ground-truth math can't (wrong module orientation, inverted colors, broken quiet zone). Uses OpenCV's QR decoder on the *easy* subset only (it is far weaker than our target scanner; hard scenarios are excluded by design).

`tools/fixtures/test_suite_sanity.py`:
```python
import json
from pathlib import Path

import cv2
import numpy as np
import pytest

SUITE = Path(__file__).resolve().parents[2] / "fixtures"
EASY_PREFIXES = ("near_", "rot_")


@pytest.fixture(scope="module")
def suite():
    metas = sorted(SUITE.glob("*.json"))
    if not metas:
        pytest.fail("fixture suite not generated — run generate.py first")
    return metas


def test_suite_is_complete(suite):
    names = [p.stem for p in suite]
    assert len(names) >= 65
    for stem in names:
        assert (SUITE / f"{stem}.png").exists()
        assert (SUITE / f"{stem}.luma").exists()


def test_opencv_decodes_easy_singles(suite):
    det = cv2.QRCodeDetector()
    checked = decoded = 0
    for meta_path in suite:
        meta = json.loads(meta_path.read_text())
        if not meta_path.stem.startswith(EASY_PREFIXES):
            continue
        if len(meta["codes"]) != 1 or meta["codes"][0]["mirrored"]:
            continue
        img = np.frombuffer(
            (SUITE / f"{meta_path.stem}.luma").read_bytes(), np.uint8,
        ).reshape(meta["height"], meta["width"])
        text, pts, _ = det.detectAndDecode(img)
        checked += 1
        if text == meta["codes"][0]["payload"]:
            decoded += 1
            # OpenCV corners include no subpixel guarantee; assert coarse
            # agreement (<8 px) with ground truth to catch gross errors.
            gt = np.array(meta["codes"][0]["corners_px"])
            got = pts.reshape(4, 2)
            best = min(
                np.abs(got - np.roll(gt, k, axis=0)).max() for k in range(4))
            assert best < 8.0, meta_path.stem
    assert checked >= 15
    # OpenCV won't get everything; require a solid majority.
    assert decoded >= int(checked * 0.7), f"{decoded}/{checked}"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd tools/fixtures && .venv/bin/pytest test_suite_sanity.py -q`
Expected: FAIL — "fixture suite not generated".

- [ ] **Step 3: Generate the suite**

Run: `cd tools/fixtures && .venv/bin/python generate.py --out ../../fixtures --seed 7`
Expected: one `wrote <name>` line per fixture, ≥65 fixtures, no errors.

- [ ] **Step 4: Run the sanity test to verify it passes**

Run: `cd tools/fixtures && .venv/bin/pytest test_suite_sanity.py -q`
Expected: 2 passed. If `test_opencv_decodes_easy_singles` fails, the *generator* is presumed buggy (orientation, levels, quiet zone) — debug the generator, not the test threshold.

- [ ] **Step 5: Inspect two fixtures visually**

Open `fixtures/near_00.png` and `fixtures/multi_03.png`; confirm: light quiet zones on mid-gray background, plausible perspective, no clipped codes.

- [ ] **Step 6: Commit the suite**

```bash
git add fixtures/ tools/fixtures/test_suite_sanity.py
git commit -m "feat: golden fixture suite (65+ scenarios) with exact ground truth"
```

---

### Task 6: Rust fixture loader in `qrk-core` tests

**Files:**
- Create: `crates/qrk-core/tests/common/mod.rs`
- Create: `crates/qrk-core/tests/fixtures_smoke.rs`

**Interfaces:**
- Consumes: `qrk_core::LumaView` (Task 1); the JSON schema + `.luma` files (Tasks 4–5).
- Produces (every later plan's Rust tests consume this):
  - `common::Fixture { name: String, width: usize, height: usize, luma: Vec<u8>, codes: Vec<CodeTruth> }`
  - `common::CodeTruth { payload: String, version: u32, ecc: String, mirrored: bool, module_size_px: f64, corners_px: [[f64; 2]; 4] }`
  - `common::load_all() -> Vec<Fixture>` and `common::load(name: &str) -> Fixture` (reads `../../fixtures/` relative to the crate via `CARGO_MANIFEST_DIR`).
  - `impl Fixture { fn view(&self) -> LumaView<'_> }`

- [ ] **Step 1: Write the failing smoke test**

`crates/qrk-core/tests/fixtures_smoke.rs`:
```rust
mod common;

#[test]
fn suite_loads_and_ground_truth_is_sane() {
    let fixtures = common::load_all();
    assert!(fixtures.len() >= 65, "got {}", fixtures.len());

    let mut multi_max = 0;
    for f in &fixtures {
        let v = f.view();
        assert_eq!(v.width(), f.width);
        assert_eq!(v.height(), f.height);
        assert!(!f.codes.is_empty());
        multi_max = multi_max.max(f.codes.len());
        for c in &f.codes {
            assert!((1..=40).contains(&c.version));
            for [x, y] in c.corners_px {
                assert!(x > 0.0 && x < f.width as f64 - 1.0);
                assert!(y > 0.0 && y < f.height as f64 - 1.0);
            }
            assert!(c.module_size_px > 1.5, "{}: {}", f.name, c.module_size_px);
        }
    }
    assert_eq!(multi_max, 4, "multi_ scenarios must reach 4 codes");
}

#[test]
fn luma_pixels_match_ground_truth_ink() {
    // For every code: probe 0.5*module Euclidean along the TL->BR diagonal
    // (0.35 modules per axis) — inside is dark finder ink, outside is the
    // light quiet zone. (1.5*module would overshoot the finder's 1-module
    // dark outer ring into the white second ring.)
    for f in common::load_all() {
        let v = f.view();
        for c in &f.codes {
            let tl = c.corners_px[0];
            let br = c.corners_px[2];
            let len = ((br[0] - tl[0]).powi(2) + (br[1] - tl[1]).powi(2)).sqrt();
            let dir = [(br[0] - tl[0]) / len, (br[1] - tl[1]) / len];
            let m = c.module_size_px * 0.5;
            let inside = v.get(
                (tl[0] + dir[0] * m).round() as usize,
                (tl[1] + dir[1] * m).round() as usize,
            );
            let outside = v.get(
                (tl[0] - dir[0] * m).round() as usize,
                (tl[1] - dir[1] * m).round() as usize,
            );
            assert!(inside < 110, "{}: inside={}", f.name, inside);
            assert!(outside > 150, "{}: outside={}", f.name, outside);
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qrk-core --test fixtures_smoke`
Expected: compile error — `common` module missing.

- [ ] **Step 3: Implement the loader**

`crates/qrk-core/tests/common/mod.rs`:
```rust
//! Golden-fixture loader shared by qrk-core integration tests.
//! Schema contract: tools/fixtures/generate.py (spec §6).
use std::fs;
use std::path::PathBuf;

use qrk_core::LumaView;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CodeTruth {
    pub payload: String,
    pub version: u32,
    pub ecc: String,
    pub mirrored: bool,
    pub module_size_px: f64,
    pub corners_px: [[f64; 2]; 4],
}

#[derive(Deserialize)]
struct Meta {
    name: String,
    width: usize,
    height: usize,
    codes: Vec<CodeTruth>,
}

pub struct Fixture {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub luma: Vec<u8>,
    pub codes: Vec<CodeTruth>,
}

impl Fixture {
    pub fn view(&self) -> LumaView<'_> {
        LumaView::new(&self.luma, self.width, self.height, self.width).unwrap()
    }
}

fn suite_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

pub fn load(name: &str) -> Fixture {
    let dir = suite_dir();
    let meta: Meta = serde_json::from_str(
        &fs::read_to_string(dir.join(format!("{name}.json"))).unwrap(),
    )
    .unwrap();
    let luma = fs::read(dir.join(format!("{name}.luma"))).unwrap();
    assert_eq!(luma.len(), meta.width * meta.height, "{name}: luma size");
    Fixture {
        name: meta.name,
        width: meta.width,
        height: meta.height,
        luma,
        codes: meta.codes,
    }
}

pub fn load_all() -> Vec<Fixture> {
    let mut names: Vec<String> = fs::read_dir(suite_dir())
        .expect("fixtures/ missing — run tools/fixtures/generate.py")
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension()? == "json")
                .then(|| p.file_stem().unwrap().to_str().unwrap().to_string())
        })
        .collect();
    names.sort();
    names.iter().map(|n| load(n)).collect()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p qrk-core`
Expected: unit tests (5) + `fixtures_smoke` (2) all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/qrk-core/tests/
git commit -m "test: Rust golden-fixture loader + suite smoke tests"
```

---

## Self-review notes

- **Spec coverage (plan 1 scope = spec §6 items 0 and milestone 1):** camera model ✓ (Task 2), supersampled renderer with exact homography ✓ (Task 3), full scenario matrix incl. combos + version sweep with size scaling ✓ (Task 4), deterministic seeding ✓ (Tasks 4), committed suite ✓ (Task 5), independent-decoder sanity ✓ (Task 5), Rust contract ✓ (Tasks 1, 6). Later spec sections (detector, decode, refinement, UI, FFI) are Plans 2–5 by design.
- **Type consistency:** `LumaView::new(data, width, height, stride)` used identically in Tasks 1 and 6; JSON field names in Task 4's writer match Task 6's serde structs and Task 5's Python reader (`corners_px`, `module_size_px`, `codes`, `width`, `height`).
- **Known judgment calls:** `boost_error=False` in segno keeps requested ECC levels exact; OpenCV sanity threshold at 70% of easy singles reflects its known weakness (it validates the generator, not the detector bar); `_rng_for` uses `zlib.crc32` (not salted `hash`) for cross-process determinism.
