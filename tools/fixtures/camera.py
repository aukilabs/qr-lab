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
