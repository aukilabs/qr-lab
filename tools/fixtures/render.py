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
