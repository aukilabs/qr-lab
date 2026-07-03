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
