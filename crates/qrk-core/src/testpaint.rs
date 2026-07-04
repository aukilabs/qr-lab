//! Test-only synthetic-render painters shared across unit tests (compiled
//! only under `cfg(test)` via the module declaration in `lib.rs`):
//! `paint_finder`/`paint_finder_rotated` (finder-pattern painters, used by
//! `finder.rs`/`triplet.rs`) and `render_module_grid_transformed` (a
//! general QR-matrix rasterizer through an arbitrary homography, added in
//! Plan 4 Task 2 for `version.rs`'s synthetic-render tests — extended here
//! rather than a separate tests-support module, since it's the natural
//! home for shared render helpers and later Plan 4 tasks are expected to
//! reuse it, per the task brief's "extend testpaint.rs or add a
//! tests-support module — your call, note it").

/// Paint an axis-aligned finder pattern (7x7 modules, scale px/module)
/// at top-left pixel `(ox, oy)` into a light background.
pub(crate) fn paint_finder(img: &mut [u8], w: usize, ox: usize, oy: usize,
                           scale: usize, ink: u8, bg_ring: u8) {
    for my in 0..7 {
        for mx in 0..7 {
            let dark = my == 0 || my == 6 || mx == 0 || mx == 6
                || ((2..=4).contains(&mx) && (2..=4).contains(&my));
            let v = if dark { ink } else { bg_ring };
            for py in 0..scale {
                for px in 0..scale {
                    img[(oy + my * scale + py) * w + ox + mx * scale + px] = v;
                }
            }
        }
    }
}

/// Paint a finder pattern rotated by `angle` radians about the pixel-center
/// coordinate `center` at `scale` px/module: every pixel whose center maps
/// (via the inverse rotation) into the 7x7-module square gets the standard
/// finder dark/light predicate; pixels outside are left untouched. The
/// image height is derived from `img.len() / w` (tight buffer).
pub(crate) fn paint_finder_rotated(img: &mut [u8], w: usize, center: [f64; 2],
                                   scale: f64, angle: f64, ink: u8, bg_ring: u8) {
    let h = img.len() / w;
    let [cx, cy] = center;
    let (s, c) = angle.sin_cos();
    // Bounding radius: half-diagonal of the 7-module square, plus a pixel
    // of slack for the pixel-center sampling below.
    let r = 3.5 * scale * std::f64::consts::SQRT_2 + 1.0;
    let x0 = (cx - r).floor().max(0.0) as usize;
    let x1 = ((cx + r).ceil() as usize).min(w - 1);
    let y0 = (cy - r).floor().max(0.0) as usize;
    let y1 = ((cy + r).ceil() as usize).min(h - 1);
    for py in y0..=y1 {
        for px in x0..=x1 {
            let dx = px as f64 - cx;
            let dy = py as f64 - cy;
            let u = (dx * c + dy * s) / scale + 3.5;
            let v = (-dx * s + dy * c) / scale + 3.5;
            if (0.0..7.0).contains(&u) && (0.0..7.0).contains(&v) {
                let (mu, mv) = (u.floor() as i32, v.floor() as i32);
                let dark = mv == 0 || mv == 6 || mu == 0 || mu == 6
                    || ((2..=4).contains(&mu) && (2..=4).contains(&mv));
                img[py * w + px] = if dark { ink } else { bg_ring };
            }
        }
    }
}

/// Render an abstract `dim x dim` module grid (e.g. a `qrcode::QrCode`'s
/// matrix, via `is_dark(x, y)`) into a fresh `img_w x img_h` luma buffer
/// (tight stride == `img_w`), through an arbitrary `code_to_image`
/// homography mapping the code's own module square — normalized to the
/// unit square `[0,1]x[0,1]` — to image pixel space. This is the exact
/// transform convention `version.rs`'s `count_timing_transitions`/
/// `read_version_bits` expect, so the same `PerspectiveTransform` value
/// used to render an image here can be passed straight to the function
/// under test — a tight round-trip with no separate "ground truth
/// geometry" to keep in sync.
///
/// For every output pixel, the inverse transform recovers its
/// `(u, v)` position in code-module fractions; pixels that land outside
/// `[0,1]x[0,1]` (or whose module happens to be light) are left at
/// `light` (the buffer's initial fill), so quiet-zone/background margin
/// around the code needs no separate handling — it falls out naturally
/// from picking an `img_w`/`img_h` larger than the mapped code footprint.
/// Nearest-neighbor (nearest module) sampling per output pixel, matching
/// the style of [`paint_finder_rotated`] above.
pub(crate) fn render_module_grid_transformed(
    dim: usize,
    is_dark: impl Fn(usize, usize) -> bool,
    ink: u8,
    light: u8,
    code_to_image: &crate::homography::PerspectiveTransform,
    img_w: usize,
    img_h: usize,
) -> Vec<u8> {
    let dimf = dim as f64;
    let inv = code_to_image.inverse();
    let mut img = vec![light; img_w * img_h];
    for py in 0..img_h {
        for px in 0..img_w {
            let [u, v] = inv.map(px as f64 + 0.5, py as f64 + 0.5);
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            let mx = (u * dimf) as usize;
            let my = (v * dimf) as usize;
            if mx < dim && my < dim && is_dark(mx, my) {
                img[py * img_w + px] = ink;
            }
        }
    }
    img
}

/// Antialiased variant of [`render_module_grid_transformed`] (Plan 5 Task
/// 3): renders at `supersample`x the target resolution — through
/// `code_to_image` composed with a [`crate::homography::PerspectiveTransform::scaled`]
/// factor of `supersample`, so the SAME homography places the SAME module
/// grid, just onto a bigger canvas — then box-reduces (averages each
/// `supersample x supersample` block) back down to `img_w x img_h`.
///
/// The base helper's nearest-neighbor sampling gives every edge a hard 0/1
/// step with no sub-pixel information at all — useless for testing a
/// sub-pixel localizer. Box-reducing a hard edge rendered at `supersample`x
/// instead produces a genuine coverage-weighted gradient across
/// approximately one output pixel (exactly how standard antialiased
/// rasterization/mild lens blur looks), which is what Task 3's ≤0.05px
/// synthetic accuracy gate needs: `refine_corners`'s Devernay profile
/// localizes a smooth gradient peak, something a hard-edged render cannot
/// represent at any sub-pixel precision.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_module_grid_transformed_antialiased(
    dim: usize,
    is_dark: impl Fn(usize, usize) -> bool,
    ink: u8,
    light: u8,
    code_to_image: &crate::homography::PerspectiveTransform,
    img_w: usize,
    img_h: usize,
    supersample: usize,
) -> Vec<u8> {
    let s = supersample.max(1);
    let super_transform =
        code_to_image.then(&crate::homography::PerspectiveTransform::scaled(s as f64, s as f64));
    let hi = render_module_grid_transformed(
        dim, is_dark, ink, light, &super_transform, img_w * s, img_h * s,
    );
    let hi_stride = img_w * s;
    let mut out = vec![0u8; img_w * img_h];
    let area = (s * s) as u32;
    for y in 0..img_h {
        for x in 0..img_w {
            let mut sum = 0u32;
            for dy in 0..s {
                let row = (y * s + dy) * hi_stride;
                for dx in 0..s {
                    sum += hi[row + x * s + dx] as u32;
                }
            }
            out[y * img_w + x] = (sum / area) as u8;
        }
    }
    out
}
