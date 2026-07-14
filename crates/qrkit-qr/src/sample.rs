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
//!
//! `decode.rs` (Task 5) is this file's real caller, so the module-level
//! `dead_code` exemption that used to live here is removed.

use crate::alignment::{is_finder_corner, AlignmentGrid, AnchorSlot};
use crate::bitmatrix::BitMatrix;
use crate::consts::{
    BR_ANCHOR_FILTER_TOL_MODULES, BR_ANCHOR_POSITIONS, BR_MIN_EDGE_POINTS, BR_PROBE_COUNT,
    BR_PROBE_WINDOW_HALF_MODULES,
};
use crate::homography::PerspectiveTransform;
use crate::tiles::TileGrid;
use crate::trace::SampleRegionTrace;
use crate::triplet::TripletCandidate;
use crate::version::{sample_module_gray, sample_module_gray_bilinear, sample_module_ink};
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

impl SampleRegion {
    /// This region's Task 6 trace form: `module_rect` verbatim, plus the
    /// image-pixel quad its four module-rect corners map to through its
    /// own `transform` — corner quads, not per-module points, per the
    /// plan's trace-compactness constraint (the debug UI reconstructs the
    /// module grid from `BitsTrace`'s packed words via its own TS
    /// homography port). `dimension` is the whole candidate's module
    /// dimension (needed to normalize `module_rect`'s raw module-space
    /// corners into the `[0,1]` unit square `transform.map` expects — see
    /// this file's module doc's "Anchor / coordinate conventions").
    pub(crate) fn to_trace(&self, dimension: u32) -> SampleRegionTrace {
        let dimf = dimension as f64;
        let [x0, y0, x1, y1] = self.module_rect;
        let quad = [
            self.transform.map(x0 as f64 / dimf, y0 as f64 / dimf),
            self.transform.map(x1 as f64 / dimf, y0 as f64 / dimf),
            self.transform.map(x1 as f64 / dimf, y1 as f64 / dimf),
            self.transform.map(x0 as f64 / dimf, y1 as f64 / dimf),
        ];
        SampleRegionTrace {
            module_rect: self.module_rect,
            quad,
        }
    }
}

/// The result of one [`sample_grid`] call.
pub(crate) struct SampledGrid {
    pub bits: BitMatrix,
    /// The regions used to build `bits`. `decode.rs` reads the single
    /// region's transform back out as the FINAL whole-grid transform for
    /// `DecodedCode.corners` (single-region case); Plan 4 Task 6
    /// additionally serializes them for the debug-UI sample-grid overlay.
    pub regions: Vec<SampleRegion>,
    /// Fraction of modules (`oob_count / dimension^2`) whose sample fell
    /// outside the source image (a clamped/missing read, counted as `false`
    /// in `bits`). The caller (`decode.rs`, Task 5) rejects a candidate
    /// when this exceeds `consts::MAX_OOB_FRACTION` — this file only
    /// measures and reports it.
    pub oob_fraction: f64,
    /// Raw per-module gray value (0..255, widened to `f32`) alongside every
    /// module `bits` binarized — Plan 4B Fix B's evidence for the
    /// reference-threshold + sharpening decode round `decode.rs` runs when
    /// the tile-threshold `bits` fail rqrr. Row-major `dim*dim`
    /// (`grays[y*dim+x]`, matching `BitMatrix`'s own `(x,y)` convention),
    /// `f32::NAN` marking an out-of-image sample (same modules `bits`
    /// records as `false`/`oob_fraction`-counted). Transient: never
    /// serialized into any trace, cheap at `dim^2` floats (max 177^2 ≈ 125K
    /// = 500KB at v40, one buffer per attempt, freed with this struct).
    pub grays: Vec<f32>,
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

/// `true` iff [`sample_grid`] would take the single-region path with *no*
/// image evidence at all for its 4th anchor — i.e. no real alignment
/// pattern anywhere that could correct the parallelogram BR extrapolation.
/// This is exactly the condition under which `decode.rs` runs
/// [`refine_fourth_corner`] (Plan 4 Task 5b): v1 always (no lattice), and
/// any higher version whose probe found neither the BR alignment pattern
/// nor any interior one.
pub(crate) fn needs_refined_br(alignment: &AlignmentGrid) -> bool {
    let n = alignment.coords.len();
    (n == 0 || !has_non_br_found_ap(alignment)) && br_slot_pos(alignment).is_none()
}

/// Binarized polarity at a continuous pixel position (`true` = ink,
/// polarity-aware). `None` off-image.
fn binarized_at(view: &LumaView, grid: &TileGrid, x: f64, y: f64, inverted: bool) -> Option<bool> {
    let (w, h) = (view.width() as isize, view.height() as isize);
    let (xi, yi) = (x.round() as isize, y.round() as isize);
    if xi < 0 || yi < 0 || xi >= w || yi >= h {
        return None;
    }
    let (xu, yu) = (xi as usize, yi as usize);
    Some((view.get(xu, yu) < grid.threshold_at(xu, yu)) != inverted)
}

/// A fitted edge line: total-least-squares centroid + unit direction.
///
/// `pub(crate)` (Plan 5 Task 3): `refine.rs`'s subpixel edge fit reuses
/// this exact shape (and [`fit_line_tls_weighted`]/[`intersect_lines`]
/// below) rather than duplicating the closed-form 2x2 eigenvector line fit
/// a second time in a second module.
pub(crate) type EdgeFit = qrkit_geometry::Line2;

/// Total-least-squares line through `pts` (principal axis of the 2x2
/// scatter matrix — the closed-form eigenvector via the half-angle
/// identity, the standard orthogonal-regression result). `pts` must have
/// at least 2 entries. Thin unweighted wrapper over [`fit_line_tls_weighted`]
/// (Plan 5 Task 3 extracted the weighted core out of this function; equal
/// weights reduce to the exact same arithmetic this always did).
fn fit_line_tls(pts: &[[f64; 2]]) -> EdgeFit {
    qrkit_geometry::fit_line_tls(pts).expect("scanner line inputs are validated by the caller")
}

/// Weighted total-least-squares line through `pts` (Plan 5 Task 3):
/// `refine.rs`'s edge fit weights each point by its Devernay profile's
/// gradient magnitude (a stronger-contrast localization should out-vote a
/// weaker one), so the plain unweighted centroid/scatter this module used
/// pre-Task-3 is generalized here to a per-point weight — the closed-form
/// principal-axis-of-the-scatter-matrix result is unchanged, just every
/// sum is now weighted. `pts` and `weights` must be the same non-empty
/// length; a zero (or all-zero) weight is the caller's problem (produces a
/// `NaN` centroid) — every caller here always has a strictly positive
/// gradient-magnitude weight for any point that reached the fit at all.
pub(crate) fn fit_line_tls_weighted(pts: &[[f64; 2]], weights: &[f64]) -> EdgeFit {
    qrkit_geometry::fit_line_tls_weighted(pts, weights)
        .expect("scanner line inputs are validated by the caller")
}

/// Intersection of two `EdgeFit` lines; `None` when near-parallel (the two
/// edges of a real quadrilateral meet at a healthy angle, so a
/// near-parallel pair means at least one fit is garbage).
pub(crate) fn intersect_lines(a: &EdgeFit, b: &EdgeFit) -> Option<[f64; 2]> {
    qrkit_geometry::intersect_lines(a, b)
}

/// Which module-region outer edge [`probe_edge_line`] traces.
#[derive(Clone, Copy)]
enum ProbeEdge {
    /// Module-space `y = dim`, probed along `x in [7, dim-7]` (the span
    /// nearest the BL finder is where the provisional prediction is most
    /// accurate — see [`probe_edge_line`]'s two-pass note).
    Bottom,
    /// Module-space `x = dim`, probed along `y in [7, dim-7]` (nearest the
    /// TR finder).
    Right,
}

/// Which robust estimator [`robust_edge_fit`] uses to separate true
/// boundary points from imposters. The two have *complementary* failure
/// domains (both measured during the Task 5b gate runs), so
/// [`refine_fourth_corner`] is run once per mode by `decode.rs`, each
/// resulting corner tried against the Reed-Solomon-validated decoder —
/// a wrong estimate can never produce a wrong payload, only a failed
/// retry.
///
/// - [`AnchorLine`](Self::AnchorLine): trust the two finder-border anchor
///   points and discard everything inconsistent with their line. Immune to
///   the parallel plate-edge/background imposter contour (which an
///   outward-hull rule actively *prefers* — observed on the 45-degree-tilt
///   synthetic fixtures, where the projected quiet zone compresses into
///   the probe window's reach), but its short 3-module anchor baseline
///   amplifies per-anchor localization noise on small-module real captures
///   (~4px/module: up to ~0.9 modules of deviation at the span's far end),
///   where it can wrongly reject true far points.
/// - [`OuterHull`](Self::OuterHull): keep the outermost consistent point
///   cluster. Robust to per-point noise and to the whole-module *inward*
///   bogus points (a light edge-adjacent module), but latches onto the
///   outward imposter contour when one is in reach.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdgeFitMode {
    AnchorLine,
    OuterHull,
}

/// One probe pass over an edge: located boundary points, the mean outward
/// walk direction (orients the robust fit's outward normal), and the mean
/// local module length in pixels (scales the robust fit's tolerance).
struct EdgePass {
    points: Vec<[f64; 2]>,
    /// How many of the leading entries of `points` are accepted
    /// [`BR_ANCHOR_POSITIONS`] finder-border probes (they are probed first,
    /// so they always occupy the front of the vec).
    anchor_count: usize,
    out_dir: [f64; 2],
    mean_module_px: f64,
}

/// Run [`BR_PROBE_COUNT`] boundary probes along one edge.
///
/// Per probe: predict the boundary's image position — the provisional
/// transform's projection, or (when `guide` is given) the intersection of
/// the guide line with the probe's walk axis — then walk the perpendicular
/// (the local module-space outward direction mapped through `provisional`)
/// across a +/-[`BR_PROBE_WINDOW_HALF_MODULES`]-module window on the
/// binarized image and take the **last ink-to-background transition** (the
/// outer boundary of the outermost ink run). A probe is rejected when its
/// window contains no ink at all (e.g. the edge-adjacent module is light
/// and nothing dark sits within reach — common, ~half of a QR data
/// region's boundary modules are light) or when the window's outer end is
/// still ink (the true boundary was not bracketed, so the transition seen
/// would be an interior data-module edge, not the code's edge).
///
/// Probed positions: the two [`BR_ANCHOR_POSITIONS`] over the edge's
/// flanking finder's own dark outer border (guaranteed boundary ink,
/// payload-independent — see that constant's provenance), then the
/// [`BR_PROBE_COUNT`] positions across the amendment's `[7, dim-7]` data
/// span.
fn probe_edge_pass(
    view: &LumaView,
    grid: &TileGrid,
    inverted: bool,
    dimension: u32,
    provisional: &PerspectiveTransform,
    edge: ProbeEdge,
    guide: Option<&EdgeFit>,
) -> EdgePass {
    let dimf = dimension as f64;
    let mut points: Vec<[f64; 2]> = Vec::with_capacity(BR_ANCHOR_POSITIONS.len() + BR_PROBE_COUNT);
    let mut out_dir_sum = [0.0f64, 0.0f64];
    let mut module_px_sum = 0.0f64;

    let mut anchor_count = 0usize;
    let span_positions = (0..BR_PROBE_COUNT).map(|k| {
        let frac = k as f64 / (BR_PROBE_COUNT - 1) as f64;
        (7.0 + frac * (dimf - 14.0), false)
    });
    let anchor_positions = BR_ANCHOR_POSITIONS.into_iter().map(|a| (a, true));
    for (a, is_anchor) in anchor_positions.chain(span_positions) {
        // Boundary point and one-module-inward point, in unit-square coords.
        let ((bu, bv), (iu, iv)) = match edge {
            ProbeEdge::Bottom => (((a / dimf), 1.0), ((a / dimf), (dimf - 1.0) / dimf)),
            ProbeEdge::Right => ((1.0, (a / dimf)), ((dimf - 1.0) / dimf, (a / dimf))),
        };
        let b = provisional.map(bu, bv);
        let inner = provisional.map(iu, iv);
        let d = [b[0] - inner[0], b[1] - inner[1]];
        let m_px = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if m_px < 1.0 {
            continue; // sub-pixel modules: nothing here is measurable
        }
        let dn = [d[0] / m_px, d[1] / m_px];

        // Predicted boundary: guide-line intersection with this probe's
        // walk axis when a guide exists, else the provisional projection.
        let predicted = match guide {
            Some(fit) => {
                let cross = dn[0] * fit.dir[1] - dn[1] * fit.dir[0];
                if cross.abs() > 1e-9 {
                    let dx = fit.centroid[0] - b[0];
                    let dy = fit.centroid[1] - b[1];
                    let s = (dx * fit.dir[1] - dy * fit.dir[0]) / cross;
                    [b[0] + s * dn[0], b[1] + s * dn[1]]
                } else {
                    b
                }
            }
            None => b,
        };

        // Walk the window at ~8 samples/module (transition localized to
        // ~1/16 module), floored at a quarter-pixel so tiny modules still
        // advance through distinct pixels.
        let half = BR_PROBE_WINDOW_HALF_MODULES * m_px;
        let step = (m_px / 8.0).max(0.25);
        let n_steps = (2.0 * half / step).ceil() as usize;
        let mut last_ink: Option<usize> = None;
        let mut run_len = 0usize; // consecutive ink samples ending at the current sample
        let mut last_run_len = 0usize; // run length ending at `last_ink`
        for si in 0..=n_steps {
            let s = -half + si as f64 * step;
            let px = [predicted[0] + dn[0] * s, predicted[1] + dn[1] * s];
            // Off-image reads as background: the quiet zone continuing out
            // of frame must not veto a boundary that is itself in-frame.
            if binarized_at(view, grid, px[0], px[1], inverted).unwrap_or(false) {
                run_len += 1;
                last_ink = Some(si);
                last_run_len = run_len;
            } else {
                run_len = 0;
            }
        }
        let li = match last_ink {
            Some(li) if li < n_steps => li,
            _ => continue, // no ink at all, or boundary not bracketed
        };
        // The ink run whose outer end is about to be accepted as a boundary
        // point must span at least half a module: ISO 18004's smallest ink
        // feature is one full module, so even a blur-eroded genuine
        // boundary module leaves well over half a module of consecutive ink
        // samples, while an isolated binarization speck (sensor noise —
        // observed accepting 1-sample "runs" on the noisier golden
        // fixtures, which then pulled the line fit off the true edge) spans
        // one or two samples. A speck-terminated window rejects the probe
        // rather than yielding a false point.
        if (last_run_len as f64) * step < 0.5 * m_px {
            continue;
        }
        let s_mid = -half + (li as f64 + 0.5) * step;
        points.push([predicted[0] + dn[0] * s_mid, predicted[1] + dn[1] * s_mid]);
        if is_anchor {
            anchor_count += 1;
        }
        out_dir_sum[0] += dn[0];
        out_dir_sum[1] += dn[1];
        module_px_sum += m_px;
    }

    let n = points.len().max(1) as f64;
    EdgePass {
        points,
        anchor_count,
        out_dir: [out_dir_sum[0] / n, out_dir_sum[1] / n],
        mean_module_px: module_px_sum / n,
    }
}

/// Robust line fit over one pass's boundary points.
///
/// Two classes of false points exist (both measured on the golden suite):
/// *inward* points offset by a whole module (a light edge-adjacent module,
/// where the probe's transition is the next module row's own edge — ~1/4
/// of accepted points), and *outward* structure captured beyond the quiet
/// zone (the plate edge / dark scene background running parallel to the
/// code edge, reached when perspective compresses the projected quiet zone
/// toward the probe window's reach — observed >= 1.2 modules out on the
/// 45-degree-tilt fixtures). An outward-hull rule alone separates the
/// first class but is actively wrong for the second (it prefers the
/// outermost line — the imposter).
///
/// **Anchor-line filter (primary):** when both
/// [`BR_ANCHOR_POSITIONS`] finder-border probes accepted, the line through
/// those two points is payload-independent ground truth on the true edge,
/// so every point farther than
/// [`BR_ANCHOR_FILTER_TOL_MODULES`] (perpendicular) from it is discarded
/// — both false classes at once (see that constant's derivation) — and
/// the survivors (anchors included) are refit by total least squares.
///
/// **Outward-hull (fallback, anchors unavailable):** fit all points,
/// measure each point's signed residual along the fit's outward normal,
/// keep only points within half a module of the outermost one, refit. If
/// that keeps fewer than 2 points (an isolated outward outlier — e.g. a
/// binarization speck in a noisy real capture — grabbed the hull for
/// itself), drop that single outermost point and redo the hull once; a
/// genuine edge always has multiple points on it, a speck doesn't.
fn robust_edge_fit(pass: &EdgePass, mode: EdgeFitMode) -> EdgeFit {
    if mode == EdgeFitMode::AnchorLine && pass.anchor_count >= 2 {
        // Exact line through the two anchor points (they are the first two
        // entries — see EdgePass::anchor_count).
        let (a0, a1) = (pass.points[0], pass.points[1]);
        let d = [a1[0] - a0[0], a1[1] - a0[1]];
        let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if len > 1e-9 {
            let dir = [d[0] / len, d[1] / len];
            let normal = [-dir[1], dir[0]];
            let tol = BR_ANCHOR_FILTER_TOL_MODULES * pass.mean_module_px;
            let kept: Vec<[f64; 2]> = pass
                .points
                .iter()
                .copied()
                .filter(|p| ((p[0] - a0[0]) * normal[0] + (p[1] - a0[1]) * normal[1]).abs() <= tol)
                .collect();
            // `kept` always contains the two anchors themselves, so a TLS
            // fit is always defined.
            return fit_line_tls(&kept);
        }
    }
    fn hull_kept(points: &[[f64; 2]], out_dir: [f64; 2], tol: f64) -> Vec<[f64; 2]> {
        let fit = fit_line_tls(points);
        let mut normal = [-fit.dir[1], fit.dir[0]];
        if normal[0] * out_dir[0] + normal[1] * out_dir[1] < 0.0 {
            normal = [-normal[0], -normal[1]];
        }
        let residual = |p: &[f64; 2]| {
            (p[0] - fit.centroid[0]) * normal[0] + (p[1] - fit.centroid[1]) * normal[1]
        };
        let r_max = points
            .iter()
            .map(residual)
            .fold(f64::NEG_INFINITY, f64::max);
        points
            .iter()
            .copied()
            .filter(|p| residual(p) >= r_max - tol)
            .collect()
    }

    let tol = 0.5 * pass.mean_module_px;
    let kept = hull_kept(&pass.points, pass.out_dir, tol);
    if kept.len() >= 2 {
        return fit_line_tls(&kept);
    }
    // Isolated outward outlier: drop it and re-hull the remainder.
    if pass.points.len() >= 3 && kept.len() == 1 {
        let outlier = kept[0];
        let rest: Vec<[f64; 2]> = pass
            .points
            .iter()
            .copied()
            .filter(|p| *p != outlier)
            .collect();
        let kept2 = hull_kept(&rest, pass.out_dir, tol);
        if kept2.len() >= 2 {
            return fit_line_tls(&kept2);
        }
    }
    fit_line_tls(&pass.points)
}

/// Trace one outer edge of the module region: a provisional-guided probe
/// pass, a robust fit, then a second pass re-probing every position with
/// its window centered on that fitted line, and a final robust fit.
///
/// # Why two passes (evidence-guided re-probing)
/// The measured affine-parallelogram divergence at the far probes reaches
/// 2.06 modules on the failing 45-degree-tilt fixtures (see
/// `consts::BR_PROBE_WINDOW_HALF_MODULES`'s provenance) — beyond the
/// +/-1.5-module window — so a single provisional-guided pass can only
/// bracket the true boundary at the probes nearest the anchoring finder
/// (where the same measurement shows 0.46-0.88 modules of divergence).
/// Those near points still pin the edge's *line* (projectively straight in
/// image space), and the second pass re-probes all positions with windows
/// centered on it, recovering the far positions the provisional missed.
/// This is `alignment.rs`'s established prediction principle (evidence
/// from already-located structure over the provisional transform), applied
/// per edge rather than per lattice node. Pass 1 needs only >= 2 points (a
/// line to guide pass 2); the amendment's [`BR_MIN_EDGE_POINTS`] >= 5 gate
/// applies to the final, pass-2 accepted count.
fn probe_edge_line(
    view: &LumaView,
    grid: &TileGrid,
    inverted: bool,
    dimension: u32,
    provisional: &PerspectiveTransform,
    edge: ProbeEdge,
    mode: EdgeFitMode,
) -> Option<EdgeFit> {
    let pass1 = probe_edge_pass(view, grid, inverted, dimension, provisional, edge, None);
    if pass1.points.len() < 2 {
        return None;
    }
    let guide = robust_edge_fit(&pass1, mode);
    let pass2 = probe_edge_pass(
        view,
        grid,
        inverted,
        dimension,
        provisional,
        edge,
        Some(&guide),
    );
    if pass2.points.len() < BR_MIN_EDGE_POINTS {
        return None;
    }
    Some(robust_edge_fit(&pass2, mode))
}

/// Estimate the module region's bottom-right OUTER corner (module-space
/// `(dim, dim)`) directly from the image: trace the bottom and right outer
/// edges with [`probe_edge_line`] and intersect the two fitted lines
/// (Plan 4 Task 5b — zxing-cpp edge-tracing practice / the original GPU
/// scanner's `improve_corners` concept at detection precision).
///
/// `None` when either edge cannot produce a trustworthy fit
/// ([`BR_MIN_EDGE_POINTS`] not reached) or the two fits are near-parallel —
/// the caller then falls back to the previous parallelogram behavior.
pub(crate) fn refine_fourth_corner(
    view: &LumaView,
    grid: &TileGrid,
    t: &TripletCandidate,
    dimension: u32,
    provisional: &PerspectiveTransform,
    mode: EdgeFitMode,
) -> Option<[f64; 2]> {
    let bottom = probe_edge_line(
        view,
        grid,
        t.inverted,
        dimension,
        provisional,
        ProbeEdge::Bottom,
        mode,
    )?;
    let right = probe_edge_line(
        view,
        grid,
        t.inverted,
        dimension,
        provisional,
        ProbeEdge::Right,
        mode,
    )?;
    intersect_lines(&bottom, &right)
}

/// Plan 5 Task 2's source-resolution module sampling: the SOURCE view
/// [`sample_grid`] samples module GRAYS from (instead of the WORKING
/// `view`/`grid` every other stage — tiling, alignment, timing/version-bits
/// — still runs on) whenever a downscale actually happened, plus the
/// PER-AXIS working/source scales needed to lift a region's module→working
/// `transform` to module→source (see [`Self::lift`]).
///
/// Per-axis, not one scalar: `downscale_luma` rounds each destination axis
/// independently (`round(dim * max_dim / longest)` per axis — see its
/// doc), so for a non-cleanly-scaling source the two axes' true ratios
/// genuinely differ (e.g. 191x144 at max-dim 128 → 128x97: `sx = 128/191 ≈
/// 0.6702`, `sy = 97/144 ≈ 0.6736`). Applying the width ratio to BOTH axes
/// — as this struct's first draft did — mis-lifts the far (bottom) edge by
/// up to ~0.75 source px (~0.25 module at IMG_4832's scale), real error
/// against Plan 5's ≤0.1 px Task 3 refinement budget.
/// `Detections::source_scale` (the public scalar) deliberately stays
/// width-pinned per the plan; these two fields are the internal, exact
/// per-axis form.
///
/// Convention per axis, matching `Detections::source_scale`'s direction:
/// `working_px = source_px * s`, each `<= 1`.
#[derive(Clone, Copy)]
pub(crate) struct SourceView<'a> {
    pub view: &'a LumaView<'a>,
    /// `working_width / source_width`.
    pub sx: f64,
    /// `working_height / source_height`.
    pub sy: f64,
}

impl SourceView<'_> {
    /// Lift a module→WORKING transform to module→SOURCE: compose with the
    /// per-axis inverse scale `scaled(1/sx, 1/sy)` (`source_px =
    /// working_px / s`, per axis). [`sample_grid`]'s one lift site,
    /// factored onto the struct so the per-axis correctness is
    /// unit-testable against `downscale_luma`'s actual independent-axis
    /// rounding (see
    /// `lift_uses_per_axis_scales_when_downscale_rounding_diverges`).
    pub(crate) fn lift(&self, module_to_working: &PerspectiveTransform) -> PerspectiveTransform {
        module_to_working.then(&PerspectiveTransform::scaled(1.0 / self.sx, 1.0 / self.sy))
    }
}

/// Map a module center to its WORKING-resolution tile-threshold lookup
/// coordinates, clamped to `view`'s bounds. Used only by [`sample_grid`]'s
/// source-resolution path (Plan 5 Task 2): the tile-threshold table
/// ([`TileGrid`]) is only ever built at working resolution, so a module
/// sampled in source space still needs a WORKING pixel coordinate to look
/// one up.
///
/// This is exactly `transform.map(col / dim, row / dim)` rounded — i.e.
/// the *same* working-space position the source-space sample would reach
/// by computing `source_px = SourceView::lift(transform).map(...)` and
/// then multiplying back by `(sx, sy)` per axis (the plan's literal
/// "source pixel -> working tile via coordinate division" framing):
/// `working.then(&scaled(1/sx,1/sy)).map(p)` is `working.map(p)` divided
/// per-axis by `(sx, sy)`, so re-multiplying by `(sx, sy)` recovers
/// `working.map(p)` again, up to a floating-point epsilon from the extra
/// multiply/divide that is irrelevant to a value only ever used for a
/// rounded table lookup. Computing it directly through the untouched
/// working `transform` (rather than round-tripping through the lifted
/// transform) is simpler and marginally more precise, not a different
/// coordinate.
///
/// Clamped rather than `None`-on-OOB (unlike every other sampling
/// primitive in this file): a threshold lookup is a lookup table, not
/// sample data, and [`TileGrid::threshold_at`] panics on an out-of-bounds
/// index — real OOB accounting for a source-resolution sample is entirely
/// the source-space bilinear read's job (see [`sample_grid`]'s "OOB
/// accounting" note), so this only ever needs to produce *some* valid
/// working-space tile, never report missing data itself.
fn working_tile_coords(
    view: &LumaView,
    transform: &PerspectiveTransform,
    dimension: u32,
    col: f64,
    row: f64,
) -> (usize, usize) {
    let dim = dimension as f64;
    let [px, py] = transform.map(col / dim, row / dim);
    let xu = (px.round() as i64).clamp(0, view.width() as i64 - 1) as usize;
    let yu = (py.round() as i64).clamp(0, view.height() as i64 - 1) as usize;
    (xu, yu)
}

/// Build the per-region sampling grid for one candidate.
///
/// - **No usable alignment patterns** (v1 — `alignment.coords` is empty —
///   or every non-finder-corner, non-bottom-right-most slot is `Missing`,
///   see [`has_non_br_found_ap`]): a single region covering the whole
///   `[0, dimension) x [0, dimension)` grid, transform = the classic
///   finder-only `provisional_transform`, *except* when better 4th-anchor
///   evidence exists, in priority order: a `Found` bottom-right
///   alignment-pattern slot (zxing Java's own single-AP behavior; see
///   [`provisional_quad`]), else the caller's `refined_br` — an
///   image-derived module-region OUTER corner from
///   [`refine_fourth_corner`], paired with module-space `(dim, dim)`
///   (Plan 4 Task 5b's mixed-anchor quad; `quad_to_quad` handles arbitrary
///   4-correspondence quads, so mixing finder centers with an outer corner
///   is fine). Only with neither does the pure parallelogram guess remain.
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
/// # Plan 5 Task 2: source-resolution module sampling
/// `source` is `None` whenever `scan()` didn't downscale (or the caller is
/// `detect`/`detect_with`, which never does) — sampling then behaves
/// EXACTLY as before, reading nearest-neighbor grays off `view` (the
/// bit-identical regression pin for the 3 near-res golden fixtures relies
/// on this branch being untouched). When `Some`, every module's GRAY is
/// instead read via [`crate::version::sample_module_gray_bilinear`] off
/// the SOURCE view, through the region's `transform` lifted to
/// module→source by [`SourceView::lift`] (per-axis inverse scales — see
/// that method's and the struct's docs) — the fix
/// for far codes that DETECT fine at working resolution but can't sample
/// at ~2 working px/module (real-photo evidence: `IMG_4832`). The
/// tile-threshold lookup that binarizes that gray still comes from the
/// WORKING `grid` (there is no source-resolution threshold table — see
/// [`working_tile_coords`]'s doc), so the resulting `ink`/`bits` are a
/// working-threshold-against-source-gray hybrid, same as the plan's
/// "document the approximation" note calls for. **OOB accounting is
/// entirely in source space** in this branch: a module's contribution to
/// `oob_fraction` (and its `bits` entry, forced `false`) is decided solely
/// by whether the source-space bilinear sample succeeded, never by the
/// working-space threshold lookup (which is clamped, so it can't itself
/// produce a `None`). Scope note: detection itself (tiling, finder/triplet
/// grouping, alignment-pattern location, timing/version-bits reads) is
/// unaffected either way — only this function's module sampling moves to
/// source resolution.
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
    refined_br: Option<[f64; 2]>,
    source: Option<SourceView>,
) -> Option<SampledGrid> {
    let dim = dimension as usize;
    let n = alignment.coords.len();
    let dimf = dimension as f64;

    let regions: Vec<SampleRegion> = if n == 0 || !has_non_br_found_ap(alignment) {
        let (src, dst) = match (br_slot_pos(alignment), refined_br) {
            // A real BR alignment pattern beats everything (it's a probed,
            // re-centered 5x5 target, not an edge-fit extrapolation).
            (Some(ap), _) => provisional_quad(t, dimension, Some(ap)),
            // Task 5b: image-derived outer corner as the 4th anchor.
            (None, Some(rc)) => (
                [
                    [3.5, 3.5],
                    [dimf - 3.5, 3.5],
                    [dimf, dimf],
                    [3.5, dimf - 3.5],
                ],
                [t.tl, t.tr, rc, t.bl],
            ),
            (None, None) => provisional_quad(t, dimension, None),
        };
        let transform = build_transform(src, dst, dimension as f64)?;
        vec![SampleRegion {
            module_rect: [0, 0, dimension, dimension],
            transform,
        }]
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

                regions.push(SampleRegion {
                    module_rect: [col0, row0, col1, row1],
                    transform,
                });
            }
        }
        regions
    };

    let mut bits = BitMatrix::new(dim);
    let mut grays = vec![f32::NAN; dim * dim];
    let mut oob = 0u64;
    for region in &regions {
        // Lifted once per region (not per module — every module in a
        // region shares the same module→working `region.transform`, so the
        // module→source composition is invariant across the whole loop
        // below).
        let source_transform = source.map(|src| src.lift(&region.transform));

        let [x0, y0, x1, y1] = region.module_rect;
        for y in y0..y1 {
            for x in x0..x1 {
                let (mx, my) = (x as f64 + 0.5, y as f64 + 0.5);
                let (ink, gray) = match (source, &source_transform) {
                    (Some(src), Some(src_transform)) => {
                        let gray =
                            sample_module_gray_bilinear(src.view, src_transform, dimension, mx, my);
                        let ink = gray.map(|g| {
                            let (xu, yu) =
                                working_tile_coords(view, &region.transform, dimension, mx, my);
                            (g < grid.threshold_at(xu, yu) as f32) != t.inverted
                        });
                        (ink, gray)
                    }
                    _ => (
                        sample_module_ink(
                            view,
                            grid,
                            &region.transform,
                            dimension,
                            mx,
                            my,
                            t.inverted,
                        ),
                        sample_module_gray(view, &region.transform, dimension, mx, my),
                    ),
                };
                match ink {
                    Some(v) => bits.set(x as usize, y as usize, v),
                    None => {
                        oob += 1;
                        bits.set(x as usize, y as usize, false);
                    }
                }
                if let Some(gray) = gray {
                    grays[y as usize * dim + x as usize] = gray;
                }
            }
        }
    }
    let oob_fraction = oob as f64 / (dim * dim) as f64;
    Some(SampledGrid {
        bits,
        regions,
        oob_fraction,
        grays,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alignment::locate_alignment_patterns;
    use crate::downscale::downscale_luma;
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
            finder_indices: [0, 1, 2],
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
        let code = qrcode::QrCode::with_version(
            payload,
            qrcode::Version::Normal(version),
            qrcode::EcLevel::M,
        )
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
        assert_eq!(
            code.width(),
            dim,
            "{label}: qrcode crate produced an unexpected dimension"
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let tile_grid = TileGrid::build(&view);

        let t = triplet_from_transform(transform, dim);
        // The provisional transform is built from the triplet alone — NOT
        // handed the true render transform — exactly what a real upstream
        // pipeline stage would have to work with before any alignment
        // pattern has been located.
        let provisional = provisional_transform(&t, dim as u32);
        let alignment = locate_alignment_patterns(
            &view,
            &tile_grid,
            &provisional,
            version as u32,
            false,
            false,
        );
        let sampled = sample_grid(&view, &tile_grid, &t, dim as u32, &alignment, None, None)
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
                format!("AA{version}").as_bytes(),
                version,
                &aa_transform,
                aa_side,
                &format!("v{version} axis-aligned"),
            );

            let rot_side = rotated_img_side(dim, scale);
            let rot_quad = rotated_quad(dim, scale, 30.0, rot_side);
            let rot_transform = PerspectiveTransform::square_to_quad(rot_quad).unwrap();
            run_pipeline_and_assert(
                format!("RO{version}").as_bytes(),
                version,
                &rot_transform,
                rot_side,
                &format!("v{version} rotated-30deg"),
            );

            let persp_side = rotated_img_side(dim, scale);
            let persp_quad = perspective_quad(dim, scale, 0.97, persp_side);
            let persp_transform = PerspectiveTransform::square_to_quad(persp_quad).unwrap();
            run_pipeline_and_assert(
                format!("PW{version}").as_bytes(),
                version,
                &persp_transform,
                persp_side,
                &format!("v{version} perspective-warped"),
            );
        }
    }

    /// Pins the *superiority* of the AP-anchored correction over the
    /// finder-only single-transform fallback — the entire reason the
    /// per-region tiling (and the single-AP 4th-anchor override) exists.
    /// At keystone `narrow = 0.90` (a 10% top/bottom width difference),
    /// v1 — which has NO alignment patterns, so its sampling is bounded by
    /// `provisional_transform`'s 3-finders-plus-parallelogram
    /// reconstruction — measurably fails bit-for-bit **by design** (it
    /// already fails at the milder 0.94 keystone with 3/441 mismatches at
    /// the bottom-right corner; see `perspective_quad`'s doc). Versions
    /// 2/7/20, whose located alignment patterns feed real correction
    /// (v2: the single BR AP as 4th anchor; v7/v20: full region tiling),
    /// must still sample bit-for-bit exact at this same 0.90 keystone
    /// through the same real pipeline. Without this test, a regression in
    /// the AP-anchored paths could hide behind the main gate's
    /// deliberately mild 3% warp (chosen there only so v1 can pass too).
    ///
    /// Do NOT pin a harsher keystone than 0.90: 0.90 is the strongest
    /// value empirically confirmed for all three versions; harsher warps
    /// (e.g. 0.85, where v20 was observed to fail) are outside this task's
    /// verified envelope.
    #[test]
    fn ap_anchored_versions_survive_keystone_that_defeats_v1_fallback() {
        let scale = 4.0;
        let narrow = 0.90;
        for &version in &[2i16, 7, 20] {
            let dim = 17 + 4 * version as usize;
            let img_side = rotated_img_side(dim, scale);
            let quad = perspective_quad(dim, scale, narrow, img_side);
            let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
            run_pipeline_and_assert(
                format!("KS{version}").as_bytes(),
                version,
                &transform,
                img_side,
                &format!("v{version} keystone-0.90 (AP-anchored)"),
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
        let alignment = locate_alignment_patterns(
            &view,
            &tile_grid,
            &provisional,
            version as u32,
            false,
            false,
        );
        let sampled = sample_grid(&view, &tile_grid, &t, dim as u32, &alignment, None, None)
            .expect("sample_grid should still return a (bad) result, not None");
        assert!(
            sampled.oob_fraction > 0.02,
            "expected oob_fraction > 0.02 for a half-cropped frame, got {}",
            sampled.oob_fraction
        );
    }

    // --- Plan 5 Task 2: source-resolution module sampling ---

    /// Render `payload` at high resolution (`source_scale_px` px/module —
    /// the SOURCE), NN-downscale it (the production formula, same as
    /// `scan()` uses) to a working resolution whose OWN module scale is
    /// `working_module_px` px/module — deliberately as coarse as the
    /// `IMG_4832` real-photo regime this task's binding design targets
    /// (~2 working px/module: DETECT succeeds, nearest-neighbor SAMPLING at
    /// that scale is unreliable) — then runs the real
    /// `provisional_transform` -> `locate_alignment_patterns` ->
    /// `sample_grid` pipeline with `sample_grid`'s new `source` parameter
    /// pointing back at the SOURCE view. Returns the sampled bits, the
    /// `qrcode` crate's own ground-truth matrix, and the working/source
    /// scale actually used, so callers can assert bit-for-bit correctness
    /// exactly like this file's existing working-resolution-only gate does.
    ///
    /// The working-space `TripletCandidate`/`provisional_transform`/
    /// `locate_alignment_patterns` calls all run on a transform derived
    /// EXACTLY the way Task 2's own lifting works in reverse — the
    /// known SOURCE transform composed with `scaled(scale, scale)` — the
    /// same relationship `Detections::source_scale`'s doc documents
    /// (`working_px = source_px * scale`), so this test's own working
    /// geometry is consistent with what a real `scan()` call would hand
    /// `decode_candidates`.
    fn run_source_resolution_pipeline(
        payload: &[u8],
        version: i16,
        source_scale_px: f64,
        working_module_px: f64,
    ) -> (BitMatrix, qrcode::QrCode, f64) {
        let dim = 17 + 4 * version as usize;
        let (quad, img_side) = axis_aligned_quad(dim, source_scale_px, 4.0);
        let source_transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let (source_img, code) = render_code(payload, version, &source_transform, img_side);
        let source_view = LumaView::new(&source_img, img_side, img_side, img_side).unwrap();

        // Downscale (production NN formula) to a working resolution whose
        // own module scale is `working_module_px` px/module.
        let max_working_dim =
            (img_side as f64 * working_module_px / source_scale_px).round() as u32;
        let (working_buf, working_w, working_h) = downscale_luma(&source_view, max_working_dim)
            .expect("test setup: a downscale must actually be needed here");
        let working_view = LumaView::new(&working_buf, working_w, working_h, working_w).unwrap();
        let working_tile_grid = TileGrid::build(&working_view);
        // Per-axis, matching `SourceView`'s own convention (a square canvas
        // downscales to equal per-axis ratios here, but computing each from
        // its own axis keeps the helper honest either way).
        let sx = working_w as f64 / img_side as f64;
        let sy = working_h as f64 / img_side as f64;

        // The working-space transform a real `scan()` downscale would have
        // implied: the known SOURCE transform lifted DOWN to working px —
        // the inverse direction of Task 2's own module->source lift.
        let working_transform = source_transform.then(&PerspectiveTransform::scaled(sx, sy));
        let t = triplet_from_transform(&working_transform, dim);
        let provisional = provisional_transform(&t, dim as u32);
        let alignment = locate_alignment_patterns(
            &working_view,
            &working_tile_grid,
            &provisional,
            version as u32,
            false,
            false,
        );

        let sampled = sample_grid(
            &working_view,
            &working_tile_grid,
            &t,
            dim as u32,
            &alignment,
            None,
            Some(SourceView {
                view: &source_view,
                sx,
                sy,
            }),
        )
        .unwrap_or_else(|| panic!("sample_grid returned None"));
        (sampled.bits, code, sx)
    }

    #[test]
    fn source_resolution_sampling_is_bit_exact_at_2px_per_module_working_v1() {
        let (bits, code, scale) = run_source_resolution_pipeline(b"SRCRES1", 1, 16.0, 2.0);
        assert_bit_for_bit(
            &bits,
            &code,
            &format!("v1 source-res (working scale {scale:.4})"),
        );
    }

    #[test]
    fn source_resolution_sampling_is_bit_exact_at_2px_per_module_working_v7_multiregion() {
        let (bits, code, scale) =
            run_source_resolution_pipeline(b"SRCRESMULTIREGION", 7, 16.0, 2.0);
        assert_bit_for_bit(
            &bits,
            &code,
            &format!("v7 source-res multi-region (working scale {scale:.4})"),
        );
    }

    // Note: an analogous "working-only (source: None) sampling is measurably
    // WORSE at this same coarse scale" counter-check was deliberately tried
    // and dropped here — on a clean, noise-free axis-aligned synthetic
    // render, NN downscaling by an exact integer ratio (as this file's
    // helpers produce) lands every module boundary back on an exact pixel
    // boundary, so nearest-neighbor-only sampling stays bit-exact even at
    // 2 working px/module. That's a property of noise-free synthetic
    // renders, not evidence the mechanism above is a no-op: real capture
    // noise/blur/sub-pixel misalignment is exactly what source-resolution
    // sampling helps with, and that's what `plan5_gate3_img4832_decodes_at_source_resolution`
    // (`decode_gate.rs`) demonstrates on an actual photo instead.

    /// [`run_source_resolution_pipeline`]'s working-view module scale is
    /// reported so a caller can confirm the scenario is actually as coarse
    /// as intended (a regression here — e.g. an accidental change to
    /// `axis_aligned_quad`'s quiet-zone margin — could silently make the
    /// working view coarser or finer than the `2.0` px/module this test
    /// suite documents targeting).
    #[test]
    fn run_source_resolution_pipeline_actually_hits_the_targeted_working_scale() {
        let (_bits, _code, scale) = run_source_resolution_pipeline(b"SCALECHK", 1, 16.0, 2.0);
        // scale = working_px / source_px, and working_module_px =
        // source_scale_px * scale by construction (see the helper's doc).
        let working_module_px = 16.0 * scale;
        assert!(
            (working_module_px - 2.0).abs() < 1e-6,
            "expected ~2.0 working px/module, got {working_module_px}"
        );
    }

    /// Review finding (post-Task-2, pre-Task-3): `downscale_luma` rounds
    /// each destination axis INDEPENDENTLY, so for a non-cleanly-scaling
    /// source the width and height ratios genuinely differ — and a lift
    /// that applies the width ratio to both axes (the first draft's single
    /// scalar) mis-places the far edge by most of a source pixel, real
    /// error against Task 3's ≤0.1 px refinement budget.
    ///
    /// Concretely, through the REAL production downscale (not hand-picked
    /// ratios): 191x144 at max-dim 128 → `dst_w = round(191·128/191) =
    /// 128`, `dst_h = round(144·128/191) = round(96.503) = 97`, so `sx =
    /// 128/191 ≈ 0.67016` but `sy = 97/144 ≈ 0.67361`. Lifting the
    /// bottom-right working corner `(128, 97)` with per-axis scales lands
    /// exactly on source `(191, 144)`; lifting with the width scalar on
    /// both axes lands at y `= 97/sx ≈ 144.742` — 0.742 source px off.
    /// This test drives `SourceView::lift` (the production lift used by
    /// `sample_grid`) and FAILS against the pre-fix scalar behavior
    /// (verified by temporarily constructing `sy = sx`: the 0.05 px
    /// assertion trips at 0.742 px).
    #[test]
    fn lift_uses_per_axis_scales_when_downscale_rounding_diverges() {
        let (src_w, src_h, max_dim) = (191usize, 144usize, 128u32);
        let source_buf = vec![128u8; src_w * src_h];
        let source_view = LumaView::new(&source_buf, src_w, src_h, src_w).unwrap();
        let (_working_buf, working_w, working_h) = downscale_luma(&source_view, max_dim)
            .expect("test setup: a downscale must actually be needed here");
        assert_eq!(
            (working_w, working_h),
            (128, 97),
            "production rounding changed?"
        );

        let sx = working_w as f64 / src_w as f64;
        let sy = working_h as f64 / src_h as f64;
        assert!(
            (sx - sy).abs() > 1e-3,
            "test setup: axis ratios must actually diverge (sx={sx}, sy={sy}) — \
             pick different dims otherwise"
        );

        // Module→working transform spanning the full working frame (any
        // non-degenerate transform works; full-frame makes the analytic
        // source truth trivial: unit (1,1) → source (src_w, src_h)).
        let working = PerspectiveTransform::square_to_quad([
            [0.0, 0.0],
            [working_w as f64, 0.0],
            [working_w as f64, working_h as f64],
            [0.0, working_h as f64],
        ])
        .unwrap();

        let src = SourceView {
            view: &source_view,
            sx,
            sy,
        };
        let lifted = src.lift(&working);
        let got = lifted.map(1.0, 1.0); // bottom-right corner
        let want = [src_w as f64, src_h as f64];
        let err = ((got[0] - want[0]).powi(2) + (got[1] - want[1]).powi(2)).sqrt();
        assert!(
            err <= 0.05,
            "per-axis lift of the bottom-right corner is {err:.4} px off: got {got:?}, want {want:?}"
        );

        // Pin WHY per-axis matters: the scalar (width-ratio-on-both-axes)
        // lift is measurably wrong at this same corner — if this stops
        // holding, the fixture dims no longer exercise the divergence and
        // the test above has gone vacuous.
        let scalar_lift = working.then(&PerspectiveTransform::scaled(1.0 / sx, 1.0 / sx));
        let scalar_got = scalar_lift.map(1.0, 1.0);
        let scalar_err =
            ((scalar_got[0] - want[0]).powi(2) + (scalar_got[1] - want[1]).powi(2)).sqrt();
        assert!(
            scalar_err > 0.5,
            "expected the scalar lift to be >0.5 px off at the bottom-right corner \
             (measured 0.742 px at these dims), got {scalar_err:.4} px"
        );
    }

    // --- fit_line_tls_weighted (Plan 5 Task 3 extraction) ---

    #[test]
    fn fit_line_tls_weighted_with_equal_weights_matches_unweighted() {
        let pts = [
            [0.0, 0.1],
            [1.0, -0.1],
            [2.0, 0.15],
            [3.0, -0.05],
            [4.0, 0.0],
        ];
        let unweighted = fit_line_tls(&pts);
        let weighted = fit_line_tls_weighted(&pts, &[1.0; 5]);
        assert!((unweighted.centroid[0] - weighted.centroid[0]).abs() < 1e-12);
        assert!((unweighted.centroid[1] - weighted.centroid[1]).abs() < 1e-12);
        assert!((unweighted.dir[0] - weighted.dir[0]).abs() < 1e-12);
        assert!((unweighted.dir[1] - weighted.dir[1]).abs() < 1e-12);
    }

    #[test]
    fn fit_line_tls_weighted_favors_the_heavily_weighted_points() {
        // Two clusters of points on two different horizontal lines
        // (y=0 and y=10); a heavy weight on the y=0 cluster should pull
        // the fitted line's centroid toward it, away from the unweighted
        // midpoint (y=5).
        let pts = [[0.0, 0.0], [1.0, 0.0], [0.0, 10.0], [1.0, 10.0]];
        let weights = [100.0, 100.0, 1.0, 1.0];
        let fit = fit_line_tls_weighted(&pts, &weights);
        assert!(
            fit.centroid[1] < 1.0,
            "expected the heavily-weighted y=0 cluster to dominate the centroid, got {:?}",
            fit.centroid
        );
    }
}
