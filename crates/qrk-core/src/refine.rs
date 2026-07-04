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
//!    [`REFINE_PROFILE_STEP_MODULES`]-module spacing along the probe's
//!    LOCAL outward normal (derived per probe, like `sample.rs`'s
//!    `probe_edge_pass` — see [`refine_round`]'s probe loop); central-
//!    difference gradient, bar-unfolded when the probed module's inward
//!    neighbor differs in color; 3-point quadratic peak interpolation on
//!    the CORRECT-SIGN gradient peak nearest the profile's center — the
//!    true outer edge's gradient sign is categorically determined by the
//!    code's polarity and the outward direction, which rejects the
//!    second, oppositely-signed transition QR's checkerboard-like border
//!    modules routinely place one module further inward (see
//!    [`localize_edge_point_pass`]'s sign-convention and bar-unfolding
//!    docs). Each probe runs three re-centered passes with Aitken Δ²
//!    extrapolation (see [`localize_edge_point`]). A probe with no usable
//!    correct-sign peak is REJECTED (contributes nothing) — never
//!    substituted with any coarse-derived fallback position.
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
//! The whole edge-probe/fit/intersect round runs TWICE — the second round
//! re-anchored on the first round's refined corners, so every quantity
//! the probes derive from the anchor quad (normals, steps, the
//! bar-unfolding's exact 1-module offset) is computed from near-exact
//! geometry rather than the coarse input — see [`refine_corners`]'s
//! two-round doc.
//!
//! # Bit polarity
//! [`BitMatrix::get`] already encodes the LOGICAL bit (dark == the
//! finder-pattern-black convention), independent of `inverted` — see
//! `bitmatrix.rs`'s own binarization doc (`dark = (v < threshold) !=
//! inverted`). A logically dark border module contrasts against the quiet
//! zone either way: physically dark-ink-on-light-background when
//! `!inverted`, physically light-ink-on-dark-background when `inverted`
//! (a code's whole scene polarity flips together, quiet zone included) —
//! so probe SELECTION needs no separate polarity branch. Localization,
//! however, is genuinely polarity-SENSITIVE: the true outer edge's
//! gradient SIGN along the outward normal flips with `inverted` (dark→
//! light vs light→dark), and [`localize_edge_point_pass`] relies on
//! exactly that sign to categorically reject the oppositely-signed inward
//! imposter transition — see its sign-convention doc for the derivation.

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
    /// by the outlier-residual refit specifically (not counting probes
    /// rejected by localization itself — an off-image profile, no
    /// correct-sign gradient peak, or a degenerate/out-of-range quadratic;
    /// see [`localize_edge_point_pass`]'s rejection cases).
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
/// at the image boundary, since a silently repeated edge pixel would
/// corrupt the gradient profile rather than just failing loudly).
///
/// # Pixel-center convention
/// Pixel index `i` holds the image value AT continuous coordinate `i` —
/// integer pixel centers. This is the crate-wide (and fixture-generator —
/// see `tools/fixtures/camera.py`: "pixel centers at integer coordinates")
/// convention, identical to `version.rs`'s `sample_module_gray_bilinear`.
/// A convention mismatch here is NOT a subtlety at this module's accuracy
/// scale: a half-pixel-per-axis disagreement between the sampler and the
/// caller's corner coordinates shows up as a constant ~0.71 px corner
/// error (measured on the near_00 end-to-end check while this sampler
/// briefly used a half-integer-center convention). Note the TEST renderer
/// (`testpaint::render_module_grid_transformed*`) samples pixel `i`'s
/// content at transform-space coordinate `i + 0.5`, i.e. its transform
/// space has HALF-integer pixel centers — the synthetic tests convert
/// their analytic truth into this integer-center convention by
/// subtracting 0.5 per axis (see `pose_quad`'s callers) rather than this
/// function adapting to the test renderer.
fn bilinear_at(view: &LumaView, x: f64, y: f64) -> Option<f64> {
    let (w, h) = (view.width(), view.height());
    if x < 0.0 || y < 0.0 || x > (w - 1) as f64 || y > (h - 1) as f64 {
        return None;
    }
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

/// One Devernay sub-pixel edge localization pass —
/// [`localize_edge_point`]'s per-pass body (see that function for the
/// two-pass re-centering rationale): [`REFINE_PROFILE_SAMPLES`] bilinear
/// samples along `normal` (unit vector, `step` px spacing) centered on
/// `center`; central-difference gradient; bar-unfolding correction when
/// the probed border module's inward neighbor differs in color
/// (`inward_differs`, from the RS-validated `BitMatrix` — see below);
/// then 3-point quadratic peak interpolation on the CORRECT-SIGN
/// gradient peak nearest the profile's center. `None` (probe rejected —
/// it simply doesn't contribute to the edge fit;
/// [`REFINE_MIN_EDGE_POINTS`] guards fit quality downstream) when: the
/// profile runs off the source image; no interior gradient value with the
/// expected sign clears [`crate::consts::CONTRAST_FLOOR`]; or the
/// quadratic is degenerate / its vertex lands outside (-1, 1) samples of
/// the peak. A rejected probe NEVER falls back to any coarse-derived
/// position — a fallback anchored at `coarse` would make the probe vote
/// FOR whatever error the coarse corners carry (a "coarse echo"),
/// silently capping how much refinement can actually correct (review
/// finding: with such a fallback, the near_00 end-to-end run recovered
/// only ~27% of the coarse error).
///
/// # Sign convention (why `inverted` matters here)
/// The profile is parameterized by an offset that INCREASES along the
/// OUTWARD normal — from the code's interior toward the quiet zone — and
/// `grad[i] = samples[i+1] - samples[i-1]` is proportional to the luma
/// derivative along that direction. The true outer-edge transition always
/// crosses from INK (the dark border module) to BACKGROUND (quiet zone)
/// as the offset increases, so its gradient sign is categorically
/// determined by polarity alone:
/// - `!inverted`: ink is dark (low luma), background light → luma RISES
///   outward → **positive** gradient;
/// - `inverted`: ink is light, background dark (the whole scene's polarity
///   flips together, quiet zone included) → **negative** gradient.
///
/// Filtering candidate peaks by this expected sign rejects any
/// oppositely-signed transition (e.g. a residue of the inward imposter
/// below) categorically, not heuristically. Among correct-sign
/// candidates, the one nearest the profile center (the expected edge
/// position) wins; a distance tie breaks toward the larger magnitude.
///
/// # Bar unfolding (`inward_differs`)
/// When the probed border module's immediate INWARD neighbor differs in
/// color (true for roughly half of all border modules — known
/// categorically from the decoded `BitMatrix`, not inferred from pixels),
/// the luma profile is not an isolated step but a 1-module-wide BAR: the
/// true outer transition plus a mirrored, opposite-signed copy exactly
/// 1 module (= `1 / REFINE_PROFILE_STEP_MODULES` = 2 samples) further
/// inward. Superposition: the observed gradient is
/// `g(u) = p(u) - p(u + 2)`, with `p` the isolated transition's gradient
/// profile (the `+2`: the mirrored copy sits 2 samples INWARD, i.e. at
/// `u = -2` in outward-increasing profile coordinates). This measurably
/// corrupts the quadratic's inward neighbor: `gm1`'s two-sample stencil
/// spans exactly the bar's width, so the two edges' slopes cancel inside
/// it and `gm1 ≈ 0` regardless of where the true edge actually is —
/// interpolating through it biased every such probe by a systematic
/// ~0.17 samples (~0.5 px at 6 px/module) OUTWARD (measured on the Task 3
/// synthetic gate).
///
/// The superposition inverts exactly, by telescoping:
/// `p(u) = g(u) + p(u + 2) = g(u) + g(u + 2) + g(u + 4) + ...` (the tail
/// terminates at the profile's outward end, where the quiet zone is
/// flat). Applying it reconstructs the isolated transition's own gradient
/// — the imposter peak literally cancels out of the corrected profile —
/// using only exact QR geometry (a module is 1 module wide; the neighbor
/// color from RS-validated bits): no tuned constants, no coarse-position
/// dependence. When the inward neighbor does NOT differ, the nearest
/// opposite transition is ≥ 2 modules (≥ 4 samples) inward and its
/// leakage into the 3-sample quadratic stencil is negligible — no
/// correction applied.
fn localize_edge_point_pass(
    source: &LumaView,
    center: [f64; 2],
    normal: [f64; 2],
    step: f64,
    inverted: bool,
    inward_differs: bool,
) -> Option<EdgePoint> {
    let n = REFINE_PROFILE_SAMPLES;
    let half = (n / 2) as f64; // 3 for n = 7
    let coarse = center;
    let mut samples = Vec::with_capacity(n);
    for k in 0..n {
        let offset = k as f64 - half;
        let p = [coarse[0] + normal[0] * offset * step, coarse[1] + normal[1] * offset * step];
        samples.push(bilinear_at(source, p[0], p[1])?);
    }
    // Central-difference gradient at interior indices 1..=n-2 only (needs
    // both neighbors); n = 7 -> indices 1..=5. Index 0 and n-1 stay 0.0 —
    // consistent with the flat quiet zone the profile's outward tail
    // reads, which the telescoped correction below relies on.
    let mut grad = vec![0.0f64; n];
    for i in 1..n - 1 {
        grad[i] = samples[i + 1] - samples[i - 1];
    }
    // Bar unfolding (see the doc section above): `p(u) = Σ_k g(u + 2k)`,
    // summed while the index stays inside the profile. The 2-sample
    // stride is `1 module / REFINE_PROFILE_STEP_MODULES` — exact module
    // geometry, not a tunable.
    let stride = (1.0 / REFINE_PROFILE_STEP_MODULES).round() as usize;
    let corrected: Vec<f64> = if inward_differs {
        (0..n)
            .map(|i| {
                let mut sum = 0.0;
                let mut j = i;
                while j < n {
                    sum += grad[j];
                    j += stride;
                }
                sum
            })
            .collect()
    } else {
        grad
    };
    // Candidate peaks: indices whose SIGNED corrected gradient (see the
    // sign convention above) clears the noise floor. `CONTRAST_FLOOR`
    // (already pinned for tile-threshold contrast — see its own doc) is
    // reused here under the identical rationale: two samples straddling
    // real ink/background contrast differ by well more than sensor noise.
    // Only indices 2..=n-3 qualify at all — the 3-point quadratic below
    // needs `corrected[best_i - 1]` and `corrected[best_i + 1]` to
    // themselves be valid (interior) gradient entries.
    let expected_sign = if inverted { -1.0 } else { 1.0 };
    let center = n / 2; // == half as usize, 3 for n = 7
    let best_i = (2..=n - 3)
        .filter(|&i| corrected[i] * expected_sign >= CONTRAST_FLOOR as f64)
        .min_by(|&a, &b| {
            let da = (a as isize - center as isize).unsigned_abs();
            let db = (b as isize - center as isize).unsigned_abs();
            da.cmp(&db).then(corrected[b].abs().total_cmp(&corrected[a].abs()))
        })?;
    let (gm1, g0, gp1) = (corrected[best_i - 1], corrected[best_i], corrected[best_i + 1]);
    let denom = gm1 - 2.0 * g0 + gp1;
    if denom.abs() < 1e-9 {
        return None;
    }
    let delta = 0.5 * (gm1 - gp1) / denom;
    if !delta.is_finite() || delta.abs() > 1.0 {
        return None;
    }
    let offset = (best_i as f64 - half) + delta;
    let pos = [coarse[0] + normal[0] * offset * step, coarse[1] + normal[1] * offset * step];
    Some(EdgePoint { pos, weight: g0.abs() })
}

/// Three-pass Devernay localization with Aitken Δ² extrapolation: run
/// [`localize_edge_point_pass`] centered on the coarse prediction, then
/// twice more, each re-centered on the previous measurement, and
/// extrapolate the three results' fixed point.
///
/// # Why re-center, and why Aitken (evidence-guided iteration)
/// The 3-point quadratic's vertex estimate has a PHASE-dependent error
/// that behaves, to first order, like a linear GAIN on the true phase:
/// `estimated ≈ g · true_phase`, with `g` depending on how well the
/// profile's 0.5-module sampling resolves the edge-spread width. When the
/// edge blur is comparable to the sample spacing, `g ≈ 1` (the textbook
/// Devernay regime — the estimate is essentially exact and re-centering
/// converges immediately). When the blur is NARROW relative to the
/// spacing (sharp renders / large source modules — measured `g ≈ 2` on
/// the Task 3 perturbed synthetic gate's perspective pose, whose local
/// perpendicular modules reach ~8 px against ~1 px of blur), a single
/// pass OVERSHOOTS the true edge by roughly the phase itself, and naive
/// re-centering oscillates around the truth instead of converging
/// (measured: successive passes landing −0.39 px then +0.41 px from the
/// true edge). Note the systematic danger: every probe on an edge shares
/// the same coarse-induced phase, so this error does NOT average out
/// across the edge fit — it becomes a coherent edge shift proportional to
/// the coarse error.
///
/// A fixed-point iteration `x_{k+1} = f(x_k)` whose error follows a
/// locally-constant linear factor (`e_{k+1} = (1 − g) e_k`, geometric —
/// decaying, oscillating, either) is exactly the setting Aitken's Δ²
/// process accelerates: from three consecutive iterates it recovers the
/// fixed point of the linearized map in closed form,
/// `x* = x_2 − (x_2 − x_1)² / (x_2 − 2 x_1 + x_0)` — for the measured
/// oscillating sequence above it lands within 0.005 px of the true edge.
/// This is standard numerical acceleration (no tuned constants), each
/// pass is still the pinned 3-point quadratic on the pinned profile, and
/// the window-recentering structure mirrors `sample.rs`'s
/// `probe_edge_line` two-pass precedent (provisional-guided first pass,
/// evidence-centered re-probing), applied per probe.
///
/// Guards: a rejected pass rejects the whole probe (never a partial
/// fallback — see [`localize_edge_point_pass`]'s no-coarse-echo note); a
/// near-zero Δ² denominator means the sequence already converged (`g ≈
/// 1`), so the last iterate is returned as-is; an extrapolation that
/// moves more than one sample step is a sign the sequence is not
/// locally linear at all, and the probe is rejected rather than trusted.
fn localize_edge_point(
    source: &LumaView,
    coarse: [f64; 2],
    normal: [f64; 2],
    step: f64,
    inverted: bool,
    inward_differs: bool,
) -> Option<EdgePoint> {
    let p0 = localize_edge_point_pass(source, coarse, normal, step, inverted, inward_differs)?;
    let p1 = localize_edge_point_pass(source, p0.pos, normal, step, inverted, inward_differs)?;
    let p2 = localize_edge_point_pass(source, p1.pos, normal, step, inverted, inward_differs)?;
    // Scalar Aitken along the profile axis: project the three positions
    // onto `normal` (they differ only along it, up to floating-point
    // noise, since every pass walks the same axis).
    let proj = |p: &EdgePoint| p.pos[0] * normal[0] + p.pos[1] * normal[1];
    let (x0, x1, x2) = (proj(&p0), proj(&p1), proj(&p2));
    let denom = x2 - 2.0 * x1 + x0;
    if denom.abs() < 1e-6 {
        // Sequence converged (or never moved): the last iterate IS the
        // fixed point to within noise.
        return Some(p2);
    }
    let correction = (x2 - x1) * (x2 - x1) / denom;
    if !correction.is_finite() || correction.abs() > step {
        return None;
    }
    let x_star = x2 - correction;
    let along = x_star - x2;
    Some(EdgePoint {
        pos: [p2.pos[0] + normal[0] * along, p2.pos[1] + normal[1] * along],
        weight: p2.weight,
    })
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

/// One full refinement round against a given set of anchor corners: probe
/// all four edges, fit lines, intersect — [`refine_corners`]'s per-round
/// body (see that function for the two-round rationale).
fn refine_round(
    source: &LumaView,
    corners_in: &[[f64; 2]; 4],
    bits: &BitMatrix,
    inverted: bool,
) -> Option<RefinedCorners> {
    let dim = bits.dim;
    let dimf = dim as f64;
    let corners_source = *corners_in;
    let region = PerspectiveTransform::square_to_quad(corners_source)?;

    let mut edge_fits: [Option<EdgeFit>; 4] = [None, None, None, None];
    let mut edge_stats: [EdgeStat; 4] = [EdgeStat::default(); 4];

    for (edge_idx, &(a, b)) in EDGE_TO_CORNERS.iter().enumerate() {
        let d = [corners_source[b][0] - corners_source[a][0], corners_source[b][1] - corners_source[a][1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len < 1e-9 {
            continue; // degenerate (coincident corners) — no line possible
        }
        // Edge-average module length, used only to scale `fit_edge`'s
        // outlier-residual threshold. The PROBES do not use it: each probe
        // derives its own local outward normal and sample step below —
        // under perspective, the module size ALONG THE NORMAL varies along
        // the edge and can differ from this edge-direction average by tens
        // of percent, which would both mis-scale the profile (a "0.5
        // module" step that isn't) and break the bar-unfolding's exact
        // 2-sample imposter offset (see `localize_edge_point`'s doc).
        let module_px = len / dimf;

        // Probe selection: dark border modules, middle ~80% of the edge,
        // 2 sub-positions each, capped. Each candidate carries whether the
        // border module's immediate INWARD neighbor (one module further
        // into the grid) differs in color — categorical input to
        // `localize_edge_point`'s bar-unfolding correction (see its doc).
        let mut candidates: Vec<(f64, bool)> = Vec::new();
        for m in 0..dim {
            let (dark, inward_dark) = match edge_idx {
                0 => (bits.get(m, 0), bits.get(m, 1)),
                1 => (bits.get(dim - 1, m), bits.get(dim - 2, m)),
                2 => (bits.get(m, dim - 1), bits.get(m, dim - 2)),
                3 => (bits.get(0, m), bits.get(1, m)),
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
                candidates.push((pos_modules / dimf, !inward_dark));
            }
        }
        if candidates.len() > REFINE_MAX_POINTS_PER_EDGE {
            let stride = (candidates.len() as f64 / REFINE_MAX_POINTS_PER_EDGE as f64).ceil() as usize;
            candidates = candidates.into_iter().step_by(stride.max(1)).take(REFINE_MAX_POINTS_PER_EDGE).collect();
        }
        let probed = candidates.len();

        // Devernay profile + localization per candidate. The outward
        // normal and profile step are LOCAL, per probe: the boundary point
        // and its one-module-inward counterpart both map through `region`,
        // and their px difference gives simultaneously the true outward
        // direction AND the local module length along it — the same
        // per-probe construction `sample.rs`'s `probe_edge_pass` already
        // established. Under perspective the two vary along the edge, and
        // the bar-unfolding correction specifically NEEDS the imposter to
        // sit at exactly `1 / REFINE_PROFILE_STEP_MODULES` samples inward,
        // which only the local perpendicular module length guarantees.
        let inv_dim = 1.0 / dimf;
        let mut points: Vec<EdgePoint> = Vec::with_capacity(candidates.len());
        for &(t, inward_differs) in &candidates {
            // Boundary point and one-module-inward point, unit coords.
            let ((bu, bv), (iu, iv)) = match edge_idx {
                0 => ((t, 0.0), (t, inv_dim)),
                1 => ((1.0, t), (1.0 - inv_dim, t)),
                2 => ((t, 1.0), (t, 1.0 - inv_dim)),
                3 => ((0.0, t), (inv_dim, t)),
                _ => unreachable!("EDGE_TO_CORNERS has exactly 4 entries"),
            };
            let coarse = region.map(bu, bv);
            let inner = region.map(iu, iv);
            let out = [coarse[0] - inner[0], coarse[1] - inner[1]];
            let perp_module_px = (out[0] * out[0] + out[1] * out[1]).sqrt();
            if perp_module_px < 1e-9 {
                continue; // degenerate local geometry — no direction to walk
            }
            let normal = [out[0] / perp_module_px, out[1] / perp_module_px];
            let step = REFINE_PROFILE_STEP_MODULES * perp_module_px;
            if let Some(pt) =
                localize_edge_point(source, coarse, normal, step, inverted, inward_differs)
            {
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

/// Refine `code_corners_working` (TL, TR, BR, BL, WORKING px) against the
/// SOURCE image, per the module doc's algorithm. `sx`/`sy` are the
/// per-axis working/source ratios (`source_px = working_px / s` — see
/// `sample::SourceView`'s convention); pass `1.0, 1.0` when source ==
/// working (no downscale happened — refinement still runs in that case,
/// it just has nothing to lift). `bits` is the RS-validated bit matrix the
/// decode actually used. Returns `None` when fewer than 2 of the 4 edges
/// produce a usable line.
///
/// # Two rounds (evidence-guided re-anchoring)
/// [`refine_round`] runs twice: once anchored on the caller's coarse
/// corners, then once more anchored on the first round's refined corners,
/// and the second round's result wins. The probe GEOMETRY — each probe's
/// outward normal, its 0.5-module profile step, and critically the
/// bar-unfolding correction's exact 1-module imposter offset (see
/// [`localize_edge_point_pass`]) — all derive from the anchor quad, so an
/// anchor scale error translates directly into a small systematic
/// localization bias on every bar-corrected probe (measured on the Task 3
/// perturbed synthetic gate: ±0.3-module coarse corner displacement ⇒
/// ~2.6% quad scale error ⇒ ~0.1 px of coherent, non-averaging bias on
/// roughly half of each edge's probes). Round 1's corners land within a
/// couple tenths of a px of the truth, making round 2's derived geometry
/// accurate to ~0.2%, at which point the bias is far below the gate's
/// budget. This is the same evidence-over-provisional principle
/// `alignment.rs`'s parallelogram rule and `sample.rs`'s
/// `probe_edge_line` two-pass already establish, applied at the corner-
/// geometry level. Exactly 2 rounds, not iterate-to-convergence: the
/// geometry error is quadratically suppressed after one re-anchoring, and
/// a fixed count keeps the cost bound trivial. If round 2 fails outright
/// (it can only see fewer valid edges than round 1 if the tighter anchor
/// exposes a genuinely marginal edge), round 1's result is returned — a
/// weaker but still evidence-based refinement, never a coarse echo.
pub(crate) fn refine_corners(
    source: &LumaView,
    sx: f64,
    sy: f64,
    code_corners_working: &[[f64; 2]; 4],
    bits: &BitMatrix,
    inverted: bool,
) -> Option<RefinedCorners> {
    // `< 2` (not just `== 0`): each round's probe-selection loop reads
    // border modules' inward neighbors at `dim - 2`. Any real decoded QR
    // has `dim >= 21`; this is purely a defensive bound.
    if bits.dim < 2 {
        return None;
    }
    let corners_source: [[f64; 2]; 4] = code_corners_working.map(|[x, y]| [x / sx, y / sy]);
    let first = refine_round(source, &corners_source, bits, inverted)?;
    match refine_round(source, &first.corners, bits, inverted) {
        Some(second) => Some(second),
        None => Some(first),
    }
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
    /// worth handling (see `localize_edge_point`'s sign-filtered
    /// nearest-to-center selection), but it defeats the POINT of testing
    /// against an antialiased render specifically, so the jitter ensures
    /// genuine partial-coverage antialiasing actually occurs for every
    /// pose.
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

    const MODULE_PX: f64 = 6.0;
    const SUPERSAMPLE: usize = 8;

    /// Deterministic per-corner coarse-input jitter for the perturbed gate
    /// variant, in MODULES (scaled by the render's px/module at use):
    /// every corner displaced by a different direction/magnitude up to the
    /// full ±0.3-module budget on at least one axis, fixed (not RNG) so
    /// the gate is reproducible. ±0.3 module is well beyond the coarse
    /// error a decode-surviving candidate actually carries (a grid
    /// misplaced by ~half a module stops RS-decoding), so passing this
    /// bounds the "coarse echo" failure mode categorically.
    const PERTURB_MODULES: [[f64; 2]; 4] =
        [[0.3, -0.2], [-0.25, 0.3], [0.2, 0.25], [-0.3, -0.15]];

    /// Shared body of the two synthetic accuracy gate variants: render 3
    /// poses x 2 versions (antialiased 8x supersample + box reduce + mild
    /// blur), refine with the given coarse input (ground truth itself, or
    /// truth + [`PERTURB_MODULES`]), and return the per-config
    /// (version, pose, mean px, max px) error table vs analytic truth.
    /// Asserts every corner actually refined (not a fallback) — a silent
    /// fallback would mask a real accuracy regression behind whatever
    /// error the coarse input happens to carry.
    fn run_accuracy_gate(label: &str, perturb: Option<&[[f64; 2]; 4]>) -> Vec<(i16, &'static str, f64, f64)> {
        let mut table: Vec<(i16, &'static str, f64, f64)> = Vec::new();
        let cases: [(i16, &[u8]); 2] = [(1, b"REFINE1"), (7, b"REFINEV7TESTPAYLOAD")];
        let poses: [(&'static str, Pose); 3] =
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
                // `- 0.5` per axis: convert the render transform's
                // half-integer-pixel-center coordinates (testpaint samples
                // pixel `i`'s content at `i + 0.5`) into the crate's
                // integer-pixel-center convention `refine_corners`
                // operates in — see `bilinear_at`'s convention doc.
                let truth: [[f64; 2]; 4] = [
                    transform.map(0.0, 0.0),
                    transform.map(1.0, 0.0),
                    transform.map(1.0, 1.0),
                    transform.map(0.0, 1.0),
                ]
                .map(|[x, y]| [x - 0.5, y - 0.5]);
                let coarse: [[f64; 2]; 4] = match perturb {
                    Some(offsets) => std::array::from_fn(|i| {
                        [
                            truth[i][0] + offsets[i][0] * MODULE_PX,
                            truth[i][1] + offsets[i][1] * MODULE_PX,
                        ]
                    }),
                    None => truth,
                };
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

                let refined = refine_corners(&view, 1.0, 1.0, &coarse, &bits, false).unwrap_or_else(|| {
                    panic!("{label} v{version} {pose_name}: refine_corners returned None")
                });
                for i in 0..4 {
                    assert!(
                        refined.corner_refined[i],
                        "{label} v{version} {pose_name}: corner {i} fell back to unrefined \
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

        eprintln!("Task 3 synthetic accuracy gate [{label}] (version, pose, mean px, max px):");
        for (v, p, mean, max) in &table {
            eprintln!("  v{v:<3} {p:<11} mean={mean:.4}px max={max:.4}px");
        }
        table
    }

    fn assert_gate(label: &str, table: &[(i16, &str, f64, f64)]) {
        for (version, pose_name, mean, max) in table {
            assert!(
                *mean <= 0.05,
                "{label} v{version} {pose_name}: mean corner error {mean:.4}px exceeds 0.05px"
            );
            assert!(
                *max <= 0.15,
                "{label} v{version} {pose_name}: max corner error {max:.4}px exceeds 0.15px"
            );
        }
    }

    /// Task 3's synthetic accuracy gate (Global Constraints gate 1),
    /// truth-conditioned variant: the coarse input handed to
    /// `refine_corners` IS the ground-truth quad — isolates refinement's
    /// own numerical precision (Devernay localization + weighted TLS +
    /// intersection) from coarse-detection accuracy.
    #[test]
    fn synthetic_accuracy_gate() {
        let table = run_accuracy_gate("truth-conditioned", None);
        assert_gate("truth-conditioned", &table);
    }

    /// Perturbed-coarse variant (review follow-up): same renders and the
    /// same ≤0.05px-mean / ≤0.15px-max bar, but the coarse input quad is
    /// displaced by the deterministic [`PERTURB_MODULES`] per-corner
    /// offsets (up to ±0.3 module) before refinement. This categorically
    /// catches any "coarse echo" — any code path that lets the coarse
    /// position leak into a localized point's coordinates (as the
    /// pre-review `delta = 0` contamination fallback did: anchored at the
    /// COARSE profile center, it made every fallback probe vote FOR the
    /// coarse error, which the truth-conditioned variant can never see
    /// because there coarse == truth). Errors are still measured against
    /// TRUTH, so passing requires refinement to fully shed the injected
    /// coarse displacement.
    #[test]
    fn synthetic_accuracy_gate_perturbed_coarse() {
        let table = run_accuracy_gate("perturbed-coarse", Some(&PERTURB_MODULES));
        assert_gate("perturbed-coarse", &table);
    }

    /// Inverted-polarity pin for `localize_edge_point`'s sign convention
    /// (light ink on dark background flips the expected gradient sign —
    /// see its sign-convention doc): a v1 render with swapped ink/light
    /// levels and `inverted: true` must clear the exact same accuracy bar,
    /// under the perturbed coarse input (the stricter variant). If the
    /// sign convention were derived backwards, the selector would lock
    /// onto the inward imposter transitions instead and miss this bar by
    /// whole pixels.
    #[test]
    fn synthetic_accuracy_gate_inverted_polarity() {
        let bits = qr_bitmatrix(b"REFINE1", 1);
        let dim = bits.dim;
        let size = dim as f64 * MODULE_PX;
        let canvas = (size * 1.6 + 40.0).ceil();
        let img_side = canvas as usize;

        let quad = pose_quad(Pose::Perspective, size, canvas);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        // `- 0.5` per axis: testpaint→crate pixel-center-convention
        // conversion, same as `run_accuracy_gate`'s (see the note there).
        let truth: [[f64; 2]; 4] = [
            transform.map(0.0, 0.0),
            transform.map(1.0, 0.0),
            transform.map(1.0, 1.0),
            transform.map(0.0, 1.0),
        ]
        .map(|[x, y]| [x - 0.5, y - 0.5]);
        let coarse: [[f64; 2]; 4] = std::array::from_fn(|i| {
            [
                truth[i][0] + PERTURB_MODULES[i][0] * MODULE_PX,
                truth[i][1] + PERTURB_MODULES[i][1] * MODULE_PX,
            ]
        });
        // Swapped levels: logically-dark modules render LIGHT (235) on a
        // DARK (25) background — the inverted-polarity scene.
        let img = render_module_grid_transformed_antialiased(
            dim,
            |x, y| bits.get(x, y),
            235,
            25,
            &transform,
            img_side,
            img_side,
            SUPERSAMPLE,
        );
        let img = mild_blur(&img, img_side, img_side);
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();

        let refined = refine_corners(&view, 1.0, 1.0, &coarse, &bits, true)
            .expect("inverted v1 perspective: refine_corners returned None");
        for i in 0..4 {
            assert!(refined.corner_refined[i], "inverted: corner {i} fell back to unrefined");
        }
        let errors: [f64; 4] = std::array::from_fn(|i| corner_error(refined.corners[i], truth[i]));
        let mean = errors.iter().sum::<f64>() / 4.0;
        let max = errors.iter().cloned().fold(0.0, f64::max);
        eprintln!("inverted v1 perspective: mean={mean:.4}px max={max:.4}px");
        assert!(mean <= 0.05, "inverted: mean corner error {mean:.4}px exceeds 0.05px");
        assert!(max <= 0.15, "inverted: max corner error {max:.4}px exceeds 0.15px");
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
