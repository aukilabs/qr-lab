//! Subpixel corner refinement (Plan 5 Task 3): for a successfully decoded
//! code, replaces its coarse (working-resolution-detection-derived, then
//! scaled to source px) corners with edge-line-intersection estimates
//! traced directly against the SOURCE-resolution image — the
//! pose-estimation payoff Plan 5 exists for.
//!
//! # Algorithm (Global Constraints, transcribed)
//!
//! For each of the four OUTER module-region edges (top: `BitMatrix` row 0,
//! right: column `dim-1`, bottom: row `dim-1`, left: column 0):
//!
//! 1. **Probe selection**: only LOGICALLY DARK border modules
//!    ([`BitMatrix::get`] == `true`) in the middle ~80% of the edge (skip
//!    [`REFINE_EDGE_MARGIN_MODULES`] at each end — corner rounding), 2
//!    sub-positions per module ([`REFINE_PROBE_MODULE_FRACTIONS`]), capped
//!    at [`REFINE_MAX_POINTS_PER_EDGE`].
//! 2. **Devernay profile**: [`REFINE_PROFILE_SAMPLES`] bilinear samples at
//!    [`REFINE_PROFILE_STEP_MODULES`]-module spacing along the edge's
//!    outward normal, centered on the coarse edge position; central-
//!    difference gradient; 3-point quadratic peak interpolation locates the
//!    sub-sample gradient crossing nearest the profile's own center (see
//!    [`localize_edge_point`]'s doc for why "nearest", not "largest
//!    magnitude", and for how it handles the second, unrelated transition
//!    QR's checkerboard-like border modules routinely place one module
//!    further inward).
//! 3. **Weighted TLS fit + one outlier refit**: gradient-magnitude-weighted
//!    total-least-squares line through the located points ([`fit_line_tls_weighted`]);
//!    drop residuals over `max(`[`REFINE_OUTLIER_FLOOR_MODULES`]`, `[`REFINE_OUTLIER_MEDIAN_MULTIPLIER`]` * median residual)`;
//!    refit once on the survivors. An edge needs >= [`REFINE_MIN_EDGE_POINTS`]
//!    survivors to produce a line at all.
//! 4. **Corner intersection**: each of the 4 corners intersects its own
//!    two adjacent edges' lines when both exist; a corner missing either
//!    adjacent line keeps the caller's unrefined (source-scaled) corner
//!    instead. Fewer than 2 valid edge lines overall -> [`None`] (nothing
//!    to refine with).
//!
//! # Bit polarity
//! [`BitMatrix::get`] already encodes the LOGICAL bit (dark == the
//! finder-pattern-black convention), independent of `inverted` — see
//! `bitmatrix.rs`'s own binarization doc (`dark = (v < threshold) !=
//! inverted`). A logically dark border module contrasts against the quiet
//! zone either way: physically dark-ink-on-light-background when
//! `!inverted`, physically light-ink-on-dark-background when `inverted`
//! (a code's whole scene polarity flips together, quiet zone included) —
//! so edge/probe SELECTION needs no separate polarity branch, and the
//! Devernay profile locates the transition by gradient MAGNITUDE (not
//! signed direction), which is polarity-agnostic by construction. `inverted`
//! is accepted (and threaded through the call site) purely so a future
//! reader isn't left wondering why a bit-polarity-sensitive stage doesn't
//! take it — not because this module currently branches on it.

use crate::bitmatrix::BitMatrix;
use crate::consts::{
    CONTRAST_FLOOR, REFINE_EDGE_MARGIN_MODULES, REFINE_MAX_POINTS_PER_EDGE, REFINE_MIN_EDGE_POINTS,
    REFINE_OUTLIER_FLOOR_MODULES, REFINE_OUTLIER_MEDIAN_MULTIPLIER, REFINE_PROBE_MODULE_FRACTIONS,
    REFINE_PROFILE_SAMPLES, REFINE_PROFILE_STEP_MODULES,
};
use crate::homography::PerspectiveTransform;
use crate::sample::{fit_line_tls_weighted, intersect_lines, EdgeFit};
use crate::LumaView;

/// One outer edge's point-count bookkeeping (Plan 5 Task 3 trace
/// requirement) — mirrored into [`crate::trace::RefineTrace`] (a decoupled
/// DTO — see [`RefinedCorners::to_trace`]) whenever a `Trace` is being
/// collected.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EdgeStat {
    /// Candidate probe positions attempted (dark border modules in the
    /// middle ~80% of the edge, 2 sub-positions each, post-cap).
    pub points_probed: u32,
    /// Points actually used in the final (post-outlier-refit) line —
    /// `0` when the edge never reached [`REFINE_MIN_EDGE_POINTS`].
    pub points_fit: u32,
    /// Points whose Devernay localization succeeded but were then dropped
    /// by the outlier-residual refit specifically (not counting points
    /// that failed localization itself — an off-image profile or a
    /// gradient peak at the profile's own boundary).
    pub dropped_outliers: u32,
    /// `true` iff this edge produced a usable line.
    pub valid: bool,
}

/// Subpixel-refined corners plus per-edge diagnostics (Plan 5 Task 3).
pub(crate) struct RefinedCorners {
    /// TL, TR, BR, BL, SOURCE px — refined at index `i` iff
    /// `corner_refined[i]`, else the caller's unrefined (source-scaled)
    /// corner (see the module doc's per-corner fallback rule).
    pub corners: [[f64; 2]; 4],
    /// `[top, right, bottom, left]` — see [`EDGE_TO_CORNERS`]'s doc for the
    /// exact edge/corner adjacency.
    pub edge_stats: [EdgeStat; 4],
    /// `[TL, TR, BR, BL]`, `true` where the corner came from intersecting
    /// its two adjacent fitted edge lines rather than the fallback.
    pub corner_refined: [bool; 4],
}

impl RefinedCorners {
    /// Convert to the wire/trace DTO (`crate::trace::RefineTrace`) — kept
    /// as a separate type there (not a re-export) so this module's own
    /// representation can evolve without touching the wire contract, the
    /// same separation `AlignmentTraceEntry`/`SampleRegionTrace` already
    /// establish for their own source modules.
    pub(crate) fn to_trace(&self) -> crate::trace::RefineTrace {
        crate::trace::RefineTrace {
            edges: self.edge_stats.map(|e| crate::trace::EdgeRefineTrace {
                points_probed: e.points_probed,
                points_fit: e.points_fit,
                dropped_outliers: e.dropped_outliers,
                valid: e.valid,
            }),
            corner_refined: self.corner_refined,
        }
    }
}

/// Edge index convention: `edges[i]` connects `corners[i]` to
/// `corners[(i + 1) % 4]` (corners are TL=0, TR=1, BR=2, BL=3) — i.e. edge
/// 0 = top (module-space `y=0`), 1 = right (`x=dim`), 2 = bottom
/// (`y=dim`), 3 = left (`x=0`). Corner `i` is therefore adjacent to edges
/// `(i + 3) % 4` and `i` (e.g. corner 0/TL is adjacent to edge 3/left and
/// edge 0/top).
const EDGE_TO_CORNERS: [(usize, usize); 4] = [(0, 1), (1, 2), (2, 3), (3, 0)];

/// Bilinear luma sample at an arbitrary source-pixel position; `None` off
/// the image (needs a real pixel on both sides of both axes, same bound as
/// `version.rs`'s `sample_module_gray_bilinear` — no clamped-edge fallback
/// at the FAR image boundary, since a silently repeated edge pixel would
/// corrupt the gradient profile rather than just failing loudly).
///
/// # Pixel-center convention (the `- 0.5`)
/// `x`/`y` are treated as continuous positions where pixel index `i`
/// represents the value AT `i + 0.5` (its center), matching how
/// `testpaint::render_module_grid_transformed`'s antialiased renders (and
/// real camera sensors, which integrate light over each photosite's area)
/// actually work: pixel `i` is the box-filtered AVERAGE over the continuous
/// interval `[i, i+1)`, which for a roughly-linear signal over that
/// interval is best approximated as the point value at the interval's
/// CENTER, not its left edge. Without this shift, a bilinear read is
/// systematically off by half a sample toward `-x`/`-y` — small at low
/// precision requirements, but a dominant error source at the sub-0.1px
/// scale this module targets (measured: ~1px of corner error on the Task 3
/// synthetic gate before this fix, since the profile's own sample spacing
/// is a few px). The `.max(0.0)` after subtracting only matters within the
/// first half-pixel of the image edge (`x`/`y` `< 0.5`) — there is no
/// pixel `-1` to reconstruct a center for, so that thin margin clamps to
/// pixel `0`'s own value instead of extrapolating.
fn bilinear_at(view: &LumaView, x: f64, y: f64) -> Option<f64> {
    let (w, h) = (view.width(), view.height());
    if x < 0.0 || y < 0.0 || x > (w - 1) as f64 || y > (h - 1) as f64 {
        return None;
    }
    let x = (x - 0.5).max(0.0);
    let y = (y - 0.5).max(0.0);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f64;
    let fy = y - y0 as f64;
    let v00 = view.get(x0, y0) as f64;
    let v10 = view.get(x1, y0) as f64;
    let v01 = view.get(x0, y1) as f64;
    let v11 = view.get(x1, y1) as f64;
    let top = v00 + (v10 - v00) * fx;
    let bot = v01 + (v11 - v01) * fx;
    Some(top + (bot - top) * fy)
}

/// One localized edge point + its Devernay-profile gradient-magnitude
/// weight (feeds [`fit_line_tls_weighted`]).
struct EdgePoint {
    pos: [f64; 2],
    weight: f64,
}

/// Devernay sub-pixel edge localization: [`REFINE_PROFILE_SAMPLES`]
/// bilinear samples along `normal` (unit vector, `step` px spacing)
/// centered on `coarse`, central-difference gradient, 3-point quadratic
/// peak interpolation on the gradient value NEAREST the profile's own
/// center (not the globally largest magnitude — see below). `None` when
/// the profile runs off the source image, no gradient value exceeds
/// [`crate::consts::CONTRAST_FLOOR`] anywhere (no discernible transition
/// at all), the nearest-to-center candidate sits at the profile's own
/// boundary (no interior neighbor on one side for the quadratic fit — the
/// true edge likely lies outside this profile's window entirely), or the
/// 3 points around it are degenerate (near-zero curvature — no clear
/// extremum).
///
/// # Why nearest-to-center, not largest-magnitude
/// The profile spans +/-1.5 modules (`REFINE_PROFILE_STEP_MODULES *
/// (REFINE_PROFILE_SAMPLES / 2)`) around the coarse position, but the dark
/// border module itself is only 1 module wide — so whenever that module's
/// immediate INWARD neighbor (one module further into the code) has the
/// OPPOSITE color, the profile also crosses that second, inward
/// transition, at roughly the same gradient magnitude as the true outward
/// one (both are the same kind of ink/background edge, just at a different
/// sub-pixel phase against the sample grid — magnitude alone does not
/// reliably rank "real" over "real but wrong"). This is not a rare
/// pathology: an inward neighbor differs in color from the border module
/// roughly as often as it doesn't. Taking the transition closest to the
/// coarse prediction resolves the ambiguity correctly, since the coarse
/// corners this stage refines are already within a fraction of a module of
/// the truth (that is what makes them a usable starting point at all) —
/// the true edge is essentially always the NEAREST plausible transition,
/// never the second-nearest.
fn localize_edge_point(
    source: &LumaView,
    coarse: [f64; 2],
    normal: [f64; 2],
    step: f64,
) -> Option<EdgePoint> {
    let n = REFINE_PROFILE_SAMPLES;
    let half = (n / 2) as f64; // 3 for n = 7
    let mut samples = Vec::with_capacity(n);
    for k in 0..n {
        let offset = k as f64 - half;
        let p = [coarse[0] + normal[0] * offset * step, coarse[1] + normal[1] * offset * step];
        samples.push(bilinear_at(source, p[0], p[1])?);
    }
    // Central-difference gradient at interior indices 1..=n-2 only (needs
    // both neighbors); n = 7 -> indices 1..=5.
    let mut grad = vec![0.0f64; n];
    for i in 1..n - 1 {
        grad[i] = samples[i + 1] - samples[i - 1];
    }
    // Nearest-to-center selection: scan candidate indices in order of
    // increasing distance from the center (`half`), take the first whose
    // gradient magnitude clears the noise floor. `CONTRAST_FLOOR` (already
    // pinned for tile-threshold contrast — see its own doc) is reused here
    // under the identical rationale: two samples straddling real
    // ink/background contrast differ by well more than sensor noise.
    let center = n / 2; // == half as usize, 3 for n = 7
    let mut order: Vec<usize> = (1..n - 1).collect();
    order.sort_by(|&a, &b| {
        let da = (a as isize - center as isize).unsigned_abs();
        let db = (b as isize - center as isize).unsigned_abs();
        da.cmp(&db).then(grad[b].abs().total_cmp(&grad[a].abs()))
    });
    let best_i = order.into_iter().find(|&i| grad[i].abs() >= CONTRAST_FLOOR as f64)?;
    let best_mag = grad[best_i].abs();
    // The 3-point quadratic needs grad[best_i - 1] and grad[best_i + 1] to
    // themselves be valid (interior) gradient entries.
    if best_i < 2 || best_i > n - 3 {
        return None;
    }
    let (gm1, g0, gp1) = (grad[best_i - 1], grad[best_i], grad[best_i + 1]);
    // Detect INWARD contamination of `gm1` two ways. QR's checkerboard-like
    // border modules routinely put a second, unrelated transition exactly
    // 1 module further inward (the border module's own far boundary
    // against a differently-colored neighbor), whose partial-coverage
    // value contaminates `gm1`:
    // (a) **Sign flip** (unambiguous): a genuine isolated transition's
    //     central-difference gradient is single-humped — same sign
    //     throughout its local neighborhood, only the magnitude falls off
    //     away from the peak — so `gm1` disagreeing in sign with `g0` can
    //     only mean a second, oppositely-directed transition nearby.
    // (b) **A comparably strong feature one step further inward still**
    //     (`grad[best_i - 2]`, when in range): evidence that `gm1` itself
    //     sits on the SHOULDER of that other transition rather than
    //     reflecting pure local curvature of this one — the subtler case a
    //     sign check alone misses.
    // The OUTWARD side (`gp1`) never has either problem: it always looks
    // into the quiet zone, which has no further structure to contaminate
    // it. When contamination is detected, the 3-point quadratic's
    // curvature estimate is unusable — rather than guess at a
    // substitute (measured to still bias the result by several tenths of
    // a px) or discard the point outright (measured to leave some edges,
    // whose payload bits happen to make EVERY border module's inward
    // neighbor differ, with too few survivors to fit a line at all), fall
    // back to the un-interpolated sample position: `best_i` itself is
    // already within +/-0.5 sample (a quarter module) of the truth by
    // construction (it is the profile's own nearest-to-center, above-floor
    // candidate), which the weighted TLS fit then blends with every
    // genuinely clean (fully interpolated, sub-sample-precise) point on
    // the same edge.
    let sign_mismatch = gm1.signum() != g0.signum() && gm1 != 0.0;
    let far_contamination = best_i >= 3 && grad[best_i - 2].abs() >= 0.5 * g0.abs();
    let delta = if sign_mismatch || far_contamination {
        0.0
    } else {
        let denom = gm1 - 2.0 * g0 + gp1;
        if denom.abs() < 1e-9 {
            return None;
        }
        let delta = 0.5 * (gm1 - gp1) / denom;
        if !delta.is_finite() || delta.abs() > 1.0 {
            return None;
        }
        delta
    };
    let offset = (best_i as f64 - half) + delta;
    let pos = [coarse[0] + normal[0] * offset * step, coarse[1] + normal[1] * offset * step];
    Some(EdgePoint { pos, weight: best_mag })
}

/// Median of `values` (sorted in place) — used only by the outlier refit,
/// on small (`<= 64`-element) per-edge residual sets, so a full sort is
/// cheap. `values` must be non-empty.
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("residuals are never NaN"));
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        0.5 * (values[n / 2 - 1] + values[n / 2])
    }
}

/// Weighted TLS fit + one outlier-rejection refit over one edge's located
/// points (Global Constraints step 3). `probed` is the pre-localization
/// candidate count (for [`EdgeStat::points_probed`] — see that field's
/// doc for why it's tracked separately from `localized.len()`).
fn fit_edge(localized: &[EdgePoint], probed: usize, module_px: f64) -> (Option<EdgeFit>, EdgeStat) {
    if localized.len() < REFINE_MIN_EDGE_POINTS {
        return (
            None,
            EdgeStat { points_probed: probed as u32, points_fit: 0, dropped_outliers: 0, valid: false },
        );
    }
    let pts: Vec<[f64; 2]> = localized.iter().map(|p| p.pos).collect();
    let weights: Vec<f64> = localized.iter().map(|p| p.weight).collect();
    let fit0 = fit_line_tls_weighted(&pts, &weights);
    let normal = [-fit0.dir[1], fit0.dir[0]];
    let residual = |p: &[f64; 2]| {
        ((p[0] - fit0.centroid[0]) * normal[0] + (p[1] - fit0.centroid[1]) * normal[1]).abs()
    };
    let mut residuals: Vec<f64> = pts.iter().map(residual).collect();
    let med = median(&mut residuals);
    let threshold =
        (REFINE_OUTLIER_FLOOR_MODULES * module_px).max(REFINE_OUTLIER_MEDIAN_MULTIPLIER * med);
    let mut kept_pts = Vec::with_capacity(pts.len());
    let mut kept_w = Vec::with_capacity(pts.len());
    for (p, w) in pts.iter().zip(&weights) {
        if residual(p) <= threshold {
            kept_pts.push(*p);
            kept_w.push(*w);
        }
    }
    let dropped = (pts.len() - kept_pts.len()) as u32;
    if kept_pts.len() < REFINE_MIN_EDGE_POINTS {
        return (
            None,
            EdgeStat {
                points_probed: probed as u32,
                points_fit: kept_pts.len() as u32,
                dropped_outliers: dropped,
                valid: false,
            },
        );
    }
    let fit1 = fit_line_tls_weighted(&kept_pts, &kept_w);
    (
        Some(fit1),
        EdgeStat {
            points_probed: probed as u32,
            points_fit: kept_pts.len() as u32,
            dropped_outliers: dropped,
            valid: true,
        },
    )
}

/// Refine `code_corners_working` (TL, TR, BR, BL, WORKING px) against the
/// SOURCE image, per the module doc's algorithm. `sx`/`sy` are the
/// per-axis working/source ratios (`source_px = working_px / s` — see
/// `sample::SourceView`'s convention); pass `1.0, 1.0` when source ==
/// working (no downscale happened — refinement still runs in that case,
/// it just has nothing to lift). `bits` is the RS-validated bit matrix the
/// decode actually used. Returns `None` when fewer than 2 of the 4 edges
/// produce a usable line.
pub(crate) fn refine_corners(
    source: &LumaView,
    sx: f64,
    sy: f64,
    code_corners_working: &[[f64; 2]; 4],
    bits: &BitMatrix,
    _inverted: bool,
) -> Option<RefinedCorners> {
    let dim = bits.dim;
    if dim == 0 {
        return None;
    }
    let dimf = dim as f64;
    let corners_source: [[f64; 2]; 4] = code_corners_working.map(|[x, y]| [x / sx, y / sy]);
    let region = PerspectiveTransform::square_to_quad(corners_source)?;
    let centroid = [
        corners_source.iter().map(|p| p[0]).sum::<f64>() / 4.0,
        corners_source.iter().map(|p| p[1]).sum::<f64>() / 4.0,
    ];

    let mut edge_fits: [Option<EdgeFit>; 4] = [None, None, None, None];
    let mut edge_stats: [EdgeStat; 4] = [EdgeStat::default(); 4];

    for (edge_idx, &(a, b)) in EDGE_TO_CORNERS.iter().enumerate() {
        let d = [corners_source[b][0] - corners_source[a][0], corners_source[b][1] - corners_source[a][1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len < 1e-9 {
            continue; // degenerate (coincident corners) — no line possible
        }
        let module_px = len / dimf;
        let tangent = [d[0] / len, d[1] / len];
        let mut normal = [-tangent[1], tangent[0]];
        let mid = [
            0.5 * (corners_source[a][0] + corners_source[b][0]),
            0.5 * (corners_source[a][1] + corners_source[b][1]),
        ];
        if normal[0] * (mid[0] - centroid[0]) + normal[1] * (mid[1] - centroid[1]) < 0.0 {
            normal = [-normal[0], -normal[1]];
        }

        // Probe selection: dark border modules, middle ~80% of the edge,
        // 2 sub-positions each, capped.
        let mut candidates: Vec<f64> = Vec::new();
        for m in 0..dim {
            let dark = match edge_idx {
                0 => bits.get(m, 0),
                1 => bits.get(dim - 1, m),
                2 => bits.get(m, dim - 1),
                3 => bits.get(0, m),
                _ => unreachable!("EDGE_TO_CORNERS has exactly 4 entries"),
            };
            if !dark {
                continue;
            }
            for &frac in &REFINE_PROBE_MODULE_FRACTIONS {
                let pos_modules = m as f64 + frac;
                if pos_modules < REFINE_EDGE_MARGIN_MODULES
                    || pos_modules > dimf - REFINE_EDGE_MARGIN_MODULES
                {
                    continue;
                }
                candidates.push(pos_modules / dimf);
            }
        }
        if candidates.len() > REFINE_MAX_POINTS_PER_EDGE {
            let stride = (candidates.len() as f64 / REFINE_MAX_POINTS_PER_EDGE as f64).ceil() as usize;
            candidates = candidates.into_iter().step_by(stride.max(1)).take(REFINE_MAX_POINTS_PER_EDGE).collect();
        }
        let probed = candidates.len();

        // Devernay profile + localization per candidate.
        let step = REFINE_PROFILE_STEP_MODULES * module_px;
        let mut points: Vec<EdgePoint> = Vec::with_capacity(candidates.len());
        for &t in &candidates {
            let (u, v) = match edge_idx {
                0 => (t, 0.0),
                1 => (1.0, t),
                2 => (t, 1.0),
                3 => (0.0, t),
                _ => unreachable!("EDGE_TO_CORNERS has exactly 4 entries"),
            };
            let coarse = region.map(u, v);
            if let Some(pt) = localize_edge_point(source, coarse, normal, step) {
                points.push(pt);
            }
        }

        let (fit, stat) = fit_edge(&points, probed, module_px);
        edge_fits[edge_idx] = fit;
        edge_stats[edge_idx] = stat;
    }

    if edge_fits.iter().filter(|f| f.is_some()).count() < 2 {
        return None;
    }

    let mut corners = corners_source;
    let mut corner_refined = [false; 4];
    for corner_idx in 0..4 {
        let prev_edge = (corner_idx + 3) % 4;
        if let (Some(a), Some(b)) = (&edge_fits[prev_edge], &edge_fits[corner_idx]) {
            if let Some(p) = intersect_lines(a, b) {
                corners[corner_idx] = p;
                corner_refined[corner_idx] = true;
            }
        }
    }

    Some(RefinedCorners { corners, edge_stats, corner_refined })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitmatrix::BitMatrix;
    use crate::testpaint::render_module_grid_transformed_antialiased;

    fn qr_bitmatrix(payload: &[u8], version: i16) -> BitMatrix {
        let code = qrcode::QrCode::with_version(payload, qrcode::Version::Normal(version), qrcode::EcLevel::M)
            .unwrap();
        let dim = code.width();
        let mut m = BitMatrix::new(dim);
        for y in 0..dim {
            for x in 0..dim {
                m.set(x, y, code[(x, y)] == qrcode::Color::Dark);
            }
        }
        m
    }

    #[derive(Clone, Copy)]
    enum Pose {
        Frontal,
        Rotated30,
        Perspective,
    }

    /// Image-space quad (TL, TR, BR, BL) for one test pose: a `size x
    /// size` code footprint centered on a `canvas x canvas` image. The
    /// `+ 0.37` center jitter is deliberate: without it, an integer
    /// `MODULE_PX` and an integer `canvas`/2 conspire to land every module
    /// boundary of the (unrotated) `Frontal` pose exactly on an integer
    /// pixel coordinate, which — combined with an integer supersample
    /// factor — makes `render_module_grid_transformed_antialiased`'s
    /// output a perfectly hard 0/255 step with NO sub-pixel blending at
    /// all (every supersampled pixel on either side of the boundary lands
    /// wholly in one module or the other). A hard step is a stress case
    /// worth handling (see `localize_edge_point`'s nearest-to-center
    /// selection), but it defeats the POINT of testing against an
    /// antialiased render specifically, so the jitter ensures genuine
    /// partial-coverage antialiasing actually occurs for every pose.
    fn pose_quad(pose: Pose, size: f64, canvas: f64) -> [[f64; 2]; 4] {
        let c = canvas / 2.0 + 0.37;
        let half = size / 2.0;
        match pose {
            Pose::Frontal => [
                [c - half, c - half],
                [c + half, c - half],
                [c + half, c + half],
                [c - half, c + half],
            ],
            Pose::Rotated30 => {
                let (s, co) = 30f64.to_radians().sin_cos();
                let base = [[-half, -half], [half, -half], [half, half], [-half, half]];
                base.map(|[x, y]| [c + x * co - y * s, c + x * s + y * co])
            }
            // Mild perspective: the right side foreshortened ~15% (as if
            // the plane were tilted slightly about a vertical axis) —
            // still a well-conditioned, convex quad.
            Pose::Perspective => {
                let shrink = 0.15 * size;
                [
                    [c - half, c - half],
                    [c + half, c - half + shrink],
                    [c + half, c + half - shrink],
                    [c - half, c + half],
                ]
            }
        }
    }

    fn corner_error(a: [f64; 2], b: [f64; 2]) -> f64 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    /// One separable 3-tap `[0.25, 0.5, 0.25]` box-blur pass (both axes) —
    /// a small, deterministic amount of mild lens-blur-like spread on top
    /// of the antialiased render, so the profile's linear (bilinear)
    /// reconstruction of the underlying step edge sees a few pixels of
    /// genuine gradient instead of a near-instantaneous box-filter
    /// transition confined to a single pixel — closer to what an actual
    /// camera capture's PSF produces. Clamps at the image border (repeats
    /// the edge pixel) rather than reading out of bounds.
    fn mild_blur(img: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut tmp = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let l = img[y * w + x.saturating_sub(1)] as f32;
                let c = img[y * w + x] as f32;
                let r = img[y * w + (x + 1).min(w - 1)] as f32;
                tmp[y * w + x] = 0.25 * l + 0.5 * c + 0.25 * r;
            }
        }
        let mut out = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let l = tmp[y.saturating_sub(1) * w + x];
                let c = tmp[y * w + x];
                let r = tmp[(y + 1).min(h - 1) * w + x];
                out[y * w + x] = (0.25 * l + 0.5 * c + 0.25 * r).round() as u8;
            }
        }
        out
    }

    /// Task 3's synthetic accuracy gate (Global Constraints gate 1): on
    /// clean (antialiased, no sensor noise) synthetic renders at 3 poses x
    /// 2 versions, refined corners must be <= 0.05px mean / <= 0.15px max
    /// from the analytic ground truth (the exact quad used to render).
    ///
    /// The coarse input handed to `refine_corners` here IS the ground
    /// truth quad (not a perturbed/detected estimate) — this isolates
    /// refinement's own numerical precision (Devernay localization +
    /// weighted TLS + intersection) from coarse-detection accuracy, which
    /// is a separate concern exercised by the real-fixture decode gates
    /// (and, end-to-end, by whatever coarse corners `scan()`'s own
    /// detection pipeline hands `refine_corners` in production). Every
    /// corner is additionally asserted `corner_refined` (not a fallback) —
    /// a well-conditioned clean synthetic render should always produce 4
    /// valid edge lines, so a silent fallback would mask a real accuracy
    /// regression behind a vacuously-zero error.
    #[test]
    fn synthetic_accuracy_gate() {
        const MODULE_PX: f64 = 6.0;
        const SUPERSAMPLE: usize = 8;

        let mut table: Vec<(i16, &str, f64, f64)> = Vec::new();
        let cases: [(i16, &[u8]); 2] = [(1, b"REFINE1"), (7, b"REFINEV7TESTPAYLOAD")];
        let poses: [(&str, Pose); 3] =
            [("frontal", Pose::Frontal), ("rot30", Pose::Rotated30), ("perspective", Pose::Perspective)];

        for (version, payload) in cases {
            let bits = qr_bitmatrix(payload, version);
            let dim = bits.dim;
            let size = dim as f64 * MODULE_PX;
            let canvas = (size * 1.6 + 40.0).ceil();
            let img_side = canvas as usize;

            for (pose_name, pose) in poses {
                let quad = pose_quad(pose, size, canvas);
                let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
                let truth: [[f64; 2]; 4] = [
                    transform.map(0.0, 0.0),
                    transform.map(1.0, 0.0),
                    transform.map(1.0, 1.0),
                    transform.map(0.0, 1.0),
                ];
                let img = render_module_grid_transformed_antialiased(
                    dim,
                    |x, y| bits.get(x, y),
                    25,
                    235,
                    &transform,
                    img_side,
                    img_side,
                    SUPERSAMPLE,
                );
                let img = mild_blur(&img, img_side, img_side);
                let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();

                let refined = refine_corners(&view, 1.0, 1.0, &truth, &bits, false).unwrap_or_else(|| {
                    panic!("v{version} {pose_name}: refine_corners returned None")
                });
                for i in 0..4 {
                    assert!(
                        refined.corner_refined[i],
                        "v{version} {pose_name}: corner {i} fell back to unrefined \
                         (edge_stats={:?})",
                        refined.edge_stats
                    );
                }
                let errors: [f64; 4] =
                    std::array::from_fn(|i| corner_error(refined.corners[i], truth[i]));
                let mean = errors.iter().sum::<f64>() / 4.0;
                let max = errors.iter().cloned().fold(0.0, f64::max);
                table.push((version, pose_name, mean, max));
            }
        }

        eprintln!("Task 3 synthetic accuracy gate (version, pose, mean px, max px):");
        for (v, p, mean, max) in &table {
            eprintln!("  v{v:<3} {p:<11} mean={mean:.4}px max={max:.4}px");
        }
        for (version, pose_name, mean, max) in &table {
            assert!(
                *mean <= 0.05,
                "v{version} {pose_name}: mean corner error {mean:.4}px exceeds 0.05px"
            );
            assert!(
                *max <= 0.15,
                "v{version} {pose_name}: max corner error {max:.4}px exceeds 0.15px"
            );
        }
    }

    #[test]
    fn refine_corners_returns_none_for_a_degenerate_bit_matrix() {
        let bits = BitMatrix::new(0);
        let data = vec![128u8; 16 * 16];
        let view = LumaView::new(&data, 16, 16, 16).unwrap();
        let corners = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(refine_corners(&view, 1.0, 1.0, &corners, &bits, false).is_none());
    }

    #[test]
    fn refine_corners_returns_none_on_a_flat_image() {
        // A real (structured) bit matrix but a perfectly flat image: no
        // gradient anywhere, so every edge should fail to localize any
        // point at all, and the whole call returns `None`.
        let bits = qr_bitmatrix(b"FLATTEST", 1);
        let dim = bits.dim;
        let size = dim as f64 * 6.0;
        let img_side = (size * 1.6 + 40.0).ceil() as usize;
        let data = vec![128u8; img_side * img_side];
        let view = LumaView::new(&data, img_side, img_side, img_side).unwrap();
        let half = size / 2.0;
        let c = img_side as f64 / 2.0;
        let corners = [[c - half, c - half], [c + half, c - half], [c + half, c + half], [c - half, c + half]];
        assert!(refine_corners(&view, 1.0, 1.0, &corners, &bits, false).is_none());
    }
}
