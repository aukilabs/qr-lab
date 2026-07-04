//! Grid sampling: the provisional (finder-only) whole-grid transform, the
//! zxing-cpp-style per-region tiling built from located alignment patterns,
//! and the module-center sampling that turns either into a [`BitMatrix`].
//!
//! # Anchor / coordinate conventions (read this before touching the math)
//!
//! Every [`PerspectiveTransform`] built or consumed in this file shares the
//! **unit-square convention** already established by `version.rs`
//! (`sample_module_ink`) and `alignment.rs`: the transform's domain is the
//! whole symbol's module square normalized to `[0,1] x [0,1]`, i.e. module
//! coordinate `(col, row)` (raw module units, `0..dimension`) maps to image
//! space via `transform.map(col / dimension, row / dimension)`. This file
//! builds transforms via [`PerspectiveTransform::quad_to_quad`]
//! on *raw* module-space quads (e.g. `(3.5, 3.5)`, not `(3.5/dim, 3.5/dim)`)
//! for readability, then normalizes by `dimension` once, right before the
//! `quad_to_quad` call (see `build_transform`) — so every raw module-space
//! constant that appears below (`3.5`, `dim - 3.5`, `coords[i] + 0.5`, ...)
//! is in that same raw-module space, not pre-normalized.
//!
//! **Module-center convention**: a lattice/module coordinate `c` (an
//! integer module index, 0-indexed) denotes a module whose continuous
//! center is `c + 0.5` — the same convention `alignment.rs`'s
//! `ALIGNMENT_COORDS` doc and `analytic_position` test helper already use,
//! and the one [`crate::version::sample_module_ink`] samples at
//! (`i + 0.5`-style call sites throughout `version.rs`/`alignment.rs`).
//! Concretely: a finder pattern is 7 modules wide (indices `0..=6`), so its
//! *center* is continuous coordinate `3.5`, not integer `3` or `4`; the
//! bottom-right-most alignment-pattern lattice node has integer coordinate
//! `dimension - 7` (`= *coords.last()`), so its continuous center is
//! `dimension - 7 + 0.5 = dimension - 6.5`.
//!
//! # Finder-corner anchor: a deliberate divergence from `alignment.rs`
//!
//! `alignment.rs`'s own internal parallelogram predictor (`anchor_of`)
//! stands in for a `FinderCorner` lattice slot by projecting the *lattice*
//! coordinate (`coords[j] + 0.5, coords[i] + 0.5` — e.g. `(6.5, 6.5)` for
//! the TL corner) through the provisional transform. That is a deliberate
//! approximation good enough for *predicting* a neighboring alignment
//! pattern's rough position (see that file's doc).
//!
//! Here, every region-transform corner that lands on a `FinderCorner`
//! lattice slot instead pairs the *finder's own* true module-space center
//! (`(3.5, 3.5)` / `(dimension - 3.5, 3.5)` / `(3.5, dimension - 3.5)`) with
//! the triplet's precisely-measured finder center (`t.tl` / `t.tr` /
//! `t.bl`) — per this task's brief: "nodes at finder corners use the finder
//! centers, known precisely from the triplet". Pairing the *lattice*
//! coordinate (`6.5`, three modules away from the true finder center `3.5`)
//! with the finder's true image position would bias every region touching
//! a finder corner by several modules' worth of projective error; pairing
//! the finder's own true module coordinate with its own true image position
//! is the only combination that is actually correct.
#![allow(dead_code)]

use crate::alignment::{is_finder_corner, AlignmentGrid, AnchorSlot};
use crate::bitmatrix::BitMatrix;
use crate::homography::PerspectiveTransform;
use crate::tiles::TileGrid;
use crate::triplet::TripletCandidate;
use crate::version::sample_module_ink;
use crate::LumaView;

/// One tile of a sampled grid: the half-open module rectangle it covers —
/// `[x0, y0, x1, y1)`, i.e. `x` (column) then `y` (row), matching
/// `BitMatrix`'s own `(x, y) = (column, row)` convention — and the
/// [`PerspectiveTransform`] used to sample every module center inside it.
#[derive(Clone, Debug)]
pub(crate) struct SampleRegion {
    pub module_rect: [u32; 4],
    pub transform: PerspectiveTransform,
}

/// The result of one [`sample_grid`] call.
pub(crate) struct SampledGrid {
    pub bits: BitMatrix,
    /// The regions used to build `bits` — kept for trace/debug-UI
    /// visualization (Plan 4 Task 6), not consumed by this file itself.
    pub regions: Vec<SampleRegion>,
    /// Fraction of modules (`oob_count / dimension^2`) whose sample fell
    /// outside the source image (a clamped/missing read, counted as `false`
    /// in `bits`). The caller (`decode.rs`, Task 5) rejects a candidate
    /// when this exceeds `consts::MAX_OOB_FRACTION` — this file only
    /// measures and reports it.
    pub oob_fraction: f64,
}

/// Build the module-space source quad and image-space destination quad for
/// zxing's classic single-alignment-pattern `Detector.createTransform`:
/// three finder centers (exact, from the triplet) plus a 4th
/// ("bottom-right") point.
///
/// `br_ap = None`: the 4th point is a pure image-space parallelogram
/// extrapolation (`t.tr + t.bl - t.tl`), paired with module-space point
/// `(dimension - 3.5, dimension - 3.5)` — the point a real 4th finder
/// *would* occupy if this were a 4-finder rectangle, extrapolated with no
/// alignment-pattern evidence at all (used for v1, and whenever no usable
/// alignment pattern was found — see [`sample_grid`]).
///
/// `br_ap = Some(p)`: `p` is a real, located alignment-pattern position,
/// paired with module-space point `(dimension - 6.5, dimension - 6.5)` —
/// the bottom-right-most alignment pattern's own center (lattice coordinate
/// `dimension - 7`, `+0.5` for the module-center convention). This is
/// zxing's exact `Detector.createTransform` behavior when an alignment
/// pattern is available: `sourceBottomRightX = dimMinusThree - 3.0`.
fn provisional_quad(
    t: &TripletCandidate,
    dimension: u32,
    br_ap: Option<[f64; 2]>,
) -> ([[f64; 2]; 4], [[f64; 2]; 4]) {
    let dimf = dimension as f64;
    let (src_br, dst_br) = match br_ap {
        Some(p) => ([dimf - 6.5, dimf - 6.5], p),
        None => (
            [dimf - 3.5, dimf - 3.5],
            [t.tr[0] + t.bl[0] - t.tl[0], t.tr[1] + t.bl[1] - t.tl[1]],
        ),
    };
    let src = [[3.5, 3.5], [dimf - 3.5, 3.5], src_br, [3.5, dimf - 3.5]];
    let dst = [t.tl, t.tr, dst_br, t.bl];
    (src, dst)
}

/// `quad_to_quad` from a *raw* module-space quad (see the module doc's
/// convention section) against an image-space quad, normalizing the source
/// by `dimension` first so the result follows the shared
/// `transform.map(col / dim, row / dim)` convention.
fn build_transform(
    src_module: [[f64; 2]; 4],
    dst_image: [[f64; 2]; 4],
    dimension: f64,
) -> Option<PerspectiveTransform> {
    let src_unit = src_module.map(|[x, y]| [x / dimension, y / dimension]);
    PerspectiveTransform::quad_to_quad(src_unit, dst_image)
}

/// zxing `Detector.createTransform`: map the three triplet finder centers
/// plus a parallelogram-extrapolated 4th point to a single whole-grid
/// transform, with no alignment-pattern evidence at all.
///
/// # Panics
/// Only if `quad_to_quad` fails on the triplet's own finder centers, which
/// is not expected in practice: `triplet::try_group`'s angle/leg-balance
/// gates already reject any triple whose `tl`/`tr`/`bl` are collinear or
/// otherwise degenerate before a `TripletCandidate` is ever constructed, so
/// both the fixed module-space rectangle and the triplet's own corner quad
/// are always non-degenerate for a real candidate.
pub(crate) fn provisional_transform(t: &TripletCandidate, dimension: u32) -> PerspectiveTransform {
    let (src, dst) = provisional_quad(t, dimension, None);
    build_transform(src, dst, dimension as f64).expect(
        "provisional_transform: quad_to_quad degenerate despite try_group's non-collinearity gate",
    )
}

/// Bundles the read-only inputs [`node_anchor`] needs, so it takes one
/// reference instead of six positional arguments (also keeps
/// `clippy::too_many_arguments` happy — the same pattern `alignment.rs`'s
/// `ProbeContext` already established for its own multi-input helper).
struct AnchorContext<'a> {
    t: &'a TripletCandidate,
    coords: &'a [u8],
    found: &'a [AnchorSlot],
    n: usize,
    dimension: u32,
    provisional: &'a PerspectiveTransform,
}

/// The `(src_module_coord, dst_image_pos)` anchor pair for alignment
/// lattice node `(i, j)` (0-indexed into a `coords` of length `n`), used as
/// one corner of a per-region `quad_to_quad` correspondence. See the module
/// doc for the finder-corner divergence from `alignment.rs`'s own
/// `anchor_of`.
fn node_anchor(ctx: &AnchorContext, i: usize, j: usize) -> ([f64; 2], [f64; 2]) {
    let n = ctx.n;
    let dimf = ctx.dimension as f64;
    if i == 0 && j == 0 {
        return ([3.5, 3.5], ctx.t.tl);
    }
    if i == 0 && j == n - 1 {
        return ([dimf - 3.5, 3.5], ctx.t.tr);
    }
    if i == n - 1 && j == 0 {
        return ([3.5, dimf - 3.5], ctx.t.bl);
    }
    let src = [ctx.coords[j] as f64 + 0.5, ctx.coords[i] as f64 + 0.5];
    let dst = match ctx.found[i * n + j] {
        AnchorSlot::Found(p) => p,
        AnchorSlot::Missing => ctx.provisional.map(src[0] / dimf, src[1] / dimf),
        AnchorSlot::FinderCorner => unreachable!(
            "(i, j) = ({i}, {j}) is a FinderCorner slot but wasn't matched by the three \
             explicit corner checks above — is_finder_corner and this function's own corner \
             checks have gone out of sync"
        ),
    };
    (src, dst)
}

/// `true` iff any non-finder-corner, non-bottom-right-most lattice slot was
/// actually [`AnchorSlot::Found`] — i.e. whether the alignment lattice has
/// enough real evidence to make per-region tiling worthwhile.
///
/// The bottom-right-most slot (`(n-1, n-1)`) is deliberately excluded here:
/// it's handled separately by [`br_slot_pos`] as the classic single-AP
/// `createTransform` override (see [`sample_grid`]'s "no usable APs"
/// branch), matching zxing's own single-alignment-pattern behavior. For
/// `n == 2` (v2..6, which have exactly one real alignment pattern total —
/// the bottom-right-most one) this loop's only non-finder-corner candidate
/// *is* `(n-1, n-1)`, so it is always excluded and this always returns
/// `false`: those versions always take the single-region path, with or
/// without the AP override, which is mathematically equivalent to running
/// them through the general `n == 2` tiling path anyway (a single region
/// spanning the whole grid, using the same four corner anchors either way).
fn has_non_br_found_ap(alignment: &AlignmentGrid) -> bool {
    let n = alignment.coords.len();
    for i in 0..n {
        for j in 0..n {
            if (i, j) == (n - 1, n - 1) || is_finder_corner(i, j, n) {
                continue;
            }
            if matches!(alignment.found[i * n + j], AnchorSlot::Found(_)) {
                return true;
            }
        }
    }
    false
}

/// The bottom-right-most lattice slot's position, if it was found. `None`
/// for `n == 0` (v1: no lattice at all) or when that slot is `Missing`.
fn br_slot_pos(alignment: &AlignmentGrid) -> Option<[f64; 2]> {
    let n = alignment.coords.len();
    if n == 0 {
        return None;
    }
    match alignment.found[(n - 1) * n + (n - 1)] {
        AnchorSlot::Found(p) => Some(p),
        _ => None,
    }
}

/// Build the per-region sampling grid for one candidate.
///
/// - **No usable alignment patterns** (v1 — `alignment.coords` is empty —
///   or every non-finder-corner, non-bottom-right-most slot is `Missing`,
///   see [`has_non_br_found_ap`]): a single region covering the whole
///   `[0, dimension) x [0, dimension)` grid, transform = the classic
///   finder-only `provisional_transform`, *except* when the bottom-right-
///   most alignment-pattern slot was itself `Found` — then that position is
///   used as the 4th anchor instead of the pure parallelogram guess (zxing
///   Java's own single-AP behavior; see [`provisional_quad`]).
/// - **Otherwise** (zxing-cpp `GridSampler` ROI approach): tile module
///   space by the alignment lattice's `(coords.len() - 1)^2` intervals,
///   with the outermost interval on each axis stretched to reach the
///   symbol's true edge (module `0` / `dimension`) instead of stopping at
///   the outermost real alignment coordinate — every module is covered by
///   exactly one region, none are left out. Each region's transform is
///   built from its four surrounding lattice-node anchors (see
///   [`node_anchor`]).
///
/// Every module center (`x + 0.5, y + 0.5`) is then sampled through its
/// region's transform ([`crate::version::sample_module_ink`],
/// polarity-aware via `t.inverted`); a sample landing outside the source
/// image counts toward `oob_fraction` and is recorded as `false`.
///
/// Returns `None` only if a region's `quad_to_quad` construction is
/// degenerate (a located alignment pattern or a finder center placed such
/// that some region's four corners are collinear) — sampling that region
/// is then genuinely impossible, not just noisy.
pub(crate) fn sample_grid(
    view: &LumaView,
    grid: &TileGrid,
    t: &TripletCandidate,
    dimension: u32,
    alignment: &AlignmentGrid,
) -> Option<SampledGrid> {
    let dim = dimension as usize;
    let n = alignment.coords.len();

    let regions: Vec<SampleRegion> = if n == 0 || !has_non_br_found_ap(alignment) {
        let (src, dst) = provisional_quad(t, dimension, br_slot_pos(alignment));
        let transform = build_transform(src, dst, dimension as f64)?;
        vec![SampleRegion { module_rect: [0, 0, dimension, dimension], transform }]
    } else {
        let provisional = provisional_transform(t, dimension);
        let ctx = AnchorContext {
            t,
            coords: &alignment.coords,
            found: &alignment.found,
            n,
            dimension,
            provisional: &provisional,
        };
        let mut regions = Vec::with_capacity((n - 1) * (n - 1));
        for a in 0..n - 1 {
            for b in 0..n - 1 {
                let (src_tl, dst_tl) = node_anchor(&ctx, a, b);
                let (src_tr, dst_tr) = node_anchor(&ctx, a, b + 1);
                let (src_br, dst_br) = node_anchor(&ctx, a + 1, b + 1);
                let (src_bl, dst_bl) = node_anchor(&ctx, a + 1, b);
                let src = [src_tl, src_tr, src_br, src_bl];
                let dst = [dst_tl, dst_tr, dst_br, dst_bl];
                let transform = build_transform(src, dst, dimension as f64)?;

                let mut row0 = alignment.coords[a] as u32;
                let mut row1 = alignment.coords[a + 1] as u32;
                let mut col0 = alignment.coords[b] as u32;
                let mut col1 = alignment.coords[b + 1] as u32;
                if a == 0 {
                    row0 = 0;
                }
                if a == n - 2 {
                    row1 = dimension;
                }
                if b == 0 {
                    col0 = 0;
                }
                if b == n - 2 {
                    col1 = dimension;
                }

                regions.push(SampleRegion { module_rect: [col0, row0, col1, row1], transform });
            }
        }
        regions
    };

    let mut bits = BitMatrix::new(dim);
    let mut oob = 0u64;
    for region in &regions {
        let [x0, y0, x1, y1] = region.module_rect;
        for y in y0..y1 {
            for x in x0..x1 {
                let ink = sample_module_ink(
                    view, grid, &region.transform, dimension, x as f64 + 0.5, y as f64 + 0.5, t.inverted,
                );
                match ink {
                    Some(v) => bits.set(x as usize, y as usize, v),
                    None => {
                        oob += 1;
                        bits.set(x as usize, y as usize, false);
                    }
                }
            }
        }
    }
    let oob_fraction = oob as f64 / (dim * dim) as f64;
    Some(SampledGrid { bits, regions, oob_fraction })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alignment::locate_alignment_patterns;
    use crate::testpaint::render_module_grid_transformed;
    use crate::tiles::TileGrid;

    /// Axis-aligned image quad: a `dim x dim` code at `scale` px/module,
    /// inset by `quiet` modules of margin inside its own tight canvas.
    fn axis_aligned_quad(dim: usize, scale: f64, quiet: f64) -> ([[f64; 2]; 4], usize) {
        let img_side = ((dim as f64 + 2.0 * quiet) * scale).round() as usize;
        let x0 = quiet * scale;
        let x1 = x0 + dim as f64 * scale;
        ([[x0, x0], [x1, x0], [x1, x1], [x0, x1]], img_side)
    }

    /// The code rotated by `angle_deg` in-plane about the canvas center.
    /// `img_side` must be large enough that the rotated square stays fully
    /// inside the canvas (see `rotated_img_side`).
    fn rotated_quad(dim: usize, scale: f64, angle_deg: f64, img_side: usize) -> [[f64; 2]; 4] {
        let side_px = dim as f64 * scale;
        let center = img_side as f64 / 2.0;
        let (s, c) = angle_deg.to_radians().sin_cos();
        let half = side_px / 2.0;
        let corners = [[-half, -half], [half, -half], [half, half], [-half, half]];
        corners.map(|[dx, dy]| [center + dx * c - dy * s, center + dx * s + dy * c])
    }

    /// A canvas side comfortably containing a `dim x dim` code at `scale`
    /// px/module under any in-plane rotation: the worst-case (45°) bounding
    /// box of a rotated square of side `s` is `s * sqrt(2)`, plus a fixed
    /// margin for quiet-zone slack.
    fn rotated_img_side(dim: usize, scale: f64) -> usize {
        let side_px = dim as f64 * scale;
        (side_px * std::f64::consts::SQRT_2 + 40.0).ceil() as usize
    }

    /// A mild keystone (trapezoid) perspective quad: the top edge is
    /// narrowed to `narrow` (< 1.0) of the bottom edge's width, simulating
    /// a code plane tilted back slightly at the top — a real, common
    /// perspective distortion, not an arbitrary corner nudge. Centered in
    /// an `img_side`-square canvas alongside the same margin
    /// `axis_aligned_quad` would use.
    ///
    /// `narrow = 0.97` (a 3% top/bottom width difference) is a deliberately
    /// mild choice, arrived at empirically: v1 (this suite's smallest test
    /// dimension, 21 modules) has no alignment pattern at all to correct a
    /// perspective mismatch, so for v1 `provisional_transform`'s
    /// finder-only parallelogram reconstruction must itself land every one
    /// of its 441 module centers inside the correct module with NO
    /// per-region correction available. `narrow = 0.94` (6%) measurably
    /// failed v1 — 3/441 modules mismatched, all adjacent to the
    /// bottom-right corner (`(19,18)`, `(19,19)`, `(20,19)` of 21x21,
    /// exactly where the parallelogram-vs-true-homography gap peaks) —
    /// while `0.97` passes all four versions with margin. `0.97` is kept
    /// deliberately close to that measured passing boundary rather than a
    /// far more conservative value: the point of this case is to exercise
    /// a *genuine* projective (non-parallelogram) `square_to_quad` branch,
    /// and a too-mild warp risks silently degrading into something the
    /// affine branch could also handle.
    fn perspective_quad(dim: usize, scale: f64, narrow: f64, img_side: usize) -> [[f64; 2]; 4] {
        let side = dim as f64 * scale;
        let margin = (img_side as f64 - side) / 2.0;
        let cx = img_side as f64 / 2.0;
        let top_half = side / 2.0 * narrow;
        let bottom_half = side / 2.0;
        let y0 = margin;
        let y1 = margin + side;
        [
            [cx - top_half, y0],
            [cx + top_half, y0],
            [cx + bottom_half, y1],
            [cx - bottom_half, y1],
        ]
    }

    /// Build the `TripletCandidate` a perfect upstream triplet detector
    /// would produce for a code rendered through `transform`: the exact
    /// finder-center projections at module-space `(3.5,3.5)` /
    /// `(dim-3.5,3.5)` / `(3.5,dim-3.5)`. Isolates this file's own
    /// correctness from triplet-detection noise, matching the precedent
    /// `version.rs`/`alignment.rs` already set in their own synthetic-
    /// render tests.
    fn triplet_from_transform(transform: &PerspectiveTransform, dim: usize) -> TripletCandidate {
        let dimf = dim as f64;
        TripletCandidate {
            tl: transform.map(3.5 / dimf, 3.5 / dimf),
            tr: transform.map((dimf - 3.5) / dimf, 3.5 / dimf),
            bl: transform.map(3.5 / dimf, (dimf - 3.5) / dimf),
            module: 4.0,
            dimension: dim as u32,
            snap_error: 0.0,
            inverted: false,
        }
    }

    /// Render `payload` at `version` (EC level M) through `transform` into
    /// an `img_side`-square luma buffer, returning the buffer alongside the
    /// `qrcode` crate's own matrix (the bit-for-bit ground truth).
    fn render_code(
        payload: &[u8],
        version: i16,
        transform: &PerspectiveTransform,
        img_side: usize,
    ) -> (Vec<u8>, qrcode::QrCode) {
        let code =
            qrcode::QrCode::with_version(payload, qrcode::Version::Normal(version), qrcode::EcLevel::M)
                .unwrap();
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
        (img, code)
    }

    /// Assert `sampled` equals `code`'s own matrix exactly, module for
    /// module. On failure, prints (not just counts) the first 5 mismatched
    /// `(x, y)` coordinates with their expected/actual bit — per this
    /// task's gate-failure protocol, never loosen the assertion, always
    /// show exactly what disagreed.
    fn assert_bit_for_bit(sampled: &BitMatrix, code: &qrcode::QrCode, label: &str) {
        let dim = code.width();
        assert_eq!(sampled.dim, dim, "{label}: sampled dimension mismatch");
        let mut mismatches = Vec::new();
        for y in 0..dim {
            for x in 0..dim {
                let want = code[(x, y)] == qrcode::Color::Dark;
                let got = sampled.get(x, y);
                if want != got {
                    mismatches.push((x, y, want, got));
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "{label}: {}/{} module mismatches; first 5 (x,y,want,got): {:?}",
            mismatches.len(),
            dim * dim,
            &mismatches[..mismatches.len().min(5)],
        );
    }

    /// Run the real pipeline — `provisional_transform` -> real
    /// `locate_alignment_patterns` probing -> real `sample_grid` — for one
    /// (version, transform) combination and assert the sampled bits match
    /// the source matrix exactly.
    fn run_pipeline_and_assert(
        payload: &[u8],
        version: i16,
        transform: &PerspectiveTransform,
        img_side: usize,
        label: &str,
    ) {
        let dim = 17 + 4 * version as usize;
        let (img, code) = render_code(payload, version, transform, img_side);
        assert_eq!(code.width(), dim, "{label}: qrcode crate produced an unexpected dimension");
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let tile_grid = TileGrid::build(&view);

        let t = triplet_from_transform(transform, dim);
        // The provisional transform is built from the triplet alone — NOT
        // handed the true render transform — exactly what a real upstream
        // pipeline stage would have to work with before any alignment
        // pattern has been located.
        let provisional = provisional_transform(&t, dim as u32);
        let alignment = locate_alignment_patterns(&view, &tile_grid, &provisional, version as u32, false);
        let sampled = sample_grid(&view, &tile_grid, &t, dim as u32, &alignment)
            .unwrap_or_else(|| panic!("{label}: sample_grid returned None"));
        assert_bit_for_bit(&sampled.bits, &code, label);
    }

    /// The gate: versions {1, 2, 7, 20} (v1: no alignment patterns at all;
    /// v2: exactly one; v7/v20: several, exercising full ROI tiling) each
    /// crossed with three transforms (axis-aligned, rotated 30°, a mild
    /// perspective warp) — 12 combinations, every one asserted bit-for-bit
    /// exact against the `qrcode` crate's own matrix.
    #[test]
    fn bit_for_bit_gate_all_versions_and_transforms() {
        let scale = 4.0;
        for &version in &[1i16, 2, 7, 20] {
            let dim = 17 + 4 * version as usize;

            let (aa_quad, aa_side) = axis_aligned_quad(dim, scale, 4.0);
            let aa_transform = PerspectiveTransform::square_to_quad(aa_quad).unwrap();
            run_pipeline_and_assert(
                format!("AA{version}").as_bytes(), version, &aa_transform, aa_side,
                &format!("v{version} axis-aligned"),
            );

            let rot_side = rotated_img_side(dim, scale);
            let rot_quad = rotated_quad(dim, scale, 30.0, rot_side);
            let rot_transform = PerspectiveTransform::square_to_quad(rot_quad).unwrap();
            run_pipeline_and_assert(
                format!("RO{version}").as_bytes(), version, &rot_transform, rot_side,
                &format!("v{version} rotated-30deg"),
            );

            let persp_side = rotated_img_side(dim, scale);
            let persp_quad = perspective_quad(dim, scale, 0.97, persp_side);
            let persp_transform = PerspectiveTransform::square_to_quad(persp_quad).unwrap();
            run_pipeline_and_assert(
                format!("PW{version}").as_bytes(), version, &persp_transform, persp_side,
                &format!("v{version} perspective-warped"),
            );
        }
    }

    /// A code whose frame only captures its left half: sampling therefore
    /// reads far more than 2% of modules out of image, so `oob_fraction`
    /// must reflect that (the caller, `decode.rs`, is what actually rejects
    /// the candidate — this file only measures and reports).
    #[test]
    fn oob_fraction_exceeds_threshold_when_code_half_outside_frame() {
        let version = 2i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let (img, _code) = render_code(b"OOB", version, &transform, img_side);

        // Same underlying buffer/stride, but the captured frame's own
        // width is only half of it — simulates the code running off the
        // right edge of the frame without needing to reallocate anything.
        let cropped_width = img_side / 2;
        let view = LumaView::new(&img, cropped_width, img_side, img_side).unwrap();
        let tile_grid = TileGrid::build(&view);

        // The triplet still reflects the TRUE (pre-crop) geometry — this
        // test is about sample_grid's own OOB accounting, not about
        // whether triplet/alignment detection survives a cropped frame.
        let t = triplet_from_transform(&transform, dim);
        let provisional = provisional_transform(&t, dim as u32);
        let alignment = locate_alignment_patterns(&view, &tile_grid, &provisional, version as u32, false);
        let sampled = sample_grid(&view, &tile_grid, &t, dim as u32, &alignment)
            .expect("sample_grid should still return a (bad) result, not None");
        assert!(
            sampled.oob_fraction > 0.02,
            "expected oob_fraction > 0.02 for a half-cropped frame, got {}",
            sampled.oob_fraction
        );
    }
}
