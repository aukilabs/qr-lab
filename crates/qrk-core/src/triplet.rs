//! Groups verified finder-pattern candidates (Task 5's `FinderCandidate`)
//! into triplets that plausibly form one QR code's three finder corners,
//! and estimates the code's module dimension from the triplet's geometry.
//!
//! Dimension estimation follows the amended Plan 2 Task 6 contract
//! (recorded decision after the first gate run — see the plan's Task 6
//! section): the scan-measured `FinderCandidate.module` is systematically
//! biased by `1/cos(rotation)` (axis-aligned scan lines cross a rotated
//! grid along a chord) and cannot represent per-axis perspective
//! foreshortening, so the module is instead measured **along each leg**
//! on the binarized image, per zxing's established
//! `Detector.calculateModuleSize` approach. Every tolerance below is
//! derived from QR geometry or the perspective envelope — never a value
//! tuned against a fixture (Plan 2 "No overfitting").

// PLAN 3 (recorded): the candidate-input cap (bounding `finders` before
// the O(n^3) triple search below, by taking the top-K candidates ranked
// by `hits`) is a decision deferred to Plan 3, not yet implemented here.
// Plan 3 may also need `try_group`'s per-leg `module_tr`/`module_bl`
// exposed on `TripletCandidate` separately (today only their mean is
// kept) for downstream dimension refinement.

use crate::finder::FinderCandidate;
use crate::tiles::TileGrid;
use crate::LumaView;

/// A grouped triplet, canonically ordered `tl`/`tr`/`bl` (cross product
/// `(tr-tl)x(bl-tl) > 0` in y-down image coordinates — normal reading
/// order), plus the dimension estimate derived from per-leg module
/// measurements.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TripletCandidate {
    pub tl: [f64; 2],
    pub tr: [f64; 2],
    pub bl: [f64; 2],
    pub module: f64,
    pub dimension: u32,
    pub snap_error: f64,
    pub inverted: bool,
    /// Indices into the `finders` slice [`group_triplets`] was called with,
    /// in the same `[tl, tr, bl]` canonical order as the point fields above
    /// (Plan 4 Task 5's recorded addition): `decode.rs`'s arbitration needs
    /// to know which finder candidates a triplet is built from, both to
    /// proximity-dedup triplets that share ≥2 of them and to mark them
    /// consumed once a shared candidate decodes successfully.
    pub finder_indices: [usize; 3],
}

/// `|cos|` of the between-leg angle at a candidate corner must be at most
/// this to accept the corner as (near) a right angle. `arccos(0.4) ≈
/// 66.4°`, so the admitted included angle spans `[66.4°, 113.6°]` — i.e.
/// ±23.6° either side of 90°. The fixture suite's steepest scenario
/// (`tilt45_*`) tilts the code plane up to 45° from fronto-parallel; a
/// camera-plane right angle projected through a 45° tilt about an in-plane
/// axis foreshortens one leg but does not, by itself, rotate the *angle*
/// between the two legs anywhere near that far off 90° (foreshortening is
/// captured separately by the leg-balance check below) — the ±23.6°
/// budget here is headroom for the combined effect of tilt plus whatever
/// in-plane rotation and detector jitter perturb the corner's measured
/// vertex position. Comfortably excludes collinear/near-collinear triples
/// (`|cos|` near 1).
const MAX_ABS_COS: f64 = 0.4;

/// Leg balance: the shorter leg must be at least this fraction of the
/// longer one (`|1 - shorter/longer| ≤ 0.5` ⇔ `shorter/longer ≥ 0.5`). A
/// 45° perspective tilt foreshortens the far leg by a factor of
/// `cos(45°) ≈ 0.707` relative to the near one, i.e. `|1-0.707| ≈ 0.293`;
/// 0.5 keeps that margin plus headroom for combined tilt + in-plane
/// rotation without admitting a leg pair skewed enough to indicate a
/// wrong-corner pairing.
const MAX_LEG_IMBALANCE: f64 = 0.5;

/// Pairwise finder scan-module-size ratio must be at most this. Per the
/// amended contract this is a *coarse pre-filter only* (the scan module
/// is rotation-biased; the authoritative module comes from the per-leg
/// measurement below): a single QR code's three finders share one
/// physical module size, but the scan module is a horizontal-crossing
/// measurement, so under the spec's operating envelope (§6: perspective
/// tilt up to ~45° from fronto-parallel) a tilted finder's scan-measured
/// module can foreshorten by up to `1/cos(45°) ≈ 1.41` relative to an
/// untilted finder sharing the same frame. 1.5 keeps that spread plus
/// headroom for scan-line jitter. A larger spread is evidence the
/// candidates come from different codes (cross-code noise in multi-code
/// frames).
const MAX_MODULE_RATIO: f64 = 1.5;

/// Floor of the noise-scaled leg-agreement bound (see [`dim_agree_tol`]):
/// the two per-leg dimension estimates must agree within
/// `max(4, 2*(dim_mean-7)/(7*module_mean))` modules, or the legs are not
/// both measuring the same code's side (mismatched/degenerate grouping).
const MIN_DIM_AGREE_TOL: f64 = 4.0;

/// Noise-scaled leg-agreement bound — the plan's second recorded Task 6
/// amendment: the fixed ≤4 bound is provably exceeded by pure DDA
/// quantization noise at large dimensions / small modules. Derivation:
/// ±1 px quantization at each end of a 7-module BWB crossing gives a
/// per-leg module error of `δm ≈ 2/14 px`; a leg spans `n − 7` modules,
/// so the per-leg dimension noise is `≈ (n−7)·δm/module =
/// (n−7)/(7·module)`; two independent legs disagree by up to twice that,
/// hence the factor 2. False triples that survive the leg-balance ≤0.5
/// filter disagree by roughly an order of magnitude more, so the gate
/// keeps its discriminative purpose; the [21,177] and mod-4 snap gates
/// are unchanged.
fn dim_agree_tol(dim_mean: f64, module_mean: f64) -> f64 {
    (2.0 * (dim_mean - 7.0) / (7.0 * module_mean)).max(MIN_DIM_AGREE_TOL)
}

/// QR Model 2 dimension range: version 1..=40 -> `21 + 4*(v-1)` spans
/// `[21, 177]` (the QR spec).
const MIN_DIMENSION: i64 = 21;
const MAX_DIMENSION: i64 = 177;

/// Output cap: bounds worst-case downstream decode work per frame.
const MAX_TRIPLETS: usize = 64;

/// `a`/`b` as the larger-over-smaller ratio (always ≥ 1), so one threshold
/// works regardless of which of `a`/`b` is bigger.
fn ratio_ge1(a: f64, b: f64) -> f64 {
    if a > b {
        a / b
    } else {
        b / a
    }
}

/// `|cos|` of the angle between legs `corner->p` and `corner->q`; `1.0`
/// (treated as maximally non-perpendicular, i.e. rejected downstream) if
/// either leg is degenerate (zero length).
fn abs_cos_angle(corner: [f64; 2], p: [f64; 2], q: [f64; 2]) -> f64 {
    let v1 = [p[0] - corner[0], p[1] - corner[1]];
    let v2 = [q[0] - corner[0], q[1] - corner[1]];
    let dot = v1[0] * v2[0] + v1[1] * v2[1];
    let n1 = (v1[0] * v1[0] + v1[1] * v1[1]).sqrt();
    let n2 = (v2[0] * v2[0] + v2[1] * v2[1]).sqrt();
    if n1 == 0.0 || n2 == 0.0 {
        return 1.0;
    }
    (dot / (n1 * n2)).abs()
}

/// Legs `|corner->p|`, `|corner->q|` as shorter-over-longer (`<= 1.0`), so
/// `1.0 - ratio` is the fractional imbalance regardless of which leg is
/// longer.
fn leg_balance(corner: [f64; 2], p: [f64; 2], q: [f64; 2]) -> f64 {
    let a = ((p[0] - corner[0]).powi(2) + (p[1] - corner[1]).powi(2)).sqrt();
    let b = ((q[0] - corner[0]).powi(2) + (q[1] - corner[1]).powi(2)).sqrt();
    if a > b {
        b / a
    } else {
        a / b
    }
}

/// Snap `mean` to the nearest integer that is `≡ 1 (mod 4)` — the QR
/// dimension congruence `21, 25, 29, ..., 177` — returning
/// `(snapped, |mean - snapped|)`.
fn snap_dimension(mean: f64) -> (i64, f64) {
    let base = ((mean - 1.0) / 4.0).round() as i64;
    let snapped = base * 4 + 1;
    (snapped, (mean - snapped as f64).abs())
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Walk the binarized line from `from` toward `to` (DDA, unit steps along
/// the dominant axis — the same mild Bresenham variant zxing's
/// `Detector.sizeOfBlackWhiteBlackRun` uses) and measure the
/// ink(1.5)+space(1)+ink(1) crossing that starts at a finder's center:
/// from inside the 3-module core (1.5 modules of ink from center to
/// edge), through the 1-module light separator, through the 1-module dark
/// ring, ending at the first sample past the ring — 3.5 modules of
/// distance in total. Ink is polarity-aware against the tile thresholds:
/// `(pixel < threshold) != inverted`.
///
/// Returns `Some(distance from `from` to the exit sample)` when the
/// crossing completes; `None` when the walk leaves the image or reaches
/// `to` first (a truncated walk — the caller applies the amendment's
/// border correction).
fn bwb_run(
    view: &LumaView,
    grid: &TileGrid,
    inverted: bool,
    from: [f64; 2],
    to: [f64; 2],
) -> Option<f64> {
    let (w, h) = (view.width() as isize, view.height() as isize);
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let steps = dx.abs().max(dy.abs()).ceil() as usize;
    if steps == 0 {
        return None;
    }
    let sx = dx / steps as f64;
    let sy = dy / steps as f64;
    // state 0: in core ink; 1: in separator space; 2: in ring ink.
    // zxing's transition test: states 0 and 2 scan ink, state 1 scans
    // space; finding the "wrong color" advances the state, and leaving
    // state 2 completes the crossing.
    let mut state = 0u8;
    for i in 0..=steps {
        let x = from[0] + sx * i as f64;
        let y = from[1] + sy * i as f64;
        let (xi, yi) = (x.round() as isize, y.round() as isize);
        if xi < 0 || yi < 0 || xi >= w || yi >= h {
            return None; // truncated at the image border mid-crossing
        }
        let (xu, yu) = (xi as usize, yi as usize);
        let ink = (view.get(xu, yu) < grid.threshold_at(xu, yu)) != inverted;
        if (state == 1) == ink {
            if state == 2 {
                return Some(((x - from[0]).powi(2) + (y - from[1]).powi(2)).sqrt());
            }
            state += 1;
        }
    }
    None // reached `to` without completing the crossing
}

/// zxing `Detector.sizeOfBlackWhiteBlackRunBothWays`: the crossing from
/// `a` toward `b` (3.5 modules) plus the crossing from `a` directly away
/// from `b` (another 3.5 modules) — 7 modules of distance in total. The
/// paired sample-quantized walks overshoot the crossing by one sample
/// between them (zxing: "middle pixel is double-counted this way;
/// subtract 1"), hence the `- 1.0`. A walk truncated at the image border
/// contributes its partner's value doubled instead (the amendment's zxing
/// border correction); both truncated means this endpoint measures
/// nothing.
fn bwb_both(
    view: &LumaView,
    grid: &TileGrid,
    inverted: bool,
    a: [f64; 2],
    b: [f64; 2],
) -> Option<f64> {
    let toward = bwb_run(view, grid, inverted, a, b);
    let away_target = [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]];
    let away = bwb_run(view, grid, inverted, a, away_target);
    match (toward, away) {
        (Some(t), Some(w)) => Some(t + w - 1.0),
        (Some(t), None) => Some(2.0 * t - 1.0),
        (None, Some(w)) => Some(2.0 * w - 1.0),
        (None, None) => None,
    }
}

/// zxing `Detector.calculateModuleSizeOneWay`: the module size along the
/// leg `a`—`b`, as the mean of the 7-module both-ways crossings measured
/// from each endpoint — `(bwb_both(a,b) + bwb_both(b,a)) / 14`. If one
/// endpoint's measurement failed entirely, the other alone divided by 7
/// (zxing's NaN fallback); both failed -> `None` (caller rejects the
/// triple).
fn leg_module(
    view: &LumaView,
    grid: &TileGrid,
    inverted: bool,
    a: [f64; 2],
    b: [f64; 2],
) -> Option<f64> {
    match (
        bwb_both(view, grid, inverted, a, b),
        bwb_both(view, grid, inverted, b, a),
    ) {
        (Some(x), Some(y)) => Some((x + y) / 14.0),
        (Some(x), None) => Some(x / 7.0),
        (None, Some(y)) => Some(y / 7.0),
        (None, None) => None,
    }
}

/// Evaluate one unordered candidate triple: image-free geometry gates
/// (polarity, coarse scan-module ratio, corner identification, angle +
/// leg-balance), canonical ordering, then per-leg module measurement on
/// the binarized image and the dimension estimate + its gates. `None` if
/// any gate fails.
#[allow(clippy::too_many_arguments)]
fn try_group(
    view: &LumaView,
    grid: &TileGrid,
    a: FinderCandidate,
    ai: usize,
    b: FinderCandidate,
    bi: usize,
    c: FinderCandidate,
    ci: usize,
) -> Option<TripletCandidate> {
    if a.inverted != b.inverted || b.inverted != c.inverted {
        return None;
    }
    if ratio_ge1(a.module, b.module) > MAX_MODULE_RATIO
        || ratio_ge1(b.module, c.module) > MAX_MODULE_RATIO
        || ratio_ge1(a.module, c.module) > MAX_MODULE_RATIO
    {
        return None;
    }

    let pts = [[a.x, a.y], [b.x, b.y], [c.x, c.y]];
    let idxs = [ai, bi, ci];

    // Corner = the finder whose two legs have the smallest |cos| between
    // them (closest to perpendicular), chosen once and independent of the
    // leg-balance/angle thresholds below.
    let mut corner_idx = 0usize;
    let mut best_cos = f64::INFINITY;
    for i in 0..3 {
        let (p, q) = match i {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let cos = abs_cos_angle(pts[i], pts[p], pts[q]);
        if cos < best_cos {
            best_cos = cos;
            corner_idx = i;
        }
    }
    if best_cos > MAX_ABS_COS {
        return None;
    }

    let (p_idx, q_idx) = match corner_idx {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    if leg_balance(pts[corner_idx], pts[p_idx], pts[q_idx]) < 1.0 - MAX_LEG_IMBALANCE {
        return None;
    }

    let tl_idx = idxs[corner_idx];
    let tl = pts[corner_idx];
    let (mut tr, mut bl) = (pts[p_idx], pts[q_idx]);
    let (mut tr_idx, mut bl_idx) = (idxs[p_idx], idxs[q_idx]);
    let cross = (tr[0] - tl[0]) * (bl[1] - tl[1]) - (tr[1] - tl[1]) * (bl[0] - tl[0]);
    // cross == 0.0 (collinear) cannot reach here: a collinear corner has
    // |cos| == 1, already rejected by the MAX_ABS_COS gate above.
    if cross < 0.0 {
        std::mem::swap(&mut tr, &mut bl);
        std::mem::swap(&mut tr_idx, &mut bl_idx);
    }

    // Per-leg module (amended contract): measured along each leg on the
    // binarized image, so in-plane rotation cancels (the walk direction
    // rotates with the code) and perspective foreshortening is captured
    // per axis.
    let inverted = a.inverted;
    let module_tr = leg_module(view, grid, inverted, tl, tr)?;
    let module_bl = leg_module(view, grid, inverted, tl, bl)?;
    let dim_tr = (dist(tl, tr) / module_tr).round() as i64 + 7;
    let dim_bl = (dist(tl, bl) / module_bl).round() as i64 + 7;
    let mean = (dim_tr + dim_bl) as f64 / 2.0;
    let module_mean = (module_tr + module_bl) / 2.0;
    if (dim_tr - dim_bl).abs() as f64 > dim_agree_tol(mean, module_mean) {
        return None;
    }
    let (snapped, snap_error) = snap_dimension(mean);
    if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&snapped) {
        return None;
    }

    Some(TripletCandidate {
        tl,
        tr,
        bl,
        module: module_mean,
        dimension: snapped as u32,
        snap_error,
        inverted,
        finder_indices: [tl_idx, tr_idx, bl_idx],
    })
}

/// Groups `finders` into every plausible finder triplet: tests each
/// unordered triple against [`try_group`]'s gates (geometry first, then
/// per-leg module measurement on the binarized `view`), then sorts
/// survivors by ascending `snap_error` and caps the result at
/// [`MAX_TRIPLETS`].
pub fn group_triplets(
    view: &LumaView,
    grid: &TileGrid,
    finders: &[FinderCandidate],
) -> Vec<TripletCandidate> {
    let n = finders.len();
    if n < 3 {
        return Vec::new();
    }
    let mut out = Vec::new();

    // Build the cheap compatibility graph once instead of rediscovering
    // the same polarity/module-size rejection inside every O(n^3) triple.
    // Each surviving triangle is still passed through `try_group`, so the
    // geometry, image-space module measurement, dimension, ordering, and
    // output are unchanged. This follows the candidate-locality direction
    // used by current zxing-cpp while keeping this scanner exhaustive: no
    // spatial or top-K cap can discard a valid multi-code candidate.
    // Below this point the cubic loop has at most 165 triples, cheaper than
    // allocating and filling the graph on ordinary one-to-four-code frames.
    const COMPAT_GRAPH_MIN_FINDERS: usize = 12;
    if n < COMPAT_GRAPH_MIN_FINDERS {
        for i in 0..n {
            for j in (i + 1)..n {
                for k in (j + 1)..n {
                    if let Some(t) =
                        try_group(view, grid, finders[i], i, finders[j], j, finders[k], k)
                    {
                        out.push(t);
                    }
                }
            }
        }
    } else {
        let mut compatible = vec![false; n * n];
        for i in 0..n {
            for j in (i + 1)..n {
                let a = finders[i];
                let b = finders[j];
                compatible[i * n + j] =
                    a.inverted == b.inverted && ratio_ge1(a.module, b.module) <= MAX_MODULE_RATIO;
            }
        }
        for i in 0..n {
            for j in (i + 1)..n {
                if !compatible[i * n + j] {
                    continue;
                }
                for k in (j + 1)..n {
                    if !compatible[i * n + k] || !compatible[j * n + k] {
                        continue;
                    }
                    if let Some(t) =
                        try_group(view, grid, finders[i], i, finders[j], j, finders[k], k)
                    {
                        out.push(t);
                    }
                }
            }
        }
    }
    out.sort_by(|x, y| {
        x.snap_error
            .partial_cmp(&y.snap_error)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(MAX_TRIPLETS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder::FinderCandidate;
    use crate::testpaint::{paint_finder, paint_finder_rotated};

    fn f(x: f64, y: f64) -> FinderCandidate {
        FinderCandidate {
            x,
            y,
            module: 4.0,
            inverted: false,
            hits: 3,
        }
    }

    /// 220x220 light image with the axis-aligned v1 layout the accept-path
    /// tests use: finder centers (100,100), (156,100), (100,156) at
    /// 4px/module (top-left corners at center - 3.5*4 = 14).
    fn v1_image() -> Vec<u8> {
        let mut img = vec![225u8; 220 * 220];
        for (ox, oy) in [(86, 86), (142, 86), (86, 142)] {
            paint_finder(&mut img, 220, ox, oy, 4, 25, 225);
        }
        img
    }

    /// Flat light image for the pure-geometry reject tests: they
    /// short-circuit in the image-free filters, so the content is never
    /// sampled — the view/grid just have to exist.
    fn flat_image() -> Vec<u8> {
        vec![200u8; 64 * 64]
    }

    #[test]
    fn groups_axis_aligned_v1() {
        // v1: n=21, leg = 14 modules = 56px.
        let img = v1_image();
        let view = crate::LumaView::new(&img, 220, 220, 220).unwrap();
        let grid = TileGrid::build(&view);
        let t = group_triplets(
            &view,
            &grid,
            &[f(100.0, 100.0), f(156.0, 100.0), f(100.0, 156.0)],
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].dimension, 21);
        assert_eq!(t[0].tl, [100.0, 100.0]);
        assert_eq!(t[0].tr, [156.0, 100.0]);
        assert_eq!(t[0].bl, [100.0, 156.0]);
        assert!(t[0].snap_error < 0.6);
        // The per-leg measurement must recover the painted 4px module.
        assert!((t[0].module - 4.0).abs() < 0.3, "module={}", t[0].module);
        // Input order was [tl, tr, bl] already, so finder_indices should
        // pass through unchanged (no corner/tr-bl swap needed).
        assert_eq!(t[0].finder_indices, [0, 1, 2]);
    }

    #[test]
    fn canonicalizes_mirrored_order() {
        // Same three points fed so the natural order is mirrored;
        // output must still satisfy the cross-product convention.
        let img = v1_image();
        let view = crate::LumaView::new(&img, 220, 220, 220).unwrap();
        let grid = TileGrid::build(&view);
        let t = group_triplets(
            &view,
            &grid,
            &[f(100.0, 100.0), f(100.0, 156.0), f(156.0, 100.0)],
        );
        assert_eq!(t.len(), 1);
        let (tl, tr, bl) = (t[0].tl, t[0].tr, t[0].bl);
        let cross = (tr[0] - tl[0]) * (bl[1] - tl[1]) - (tr[1] - tl[1]) * (bl[0] - tl[0]);
        assert!(cross > 0.0);
        // Input order was [tl@0, bl@1, tr@2]: the tr/bl swap that fixes the
        // point order must swap the paired indices in lockstep too.
        assert_eq!(t[0].finder_indices, [0, 2, 1]);
    }

    #[test]
    fn rejects_mixed_polarity_and_bad_geometry() {
        let img = flat_image();
        let view = crate::LumaView::new(&img, 64, 64, 64).unwrap();
        let grid = TileGrid::build(&view);
        let mut inv = f(156.0, 100.0);
        inv.inverted = true;
        assert!(group_triplets(&view, &grid, &[f(100.0, 100.0), inv, f(100.0, 156.0)]).is_empty());
        // Collinear points: no ~90° corner.
        assert!(
            group_triplets(&view, &grid, &[f(0.0, 0.0), f(50.0, 0.0), f(100.0, 0.0)]).is_empty()
        );
        // Legs too unbalanced (14 vs 40 modules).
        assert!(
            group_triplets(&view, &grid, &[f(0.0, 0.0), f(56.0, 0.0), f(0.0, 160.0)]).is_empty()
        );
    }

    #[test]
    fn rotated_45_still_groups() {
        // The whole v1 code rotated 45° in-plane: the finder squares
        // rotate with the code, so the bwb walks (which run along the
        // legs, i.e. along the rotated grid axes) must still measure
        // ~4px/module — exactly the 1/cos(rotation) bias the amendment
        // removes from the old scan-module approach.
        let (w, h) = (400usize, 400usize);
        let mut img = vec![225u8; w * h];
        let ang = std::f64::consts::FRAC_PI_4;
        for center in [
            [200.0, 200.0],
            [200.0 + 39.6, 200.0 + 39.6],
            [200.0 - 39.6, 200.0 + 39.6],
        ] {
            paint_finder_rotated(&mut img, w, center, 4.0, ang, 25, 225);
        }
        let view = crate::LumaView::new(&img, w, h, w).unwrap();
        let grid = TileGrid::build(&view);
        let t = group_triplets(
            &view,
            &grid,
            &[
                f(200.0, 200.0),
                f(200.0 + 39.6, 200.0 + 39.6),
                f(200.0 - 39.6, 200.0 + 39.6),
            ],
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].dimension, 21);
    }

    #[test]
    fn compatibility_graph_keeps_valid_triplet_in_crowded_frame() {
        let img = v1_image();
        let view = crate::LumaView::new(&img, 220, 220, 220).unwrap();
        let grid = TileGrid::build(&view);
        let mut finders = vec![f(100.0, 100.0), f(156.0, 100.0), f(100.0, 156.0)];
        // Force the >=12-finder compatibility-graph path with candidates
        // that are pair-incompatible by polarity or module scale.
        for i in 0..9 {
            let mut noise = f(12.0 + i as f64 * 18.0, 20.0);
            if i % 2 == 0 {
                noise.inverted = true;
            } else {
                noise.module = 12.0;
            }
            finders.push(noise);
        }
        let t = group_triplets(&view, &grid, &finders);
        assert!(t.iter().any(|candidate| {
            candidate.dimension == 21
                && candidate.finder_indices == [0, 1, 2]
                && candidate.snap_error < 0.6
        }));
    }
}
