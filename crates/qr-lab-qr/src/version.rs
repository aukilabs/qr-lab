//! Version cross-checks: two independent ways to confirm (or correct) a
//! triplet's dimension estimate before committing to a sampling grid.
//!
//! - [`count_timing_transitions`]: for any version, ISO 18004's timing
//!   patterns (row 6 / column 6) alternate dark/light in a fixed,
//!   version-independent way; counting polarity transitions along the
//!   whole row/column and comparing against the derived formula below
//!   either confirms the triplet's `dimension` or flags it as wrong.
//! - [`read_version_bits`] + [`bch_decode_version`]: for v >= 7, the QR
//!   symbol itself encodes its version as an 18-bit BCH(18,6) codeword in
//!   two redundant 6x3 blocks (ISO 18004 §8.10) — authoritative over any
//!   geometric estimate when it decodes (see Plan 4's recorded
//!   `ver_12_v40` finding: the triplet estimate can be off by more than the
//!   timing check's tolerance at high versions).
//!
//! Both are read from the image via a caller-supplied
//! [`PerspectiveTransform`] mapping the *code's own* `dimension x dimension`
//! module square, normalized to the unit square `[0,1]x[0,1]`, to image
//! pixel space — i.e. `transform.map(col / dim, row / dim)` gives the pixel
//! location of module-space point `(col, row)`. This is the same transform
//! convention `sample.rs` (Plan 4 Task 4) will build as
//! `provisional_transform`; that function doesn't exist yet in this task,
//! so both functions here take the transform directly as an argument
//! rather than building it from a `TripletCandidate` themselves (a
//! deliberate, documented deviation from the brief's literal
//! `count_timing_transitions(view, grid, t)` signature — the brief's own
//! prose immediately qualifies it: "for this task take a
//! `&PerspectiveTransform` argument").
//!
//! Task 5 (`decode.rs`'s per-candidate orchestration) wires this file's
//! `pub(crate)` API in — the module-level `dead_code` exemption that used
//! to live here (while nothing outside this file's own tests called any of
//! it) is removed now that `decode.rs` is a real caller.

use crate::homography::PerspectiveTransform;
use crate::tiles::TileGrid;
use crate::triplet::TripletCandidate;
use crate::LumaView;
use qr_lab_geometry::{sample_bilinear, BorderMode};

/// BCH(18,6) version-info codewords for versions 7..=40 (index 0 = v7,
/// index 33 = v40), transcribed verbatim from zxing's
/// `core/src/main/java/com/google/zxing/qrcode/decoder/Version.java`
/// `VERSION_DECODE_INFO` table — itself the ISO 18004 Annex D version
/// information reference values (each codeword is 6 data bits + 12 BCH
/// parity bits for the generator polynomial `x^12+x^11+x^10+x^9+x^8+x^5+x^2+1`).
/// Cross-checked two ways in `tests`: the brief's three pinned exact values
/// (v7/v12/v40), and the code's defining property — minimum pairwise
/// Hamming distance across all 34 entries is exactly 8 (ISO 18004 §D.2),
/// which independently confirms a correct transcription (a single mistyped
/// hex digit almost always collapses some pairwise distance below 8).
const VERSION_DECODE_INFO: [u32; 34] = [
    0x07C94, 0x085BC, 0x09A99, 0x0A4D3, 0x0BBF6, 0x0C762, 0x0D847, 0x0E60D, 0x0F928, 0x10B78,
    0x1145D, 0x12A17, 0x13532, 0x149A6, 0x15683, 0x168C9, 0x177EC, 0x18EC4, 0x191E1, 0x1AFAB,
    0x1B08E, 0x1CC1A, 0x1D33F, 0x1ED75, 0x1F250, 0x209D5, 0x216F0, 0x228BA, 0x2379F, 0x24B0B,
    0x2542E, 0x26A64, 0x27541, 0x28C69,
];

/// Decode an 18-bit version-info reading against [`VERSION_DECODE_INFO`],
/// zxing's `Version.decodeVersionInformation` algorithm: find the table
/// entry at minimum Hamming distance from `bits`, accept it if that
/// distance is `<= 3` (ISO 18004's BCH(18,6) corrects up to 3 errors; its
/// minimum codeword distance of 8 means at most one entry can ever be
/// within 3 of a given `bits` — by the triangle inequality, two codewords
/// both within 3 of the same word would be at most 6 apart, contradicting
/// the minimum distance of 8 — so "accept at <=3" and "unique" are the same
/// condition here, not two separate checks).
pub(crate) fn bch_decode_version(bits: u32) -> Option<u32> {
    let mut best_version = 0u32;
    let mut best_diff = u32::MAX;
    for (i, &codeword) in VERSION_DECODE_INFO.iter().enumerate() {
        if codeword == bits {
            return Some(i as u32 + 7);
        }
        let diff = (bits ^ codeword).count_ones();
        if diff < best_diff {
            best_diff = diff;
            best_version = i as u32 + 7;
        }
    }
    (best_diff <= 3).then_some(best_version)
}

/// Reverse the low `n` bits of `bits` (used for the version-block "other
/// bit order" — see [`read_version_bits`]).
fn reverse_bits(bits: u32, n: u32) -> u32 {
    let mut out = 0u32;
    let mut b = bits;
    for _ in 0..n {
        out = (out << 1) | (b & 1);
        b >>= 1;
    }
    out
}

/// Map a module-space coordinate to its rounded destination pixel, `None`
/// when it falls outside the image — the shared bounds-check/rounding logic
/// behind both [`sample_module_ink`] (polarity-aware bit) and
/// [`sample_module_gray`] (raw luma), so the two stay pixel-identical in
/// which module center they read.
fn sample_pixel_coords(
    view: &LumaView,
    transform: &PerspectiveTransform,
    dimension: u32,
    col: f64,
    row: f64,
) -> Option<(usize, usize)> {
    let dim = dimension as f64;
    let [px, py] = transform.map(col / dim, row / dim);
    let (w, h) = (view.width() as isize, view.height() as isize);
    let (xi, yi) = (px.round() as isize, py.round() as isize);
    if xi < 0 || yi < 0 || xi >= w || yi >= h {
        return None;
    }
    Some((xi as usize, yi as usize))
}

/// Sample one module center in image space and read its polarity-aware ink
/// state (`true` = dark). `col`/`row` are fractional module-space
/// coordinates (e.g. `i + 0.5`); `dimension` normalizes them into
/// `transform`'s unit-square domain. `None` when the mapped pixel falls
/// outside the image — the caller's walk is then unreliable and must abort
/// rather than silently clamp (a clamped read would silently repeat an
/// edge pixel's value instead of reporting missing data).
///
/// `pub(crate)` (not private) so `alignment.rs`'s concentric re-centering
/// probe (Plan 4 Task 3) can reuse this exact sampling convention instead
/// of duplicating it — same `transform.map(col / dim, row / dim)` contract
/// documented above the module-doc's "read from the image" paragraph.
pub(crate) fn sample_module_ink(
    view: &LumaView,
    grid: &TileGrid,
    transform: &PerspectiveTransform,
    dimension: u32,
    col: f64,
    row: f64,
    inverted: bool,
) -> Option<bool> {
    let (xu, yu) = sample_pixel_coords(view, transform, dimension, col, row)?;
    Some((view.get(xu, yu) < grid.threshold_at(xu, yu)) != inverted)
}

/// Sample one module center's RAW gray value (0..255, widened to `f32`),
/// with no tile-threshold binarization — Plan 4B Fix B's per-module
/// evidence for the reference-threshold + sharpening decode round
/// (`sample.rs`'s `SampledGrid::grays`, `bitmatrix.rs`'s
/// `build_reference_threshold_bits`). Same coordinate convention and OOB
/// contract as [`sample_module_ink`] (`None` outside the image), just
/// without the `grid`/`inverted` binarization inputs it doesn't need.
pub(crate) fn sample_module_gray(
    view: &LumaView,
    transform: &PerspectiveTransform,
    dimension: u32,
    col: f64,
    row: f64,
) -> Option<f32> {
    let (xu, yu) = sample_pixel_coords(view, transform, dimension, col, row)?;
    Some(view.get(xu, yu) as f32)
}

/// Sample one module center's RAW gray value via BILINEAR interpolation of
/// its 4 neighboring pixels, rather than [`sample_module_gray`]'s
/// nearest-neighbor round. Plan 5 Task 2's source-resolution module
/// sampling: `sample.rs`'s `sample_grid` uses this (not the
/// nearest-neighbor primitive) when reading module GRAYS straight off the
/// SOURCE image through a lifted transform, since a far/small code can sit
/// at only ~2 source px/module, where nearest-neighbor's single-pixel read
/// is materially noisier than interpolating across its neighbors.
///
/// Same `transform.map(col / dim, row / dim)` coordinate convention as
/// [`sample_module_gray`]. `None` when the mapped point (or any of the 4
/// pixels bilinear interpolation needs) falls outside `[0, w-1] x [0,
/// h-1]` — i.e. strictly stricter by half a pixel on each edge than the
/// nearest-neighbor primitives' own `xi >= 0 && xi < w` bounds (which admit
/// a mapped point up to `w - 0.5`, rounding down to pixel `w-1`): bilinear
/// interpolation genuinely needs a real pixel on both sides, so there is no
/// clamped-edge fallback here either, for the same "missing data must not
/// be silently repeated" reason [`sample_pixel_coords`]'s doc gives.
pub(crate) fn sample_module_gray_bilinear(
    view: &LumaView,
    transform: &PerspectiveTransform,
    dimension: u32,
    col: f64,
    row: f64,
) -> Option<f32> {
    let dim = dimension as f64;
    let [px, py] = transform.map(col / dim, row / dim);
    sample_bilinear(*view, px, py, BorderMode::Reject).map(|value| value as f32)
}

/// Walk one timing axis (row 6 across all columns if `horizontal`, else
/// column 6 across all rows) and count polarity value changes between
/// consecutive module-center samples. `None` if any sample's mapped pixel
/// falls outside the image.
fn walk_timing_axis(
    view: &LumaView,
    grid: &TileGrid,
    transform: &PerspectiveTransform,
    dimension: u32,
    inverted: bool,
    horizontal: bool,
) -> Option<u32> {
    let mut prev: Option<bool> = None;
    let mut transitions = 0u32;
    for i in 0..dimension {
        let (col, row) = if horizontal {
            (i as f64 + 0.5, 6.5)
        } else {
            (6.5, i as f64 + 0.5)
        };
        let ink = sample_module_ink(view, grid, transform, dimension, col, row, inverted)?;
        if let Some(p) = prev {
            if p != ink {
                transitions += 1;
            }
        }
        prev = Some(ink);
    }
    Some(transitions)
}

/// Count timing-pattern polarity transitions along both row 6 and column 6
/// (the full `0..dimension` span of each, not just the strip between the
/// finders — see the derivation below) and return the common count when
/// both axes agree.
///
/// # Derivation: transitions = dimension − 13
///
/// ISO 18004 §7.3.6 places identical timing patterns at module row 6 and
/// module column 6, alternating dark/light — but only in the *strip*
/// between the two finder patterns (modules 8..=(dim-9)); the row/column
/// also passes directly through both finders' own bottom/right border
/// rings and their separators. Walking the *entire* row (or column), for
/// any valid QR dimension `dim` (always odd, `dim = 17 + 4*version`):
///
/// | span                              | width         | content                          | transitions here |
/// |------------------------------------|---------------|-----------------------------------|-------------------|
/// | modules `0..=6`                    | 7             | finder's own border ring, all dark | 0 (uniform)        |
/// | module `7`                         | 1             | separator, light                   | 1 (dark -> light, entering) |
/// | modules `8..=(dim-9)`              | `dim-16`      | timing strip, alternating, starts & ends dark (an odd-length alternation starting dark ends dark; `dim-16` is odd since `dim` is odd and 16 is even) | `1` (light -> dark, entering the strip) + `(dim-16)-1` (internal alternation) + `1` (dark -> light, leaving the strip) |
/// | module `dim-8`                     | 1             | separator, light                   | 0 (continues the parity already set by the strip's exit — no extra transition) |
/// | modules `(dim-7)..=(dim-1)`        | 7             | opposite finder's own border ring, all dark | `1` (light -> dark, entering) + 0 (uniform) |
///
/// Total = `1 + [1 + ((dim-16)-1) + 1] + 1 = dim - 13`.
///
/// Verified two ways (see `tests`): directly against `qrcode`-crate
/// matrices for v1..=6 with no image/sampling involved at all (pins the
/// formula against ground truth), then again through a full image
/// render + transform walk for a v3 code (exercises this function itself).
///
/// # Signature note
/// The brief's `Interfaces` line lists this as
/// `count_timing_transitions(view, grid, t: &TripletCandidate)`, but its
/// own prose immediately qualifies that for this task (before `sample.rs`
/// exists to build a transform from a triplet) it takes a
/// `&PerspectiveTransform` argument directly — implemented that way here;
/// `t` is still taken, for its `dimension`/`inverted` fields.
pub(crate) fn count_timing_transitions(
    view: &LumaView,
    grid: &TileGrid,
    t: &TripletCandidate,
    transform: &PerspectiveTransform,
) -> Option<u32> {
    let dim = t.dimension;
    let row = walk_timing_axis(view, grid, transform, dim, t.inverted, true)?;
    let col = walk_timing_axis(view, grid, transform, dim, t.inverted, false)?;
    // Row and column timing patterns are geometrically identical (both
    // derive to `dim - 13`); on a clean render they always agree exactly.
    // Requiring agreement here (rather than, say, averaging or preferring
    // one axis) turns this into an extra cross-check for free: a mismatch
    // means the transform/dimension/threshold combination is untrustworthy
    // for at least one axis, which the caller should treat the same as an
    // out-of-image walk (`None`) rather than silently picking a possibly
    // wrong value.
    (row == col).then_some(row)
}

/// Sample one version-info block's 18 modules and pack them MSB-first, in
/// zxing's exact enumeration order (see [`read_version_bits`]'s doc for the
/// full derivation). `top_right`: `true` for the block near the TR finder
/// (columns `dim-11..=dim-9`, rows `0..=5`), `false` for the block near the
/// BL finder (columns `0..=5`, rows `dim-11..=dim-9`).
fn read_version_block(
    view: &LumaView,
    grid: &TileGrid,
    transform: &PerspectiveTransform,
    dimension: u32,
    inverted: bool,
    top_right: bool,
) -> Option<u32> {
    let mut bits = 0u32;
    if top_right {
        let col_lo = dimension - 11;
        for row in (0..=5u32).rev() {
            for k in 0..3u32 {
                let col = col_lo + 2 - k; // dim-9, dim-10, dim-11 in that order
                let ink = sample_module_ink(
                    view,
                    grid,
                    transform,
                    dimension,
                    col as f64 + 0.5,
                    row as f64 + 0.5,
                    inverted,
                )?;
                bits = (bits << 1) | ink as u32;
            }
        }
    } else {
        let row_lo = dimension - 11;
        for col in (0..=5u32).rev() {
            for k in 0..3u32 {
                let row = row_lo + 2 - k; // dim-9, dim-10, dim-11 in that order
                let ink = sample_module_ink(
                    view,
                    grid,
                    transform,
                    dimension,
                    col as f64 + 0.5,
                    row as f64 + 0.5,
                    inverted,
                )?;
                bits = (bits << 1) | ink as u32;
            }
        }
    }
    Some(bits)
}

/// Read and BCH-decode a QR symbol's version-info bits (v >= 7 only — v<=6
/// carries no version-info blocks at all, see [`count_timing_transitions`]
/// for the estimator that covers that range instead).
///
/// # Bit-order convention
/// Transcribed exactly from zxing's `BitMatrixParser.readVersion` /
/// `copyBit` (not re-derived from the ISO spec text, which only shows the
/// block *positions*, not a sampling order) — verified line-by-line against
/// zxing's loop structure:
///
/// ```java
/// // TR block: ijMin = dimension - 11
/// for (int j = 5; j >= 0; j--)
///   for (int i = dimension - 9; i >= ijMin; i--)
///     versionBits = copyBit(i, j, versionBits);   // copyBit(x=i, y=j, ...)
/// // BL block (only if TR didn't decode):
/// for (int i = 5; i >= 0; i--)
///   for (int j = dimension - 9; j >= ijMin; j--)
///     versionBits = copyBit(i, j, versionBits);   // copyBit(x=i, y=j, ...)
/// ```
/// and `copyBit` does `versionBits = (versionBits << 1) | bit`, so the
/// *first* bit sampled ends up as the codeword's MSB (bit 17) and the last
/// as its LSB (bit 0). Concretely:
/// - **TR block** (columns `dim-11..=dim-9`, rows `0..=5`, near the TR
///   finder): outer loop row `5` downto `0`, inner loop column `dim-9`
///   downto `dim-11` — i.e. NOT row-major; the block is read bottom-to-top,
///   right-to-left.
/// - **BL block** (columns `0..=5`, rows `dim-11..=dim-9`, near the BL
///   finder): outer loop column `5` downto `0`, inner loop row `dim-9`
///   downto `dim-11` — the transposed mirror of the TR order.
///
/// # Both bit orders
/// Per the brief: each block is tried both in the order above and
/// bit-reversed (LSB<->MSB swapped). This is not redundant: a triplet
/// candidate reaching this function has not yet had its mirror/transpose
/// orientation resolved (that only happens once `bitmatrix::decode_bits`
/// runs, later in the pipeline — see Task 1's finding that orientation
/// cannot be inferred from decode success/failure), so the same physical
/// modules may need to be enumerated in the reverse order if the candidate
/// turns out to be transposed. Trying both cheaply and safely relies on
/// the same triangle-inequality argument as [`bch_decode_version`]'s
/// uniqueness note: a false accept at Hamming distance <=3 from two
/// genuinely different codewords is impossible.
///
/// Returns the first of {TR-forward, TR-reversed, BL-forward, BL-reversed}
/// that BCH-decodes successfully.
///
/// The two blocks are sampled independently: if one lands (partially)
/// outside the image (`read_version_block` returns `None`) the other is
/// still tried on its own — a candidate near a frame edge may have only
/// one of its two redundant version-info blocks in-frame, and the whole
/// point of the redundancy is to tolerate exactly that.
pub(crate) fn read_version_bits(
    view: &LumaView,
    grid: &TileGrid,
    transform: &PerspectiveTransform,
    dimension_est: u32,
    inverted: bool,
) -> Option<u32> {
    let tr = read_version_block(view, grid, transform, dimension_est, inverted, true);
    let bl = read_version_block(view, grid, transform, dimension_est, inverted, false);
    [
        tr,
        tr.map(|b| reverse_bits(b, 18)),
        bl,
        bl.map(|b| reverse_bits(b, 18)),
    ]
    .into_iter()
    .flatten()
    .find_map(bch_decode_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testpaint::render_module_grid_transformed;

    // --- sample_module_gray_bilinear (Plan 5 Task 2) ---

    /// Identity module->image transform over a 10x10 canvas (`dimension ==
    /// 10`, so `col`/`row` ARE image pixel coordinates directly) painted
    /// with a horizontal ramp (`gray(x, y) = 10*x`) — lets each assertion
    /// below hand-compute the exact expected interpolated value.
    fn ramp_view_and_transform() -> (Vec<u8>, PerspectiveTransform) {
        let dim = 10usize;
        let data: Vec<u8> = (0..dim * dim).map(|i| (10 * (i % dim)) as u8).collect();
        let transform = PerspectiveTransform::square_to_quad([
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [0.0, 10.0],
        ])
        .unwrap();
        (data, transform)
    }

    #[test]
    fn bilinear_gray_matches_exact_pixel_at_integer_coords() {
        let (data, transform) = ramp_view_and_transform();
        let view = LumaView::new(&data, 10, 10, 10).unwrap();
        // col/row = 3.0 maps to image (3.0, 3.0) exactly -> gray(3,3) = 30.
        let g = sample_module_gray_bilinear(&view, &transform, 10, 3.0, 3.0).unwrap();
        assert!((g - 30.0).abs() < 1e-6, "{g}");
    }

    #[test]
    fn bilinear_gray_interpolates_between_pixels() {
        let (data, transform) = ramp_view_and_transform();
        let view = LumaView::new(&data, 10, 10, 10).unwrap();
        // col = 3.5 -> image x = 3.5, halfway between gray(3,*)=30 and
        // gray(4,*)=40 -> exactly 35. The ramp is constant along y, so the
        // y-fraction doesn't matter here.
        let g = sample_module_gray_bilinear(&view, &transform, 10, 3.5, 4.2).unwrap();
        assert!((g - 35.0).abs() < 1e-6, "{g}");
    }

    #[test]
    fn bilinear_gray_none_outside_the_pixel_center_grid() {
        let (data, transform) = ramp_view_and_transform();
        let view = LumaView::new(&data, 10, 10, 10).unwrap();
        // image x = -0.1: strictly outside [0, w-1].
        assert!(sample_module_gray_bilinear(&view, &transform, 10, -0.1, 3.0).is_none());
        // image x = 9.4 (> w-1 = 9.0): also outside, unlike the
        // nearest-neighbor primitive which would still accept it (rounds to
        // 9) — bilinear's stricter bound (see the function's own doc).
        assert!(sample_module_gray_bilinear(&view, &transform, 10, 9.4, 3.0).is_none());
    }

    #[test]
    fn bilinear_gray_accepts_the_last_pixel_exactly() {
        let (data, transform) = ramp_view_and_transform();
        let view = LumaView::new(&data, 10, 10, 10).unwrap();
        let g = sample_module_gray_bilinear(&view, &transform, 10, 9.0, 9.0).unwrap();
        assert!((g - 90.0).abs() < 1e-6, "{g}");
    }

    // --- BCH core (verbatim per the brief) ---

    #[test]
    fn bch_exact_codewords_decode() {
        assert_eq!(bch_decode_version(0x07C94), Some(7));
        assert_eq!(bch_decode_version(0x0C762), Some(12));
        assert_eq!(bch_decode_version(0x28C69), Some(40));
    }

    #[test]
    fn bch_three_errors_ok_four_rejected() {
        let w = 0x07C94u32;
        assert_eq!(bch_decode_version(w ^ 0b111), Some(7)); // 3 flips
        assert_eq!(bch_decode_version(w ^ 0b1111), None); // 4 flips
    }

    /// Independent confirmation that [`VERSION_DECODE_INFO`] was
    /// transcribed correctly: ISO 18004's BCH(18,6) version code has
    /// minimum pairwise Hamming distance 8 by construction. A mistyped
    /// entry would almost certainly collapse some pair's distance below 8.
    #[test]
    fn version_decode_info_min_distance_is_8() {
        let mut min_dist = u32::MAX;
        for (i, &a) in VERSION_DECODE_INFO.iter().enumerate() {
            for &b in &VERSION_DECODE_INFO[i + 1..] {
                min_dist = min_dist.min((a ^ b).count_ones());
            }
        }
        assert_eq!(min_dist, 8);
    }

    // --- Timing-transition formula, pinned against real matrices first ---

    /// Sample `qrcode`-crate matrices directly (no image, no transform) to
    /// pin the `dim - 13` formula against ground truth before trusting the
    /// image-walk version below.
    #[test]
    fn timing_transitions_formula_matches_real_matrices() {
        for v in 1i16..=6 {
            let code = qrcode::QrCode::with_version(
                b"HELLO",
                qrcode::Version::Normal(v),
                qrcode::EcLevel::M,
            )
            .unwrap();
            let dim = code.width();
            let row_transitions = (1..dim)
                .filter(|&x| {
                    (code[(x, 6)] == qrcode::Color::Dark)
                        != (code[(x - 1, 6)] == qrcode::Color::Dark)
                })
                .count();
            let col_transitions = (1..dim)
                .filter(|&y| {
                    (code[(6, y)] == qrcode::Color::Dark)
                        != (code[(6, y - 1)] == qrcode::Color::Dark)
                })
                .count();
            assert_eq!(row_transitions, dim - 13, "v{v} row");
            assert_eq!(col_transitions, dim - 13, "v{v} col");
        }
    }

    // --- Synthetic-render integration tests ---

    /// Build a `code_to_image` transform mapping a `dim x dim` code's own
    /// module square (normalized `[0,1]x[0,1]`) to an axis-aligned pixel
    /// quad inset by `quiet` modules of margin inside an `img_side x
    /// img_side` canvas, at `scale` px/module.
    fn axis_aligned_transform(dim: usize, scale: f64, quiet: f64) -> (PerspectiveTransform, usize) {
        let img_side = ((dim as f64 + 2.0 * quiet) * scale).round() as usize;
        let x0 = quiet * scale;
        let x1 = x0 + dim as f64 * scale;
        let quad = [[x0, x0], [x1, x0], [x1, x1], [x0, x1]];
        (
            PerspectiveTransform::square_to_quad(quad).unwrap(),
            img_side,
        )
    }

    /// Render `payload` at `version`/`ecc` through `transform` into an
    /// `img_side`-square luma buffer (25/235 ink/light levels), returning
    /// the `(view-backing buffer, dim)` pair.
    fn render(
        payload: &[u8],
        version: i16,
        ecc: qrcode::EcLevel,
        transform: &PerspectiveTransform,
        img_side: usize,
    ) -> (Vec<u8>, usize) {
        let code =
            qrcode::QrCode::with_version(payload, qrcode::Version::Normal(version), ecc).unwrap();
        let dim = code.width();
        let img = render_module_grid_transformed(
            dim,
            |x, y| code[(x, y)] == qrcode::Color::Dark,
            25,
            235,
            transform,
            img_side,
            img_side,
        );
        (img, dim)
    }

    #[test]
    fn read_version_bits_v7_and_v20_axis_aligned() {
        for version in [7i16, 20] {
            let (transform, img_side) = axis_aligned_transform(17 + 4 * version as usize, 4.0, 4.0);
            let (img, dim) = render(
                b"HELLO WORLD",
                version,
                qrcode::EcLevel::M,
                &transform,
                img_side,
            );
            let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
            let grid = TileGrid::build(&view);
            let got = read_version_bits(&view, &grid, &transform, dim as u32, false);
            assert_eq!(got, Some(version as u32), "v{version}");
        }
    }

    /// `read_version_bits` must not give up entirely just because one of
    /// the two redundant version-info blocks is unreachable — crop the
    /// rendered image so only the BL block (near rows `dim-11..dim-9`,
    /// bottom of the code) falls outside the image, leaving the TR block
    /// (near the top, rows `0..5`) intact, and confirm the TR block alone
    /// still recovers the version.
    #[test]
    fn read_version_bits_falls_back_when_one_block_is_out_of_frame() {
        let version = 7i16;
        let dim = 17 + 4 * version as usize; // 45
        let (transform, img_side) = axis_aligned_transform(dim, 4.0, 4.0);
        let (img, _) = render(
            b"CROPPED",
            version,
            qrcode::EcLevel::M,
            &transform,
            img_side,
        );

        // TR block's sampled rows (0..5) land at pixel y in roughly
        // [16 + 0.5*4, 16 + 5.5*4] = [18, 38]; BL block's sampled rows
        // (dim-11..dim-9 = 34..36) land at roughly [154, 162] (quiet=4,
        // scale=4 -> code top at pixel y=16). Cropping to 100 rows keeps
        // the TR block fully in-frame while placing the BL block's rows
        // entirely outside the cropped image.
        let crop_h = 100usize;
        assert!(
            crop_h < img_side,
            "test assumption: crop must be a real crop"
        );
        let cropped = &img[..img_side * crop_h];
        let view = LumaView::new(cropped, img_side, crop_h, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let got = read_version_bits(&view, &grid, &transform, dim as u32, false);
        assert_eq!(got, Some(version as u32));
    }

    #[test]
    fn count_timing_transitions_v3_axis_aligned() {
        let version = 3i16;
        let (transform, img_side) = axis_aligned_transform(17 + 4 * version as usize, 4.0, 4.0);
        let (img, dim) = render(b"HI", version, qrcode::EcLevel::M, &transform, img_side);
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);
        let t = TripletCandidate {
            tl: [0.0, 0.0],
            tr: [0.0, 0.0],
            bl: [0.0, 0.0],
            module: 4.0,
            dimension: dim as u32,
            snap_error: 0.0,
            inverted: false,
            finder_indices: [0, 1, 2],
        };
        let got = count_timing_transitions(&view, &grid, &t, &transform);
        assert_eq!(got, Some(dim as u32 - 13));
    }

    /// A v7 code rendered rotated 30° in-plane (via a homography built from
    /// a rotated pixel-space quad, not an axis-aligned one) — proves the
    /// transform-based module-center sampling isn't secretly axis-locked:
    /// both `read_version_bits` and `count_timing_transitions` must still
    /// recover the right answer when every sample point is off-axis.
    #[test]
    fn rotated_30_degrees_still_reads_version_and_timing() {
        let version = 7i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let side_px = dim as f64 * scale;
        let img_side = 400usize;
        let center = img_side as f64 / 2.0;
        let angle = 30.0f64.to_radians();
        let (s, c) = angle.sin_cos();
        let half = side_px / 2.0;
        // Axis-aligned corners relative to center, then rotated.
        let corners = [[-half, -half], [half, -half], [half, half], [-half, half]];
        let quad: [[f64; 2]; 4] =
            corners.map(|[dx, dy]| [center + dx * c - dy * s, center + dx * s + dy * c]);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let (img, dim_rendered) = render(
            b"ROTATED",
            version,
            qrcode::EcLevel::M,
            &transform,
            img_side,
        );
        assert_eq!(dim_rendered, dim);
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let got_version = read_version_bits(&view, &grid, &transform, dim as u32, false);
        assert_eq!(got_version, Some(version as u32));

        let t = TripletCandidate {
            tl: [0.0, 0.0],
            tr: [0.0, 0.0],
            bl: [0.0, 0.0],
            module: scale,
            dimension: dim as u32,
            snap_error: 0.0,
            inverted: false,
            finder_indices: [0, 1, 2],
        };
        let got_timing = count_timing_transitions(&view, &grid, &t, &transform);
        assert_eq!(got_timing, Some(dim as u32 - 13));
    }
}
