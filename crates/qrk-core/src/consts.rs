//! Pinned detection constants. Every value must state a principled
//! derivation (QR geometry or established practice) — never a value tuned
//! to make a fixture pass (see Plan 2 "No overfitting" constraint).

/// Tile edge for local min/max thresholding (AprilTag's tile-extrema
/// scheme, scaled to full working resolution). Thresholds are drawn from
/// the 3×3-dilated neighborhood, i.e. an effective 48px window: the
/// smallest decodable finder (7 modules × ≥2px = 14px ≈ one tile) always
/// contributes both ink and background extrema to every tile it overlaps,
/// while tiles stay small enough to track illumination gradients.
pub const TILE: usize = 16;

/// Minimum tile-neighborhood contrast (max−min) to consider content.
/// A usable module edge needs ink/background separation well above sensor
/// noise: typical 8-bit luma sensor read noise is a few gray levels
/// (σ≈2 in our fixture model and commonly cited for phone camera sensors)
/// ⇒ 6σ ≈ 12 gray levels; below this a tile cannot contain a decodable
/// transition. The σ≈2 figure is drawn from the fixture generator, not a
/// measurement of real camera hardware — to be validated against real
/// captures in Plan 5.
pub const CONTRAST_FLOOR: u8 = 12;

/// Scan every 2nd row. The 1:1:3:1:1 cross-section only appears where a
/// horizontal scanline crosses the finder's 3-module-tall dark core band
/// (the inner square) — not anywhere across the full 7-module height, a
/// scanline outside that band still crosses ink/space but not in that
/// proportion. At the ≥2px/module decodability floor that band is ≥6px
/// tall (6 consecutive rows), and any 6 consecutive rows contain exactly
/// 3 even-indexed and 3 odd-indexed rows, so step 2 guarantees ≥3 scan
/// rows land inside the band regardless of its vertical offset. The
/// binding constraint is downstream: `find_finders` keeps only
/// candidates with `hits >= 2` (the merge gate), so ≥3 hit chances
/// leaves one spare above that floor rather than sitting exactly on it.
pub const ROW_STEP: usize = 2;

/// Alignment-pattern concentric re-centering probe half-width, in modules
/// (zxing-cpp's `AlignmentPatternFinder` uses the same ±2.25 half-width).
/// Must comfortably exceed the largest prediction error the caller can
/// hand in (parallelogram/provisional-transform drift is typically well
/// under 2 modules even at high versions and moderate perspective) while
/// staying well short of ever reaching a *neighboring* alignment pattern's
/// own 5×5 footprint: computed (not eyeballed — see the min-gap check
/// alongside `alignment.rs`'s table test) the smallest center-to-center
/// spacing between two adjacent, non-finder-corner `alignment_coords`
/// grid nodes across every version with at least one such pair (v7..40)
/// is 16 modules (v7's own coordinate list `[6, 22, 38]`, gap 16 both
/// times — the *global* minimum, not merely v7's), so a half-width of
/// 2.25 leaves `16 - 2*2.25 = 11.5` modules of dead zone between probe
/// windows — nowhere close to colliding. (v2..6's single real alignment
/// pattern has no neighboring real pattern to collide with at all, so
/// their nominal `6 -> coord` gap, sometimes < 16, doesn't apply here.)
pub const ALIGNMENT_PROBE_HALF_MODULES: f64 = 2.25;

/// Sample-out-of-image tolerance (Plan 4's Global Constraints, transcribed
/// verbatim): a candidate whose sampling grid would read more than this
/// fraction of modules outside the source image is rejected before decode —
/// a border-clamped read would otherwise silently repeat an edge pixel's
/// value instead of surfacing that the candidate's geometry runs off-frame.
/// 2% ≈ one clipped quiet-zone-adjacent row on a v1 (21×21 = 441 modules;
/// one full row is `21/441 ≈ 4.8%`, so 2% catches a *partial* row/column
/// clipping before it grows into a whole one). `sample.rs`'s `sample_grid`
/// computes and returns `oob_fraction`; `decode.rs` applies this threshold.
pub(crate) const MAX_OOB_FRACTION: f64 = 0.02;

/// Per-frame round budget for `decode.rs`'s arbitration loop (Plan 5 Task 6,
/// replacing the old `MAX_DECODE_ATTEMPTS` attempt-counted cap — see the
/// plan's Global Constraints and Plan 4's recorded follow-up (c) for the
/// full history). Same provenance chain, extended by one more factor:
/// `4 codes/frame worst case * 3 triplet permutations/code * 2 headroom * 3
/// geometry rounds/attempt = 72`.
///
/// The first three factors are Plan 4's original `MAX_DECODE_ATTEMPTS`
/// derivation, unchanged: a "triplet permutation" is the observed failure
/// mode from multi-code fixtures where more than one finder triple can
/// plausibly group around the same handful of real finders before
/// proximity dedup and consumption prune them.
///
/// The new `* 3` factor is why the cap moved from attempts to rounds:
/// `decode_candidates` counts one attempt as one corner-role rotation of
/// one triplet (`attempt_candidate`'s single call), but since Plan 4 Task
/// 5b + Plan 4B Fix B, a single attempt internally runs up to 3 "geometry
/// rounds" — one `sample_grid`+`decode_bits` cycle each, tagged
/// `parallelogram`/`anchor_line`/`outer_hull` in
/// [`crate::decode::DecodeAttemptTrace::rounds`] — when the first
/// (parallelogram) round fails and no alignment-pattern anchor was found
/// (`sample::needs_refined_br`). A round counts as ONE unit of budget
/// regardless of whether Fix B's reference-threshold retry also ran inside
/// it: that retry is one extra `decode_bits` (rqrr) call on the SAME
/// sampled grays, no resampling — real, but strictly cheaper than a whole
/// additional round, and "a round is a sampling cycle" keeps the budget's
/// unit simple and honest (the retry's extra rqrr-call cost is absorbed
/// into the per-round budget rather than tracked as its own thing). So
/// budgeting attempts at their old worst-case cost (3 rounds each) instead
/// of a flat 1 reproduces the exact old worst-case total work
/// (`24 attempts * 3 rounds = 72`), while an attempt that only needs 1
/// round (the common case — most decodes succeed on the first round, and a
/// non-decoding v7+ candidate with a located alignment pattern never enters
/// the Task 5b retry loop at all) now correctly costs only 1 unit instead
/// of being charged for headroom it never uses — letting MORE cheap
/// attempts run per frame before the budget binds, exactly the fixtures'
/// everyday case (see `tests/decode_trace_gate.rs`'s
/// `round_budget_never_binds_on_the_golden_fixture_suite`). The budget is
/// checked once per attempt, before it runs (not
/// mid-attempt), so the hard worst case is a small, bounded overrun: up to
/// 2 extra rounds beyond 72 if the very last attempt let through costs the
/// maximum 3.
pub(crate) const MAX_DECODE_ROUNDS: usize = 72;

/// Number of boundary probes per edge for `sample.rs`'s
/// `refine_fourth_corner` (Plan 4 Task 5b). Line-fit noise scales as
/// `~1/sqrt(N)` in the number of fitted points, so 8 probes cut a single
/// probe's localization noise by ~2.8x, while the probe-able span
/// `[7, dim-7]` (the amendment's range — the flanking finder/separator
/// structures are excluded) still offers one probe per module even at v1's
/// minimum `dim = 21` (7-module span).
pub(crate) const BR_PROBE_COUNT: usize = 8;

/// Half-width, in modules, of the perpendicular search window each
/// `refine_fourth_corner` probe walks around its predicted edge-boundary
/// position.
///
/// Must cover the prediction's divergence from the true boundary. Measured
/// (Task 5b investigation) against the ground-truth `corners_px` homography
/// at the 8 probe positions per edge on the gate-failing 45-degree-tilt
/// fixtures: when the prediction comes from the affine parallelogram
/// provisional transform alone, the divergence reaches 1.81 modules
/// (`tilt45_00`, right edge) and 2.06 modules (`tilt45_07`, bottom edge) at
/// the probes nearest the BR corner — *exceeding* 1.5. That measurement is
/// exactly why `refine_fourth_corner` probes each edge in two passes (see
/// `sample.rs`'s `probe_edge_line`): a provisional-guided first pass whose
/// near-finder probes — where the same measurement shows the provisional's
/// divergence is smallest, 0.46-0.88 modules across every failing
/// fixture — pin the edge's straight image-space line, then a second pass
/// re-probing every position with its window centered on that line (the
/// same evidence-over-provisional prediction principle `alignment.rs`'s
/// parallelogram rule established). 1.5 covers the measured near-probe
/// divergence with ~2x margin while staying well below half the 7-module
/// minimum probe span (a window can never reach around to the code's far
/// side).
pub(crate) const BR_PROBE_WINDOW_HALF_MODULES: f64 = 1.5;

/// Minimum accepted boundary points per edge before `refine_fourth_corner`
/// trusts a line fit: a line has 2 parameters, so 5 points leave 3 degrees
/// of freedom of redundancy — enough for least squares to average out
/// single-probe localization noise. The count includes the
/// [`BR_ANCHOR_POSITIONS`] finder-border probes (the constant's provenance
/// is fit degrees-of-freedom margin, which counts every fitted point), so
/// with 2 anchors + [`BR_PROBE_COUNT`] = 8 data-span probes the gate
/// tolerates up to 5 rejected data probes (a data probe rejects when its
/// window sees no usable ink-to-background transition, e.g. where the
/// edge-adjacent module column/row is locally light, as ~half of a QR data
/// region's boundary modules are — measured on the golden suite, several
/// v1 fixtures' bottom rows have only 4 dark columns among the 8 probed).
pub(crate) const BR_MIN_EDGE_POINTS: usize = 5;

/// Anchor-probe positions (module-space coordinate along the edge) over
/// the edge's flanking finder pattern, used by `refine_fourth_corner` in
/// addition to the [`BR_PROBE_COUNT`] data-span probes.
///
/// The bottom edge's modules `(x, dim-1)` for `x in 0..=6` are the BL
/// finder's own outer border row — dark by construction (ISO 18004 §6.3.3,
/// the 7x7 finder's 1-module dark outer ring), lying exactly ON the module
/// region's bottom edge line; likewise `(dim-1, y)`, `y in 0..=6`, the TR
/// finder's border column on the right edge. Probing there yields
/// *guaranteed* boundary points with a clean dark(border)/light(inner
/// ring) inward profile, independent of payload — unlike the data-span
/// probes, whose acceptance depends on which boundary data modules happen
/// to be dark. This is where zxing-cpp's own edge tracing starts (from the
/// finder patterns outward) — the amendment's cited practice. Two anchors
/// also stretch the fit's baseline toward the near corner, cutting the
/// line's extrapolation error at the far (BR) corner roughly in half
/// versus fitting the `[7, dim-7]` span alone.
///
/// `2.0` and `5.0`: symmetric within the border's clean central span, at
/// least 1.5 modules (one probe window half-width) away from the finder's
/// two corners at `0` and `7`, so a probe window never straddles the
/// corner rounding the amendment's data-span bounds exclude.
pub(crate) const BR_ANCHOR_POSITIONS: [f64; 2] = [2.0, 5.0];

/// Perpendicular tolerance, in modules, for `sample.rs`'s anchor-line
/// consistency filter: a data-span boundary point is kept only if it lies
/// within this distance of the line through the two
/// [`BR_ANCHOR_POSITIONS`] finder-border points.
///
/// Bounds, from the two failure modes the filter separates (both observed
/// on the golden suite):
/// - It must EXCEED the anchor line's own worst-case extrapolation error
///   across the data span: each anchor localizes to ~1/16 module (walk
///   step is 1/8 module, transition taken at the step midpoint), the two
///   anchors sit 3 modules apart, so the line's tilt error is at most
///   `(2 * 1/16) / 3 ~= 0.042 rad`; over the ~10.5 modules from the anchor
///   midpoint to the far end of a v1 data span that is ~0.44 modules of
///   deviation. (Systematic transition shifts — blur, thresholding — move
///   both anchors together and cancel out of the tilt.)
/// - It must stay WELL BELOW 1 module, the two structures a wrong point
///   snaps to: a light-boundary-module probe's inward offset is exactly
///   one module (the next module row's own edge), and the plate-edge /
///   scene-background contour captured on the 45-degree-tilt fixtures
///   (where the projected quiet zone compresses toward the probe window's
///   reach) sits >= 1.2 modules outside.
///
/// 0.75 is the midpoint of that `[0.44, 1.0]` gap. Note the v1 lever arm
/// is the practically binding one: `refine_fourth_corner` only runs when
/// no alignment pattern was found anywhere, which in practice means v1
/// (higher versions' probes virtually always find at least one AP); for a
/// hypothetical no-AP high version the longer span makes the anchor line
/// over-reject, refinement returns `None`, and the caller keeps the
/// pre-Task-5b parallelogram fallback — a no-regression outcome.
pub(crate) const BR_ANCHOR_FILTER_TOL_MODULES: f64 = 0.75;

/// Unsharp-mask strength for `bitmatrix.rs`'s `decode_sharpen` (Plan 4B Fix
/// B — real-video-capture robustness): `v' = v + K*(v - mean(available
/// N/S/E/W neighbors))`.
///
/// Provenance: AprilTag's `quad_decode.c` `decode_sharpening` default (the
/// cited prior-art unsharp-mask pass this round transcribes), chosen there
/// to counter exactly the inter-module blur crosstalk this fix targets
/// (area-averaging sampling over a blurred sensor image pulls a module's
/// read toward its neighbors' values — sharpening pushes it back). Kept at
/// the cited default rather than tuned against any fixture: the
/// investigation measured this default cutting the frame-167 probe's
/// bit-error count from 8 to 6-7 (combined with the reference threshold
/// below), and per this task's gate-failure protocol no other K value may
/// be tried without reporting it.
pub(crate) const SHARPEN_K: f32 = 0.25;

// --- Plan 5 Task 3: subpixel corner refinement (`refine.rs`) ---
// Every constant below is transcribed verbatim from the plan's Global
// Constraints ("Pinned refinement constants") — no value here may be
// tuned against a fixture; see that section's gate-failure protocol.

/// Sub-module-fraction probe positions along a dark border module's own
/// length (0.35, 0.70 — the original GPU scanner's own two-probes-per-
/// dark-module scheme). Symmetric about the module's center and
/// comfortably inside its own boundaries, so a probe's
/// [`REFINE_PROFILE_SAMPLES`]-point perpendicular profile samples the
/// module-region's outer edge crossing, not a neighboring module's own.
pub(crate) const REFINE_PROBE_MODULE_FRACTIONS: [f64; 2] = [0.35, 0.70];

/// Margin excluded from each end of an outer module-region edge before
/// `refine.rs` probes it, in modules — corner rounding (anti-aliasing and
/// blur soften the true corner into a curve over roughly a module) would
/// otherwise bias a probe landing there off the straight edge line the fit
/// assumes. Skipping 1.5 modules at each end keeps the "middle ~80%" of a
/// v1 edge (21 modules: `21 - 2*1.5 = 18`, `18/21 ≈ 86%`); the kept
/// fraction grows toward the whole edge as `dim` increases, since the
/// corner-rounding zone is a fixed few modules, not a fraction of the
/// edge.
pub(crate) const REFINE_EDGE_MARGIN_MODULES: f64 = 1.5;

/// Hard cap on probe points per edge: bounds refinement's worst-case
/// per-frame cost independent of the code's dimension — v40's 177-module
/// edge, probed at up to 2 points per dark border module, could otherwise
/// contribute on the order of 170 candidate points to a single edge fit.
pub(crate) const REFINE_MAX_POINTS_PER_EDGE: usize = 64;

/// Bilinear samples per Devernay sub-pixel edge-localization profile — an
/// odd count centered on the coarse edge position (the module-region
/// boundary the caller's, possibly-imprecise, corners predict), leaving 5
/// interior samples with a same-spacing neighbor on both sides for the
/// central-difference gradient, 3 of which (around whichever peaks) feed
/// the 3-point quadratic (Devernay) sub-sample interpolation.
pub(crate) const REFINE_PROFILE_SAMPLES: usize = 7;

/// Spacing between consecutive [`REFINE_PROFILE_SAMPLES`] profile samples,
/// in source-image modules: half a module, so the full 7-sample profile
/// spans +/-1.5 modules around the coarse edge position — wide enough to
/// bracket the true boundary despite ordinary coarse-corner imprecision,
/// while staying tight enough that the profile doesn't reach into a
/// neighboring module's own transition.
pub(crate) const REFINE_PROFILE_STEP_MODULES: f64 = 0.5;

/// Multiplier on the median residual for `refine.rs`'s one-pass outlier
/// refit ("drop residuals > max(0.15 source-module, 2x median residual)"):
/// keeps the threshold adaptive to each edge's own fit quality (a noisy
/// edge's median residual dominates) while [`REFINE_OUTLIER_FLOOR_MODULES`]
/// guards a near-perfect edge (median ~ 0) against rejecting points on
/// floating-point noise alone.
pub(crate) const REFINE_OUTLIER_MEDIAN_MULTIPLIER: f64 = 2.0;

/// Flat floor on the outlier-refit threshold, in source modules — see
/// [`REFINE_OUTLIER_MEDIAN_MULTIPLIER`]'s doc for why a floor is needed
/// alongside the adaptive multiplier.
pub(crate) const REFINE_OUTLIER_FLOOR_MODULES: f64 = 0.15;

/// Minimum surviving points an edge's outlier refit must keep before
/// `refine.rs` trusts its line — a line has 2 degrees of freedom, so 6
/// points leave 4 of redundancy, comfortably more than
/// [`BR_MIN_EDGE_POINTS`]'s own 5-point bar for the coarser Task 5b
/// BR-corner estimator this stage supersedes in precision (not in role —
/// that estimator still runs first, at decode time, making the corners
/// this stage refines possible in the first place).
pub(crate) const REFINE_MIN_EDGE_POINTS: usize = 6;

// --- Plan 5 Task 4 (carried review nit from Task 3): pin the two
// structural iteration counts `refine.rs` previously expressed only as
// literal repeated call sites (two sequential `refine_round` calls; three
// sequential `localize_edge_point_pass` calls) into named, documented
// constants — a mechanical move, no behavior change (still exactly 2
// rounds, still exactly 3 passes).

/// Number of `refine_round` invocations `refine_corners` runs, each
/// re-anchored on the previous round's own output corners (see
/// `refine_corners`'s "two rounds" doc for the full derivation): round 1
/// leaves the anchor-derived probe geometry accurate to ~0.2%, at which
/// point the coherent per-probe bias that geometry error causes (measured
/// ~0.1px at a ±0.3-module round-1 input error) falls far below the gate's
/// budget — a further round was not shown to move the measured accuracy
/// (Task 3 review), so this is fixed at 2 for a small, predictable
/// per-code cost rather than iterating to convergence.
pub(crate) const REFINE_ROUNDS: usize = 2;

/// Number of `localize_edge_point_pass` iterations `localize_edge_point`
/// runs before Aitken Δ² extrapolation (see that function's doc for the
/// phase-gain-error derivation the extrapolation corrects). This is not a
/// free-standing tunable: the closed-form Aitken step `x* = x_2 -
/// (x_2-x_1)^2 / (x_2-2x_1+x_0)` is derived from exactly 3 consecutive
/// iterates of a locally-linear fixed-point map, so changing this value
/// requires re-deriving the extrapolation itself, not just editing a loop
/// bound — `localize_edge_point` asserts this invariant with a
/// `debug_assert!`.
pub(crate) const REFINE_LOCALIZE_PASSES: usize = 3;
