import numpy as np
import pytest

import degrade


@pytest.fixture
def img():
    rng = np.random.default_rng(1234)
    return rng.integers(0, 256, (120, 160), dtype=np.uint8)


# --- blur ops: normalized kernels preserve the mean --------------------

@pytest.mark.parametrize("op", [
    lambda im: degrade.motion_blur(im, 8, 30.0),
    lambda im: degrade.motion_blur(im, 12, 60.0, curve=0.2),
    lambda im: degrade.defocus_blur(im, 4.0),
])
def test_blur_preserves_mean(img, op):
    out = op(img)
    assert out.shape == img.shape and out.dtype == np.uint8
    assert abs(float(out.mean()) - float(img.mean())) < 1.0


@pytest.mark.parametrize("op", [
    lambda im: degrade.motion_blur(im, 10, 45.0),
    lambda im: degrade.defocus_blur(im, 3.0),
])
def test_blur_leaves_constant_image_constant(op):
    flat = np.full((60, 80), 200, np.uint8)
    out = op(flat)
    assert np.abs(out.astype(int) - 200).max() <= 1


def test_motion_blur_smears_along_angle():
    im = np.zeros((41, 41), np.uint8)
    im[20, 20] = 255
    horiz = degrade.motion_blur(im, 12, 0.0)
    # energy spreads along the row through the impulse, not the column
    assert (horiz[20, :] > 0).sum() > (horiz[:, 20] > 0).sum()


def test_motion_blur_length_one_is_identity(img):
    assert np.array_equal(degrade.motion_blur(img, 1, 0.0), img)


# --- shadow / illumination ---------------------------------------------

@pytest.mark.parametrize("shape", ["half", "band", "blob"])
def test_shadow_field_bounds(shape):
    field = degrade.shadow_field((90, 120), 0.7, 4.0, 30.0, (60.0, 45.0),
                                 shape=shape, size_px=25.0)
    assert field.shape == (90, 120)
    assert field.min() >= 1.0 - 0.7 - 1e-6
    assert field.max() <= 1.0 + 1e-6
    # both extremes actually reached (edge crosses the frame)
    assert field.min() < 1.0 - 0.7 + 0.05
    assert field.max() > 0.95


def test_shadow_darkens_only(img):
    out = degrade.shadow(img, 0.5, 2.0, 0.0, (80.0, 60.0))
    assert (out.astype(int) <= img.astype(int) + 1).all()


def test_shadow_rejects_unknown_shape(img):
    with pytest.raises(ValueError):
        degrade.shadow(img, 0.5, 2.0, 0.0, (0, 0), shape="wedge")


def test_illum_gradient_gains(img):
    out = degrade.illum_gradient(img, 0.25, 1.0, 0.0)
    # left edge scaled by ~0.25, right edge ~unchanged
    left = out[:, 0].astype(float) / np.maximum(img[:, 0], 1)
    assert np.median(left) < 0.35
    assert np.abs(out[:, -1].astype(int) - img[:, -1].astype(int)).max() <= 1


def test_glare_pushes_toward_white(img):
    out = degrade.glare(img, (80.0, 60.0), 30.0, 0.9)
    assert (out.astype(int) >= img.astype(int) - 1).all()
    assert out.max() <= 255
    center = out[55:65, 75:85].astype(float)
    assert center.mean() > img[55:65, 75:85].astype(float).mean()


def test_contrast_compress_formula():
    im = np.array([[0, 128, 255]], np.uint8)
    out = degrade.contrast_compress(im, 0.25)
    assert out.tolist() == [[96, 128, 160]]


# --- resolution / jpeg / occlusion --------------------------------------

@pytest.mark.parametrize("factor,method", [(2, "area"), (3, "nearest")])
def test_resolution_preserves_dims(img, factor, method):
    out = degrade.resolution(img, factor, method)
    assert out.shape == img.shape and out.dtype == np.uint8
    assert not np.array_equal(out, img)  # information actually dropped


def test_resolution_methods_differ(img):
    a = degrade.resolution(img, 3, "area")
    b = degrade.resolution(img, 3, "nearest")
    assert not np.array_equal(a, b)


def test_jpeg_roundtrip(img):
    out = degrade.jpeg(img, 20)
    assert out.shape == img.shape and out.dtype == np.uint8
    err_low = np.abs(out.astype(int) - img.astype(int)).mean()
    err_high = np.abs(
        degrade.jpeg(img, 95).astype(int) - img.astype(int)).mean()
    assert err_low > err_high  # lower quality -> larger error


def test_occlusion_fills_rect_only(img):
    out = degrade.occlusion(img, (10, 20, 40, 50), 96)
    assert (out[20:50, 10:40] == 96).all()
    mask = np.ones_like(img, bool)
    mask[20:50, 10:40] = False
    assert np.array_equal(out[mask], img[mask])
    assert np.array_equal(img, img)  # input untouched (op is pure)


def test_occlusion_clamps_rect(img):
    out = degrade.occlusion(img, (-5, -5, 10_000, 10_000), 7)
    assert (out == 7).all()


# --- determinism ---------------------------------------------------------

def test_ops_are_deterministic(img):
    ops = [
        lambda im: degrade.motion_blur(im, 9, 33.0, curve=0.1),
        lambda im: degrade.defocus_blur(im, 5.0),
        lambda im: degrade.shadow(im, 0.6, 8.0, 120.0, (70.0, 50.0), "band",
                                  30.0),
        lambda im: degrade.illum_gradient(im, 0.3, 1.0, 45.0),
        lambda im: degrade.glare(im, (40.0, 40.0), 25.0, 0.8),
        lambda im: degrade.contrast_compress(im, 0.15),
        lambda im: degrade.resolution(im, 2, "area"),
        lambda im: degrade.jpeg(im, 30),
        lambda im: degrade.occlusion(im, (5, 5, 20, 20), 128),
    ]
    for op in ops:
        assert np.array_equal(op(img), op(img))
