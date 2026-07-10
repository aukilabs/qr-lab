"""Image-space degradation operators for the robustness fixture matrix.

Every function is pure and deterministic: (img: np.uint8 HxW, params) -> new
np.uint8 HxW of identical shape. No RNG is consumed here — randomness (noise)
stays in generate.py so the existing fixtures' RNG streams are untouched.

Application order (generate.py applies ops in exactly this order; it models
the physical image-formation chain, scene -> optics -> sensor -> codec):

  1. geometric render                (render.py — perspective warp of symbol)
  2. motion_blur / defocus_blur      (optics: camera or subject motion, focus)
  3. shadow / illum_gradient / glare / contrast_compress
                                     (scene+optics illumination, multiplicative
                                      or toward-white; before any resampling)
  4. resolution                      (soft low-res video frame: down+upscale,
                                      frame dims and ground truth unchanged)
  5. blur_sigma                      (existing per-spec Gaussian: lens softness)
  6. occlusion                       (physical object in front of the scene —
                                      occludes before the sensor adds noise)
  7. noise_sigma (+ optional shot noise)   (sensor)
  8. jpeg                            (codec, always last)
"""
import cv2
import numpy as np


def _coords(shape_hw):
    h, w = shape_hw
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    return yy, xx


def motion_blur(img, length_px, angle_deg, curve=0.0):
    """Linear (or slightly curved) motion smear: line-PSF convolution.

    curve > 0 bends the motion path: a 3-segment polyline whose two interior
    vertices are displaced perpendicular to the motion direction by
    curve * length * (1 - (2t/L)^2) — a quadratic bow, zero at the endpoints.
    """
    length = float(length_px)
    if length <= 1.0:
        return img.copy()
    a = np.radians(float(angle_deg))
    d = np.array([np.cos(a), np.sin(a)])
    perp = np.array([-np.sin(a), np.cos(a)])
    ts = np.array([-0.5, -1.0 / 6.0, 1.0 / 6.0, 0.5]) * length
    bow = float(curve) * length * (1.0 - (ts / (length / 2.0)) ** 2)
    pts = ts[:, None] * d[None, :] + bow[:, None] * perp[None, :]
    half = int(np.ceil(np.abs(pts).max())) + 2
    canvas = np.zeros((2 * half + 1, 2 * half + 1), np.uint8)
    ipts = np.round(pts + half).astype(np.int32).reshape(-1, 1, 2)
    cv2.polylines(canvas, [ipts], False, 255, 1, cv2.LINE_AA)
    kernel = canvas.astype(np.float32)
    kernel /= kernel.sum()
    return cv2.filter2D(img, -1, kernel, borderType=cv2.BORDER_REFLECT101)


def defocus_blur(img, radius_px):
    """Out-of-focus blur: normalized disc PSF with a 1px fuzzy rim."""
    r = float(radius_px)
    if r <= 0.0:
        return img.copy()
    half = int(np.ceil(r)) + 1
    yy, xx = np.mgrid[-half:half + 1, -half:half + 1]
    dist = np.sqrt((xx * xx + yy * yy).astype(np.float32))
    kernel = np.clip(r + 0.5 - dist, 0.0, 1.0).astype(np.float32)
    kernel /= kernel.sum()
    return cv2.filter2D(img, -1, kernel, borderType=cv2.BORDER_REFLECT101)


def shadow_field(shape_hw, strength, softness_px, angle_deg, offset_xy,
                 shape="half", size_px=60.0):
    """Multiplicative illumination field in [1 - strength, 1].

    The shadowed region is defined analytically from a signed distance to a
    half-plane edge / band / disc (deterministic); softness_px Gaussian-blurs
    the field edge (penumbra width).
    """
    yy, xx = _coords(shape_hw)
    a = np.radians(float(angle_deg))
    ox, oy = float(offset_xy[0]), float(offset_xy[1])
    d = (xx - ox) * np.cos(a) + (yy - oy) * np.sin(a)
    if shape == "half":
        inside = d > 0.0
    elif shape == "band":
        inside = np.abs(d) < float(size_px)
    elif shape == "blob":
        inside = (xx - ox) ** 2 + (yy - oy) ** 2 < float(size_px) ** 2
    else:
        raise ValueError(f"unknown shadow shape {shape!r}")
    mask = inside.astype(np.float32)
    if softness_px > 0.0:
        mask = cv2.GaussianBlur(mask, (0, 0), float(softness_px))
    mask = np.clip(mask, 0.0, 1.0)
    return 1.0 - float(strength) * mask


def shadow(img, strength, softness_px, angle_deg, offset_xy,
           shape="half", size_px=60.0):
    """Cast shadow: img * field, field in [1 - strength, 1] (multiplicative,
    divide-compatible ground truth for shadow-normalization rungs)."""
    field = shadow_field(img.shape, strength, softness_px, angle_deg,
                         offset_xy, shape, size_px)
    out = img.astype(np.float32) * field
    return np.clip(np.rint(out), 0, 255).astype(np.uint8)


def illum_gradient(img, min_gain, max_gain, angle_deg):
    """Linear multiplicative illumination gradient across the frame."""
    yy, xx = _coords(img.shape)
    a = np.radians(float(angle_deg))
    proj = xx * np.cos(a) + yy * np.sin(a)
    span = proj.max() - proj.min()
    t = (proj - proj.min()) / span if span > 0 else np.zeros_like(proj)
    gain = float(min_gain) + (float(max_gain) - float(min_gain)) * t
    out = img.astype(np.float32) * gain
    return np.clip(np.rint(out), 0, 255).astype(np.uint8)


def glare(img, center_xy, radius_px, gain):
    """Specular hotspot: Gaussian-weighted push toward white, clipped.

    out = img + (255 - img) * min(gain * exp(-d^2 / (2 sigma^2)), 1) with
    sigma = radius_px / 2 (so the hotspot has visibly faded by d = radius).
    At the center, local contrast scales by ~(1 - gain).
    """
    yy, xx = _coords(img.shape)
    cx, cy = float(center_xy[0]), float(center_xy[1])
    sigma = float(radius_px) / 2.0
    d2 = (xx - cx) ** 2 + (yy - cy) ** 2
    weight = np.clip(float(gain) * np.exp(-0.5 * d2 / (sigma * sigma)), 0.0, 1.0)
    imgf = img.astype(np.float32)
    out = imgf + (255.0 - imgf) * weight
    return np.clip(np.rint(out), 0, 255).astype(np.uint8)


def contrast_compress(img, scale, pivot=128):
    """Low-contrast simulation: out = pivot + (img - pivot) * scale."""
    out = float(pivot) + (img.astype(np.float32) - float(pivot)) * float(scale)
    return np.clip(np.rint(out), 0, 255).astype(np.uint8)


def resolution(img, factor, method):
    """Soft low-res video frame: downscale by `factor` (method 'area' =
    INTER_AREA box average, 'nearest' = INTER_NEAREST aliasing), then upscale
    back to the original dims with INTER_LINEAR. Frame dims and ground-truth
    corners are unchanged; only the information content drops."""
    interp = {"area": cv2.INTER_AREA, "nearest": cv2.INTER_NEAREST}[method]
    h, w = img.shape
    sw = max(1, int(round(w / float(factor))))
    sh = max(1, int(round(h / float(factor))))
    small = cv2.resize(img, (sw, sh), interpolation=interp)
    return cv2.resize(small, (w, h), interpolation=cv2.INTER_LINEAR)


def jpeg(img, quality):
    """JPEG encode/decode round-trip (blocking + ringing artifacts)."""
    ok, buf = cv2.imencode(
        ".jpg", img, [int(cv2.IMWRITE_JPEG_QUALITY), int(quality)])
    assert ok
    out = cv2.imdecode(buf, cv2.IMREAD_GRAYSCALE)
    assert out.shape == img.shape
    return out


def occlusion(img, rect_xyxy, gray):
    """Fill an axis-aligned rectangle (x0, y0, x1, y1) with a flat gray —
    a physical object in front of the code."""
    x0, y0, x1, y1 = (int(v) for v in rect_xyxy)
    h, w = img.shape
    x0, x1 = max(0, x0), min(w, x1)
    y0, y1 = max(0, y0), min(h, y1)
    out = img.copy()
    out[y0:y1, x0:x1] = int(gray)
    return out
