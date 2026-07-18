//! Pixel-level enhancement ops for the robustness ladder (Plan 6). Each op
//! is a pure function producing a fresh owned buffer following the
//! `downscale_luma` ownership pattern (`(Vec<u8>, w, h)`, caller wraps in a
//! [`LumaView`]). All arithmetic is integer/fixed-point so identical input
//! produces identical output bytes on every host (spec determinism rule).
//!
//! Every constant here carries a principled derivation (consts.rs rule):
//! nothing below was tuned against fixtures.

use qr_lab_image::Gray8View as LumaView;

/// 2×2 box-averaged half-resolution downscale — the anti-aliased pyramid
/// kernel (graphics-mipmap discipline). QR module patterns sit near Nyquist,
/// where nearest-neighbor decimation (see `downscale_luma`, kept for
/// debug-UI parity) can delete or double entire 1-module runs; a box average
/// instead degrades run lengths gracefully AND halves uncorrelated sensor
/// noise σ per octave — the two mechanisms the multi-scale ladder rung
/// exists for. Rounding: `(sum + 2) >> 2` (round-to-nearest). An odd
/// trailing row/column is dropped (a half-pixel crop; detection tolerance is
/// ±50% of a module, orders of magnitude above it). Returns `None` when the
/// source is too small to halve.
pub fn box_downscale_half(src: &LumaView) -> Option<(Vec<u8>, usize, usize)> {
    let (w, h) = (src.width(), src.height());
    if w < 2 || h < 2 {
        return None;
    }
    let (dw, dh) = (w / 2, h / 2);
    let mut out = vec![0u8; dw * dh];
    for y in 0..dh {
        let r0 = src.row(2 * y);
        let r1 = src.row(2 * y + 1);
        let dst = &mut out[y * dw..(y + 1) * dw];
        for (x, d) in dst.iter_mut().enumerate() {
            let s =
                r0[2 * x] as u16 + r0[2 * x + 1] as u16 + r1[2 * x] as u16 + r1[2 * x + 1] as u16;
            *d = ((s + 2) >> 2) as u8;
        }
    }
    Some((out, dw, dh))
}

/// Fixed-2× bilinear upscale, co-sited grid: `out[2i] = in[i]`,
/// `out[2i+1] = (in[i] + in[i+1] + 1) / 2` per axis (last column/row clamps
/// its missing neighbor). Co-siting makes the coordinate map exact and
/// trivial: `x_src = x_up / 2` — no half-pixel phase to mis-handle when
/// mapping detections back to source px.
///
/// Why bilinear and why upscaling helps at all: a camera pixel straddling a
/// module edge encodes the sub-pixel edge position in its gray value; a
/// monotone interpolator converts that into threshold crossings the integer
/// run-length scanner can measure, halving run quantization error — the
/// binding failure below ~3 px/module (Nyquist-study floor). Kernels with
/// negative lobes (bicubic/Lanczos) ring on step edges and can flip 1-module
/// cells; bilinear cannot create new extrema.
pub fn bilinear_upscale_2x(src: &LumaView) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    let (dw, dh) = (w * 2, h * 2);
    let mut out = vec![0u8; dw * dh];
    // Horizontal pass into even/odd columns, written row by row into the
    // even output rows; odd output rows are then averaged from their
    // vertical neighbors.
    for y in 0..h {
        let row = src.row(y);
        let dst = &mut out[(2 * y) * dw..(2 * y) * dw + dw];
        for x in 0..w {
            let a = row[x];
            let b = row[(x + 1).min(w - 1)];
            dst[2 * x] = a;
            dst[2 * x + 1] = ((a as u16 + b as u16 + 1) >> 1) as u8;
        }
    }
    for y in 0..h {
        let above_start = (2 * y) * dw;
        let below_start = (2 * (y + 1).min(h - 1)) * dw;
        for x in 0..dw {
            let a = out[above_start + x];
            let b = out[below_start + x];
            out[(2 * y + 1) * dw + x] = ((a as u16 + b as u16 + 1) >> 1) as u8;
        }
    }
    (out, dw, dh)
}

/// Fixed-3× bilinear upscale, same co-sited convention as
/// [`bilinear_upscale_2x`]: `out[3i] = in[i]`, `out[3i+1] =
/// round((2a+b)/3)`, `out[3i+2] = round((a+2b)/3)` per axis (last
/// column/row clamps its missing neighbor), so the coordinate map is exactly
/// `x_src = x_up / 3`. Round-to-nearest is `(v+1)/3` in integer math
/// (remainder 1 ≡ ⅓ rounds down, remainder 2 ≡ ⅔ rounds up).
///
/// Exists for the sub-Nyquist tail (E5b): below ~1.75 px/module even a 2×
/// upscale cannot lift a code to the ~3.5 px/module decode floor
/// (Nyquist-study bound — see [`bilinear_upscale_2x`]'s doc), while 3×
/// can, IF the anti-aliased gray edges still carry the phase information.
/// Cost: 9× the pixels of its input through detection — full-frame use is
/// gated hard by the ladder; ROI use is the intended mode.
pub fn bilinear_upscale_3x(src: &LumaView) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    let (dw, dh) = (w * 3, h * 3);
    let mut out = vec![0u8; dw * dh];
    // Horizontal pass into rows 3y; vertical pass then fills rows 3y+1/3y+2.
    for y in 0..h {
        let row = src.row(y);
        let dst = &mut out[(3 * y) * dw..(3 * y) * dw + dw];
        for x in 0..w {
            let a = row[x] as u16;
            let b = row[(x + 1).min(w - 1)] as u16;
            dst[3 * x] = a as u8;
            dst[3 * x + 1] = ((2 * a + b + 1) / 3) as u8;
            dst[3 * x + 2] = ((a + 2 * b + 1) / 3) as u8;
        }
    }
    for y in 0..h {
        let above = (3 * y) * dw;
        let below = (3 * (y + 1).min(h - 1)) * dw;
        for x in 0..dw {
            let a = out[above + x] as u16;
            let b = out[below + x] as u16;
            out[(3 * y + 1) * dw + x] = ((2 * a + b + 1) / 3) as u8;
            out[(3 * y + 2) * dw + x] = ((a + 2 * b + 1) / 3) as u8;
        }
    }
    (out, dw, dh)
}

/// Fixed-4× bilinear upscale for evidence-scoped, deeply sub-Nyquist QR
/// regions. Uses the same co-sited convention as the 2×/3× kernels:
/// `out[4i+k] = round(((4-k)·a + k·b)/4)`, so detections map back exactly
/// through `x_src = x_up / 4`. This is deliberately not a whole-frame rung;
/// 16× pixel growth is only affordable on the small ROIs whose measured
/// pitch is below 1.5 px/module.
pub fn bilinear_upscale_4x(src: &LumaView) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    let (dw, dh) = (w * 4, h * 4);
    let mut out = vec![0u8; dw * dh];
    for y in 0..h {
        let row = src.row(y);
        let dst = &mut out[(4 * y) * dw..(4 * y) * dw + dw];
        for x in 0..w {
            let a = row[x] as u16;
            let b = row[(x + 1).min(w - 1)] as u16;
            dst[4 * x] = a as u8;
            dst[4 * x + 1] = ((3 * a + b + 2) / 4) as u8;
            dst[4 * x + 2] = (a + b).div_ceil(2) as u8;
            dst[4 * x + 3] = ((a + 3 * b + 2) / 4) as u8;
        }
    }
    for y in 0..h {
        let above = (4 * y) * dw;
        let below = (4 * (y + 1).min(h - 1)) * dw;
        for x in 0..dw {
            let a = out[above + x] as u16;
            let b = out[below + x] as u16;
            out[(4 * y + 1) * dw + x] = ((3 * a + b + 2) / 4) as u8;
            out[(4 * y + 2) * dw + x] = (a + b).div_ceil(2) as u8;
            out[(4 * y + 3) * dw + x] = ((a + 3 * b + 2) / 4) as u8;
        }
    }
    (out, dw, dh)
}

/// Fixed-2× Catmull-Rom (bicubic, a = −0.5) upscale on the same co-sited
/// grid as [`bilinear_upscale_2x`]: even samples replicate the source,
/// odd samples interpolate at t = ½ with the Catmull-Rom weights
/// (−1/16, 9/16, 9/16, −1/16) — fixed point: `(9·(b+c) − (a+d) + 8) >> 4`,
/// clamped to `0..=255` (the negative lobes can overshoot). Edges clamp
/// their missing neighbors; each separable pass rounds to u8 (standard
/// two-pass discipline, ≤½ LSB extra error).
///
/// EXPERIMENT-ONLY (E5c): reachable solely through the benchmark/test
/// kernel switch, never from a public preset — the research verdict
/// predicts a wash or slight regression versus bilinear below
/// ~1.5 px/module, where the kernel's overshoot halo interacts with
/// 1-module checkerboard cells; this implementation exists to verify that
/// prediction, and ships disabled unless the A/B proves otherwise.
pub fn catmull_rom_upscale_2x(src: &LumaView) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    let (dw, dh) = (w * 2, h * 2);
    // Catmull-Rom t=1/2 over (a,b,c,d), fixed point /16, clamped.
    let cr = |a: i32, b: i32, c: i32, d: i32| -> u8 {
        ((9 * (b + c) - (a + d) + 8) >> 4).clamp(0, 255) as u8
    };
    let mut out = vec![0u8; dw * dh];
    // Horizontal pass into even rows.
    for y in 0..h {
        let row = src.row(y);
        let px = |i: isize| row[i.clamp(0, w as isize - 1) as usize] as i32;
        let dst = &mut out[(2 * y) * dw..(2 * y) * dw + dw];
        for x in 0..w {
            let xi = x as isize;
            dst[2 * x] = row[x];
            dst[2 * x + 1] = cr(px(xi - 1), px(xi), px(xi + 1), px(xi + 2));
        }
    }
    // Vertical pass: odd rows from the four nearest even (source) rows.
    for y in 0..h {
        let yi = y as isize;
        let row_at = |i: isize| {
            let yy = 2 * i.clamp(0, h as isize - 1) as usize;
            &out[yy * dw..yy * dw + dw]
        };
        let (ra, rb, rc, rd) = (row_at(yi - 1), row_at(yi), row_at(yi + 1), row_at(yi + 2));
        let mut odd = vec![0u8; dw];
        for (x, o) in odd.iter_mut().enumerate() {
            *o = cr(ra[x] as i32, rb[x] as i32, rc[x] as i32, rd[x] as i32);
        }
        out[(2 * y + 1) * dw..(2 * y + 1) * dw + dw].copy_from_slice(&odd);
    }
    (out, dw, dh)
}

/// Branch-free monomorphized extremum for the van Herk passes below: a
/// `const` polarity lets the per-pixel op inline (a runtime `fn` pointer
/// costs an indirect call per pixel — measured ~3x slower at 720p).
#[inline(always)]
fn ext<const IS_MAX: bool>(a: u8, b: u8) -> u8 {
    if IS_MAX {
        a.max(b)
    } else {
        a.min(b)
    }
}

/// 1-D van Herk/Gil-Werman running extremum (max when `IS_MAX`, else min)
/// over a centered window of `k` samples (`k` odd), O(n) independent of `k`:
/// block-scoped prefix and suffix extrema give any window as
/// `ext(suffix[lo], prefix[hi])`. Border windows are clamped to the signal
/// and read the extremum of the whole touching block, i.e. the effective
/// window near a border is up to 2× wider — for a background ENVELOPE this
/// only over-smooths the outermost `k` pixels, far outside any decodable
/// code's quiet zone.
fn running_extremum_1d<const IS_MAX: bool>(
    src: &[u8],
    k: usize,
    pre: &mut [u8],
    suf: &mut [u8],
    out: &mut [u8],
) {
    let n = src.len();
    debug_assert!(k >= 3 && k % 2 == 1 && pre.len() == n && suf.len() == n && out.len() == n);
    let ext = ext::<IS_MAX>;
    let r = k / 2;
    for i in 0..n {
        pre[i] = if i % k == 0 {
            src[i]
        } else {
            ext(pre[i - 1], src[i])
        };
    }
    for i in (0..n).rev() {
        suf[i] = if i % k == k - 1 || i == n - 1 {
            src[i]
        } else {
            ext(suf[i + 1], src[i])
        };
    }
    for (i, o) in out.iter_mut().enumerate() {
        let lo = i.saturating_sub(r);
        let hi = (i + r).min(n - 1);
        *o = ext(suf[lo], pre[hi]);
    }
}

/// Separable morphological pass (dilation when `is_max`, erosion otherwise)
/// with a `k`×`k` rectangular structuring element — exact, because rect-SE
/// dilation/erosion factor into a horizontal then a vertical 1-D pass, each
/// O(N) regardless of `k` (van Herk/Gil-Werman). The vertical pass keeps
/// row-major access by materializing whole prefix/suffix planes (`2·N`
/// scratch bytes) instead of gathering columns — an order of magnitude
/// faster than a strided column walk at 720p.
pub fn morph_rect<const IS_MAX: bool>(src: &[u8], w: usize, h: usize, k: usize, dst: &mut Vec<u8>) {
    dst.clear();
    dst.resize(w * h, 0);
    let ext = ext::<IS_MAX>;
    let r = k / 2;
    // Horizontal: per-row 1-D pass.
    {
        let mut pre = vec![0u8; w];
        let mut suf = vec![0u8; w];
        for y in 0..h {
            running_extremum_1d::<IS_MAX>(
                &src[y * w..(y + 1) * w],
                k,
                &mut pre,
                &mut suf,
                &mut dst[y * w..(y + 1) * w],
            );
        }
    }
    // Vertical: the same block prefix/suffix scheme along y, swept
    // row-major over whole rows.
    let hpass = dst.clone();
    let mut pre = vec![0u8; w * h];
    let mut suf = vec![0u8; w * h];
    for y in 0..h {
        let (done, cur) = pre.split_at_mut(y * w);
        let cur = &mut cur[..w];
        let row = &hpass[y * w..(y + 1) * w];
        if y % k == 0 {
            cur.copy_from_slice(row);
        } else {
            let prev = &done[(y - 1) * w..y * w];
            for x in 0..w {
                cur[x] = ext(prev[x], row[x]);
            }
        }
    }
    for y in (0..h).rev() {
        let (cur, rest) = suf[y * w..].split_at_mut(w);
        let row = &hpass[y * w..(y + 1) * w];
        if y % k == k - 1 || y == h - 1 {
            cur.copy_from_slice(row);
        } else {
            let next = &rest[..w];
            for x in 0..w {
                cur[x] = ext(next[x], row[x]);
            }
        }
    }
    for y in 0..h {
        let lo = y.saturating_sub(r);
        let hi = (y + r).min(h - 1);
        let s = &suf[lo * w..(lo + 1) * w];
        let p = &pre[hi * w..(hi + 1) * w];
        let out = &mut dst[y * w..(y + 1) * w];
        for x in 0..w {
            out[x] = ext(s[x], p[x]);
        }
    }
}

/// Area-average (box) downscale to exactly `(dw, dh)` — the anti-aliased
/// general-ratio counterpart of [`box_downscale_half`], for the robust
/// pipeline's DETECTION substrate. Each destination pixel integrates the
/// exact fractional source rectangle it covers (weights in 1/256ths per
/// axis, u64 accumulator), so QR module runs degrade gracefully at any
/// ratio where the pinned nearest-neighbor `downscale_luma` aliases them —
/// diagnosis D4 measured NN at 1920→1280 (ratio 2/3) destroying finder
/// runs the area kernel preserves (+12% finder candidates, +4 decoded
/// frames at matched substrate on the real-video corpus).
///
/// DETECTION ONLY: module sampling and refinement must keep reading the
/// pristine source (D4's arm F measured net −4 decodes when the sampling
/// path was prefiltered).
pub fn area_downscale(src: &LumaView, dw: usize, dh: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    debug_assert!(dw >= 1 && dh >= 1 && dw <= w && dh <= h);
    let mut out = vec![0u8; dw * dh];
    // Per-axis fixed-point (8-bit fraction) edge positions: destination
    // pixel k covers source [k*w/dw, (k+1)*w/dw). Each source pixel
    // contributes to at most two destination cells per axis, so the sweep
    // below is O(source pixels) overall. The horizontal coverage weights
    // are identical for every row, so they are computed ONCE into a
    // flattened per-destination-column table (this hoisting is what makes
    // the pass ~ms-scale at 1920×1440 — the arithmetic per pixel is
    // unchanged and the output byte-identical to the naive double loop:
    // Σ_sy Σ_sx p·(wx·wy) == Σ_sy wy·(Σ_sx p·wx) in exact integers).
    let fx = (w as u64) << 8; // source width in 1/256 units
    let fy = (h as u64) << 8;
    // (first source column, column count) per destination column, plus the
    // flattened per-column weights in 1/256 units.
    let mut xspan: Vec<(u32, u32)> = Vec::with_capacity(dw);
    let mut xw: Vec<u64> = Vec::with_capacity(dw * (w / dw + 2));
    for ox in 0..dw {
        let x0 = fx * ox as u64 / dw as u64;
        let x1 = fx * (ox as u64 + 1) / dw as u64;
        let (sx0, sx1) = ((x0 >> 8) as usize, (((x1 + 255) >> 8) as usize).min(w));
        xspan.push((sx0 as u32, (sx1 - sx0) as u32));
        for sx in sx0..sx1 {
            let left = ((sx as u64) << 8).max(x0);
            let right = (((sx + 1) as u64) << 8).min(x1);
            xw.push(right - left);
        }
    }
    // Separable two-pass sweep. Pass 1 shrinks every source row
    // horizontally into UNNORMALIZED u32 weighted sums (a row's sums fit
    // u32: ≤ (w/dw + 2) columns × 256 × 255 ≪ 2^32); the per-column weight
    // total `hwgt` is row-independent. Pass 2 combines whole shrunk rows
    // with the vertical weights. Exactness: Σ_sy Σ_sx p·(wx·wy) ==
    // Σ_sy wy·(Σ_sx p·wx) and Σ wx·wy == (Σ wx)·(Σ wy) in exact integer
    // arithmetic, so `acc`/`weight` — and therefore every rounded output
    // byte — are identical to the naive per-pixel double loop.
    let mut hwgt = vec![0u64; dw];
    {
        let mut k = 0usize;
        for ox in 0..dw {
            let (_, n) = xspan[ox];
            hwgt[ox] = xw[k..k + n as usize].iter().sum();
            k += n as usize;
        }
    }
    let mut hsum = vec![0u32; dw * h];
    let max_span = xspan.iter().map(|&(_, n)| n).max().unwrap_or(0) as usize;
    if max_span <= 2 {
        // Fast two-tap path: at any ratio < 2 a destination cell spans at
        // most two source columns, so the weights flatten into two dense
        // per-cell tap tables and the row loop becomes a branch-free
        // 2-MAC sweep the compiler can keep in registers (this is the
        // production video shape — 1920→1280 is ratio 3:2). Weights are
        // the same `xw` entries, so the sums are bit-identical to the
        // generic sweep below.
        let mut c0 = vec![0u32; dw];
        let mut t0 = vec![0u32; dw];
        let mut t1 = vec![0u32; dw];
        let mut k = 0usize;
        for ox in 0..dw {
            let (sx0, n) = xspan[ox];
            c0[ox] = sx0;
            t0[ox] = xw[k] as u32;
            // Second tap weight is 0 for single-column cells; its (then
            // meaningless) sample index is clamped inside the row below.
            t1[ox] = if n as usize > 1 { xw[k + 1] as u32 } else { 0 };
            k += n as usize;
        }
        for sy in 0..h {
            let row = src.row(sy);
            let hrow = &mut hsum[sy * dw..(sy + 1) * dw];
            for ox in 0..dw {
                let c = c0[ox] as usize;
                let b = row[(c + 1).min(w - 1)];
                hrow[ox] = row[c] as u32 * t0[ox] + b as u32 * t1[ox];
            }
        }
    } else {
        for sy in 0..h {
            let row = src.row(sy);
            let hrow = &mut hsum[sy * dw..(sy + 1) * dw];
            let mut k = 0usize;
            for (ox, hcell) in hrow.iter_mut().enumerate() {
                let (sx0, n) = xspan[ox];
                let mut a = 0u32;
                for i in 0..n as usize {
                    a += row[sx0 as usize + i] as u32 * xw[k + i] as u32;
                }
                *hcell = a;
                k += n as usize;
            }
        }
    }
    let mut acc = vec![0u64; dw];
    for oy in 0..dh {
        let y0 = fy * oy as u64 / dh as u64; // 1/256 units
        let y1 = fy * (oy as u64 + 1) / dh as u64;
        let (sy0, sy1) = ((y0 >> 8) as usize, (((y1 + 255) >> 8) as usize).min(h));
        acc.fill(0);
        let mut vwgt = 0u64;
        for sy in sy0..sy1 {
            // Vertical coverage of source row `sy` by [y0, y1), 0..=256.
            let top = ((sy as u64) << 8).max(y0);
            let bot = (((sy + 1) as u64) << 8).min(y1);
            let wy = bot - top;
            vwgt += wy;
            let hrow = &hsum[sy * dw..(sy + 1) * dw];
            for (a, &hs) in acc.iter_mut().zip(hrow) {
                *a += hs as u64 * wy;
            }
        }
        for ox in 0..dw {
            let weight = hwgt[ox] * vwgt;
            out[oy * dw + ox] = (acc[ox] + weight / 2).checked_div(weight).unwrap_or(0) as u8;
        }
    }
    (out, dw, dh)
}

/// Shadow normalization by background DIVISION (Plan 6 stage 3). The
/// illumination field is estimated by a grayscale morphological CLOSING
/// (dilate-then-erode, `se_px`×`se_px` rectangular structuring element, van
/// Herk/Gil-Werman O(N) independent of SE size): the upper envelope of the
/// image with every dark structure narrower than the SE removed. Output:
/// `I · 200 / max(B, 8)`.
///
/// Why an envelope and not an average (the earlier 3×-iterated-box-blur
/// estimate): an averaging estimator smears the background level ACROSS a
/// sharp shadow or glare edge — the estimate is wrong (haloed) exactly in
/// the band around the edge that decides whether an edge-crossing finder
/// pattern survives, and measured on this suite the blur estimate erased
/// finder candidates near hard glare edges. The closing envelope is exact
/// up to the shadow edge on the lit side and tracks the shadow floor on the
/// dark side; its only error region is one SE width inside the darker side.
/// It is also a true UPPER bound (`B ≥ I` pointwise), so the divided output
/// never saturates (`I·200/B ≤ 200`) and paper-white lands at ~200
/// everywhere — the same target the tile binarizer's contrast floor was
/// derived against.
///
/// `se_px` — the SE edge in pixels — must exceed the widest solid dark INK
/// structure (else ink is mistaken for shadow and brightened away) while
/// staying below the shadow's width (else the closing bridges the shadow
/// and division is a no-op inside it). The caller derives it from detection
/// evidence (see the ladder's shadow rung); this function only clamps it to
/// a legal window (odd, ≥3, ≤ the image's shorter side).
///
/// Division, never subtraction: illumination is multiplicative on
/// reflectance — a module under 4× less light keeps the SAME relative
/// contrast, which division restores and subtraction cannot. The `200`
/// numerator targets paper-white at ~200, leaving glint headroom; the
/// denominator floor `8` = 2× the tile binarizer's 4σ-class noise band
/// (2σ≈4 per `CONTRAST_FLOOR`'s sensor model): below it a region carries
/// too little module signal for normalization to do anything but amplify
/// noise.
pub fn background_divide(src: &LumaView, se_px: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    let mut flat = vec![0u8; w * h];
    for y in 0..h {
        flat[y * w..(y + 1) * w].copy_from_slice(src.row(y));
    }
    // A view too small to hold any window is returned unchanged — no
    // meaningful illumination estimate exists below a few pixels.
    if w.min(h) < 5 {
        return (flat, w, h);
    }
    // Odd, ≥3, and no wider than the shorter side (a window covering the
    // whole image degenerates to a global max — harmless but pointless).
    let k = (se_px.clamp(3, w.min(h)) | 1).min(w.min(h) | 1);
    let mut dilated = Vec::new();
    let mut bg = Vec::new();
    morph_rect::<true>(&flat, w, h, k, &mut dilated); // max: dark ink removed
    morph_rect::<false>(&dilated, w, h, k, &mut bg); // min: background geometry restored
    let mut out = flat;
    for (o, &g) in out.iter_mut().zip(bg.iter()) {
        let v = (*o as u32 * 200) / (g.max(8) as u32);
        *o = v.min(255) as u8;
    }
    (out, w, h)
}

/// Unsharp mask with the 5-tap binomial kernel `[1,4,6,4,1]/16` (σ≈1.1) and
/// amount k=1: `out = clamp(2·I − blur)`. Support is 5px, i.e. strictly
/// sub-module for every decodable code (≥2px/module ⇒ finder core ≥6px), so
/// overshoot cannot cross a module. k=1 doubles edge contrast while bounding
/// noise gain at 2× — amplified sensor noise (2σ≈4 → 8) stays under the
/// binarizer's CONTRAST_FLOOR=12, so flat regions do not speckle. Intended
/// for DETECTION buffers only; module sampling must keep reading the raw
/// source (ladder invariant — ringing must never touch sampling).
pub fn unsharp_mask(src: &LumaView) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    // Horizontal blur pass (u16 intermediate keeps full precision: max
    // 255*16 = 4080).
    let mut hpass = vec![0u16; w * h];
    for y in 0..h {
        let row = src.row(y);
        let out = &mut hpass[y * w..(y + 1) * w];
        for (x, o) in out.iter_mut().enumerate() {
            let px = |i: isize| row[i.clamp(0, w as isize - 1) as usize] as u16;
            let xi = x as isize;
            *o = px(xi - 2) + 4 * px(xi - 1) + 6 * px(xi) + 4 * px(xi + 1) + px(xi + 2);
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let yi = y as isize;
        let row_at = |i: isize| {
            let yy = i.clamp(0, h as isize - 1) as usize;
            &hpass[yy * w..(yy + 1) * w]
        };
        let (rm2, rm1, r0, rp1, rp2) = (
            row_at(yi - 2),
            row_at(yi - 1),
            row_at(yi),
            row_at(yi + 1),
            row_at(yi + 2),
        );
        let src_row = src.row(y);
        let dst = &mut out[y * w..(y + 1) * w];
        for x in 0..w {
            let blur = (rm2[x] as u32
                + 4 * rm1[x] as u32
                + 6 * r0[x] as u32
                + 4 * rp1[x] as u32
                + rp2[x] as u32
                + 128)
                >> 8;
            let sharp = 2 * src_row[x] as i32 - blur as i32;
            dst[x] = sharp.clamp(0, 255) as u8;
        }
    }
    (out, w, h)
}

/// Blur direction + anisotropy from the gradient structure tensor. Motion
/// blur suppresses gradients ALONG its direction, so the blur direction is
/// the tensor's minor eigenvector: `θ_major = 0.5·atan2(2·Sxy, Sxx−Syy)`,
/// blur = θ_major + 90°. Returns `(theta_radians, confidence)` with
/// confidence = 1 − λmin/λmax in `[0,1]`; ~0 means isotropic (defocus or no
/// blur — directional processing would be a coin flip and must not fire).
/// Sobel gradients, f64 accumulators, O(N) single pass.
pub fn structure_tensor_blur_direction(src: &LumaView) -> (f64, f64) {
    let (w, h) = (src.width(), src.height());
    if w < 3 || h < 3 {
        return (0.0, 0.0);
    }
    let (mut sxx, mut sxy, mut syy) = (0.0f64, 0.0f64, 0.0f64);
    for y in 1..h - 1 {
        let (ra, rb, rc) = (src.row(y - 1), src.row(y), src.row(y + 1));
        for x in 1..w - 1 {
            let gx = (ra[x + 1] as i32 + 2 * rb[x + 1] as i32 + rc[x + 1] as i32)
                - (ra[x - 1] as i32 + 2 * rb[x - 1] as i32 + rc[x - 1] as i32);
            let gy = (rc[x - 1] as i32 + 2 * rc[x] as i32 + rc[x + 1] as i32)
                - (ra[x - 1] as i32 + 2 * ra[x] as i32 + ra[x + 1] as i32);
            let (gx, gy) = (gx as f64, gy as f64);
            sxx += gx * gx;
            sxy += gx * gy;
            syy += gy * gy;
        }
    }
    let tr = sxx + syy;
    if tr <= 0.0 {
        return (0.0, 0.0);
    }
    let det = sxx * syy - sxy * sxy;
    let disc = ((tr * tr) / 4.0 - det).max(0.0).sqrt();
    let (l_max, l_min) = (tr / 2.0 + disc, (tr / 2.0 - disc).max(0.0));
    let confidence = if l_max > 0.0 {
        1.0 - l_min / l_max
    } else {
        0.0
    };
    let theta_major = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    (theta_major + std::f64::consts::FRAC_PI_2, confidence)
}

/// Snap an angle to the nearest raster axis {0°, 45°, 90°, 135°} as an
/// integer step vector, so 1-D directional kernels use integer taps (no
/// interpolation). Worst-case snap error 22.5° shortens the effective
/// kernel by only 1−cos(22.5°) ≈ 8%.
pub fn snap_dir(theta: f64) -> (isize, isize) {
    let deg = theta.to_degrees().rem_euclid(180.0);
    match ((deg / 45.0).round() as i64).rem_euclid(4) {
        0 => (1, 0),
        1 => (1, 1),
        2 => (0, 1),
        _ => (-1, 1),
    }
}

/// Directional 1-D unsharp along the blur direction: subtract a 1-D box
/// mean of `len` samples along θ, `out = clamp(I + g·(I − mean))` in
/// integer math (g = 3/2). θ is snapped via [`snap_dir`]. Only the
/// smeared axis is sharpened, so noise on the clean axis is untouched —
/// ~2× the restoration per unit of noise amplification vs isotropic
/// sharpening (the 1-D line-PSF physics of camera-shake blur). Like
/// [`unsharp_mask`], detection-buffer only.
pub fn directional_unsharp(src: &LumaView, theta: f64, len: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    let (dx, dy) = snap_dir(theta);
    let len = len.max(3) | 1; // odd, >= 3
    let half = (len / 2) as isize;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let dst = &mut out[y * w..(y + 1) * w];
        for (x, d) in dst.iter_mut().enumerate() {
            let mut sum: u32 = 0;
            for k in -half..=half {
                let sx = (x as isize + k * dx).clamp(0, w as isize - 1) as usize;
                let sy = (y as isize + k * dy).clamp(0, h as isize - 1) as usize;
                sum += src.get(sx, sy) as u32;
            }
            let mean = (sum / len as u32) as i32;
            let i = src.get(x, y) as i32;
            // g = 3/2 in integer math: I + (3*(I-mean))/2.
            let sharp = i + (3 * (i - mean)) / 2;
            *d = sharp.clamp(0, 255) as u8;
        }
    }
    (out, w, h)
}

/// Minimum number of measurable edge transitions for
/// [`edge_rise_extent`] to return an estimate: below a handful of samples
/// the median is dominated by incidental scene gradients rather than
/// code-like structure (standard robust-statistics small-sample caveat);
/// any frame actually containing a QR symbol presents tens of transitions
/// per axis (a v1 code alone has ≥21 module edges per row band).
const MIN_RISE_SAMPLES: usize = 8;

/// Only transitions whose amplitude clears 2× the binarizer's
/// CONTRAST_FLOOR (2×12 = 24 gray — the decode-grade contrast bar used
/// throughout the fixture expectations) are measured: weaker swings are
/// sensor noise or sub-threshold texture whose "rise" is meaningless.
const MIN_RISE_AMPLITUDE: i32 = 24;

/// Motion-blur extent estimate from 20–80% edge-rise widths along `theta`
/// (snapped via [`snap_dir`]). A step edge convolved with a length-L line
/// PSF becomes a linear ramp whose 20–80% rise spans exactly 0.6·L, so
/// L = rise / 0.6 (exact for a box/line PSF; Plan 6 research fact 9).
///
/// Method: every scan line along the snapped direction is split into
/// maximal monotonic runs (plateaus extend a run); each run with
/// amplitude ≥ [`MIN_RISE_AMPLITUDE`] contributes its 20–80% crossing
/// distance (sub-sample by linear interpolation, ×√2 on diagonals to
/// convert sample steps to px).
///
/// Completeness selection: a transition whose flanking plateaus are
/// shorter than L (a QR data region under a multi-module smear — the
/// common case) is TRUNCATED, which shortens its measured rise and its
/// amplitude together (both are integrals of the same clipped ramp), so
/// truncated rises would drag any plain average far below L. Only runs
/// whose amplitude reaches at least HALF the strongest observed swing are
/// kept — for a box PSF a ramp that reaches ≥½ of the full-contrast
/// swing has completed ≥½ of the kernel, bounding its rise bias to ×½,
/// which is exactly the low edge the caller's ×{½,1,1½} candidate
/// bracket already covers. The MEDIAN of the kept rises is returned as
/// L̂ (robust to the residual tails: still-truncated rises bias low,
/// merged same-direction edges bias high). Returns `None` when fewer
/// than [`MIN_RISE_SAMPLES`] transitions were measurable. Cost: one
/// pass over the image, O(N).
pub fn edge_rise_extent(src: &LumaView, theta: f64) -> Option<f64> {
    edge_rise_extent_with(src, theta, MIN_RISE_SAMPLES, MIN_RISE_AMPLITUDE)
}

pub fn edge_rise_extent_with(
    src: &LumaView,
    theta: f64,
    min_samples: usize,
    min_amplitude: i32,
) -> Option<f64> {
    let (w, h) = (src.width(), src.height());
    if w < 3 || h < 3 {
        return None;
    }
    let (dx, dy) = snap_dir(theta);
    let step_px = if dx != 0 && dy != 0 {
        std::f64::consts::SQRT_2
    } else {
        1.0
    };
    /// 20–80% crossing distance and amplitude of one monotonic run
    /// `line[a..=b]`.
    fn rise_of(line: &[u8], a: usize, b: usize, min_amplitude: i32) -> Option<(f64, i32)> {
        let (va, vb) = (line[a] as i32, line[b] as i32);
        let amp = vb - va;
        if amp.abs() < min_amplitude {
            return None;
        }
        let lo = va as f64 + 0.2 * amp as f64;
        let hi = va as f64 + 0.8 * amp as f64;
        let (mut p_lo, mut p_hi) = (None, None);
        for i in a..b {
            let (v0, v1) = (line[i] as f64, line[i + 1] as f64);
            let crossed = |t: f64| {
                if amp > 0 {
                    v0 <= t && t <= v1
                } else {
                    v1 <= t && t <= v0
                }
            };
            let frac = |t: f64| {
                if v1 == v0 {
                    0.0
                } else {
                    (t - v0) / (v1 - v0)
                }
            };
            if p_lo.is_none() && crossed(lo) {
                p_lo = Some(i as f64 + frac(lo));
            }
            if crossed(hi) {
                p_hi = Some(i as f64 + frac(hi));
                if p_lo.is_some() {
                    break;
                }
            }
        }
        match (p_lo, p_hi) {
            (Some(p0), Some(p1)) if p1 > p0 => Some((p1 - p0, amp.abs())),
            _ => None,
        }
    }

    /// Split one scan line into maximal monotonic runs (plateaus extend a
    /// run — a ramp's flanking plateaus belong to it; the 20/80 crossings
    /// land inside the ramp regardless) and collect their rises.
    fn measure_line(line: &[u8], step_px: f64, min_amplitude: i32, rises: &mut Vec<(f64, i32)>) {
        let n = line.len();
        if n < 3 {
            return;
        }
        let mut start = 0usize;
        let mut dir: i32 = 0; // sign of the current run
        for i in 1..n {
            let d = (line[i] as i32 - line[i - 1] as i32).signum();
            if d == 0 {
                continue;
            }
            if dir != 0 && d != dir {
                if let Some((r, a)) = rise_of(line, start, i - 1, min_amplitude) {
                    rises.push((r * step_px, a));
                }
                start = i - 1;
            }
            dir = d;
        }
        if dir != 0 {
            if let Some((r, a)) = rise_of(line, start, n - 1, min_amplitude) {
                rises.push((r * step_px, a));
            }
        }
    }

    let mut rises: Vec<(f64, i32)> = Vec::new();
    let mut line: Vec<u8> = Vec::with_capacity(w.max(h));
    let mut walk = |x0: isize, y0: isize, rises: &mut Vec<(f64, i32)>| {
        line.clear();
        let (mut x, mut y) = (x0, y0);
        while x >= 0 && (x as usize) < w && (y as usize) < h {
            line.push(src.get(x as usize, y as usize));
            x += dx;
            y += dy;
        }
        measure_line(&line, step_px, min_amplitude, rises);
    };
    match (dx, dy) {
        (1, 0) => {
            for y in 0..h {
                measure_line(src.row(y), step_px, min_amplitude, &mut rises);
            }
        }
        (0, 1) => {
            for x in 0..w {
                walk(x as isize, 0, &mut rises);
            }
        }
        (1, 1) => {
            for x in 0..w {
                walk(x as isize, 0, &mut rises);
            }
            for y in 1..h {
                walk(0, y as isize, &mut rises);
            }
        }
        _ => {
            for x in 0..w {
                walk(x as isize, 0, &mut rises);
            }
            for y in 1..h {
                walk(w as isize - 1, y as isize, &mut rises);
            }
        }
    }
    let amp_max = rises.iter().map(|&(_, a)| a).max().unwrap_or(0);
    let mut kept: Vec<f64> = rises
        .iter()
        .filter(|&&(_, a)| 2 * a >= amp_max)
        .map(|&(r, _)| r)
        .collect();
    if kept.len() < min_samples {
        return None;
    }
    kept.sort_by(f64::total_cmp);
    Some(kept[kept.len() / 2] / 0.6)
}

/// Van Cittert iteration count. Each iteration extends the truncated
/// Neumann series Σⱼ (1−H)ʲ — the polynomial approximation of the inverse
/// filter H⁻¹ — by one order: restoration deepens geometrically where the
/// box MTF is positive, while ringing from the MTF's negative side lobes
/// (a box's Dirichlet spectrum dips to −0.217) also compounds. 2–3
/// iterations is the classical spatial-domain operating point (Jansson,
/// "Deconvolution of Images and Spectra"): beyond it the noise/ringing
/// gain outpaces the residual restoration.
const VAN_CITTERT_ITERATIONS: usize = 3;

/// 1-D box mean of `len` samples (odd) along an integer direction, into
/// `dst`. Running-sum window per scan line — O(N) independent of `len`;
/// window ends clamp (replicate the line's first/last sample), and lines
/// shorter than the window degenerate gracefully to the clamped samples.
/// i64 window sums: intermediate Van Cittert images exceed `[0,255]` by
/// design (the truncated inverse overshoots before clamping).
fn directional_box_mean(
    src: &[i32],
    w: usize,
    h: usize,
    (dx, dy): (isize, isize),
    len: usize,
    dst: &mut [i32],
) {
    debug_assert!(len % 2 == 1 && len >= 3);
    let half = (len / 2) as isize;
    let mut idx: Vec<usize> = Vec::with_capacity(w.max(h) * 2);
    let run = |idx: &[usize], src: &[i32], dst: &mut [i32]| {
        let n = idx.len() as isize;
        if n == 0 {
            return;
        }
        let at = |i: isize| src[idx[i.clamp(0, n - 1) as usize]];
        let mut sum: i64 = 0;
        for k in -half..=half {
            sum += at(k) as i64;
        }
        for i in 0..n {
            dst[idx[i as usize]] = (sum.div_euclid(len as i64)) as i32;
            sum += at(i + half + 1) as i64 - at(i - half) as i64;
        }
    };
    let mut walk = |x0: isize, y0: isize, dst: &mut [i32]| {
        idx.clear();
        let (mut x, mut y) = (x0, y0);
        while x >= 0 && (x as usize) < w && (y as usize) < h {
            idx.push(y as usize * w + x as usize);
            x += dx;
            y += dy;
        }
        run(&idx, src, dst);
    };
    match (dx, dy) {
        (1, 0) => {
            for y in 0..h {
                idx.clear();
                idx.extend(y * w..(y + 1) * w);
                run(&idx, src, dst);
            }
        }
        (0, 1) => {
            for x in 0..w {
                walk(x as isize, 0, dst);
            }
        }
        (1, 1) => {
            for x in 0..w {
                walk(x as isize, 0, dst);
            }
            for y in 1..h {
                walk(0, y as isize, dst);
            }
        }
        _ => {
            for x in 0..w {
                walk(x as isize, 0, dst);
            }
            for y in 1..h {
                walk(w as isize - 1, y as isize, dst);
            }
        }
    }
}

/// 1-D Van Cittert deconvolution along the blur direction (Plan 6 deblur
/// tier, deep rung): `Iₖ₊₁ = Iₖ + β·(B − boxₗ ∗ Iₖ)` with β = 1,
/// restricted to 1-D along θ (snapped via [`snap_dir`]) —
/// [`VAN_CITTERT_ITERATIONS`] iterations, spatial domain only, integer
/// math, O(N·iterations) independent of `len` (running-sum box).
///
/// β = 1 is the classical (and maximal always-stable) relaxation factor:
/// the iteration converges exactly where the PSF's MTF satisfies
/// |1 − H| < 1, i.e. wherever H > 0, and k iterations realize the
/// truncated Neumann series Σ_{j≤k} (1−H)ʲ B → H⁻¹B. Unlike the
/// single-pass directional unsharp (a fixed first-order correction with
/// sub-module support), this inverts a smear of KNOWN length `len`
/// spanning multiple modules — the caller estimates `len` from the
/// 20–80% edge rise ([`edge_rise_extent`]) and brackets it with a
/// candidate sweep decided by the decode checksum.
///
/// The output is meant to be BOTH detected and sampled (unlike the
/// unsharp rungs): deconvolution exists to restore data-cell contrast,
/// and sampling the pristine-but-smeared source would discard exactly
/// that restoration. Intermediate values run out of `[0,255]` (i32
/// working buffer); only the final image clamps.
pub fn van_cittert_directional(src: &LumaView, theta: f64, len: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (src.width(), src.height());
    let len = len.max(3) | 1; // odd, >= 3
    let dir = snap_dir(theta);
    let mut b = vec![0i32; w * h];
    for y in 0..h {
        let row = src.row(y);
        for (x, &p) in row.iter().enumerate() {
            b[y * w + x] = p as i32;
        }
    }
    let mut x_img = b.clone();
    let mut mean = vec![0i32; w * h];
    for _ in 0..VAN_CITTERT_ITERATIONS {
        directional_box_mean(&x_img, w, h, dir, len, &mut mean);
        for i in 0..x_img.len() {
            x_img[i] += b[i] - mean[i];
        }
    }
    let out = x_img.iter().map(|&v| v.clamp(0, 255) as u8).collect();
    (out, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qr_lab_image::Gray8View as LumaView;

    fn view(data: &[u8], w: usize, h: usize) -> LumaView<'_> {
        LumaView::new(data, w, h, w).unwrap()
    }

    #[test]
    fn box_downscale_half_averages_quads() {
        let d = vec![0u8, 4, 8, 12, 16, 20, 24, 28]; // 4x2
        let (out, w, h) = box_downscale_half(&view(&d, 4, 2)).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, vec![(4 + 16 + 20 + 2) / 4, (8 + 12 + 24 + 28 + 2) / 4]);
    }

    #[test]
    fn box_downscale_half_rejects_tiny() {
        let d = vec![1u8, 2];
        assert!(box_downscale_half(&view(&d, 1, 2)).is_none());
    }

    #[test]
    fn bilinear_upscale_2x_is_cosited() {
        let d = vec![0u8, 100, 200, 60]; // 2x2
        let (out, w, h) = bilinear_upscale_2x(&view(&d, 2, 2));
        assert_eq!((w, h), (4, 4));
        // Even coords replicate the source exactly (co-sited).
        assert_eq!(out[0], 0);
        assert_eq!(out[2], 100);
        assert_eq!(out[2 * 4], 200);
        assert_eq!(out[2 * 4 + 2], 60);
        // Odd coords are neighbor means.
        assert_eq!(out[1], 50);
        assert_eq!(out[4], 100); // vertical mean of 0 and 200
    }

    #[test]
    fn bilinear_upscale_3x_is_cosited() {
        let d = vec![0u8, 90, 210, 30]; // 2x2
        let (out, w, h) = bilinear_upscale_3x(&view(&d, 2, 2));
        assert_eq!((w, h), (6, 6));
        // Multiples of 3 replicate the source exactly (co-sited).
        assert_eq!(out[0], 0);
        assert_eq!(out[3], 90);
        assert_eq!(out[3 * 6], 210);
        assert_eq!(out[3 * 6 + 3], 30);
        // Intermediate columns: round((2a+b)/3), round((a+2b)/3).
        assert_eq!(out[1], 30); // (0+0+90)/3
        assert_eq!(out[2], 60); // (0+180+1)/3 = 60.33 -> 60
                                // Intermediate rows: same weights vertically.
        assert_eq!(out[6], 70); // (2*0+210+1)/3 = 70.33 -> 70
        assert_eq!(out[2 * 6], 140); // (0+2*210+1)/3 = 140.33 -> 140
                                     // Last row/col clamp: bottom-right corner replicates in[1][1].
        assert_eq!(out[5 * 6 + 5], 30);
    }

    #[test]
    fn bilinear_upscale_4x_is_cosited() {
        let d = vec![0, 100, 200, 255];
        let (out, w, h) = bilinear_upscale_4x(&view(&d, 2, 2));
        assert_eq!((w, h), (8, 8));
        for sy in 0..2 {
            for sx in 0..2 {
                assert_eq!(out[(4 * sy) * w + 4 * sx], d[sy * 2 + sx]);
            }
        }
    }

    #[test]
    fn catmull_rom_upscale_2x_matches_weights_and_cosites() {
        // 1-D ramp with a step: verify the (9(b+c)-(a+d)+8)>>4 midpoint.
        let d = vec![10u8, 10, 10, 200, 200, 200];
        let (out, w, h) = catmull_rom_upscale_2x(&view(&d, 6, 1));
        assert_eq!((w, h), (12, 2));
        // Even samples replicate.
        for x in 0..6 {
            assert_eq!(out[2 * x], d[x]);
        }
        // Midpoint between the two flat runs: a=10,b=10,c=200,d=200:
        // (9*210 - 210 + 8) >> 4 = (1890-210+8)>>4 = 1688>>4 = 105.
        assert_eq!(out[5], 105);
        // Overshoot halo just before the step: a=10,b=10,c=10,d=200:
        // (9*20 - 210 + 8)>>4 = -22>>4 -> clamps at 0 after >>4? ((180-210+8)>>4)
        // = (-22)>>4 = -2 -> clamped to 0... verify clamp engaged (< b).
        assert!(out[3] < 10, "negative lobe must undershoot before the edge");
    }

    #[test]
    fn background_divide_flattens_a_gradient() {
        // Left half dimmed 4x: after division both halves' ink/paper
        // separation should be comparable.
        let (w, h) = (64, 32);
        let d: Vec<u8> = (0..w * h)
            .map(|i| {
                let x = i % w;
                let base = if (x / 4) % 2 == 0 { 200u32 } else { 60u32 };
                if x < w / 2 {
                    (base / 4) as u8
                } else {
                    base as u8
                }
            })
            .collect();
        // SE = 9 px: wider than the 4 px dark stripes (ink), narrower than
        // the 32 px shadowed half.
        let (out, ow, oh) = background_divide(&view(&d, w, h), 9);
        assert_eq!((ow, oh), (w, h));
        let mid = h / 2;
        let row = &out[mid * w..(mid + 1) * w];
        let contrast = |xs: std::ops::Range<usize>| {
            let (mut lo, mut hi) = (255u8, 0u8);
            for x in xs {
                lo = lo.min(row[x]);
                hi = hi.max(row[x]);
            }
            hi - lo
        };
        // Stay one SE away from the shadow edge and the border (the
        // closing's error band on the dark side of an edge).
        let dark_side = contrast(9..w / 2 - 9);
        let lit_side = contrast(w / 2 + 9..w - 9);
        // Shadowed half regains at least half the lit half's contrast
        // (before division it had exactly a quarter).
        assert!(
            dark_side as u32 * 2 >= lit_side as u32,
            "dark {dark_side} vs lit {lit_side}"
        );
    }

    #[test]
    fn running_extremum_matches_naive_window() {
        // Deterministic pseudo-random signal (LCG), window 7, both polarities.
        let n = 100;
        let mut x: u32 = 12345;
        let src: Vec<u8> = (0..n)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x >> 24) as u8
            })
            .collect();
        let k = 7;
        let r = k / 2;
        for is_max in [true, false] {
            let (mut pre, mut suf, mut out) = (vec![0u8; n], vec![0u8; n], vec![0u8; n]);
            if is_max {
                running_extremum_1d::<true>(&src, k, &mut pre, &mut suf, &mut out);
            } else {
                running_extremum_1d::<false>(&src, k, &mut pre, &mut suf, &mut out);
            }
            // Interior positions (border windows deliberately read the whole
            // touching block — see the function doc).
            for i in k..n - k {
                let win = &src[i - r..=i + r];
                let want = if is_max {
                    *win.iter().max().unwrap()
                } else {
                    *win.iter().min().unwrap()
                };
                assert_eq!(out[i], want, "i={i} is_max={is_max}");
            }
        }
    }

    #[test]
    fn morph_rect_matches_naive_extremum_in_the_interior() {
        let (w, h) = (24, 18);
        let mut x: u32 = 99;
        let src: Vec<u8> = (0..w * h)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x >> 24) as u8
            })
            .collect();
        let k = 5;
        let r = k / 2;
        for is_max in [true, false] {
            let mut dst = Vec::new();
            if is_max {
                morph_rect::<true>(&src, w, h, k, &mut dst);
            } else {
                morph_rect::<false>(&src, w, h, k, &mut dst);
            }
            for y in k..h - k {
                for xx in k..w - k {
                    let mut want = if is_max { 0u8 } else { 255u8 };
                    for dy in y - r..=y + r {
                        for dx in xx - r..=xx + r {
                            let v = src[dy * w + dx];
                            want = if is_max { want.max(v) } else { want.min(v) };
                        }
                    }
                    assert_eq!(dst[y * w + xx], want, "({xx},{y}) is_max={is_max}");
                }
            }
        }
    }

    #[test]
    fn background_divide_envelope_never_saturates() {
        // B >= I pointwise for a closing, so I*200/B <= 200 wherever the
        // denominator floor is not active.
        let (w, h) = (32, 32);
        let d: Vec<u8> = (0..w * h).map(|i| (i % 251) as u8).collect();
        let (out, _, _) = background_divide(&view(&d, w, h), 7);
        for (i, (&o, &s)) in out.iter().zip(d.iter()).enumerate() {
            if s >= 8 {
                assert!(o <= 200, "i={i} out={o} src={s}");
            }
        }
    }

    #[test]
    fn unsharp_mask_steepens_an_edge_without_moving_it() {
        let (w, h) = (16, 5);
        let d: Vec<u8> = (0..w * h)
            .map(|i| {
                let x = i % w;
                match x {
                    0..=6 => 60,
                    7 => 100,
                    8 => 160,
                    _ => 200,
                }
            })
            .collect();
        let (out, _, _) = unsharp_mask(&view(&d, w, h));
        let row = &out[2 * w..3 * w];
        // Contrast across the ramp grows; plateau values pushed apart.
        assert!(row[5] <= 60);
        assert!(row[10] >= 200);
        // Midpoint ordering preserved (no edge shift): left of ramp darker
        // than right.
        assert!(row[7] < row[8]);
    }

    #[test]
    fn structure_tensor_finds_horizontal_smear() {
        // Vertical stripes blurred horizontally = gradients mostly along x
        // survive... actually: horizontal blur kills x-gradients of what it
        // smears; simulate instead an image with ONLY vertical gradients
        // (horizontal stripes): blur direction should come out horizontal
        // (x), since gradients along x are absent.
        let (w, h) = (32, 32);
        let d: Vec<u8> = (0..w * h)
            .map(|i| if (i / w / 4) % 2 == 0 { 30 } else { 220 })
            .collect();
        let (theta, conf) = structure_tensor_blur_direction(&view(&d, w, h));
        assert!(conf > 0.9, "conf {conf}");
        // Gradients are all vertical (major axis = y); blur direction =
        // major + 90° = x, i.e. theta ≈ 0 or π.
        let t = theta.rem_euclid(std::f64::consts::PI);
        assert!(t < 0.1 || (std::f64::consts::PI - t) < 0.1, "theta {t}");
    }

    /// 1-D horizontal box blur of length `len` (float, then rounded) — the
    /// exact PSF the Van Cittert rung models.
    fn hbox_blur(d: &[u8], w: usize, h: usize, len: usize) -> Vec<u8> {
        let half = (len / 2) as isize;
        let mut out = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                for k in -half..=half {
                    let sx = (x as isize + k).clamp(0, w as isize - 1) as usize;
                    sum += d[y * w + sx] as u32;
                }
                out[y * w + x] = (sum / len as u32) as u8;
            }
        }
        out
    }

    #[test]
    fn edge_rise_extent_recovers_a_box_blur_length() {
        // Steps every 32 px (plateaus far wider than the PSF), blurred by
        // an 11-px horizontal box: 20-80% rise = 0.6 * 11 = 6.6 px, so the
        // estimate must come back ~11.
        let (w, h) = (256, 16);
        let d: Vec<u8> = (0..w * h)
            .map(|i| if ((i % w) / 32) % 2 == 0 { 30 } else { 220 })
            .collect();
        let blurred = hbox_blur(&d, w, h, 11);
        let est = edge_rise_extent(&view(&blurred, w, h), 0.0).expect("estimate");
        assert!(
            (est - 11.0).abs() < 2.0,
            "estimated {est}, expected ~11 (box length)"
        );
    }

    #[test]
    fn edge_rise_extent_returns_none_on_flat() {
        let d = vec![128u8; 64 * 64];
        assert!(edge_rise_extent(&view(&d, 64, 64), 0.0).is_none());
    }

    #[test]
    fn van_cittert_restores_amplitude_under_a_supra_module_smear() {
        // The failure class the rung exists for: stripes of 8-px half
        // period under a 13-px box smear (blur LONGER than the module).
        // The box MTF at the stripe fundamental is H = sin(13π/16) /
        // (13·sin(π/16)) ≈ 0.22, collapsing the ±85 square wave to ~±19 —
        // near the binarizer's contrast floor. Three matched Van Cittert
        // iterations apply the truncated-inverse gain (1−(1−H)⁴)/H ≈ 2.9,
        // so the restored amplitude must come back ≥ 2× the blurred one.
        let (w, h) = (160, 8);
        let d: Vec<u8> = (0..w * h)
            .map(|i| if ((i % w) / 8) % 2 == 0 { 40 } else { 210 })
            .collect();
        let blurred = hbox_blur(&d, w, h, 13);
        let (rest, rw, rh) = van_cittert_directional(&view(&blurred, w, h), 0.0, 13);
        assert_eq!((rw, rh), (w, h));
        let amp = |img: &[u8]| -> i32 {
            let row = &img[4 * w..5 * w];
            let (mut lo, mut hi) = (255u8, 0u8);
            for &v in &row[24..w - 24] {
                lo = lo.min(v);
                hi = hi.max(v);
            }
            hi as i32 - lo as i32
        };
        let (a_blur, a_rest) = (amp(&blurred), amp(&rest));
        assert!(
            a_rest >= 2 * a_blur,
            "restored amplitude {a_rest} must be >= 2x blurred {a_blur}"
        );
    }

    #[test]
    fn van_cittert_is_identity_on_the_constant_axis() {
        // Horizontal stripes deconvolved ALONG the stripes (x): every
        // horizontal line is constant, so box mean == value and the
        // iteration is a fixed point.
        let (w, h) = (32, 32);
        let d: Vec<u8> = (0..w * h)
            .map(|i| if (i / w / 4) % 2 == 0 { 30 } else { 220 })
            .collect();
        let (out, _, _) = van_cittert_directional(&view(&d, w, h), 0.0, 7);
        assert_eq!(out, d);
    }

    #[test]
    fn directional_unsharp_only_touches_the_chosen_axis() {
        // Horizontal ramp, sharpen along x: values change. Sharpen along y:
        // unchanged (every vertical line is constant).
        let (w, h) = (16, 8);
        let d: Vec<u8> = (0..w * h).map(|i| ((i % w) * 12).min(255) as u8).collect();
        let v = view(&d, w, h);
        let (along_y, _, _) = directional_unsharp(&v, std::f64::consts::FRAC_PI_2, 5);
        assert_eq!(
            along_y, d,
            "sharpening along the constant axis must be identity"
        );
        let (along_x, _, _) = directional_unsharp(&v, 0.0, 5);
        assert_ne!(along_x, d);
    }

    /// Reference implementation of the exact fractional box integral —
    /// the naive per-destination-pixel double loop the optimized
    /// separable sweep must match byte-for-byte on every ratio shape
    /// (two-tap fast path, generic path, dw == w passthrough shape).
    fn area_downscale_reference(src: &LumaView, dw: usize, dh: usize) -> Vec<u8> {
        let (w, h) = (src.width(), src.height());
        let (fx, fy) = ((w as u64) << 8, (h as u64) << 8);
        let mut out = vec![0u8; dw * dh];
        for oy in 0..dh {
            let y0 = fy * oy as u64 / dh as u64;
            let y1 = fy * (oy as u64 + 1) / dh as u64;
            for ox in 0..dw {
                let x0 = fx * ox as u64 / dw as u64;
                let x1 = fx * (ox as u64 + 1) / dw as u64;
                let (mut acc, mut weight) = (0u64, 0u64);
                for sy in (y0 >> 8) as usize..(((y1 + 255) >> 8) as usize).min(h) {
                    let wy = (((sy + 1) as u64) << 8).min(y1) - ((sy as u64) << 8).max(y0);
                    for sx in (x0 >> 8) as usize..(((x1 + 255) >> 8) as usize).min(w) {
                        let wx = (((sx + 1) as u64) << 8).min(x1) - ((sx as u64) << 8).max(x0);
                        acc += src.get(sx, sy) as u64 * wx * wy;
                        weight += wx * wy;
                    }
                }
                out[oy * dw + ox] = ((acc + weight / 2) / weight) as u8;
            }
        }
        out
    }

    #[test]
    fn area_downscale_matches_reference_on_every_path_shape() {
        // Deterministic pseudo-texture (LCG) so every weight combination
        // is exercised, not just flat fields.
        let (w, h) = (97usize, 61usize);
        let mut seed = 0x2545F491u32;
        let d: Vec<u8> = (0..w * h)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect();
        let v = view(&d, w, h);
        // (2/3-class ratio: two-tap fast path), (deep ratio: generic
        // path), (identity dims), (mixed per-axis ratios).
        for (dw, dh) in [(65, 41), (24, 15), (97, 61), (48, 61)] {
            let (fast, ow, oh) = area_downscale(&v, dw, dh);
            assert_eq!((ow, oh), (dw, dh));
            assert_eq!(
                fast,
                area_downscale_reference(&v, dw, dh),
                "mismatch at {dw}x{dh}"
            );
        }
    }

    #[test]
    fn area_downscale_averages_exact_fractions() {
        // 3x1 -> 2x1 at ratio 2/3: dst0 covers src [0, 1.5) = px0 + half of
        // px1; dst1 covers [1.5, 3) = half of px1 + px2.
        let d = vec![90u8, 30, 210];
        let (out, w, h) = area_downscale(&view(&d, 3, 1), 2, 1);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, vec![70, 150]); // (90 + 15)/1.5 = 70; (15 + 210)/1.5 = 150
    }

    #[test]
    fn area_downscale_integer_ratio_matches_box_half() {
        let d: Vec<u8> = (0..16u8).map(|i| i * 16).collect(); // 4x4
        let v = view(&d, 4, 4);
        let (a, _, _) = area_downscale(&v, 2, 2);
        let (b, _, _) = box_downscale_half(&v).unwrap();
        assert_eq!(a, b, "exact /2 must agree with the pyramid kernel");
    }
}
