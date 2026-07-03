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
