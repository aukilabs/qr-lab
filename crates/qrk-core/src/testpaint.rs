//! Test-only synthetic-finder painters shared by `finder.rs` and
//! `triplet.rs` unit tests (compiled only under `cfg(test)` via the
//! module declaration in `lib.rs`).

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
