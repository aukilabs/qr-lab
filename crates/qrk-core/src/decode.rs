//! Decode orchestration + arbitration: the per-candidate pipeline that turns
//! a [`TripletCandidate`] into a [`DecodedCode`], and the frame-level
//! bookkeeping (proximity dedup, attempt cap, finder consumption) that keeps
//! multi-code scenes from producing spurious duplicate decodes.
//!
//! # Per-attempt pipeline (Plan 4 Task 5, Global Constraints, transcribed)
//!
//! `dimension` starts at the triplet's own snapped estimate, then is
//! refined twice, each refinement rebuilding the whole-grid
//! `PerspectiveTransform` when it actually changes the value:
//!
//! 1. **Timing cross-check** (only when the *current* estimate is `<= 41`
//!    modules — [`crate::version::count_timing_transitions`]'s formula is
//!    only meaningful there; v>=7's own version-info bits are authoritative
//!    instead, see step 2): adopt the timing-derived dimension when it's
//!    within +/-2 modules of the current estimate, else keep the estimate
//!    and merely record the timing reading in the attempt trace.
//! 2. **Version-info bits** (only once the dimension implies `version >=
//!    7`): [`crate::version::read_version_bits`] BCH-decodes the redundant
//!    version-info blocks; per the plan's recorded `ver_12_v40` finding this
//!    is authoritative over the geometric estimate when it decodes, so a
//!    disagreement here always wins.
//!
//! Between the two, a defensive bounds check rejects a dimension whose
//! implied version would fall outside `1..=40` — [`crate::alignment::alignment_coords`]
//! panics on an out-of-range version, and the only realistic way to reach
//! one here is the timing cross-check nudging an already-small (v1) estimate
//! down by its full +/-2 tolerance (e.g. triplet estimate 21, timing reading
//! 19 or 20 both imply version 0). This is not expected to trigger on any
//! real fixture (a clean v1 render's timing reading exactly reproduces 21),
//! but a real camera frame's noise makes it reachable in principle, and
//! silently corrupting `dimension` into something `alignment_coords` panics
//! on would take down the whole scan. Rejected here as a normal (traced)
//! failed attempt instead.
//!
//! After the dimension settles: alignment patterns are located, the grid is
//! sampled, and [`crate::bitmatrix::decode_bits`] runs (with its own
//! internal mirrored retry). A successful decode consumes its three finder
//! indices so no later, lower-priority triplet sharing any of them can also
//! decode in the same frame — the arbitration property Gate 3 checks.

use crate::alignment::{locate_alignment_patterns, AnchorSlot};
use crate::bitmatrix::decode_bits;
use crate::consts::{MAX_DECODE_ATTEMPTS, MAX_OOB_FRACTION};
use crate::finder::FinderCandidate;
use crate::sample::{provisional_transform, sample_grid};
use crate::scanner::StageClock;
use crate::tiles::TileGrid;
use crate::triplet::TripletCandidate;
use crate::version::{count_timing_transitions, read_version_bits};
use crate::LumaView;

/// A fully decoded QR payload, arbitrated against every other candidate in
/// the same frame (its three finders are guaranteed not shared with any
/// other [`DecodedCode`] returned alongside it — see [`decode_candidates`]).
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecodedCode {
    /// Decoded content, as UTF-8 text.
    pub payload: String,
    /// Same content as raw bytes.
    pub payload_bytes: Vec<u8>,
    /// QR version (1..=40), from [`crate::bitmatrix::DecodedPayload`] —
    /// RS-validated, so trusted over the geometric estimate even when the
    /// two disagree (see [`decode_candidates`]'s cross-check note).
    pub version: u32,
    /// Error-correction level (`'L'`/`'M'`/`'Q'`/`'H'`, or `'?'` if rqrr
    /// could not determine it).
    pub ecc: char,
    /// `true` when the sampled grid's true reading orientation was
    /// x/y-swapped (a mirrored source image).
    pub mirrored: bool,
    /// The module dimension of the grid that was actually sampled and
    /// decoded (post every timing-check/version-bits override).
    pub dimension: u32,
    /// TL, TR, BR, BL image-pixel corners of the module region, mapped
    /// through the FINAL whole-grid transform (the one `dimension` and the
    /// alignment search settled on) at unit-square points `(0,0)`, `(1,0)`,
    /// `(1,1)`, `(0,1)` respectively.
    pub corners: [[f64; 2]; 4],
    /// `true` when the code's polarity is inverted (light-on-dark).
    pub inverted: bool,
    /// The three [`FinderCandidate`] indices this decode consumed —
    /// identical to the winning [`TripletCandidate::finder_indices`].
    pub finder_indices: [usize; 3],
}

/// One decode attempt's trace, whether it succeeded or failed — every
/// triplet actually attempted (i.e. not skipped because its finders were
/// already consumed, and not dropped by proximity dedup) gets exactly one
/// of these.
///
/// Every attempt is recorded (`decode_candidates` returns a `Vec` of these)
/// but nothing outside this module's own `#[cfg(test)]` tests reads a
/// field back out yet — under the default (no `serde`) feature set that
/// makes every field look "never read" to the dead-code lint. `Trace`
/// gains an `attempts: Vec<DecodeAttemptTrace>` field in Plan 4 Task 6,
/// which is this struct's real consumer; until then this is exempted here
/// rather than piecemeal, matching `version.rs`/`alignment.rs`/`sample.rs`'s
/// own precedent for a piece landing ahead of its consumer.
#[allow(dead_code)]
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecodeAttemptTrace {
    /// Index into the `triplets` slice [`decode_candidates`] was called
    /// with (i.e. `Detections.triplets`), so a trace consumer can correlate
    /// an attempt back to the candidate that produced it.
    pub triplet_index: usize,
    /// The triplet's own snapped dimension estimate, before any
    /// cross-check.
    pub dimension_est: u32,
    /// The dimension actually used to build the sampled grid (post timing
    /// cross-check and/or version-bits override, whichever ran).
    pub dimension_final: u32,
    /// The timing cross-check's raw reading (`transitions + 13`), if the
    /// walk completed — `None` if it wasn't run (estimate > 41) or the walk
    /// left the image. Recorded even when the reading disagreed enough
    /// with `dimension_est` to be discarded (see the module doc).
    pub timing_check: Option<u32>,
    /// The version decoded from the version-info bits, if it was run
    /// (implied version >= 7) and BCH-decoded successfully.
    pub version_bits: Option<u32>,
    /// Count of alignment-pattern lattice slots the concentric probe
    /// actually found (excludes the 3 finder-corner slots).
    pub alignment_found: u32,
    /// Total non-finder-corner alignment-pattern lattice slots for this
    /// dimension's version (`coords.len()^2 - 3`; `0` for v1).
    pub alignment_total: u32,
    /// Fraction of sampled modules that fell outside the source image
    /// (`0.0` if sampling wasn't reached).
    pub oob_fraction: f64,
    /// `"decoded"` on success (see [`decode_candidates`]'s cross-check note
    /// for the one case where it carries an appended discrepancy message
    /// instead of the bare string), else a short failure reason.
    pub outcome: String,
}

/// Per-attempt wall-clock totals accumulated across every attempted
/// candidate in one [`decode_candidates`] call, in nanoseconds.
///
/// # Deviation from the brief
/// The brief's `decode_candidates` interface line lists a 2-tuple return
/// (`(Vec<DecodedCode>, Vec<DecodeAttemptTrace>)`). `scanner.rs`'s own
/// module doc establishes the project's stage-timing convention: measure
/// with [`StageClock`] around each stage's call, from the *caller*, so the
/// stage functions themselves stay measurement-free. That convention
/// assumes stages run one after another as whole phases — true for
/// tiles/finders/triplets, but not for decode: version-check, alignment,
/// and sample+decode each run *per attempt*, interleaved across up to
/// [`MAX_DECODE_ATTEMPTS`] candidates, so there is no single contiguous
/// span the caller could wrap per stage. Accumulating the three totals in
/// here instead (still built from [`StageClock`], just called at this
/// level rather than `scanner.rs`'s) is the only way to produce the plan's
/// three separate `version_ns`/`alignment_ns`/`sample_decode_ns` numbers;
/// returned as a third tuple element rather than the brief's literal two,
/// consumed directly by `scanner.rs`'s `StageTimings`.
pub(crate) struct DecodeTimings {
    /// Timing cross-check + version-info-bits reading, summed across every
    /// attempt.
    pub version_ns: u64,
    /// Alignment-pattern location, summed across every attempt.
    pub alignment_ns: u64,
    /// Grid sampling + `decode_bits`, summed across every attempt (folded
    /// together per the plan's field list, which names one
    /// `sample_decode_ns`, not two).
    pub sample_decode_ns: u64,
}

/// Count of finder indices `a` and `b` have in common (each triplet's three
/// indices are always distinct, so this is a plain set-intersection size,
/// `0..=3`).
fn shared_finder_count(a: [usize; 3], b: [usize; 3]) -> usize {
    a.iter().filter(|x| b.contains(x)).count()
}

/// Proximity-dedup `triplets` and order the survivors best-first: returns
/// the original indices of the kept candidates, sorted ascending by
/// `snap_error`.
///
/// Two triplets sharing >= 2 finder indices are treated as the same
/// physical code seen through slightly different candidate groupings — per
/// the plan's Global Constraints, keep the one with the lower `snap_error`.
/// Sorting a local index list ascending by `snap_error` first (rather than
/// trusting the caller to have already done so — [`crate::triplet::group_triplets`]
/// does, but this function doesn't rely on that being true forever) and
/// then greedily keeping every candidate that doesn't share >= 2 indices
/// with an already-kept (hence lower-or-equal `snap_error`) survivor
/// implements exactly that rule: a later duplicate is always skipped in
/// favor of the earlier (lower-`snap_error`) one, regardless of which
/// pairing in a chain of duplicates is compared first. The returned order
/// is also exactly the attempt order [`decode_candidates`] needs ("attempt
/// in snap_error order" per the brief).
fn dedup_triplet_indices(triplets: &[TripletCandidate]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..triplets.len()).collect();
    order.sort_by(|&a, &b| {
        triplets[a]
            .snap_error
            .partial_cmp(&triplets[b].snap_error)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut kept: Vec<usize> = Vec::new();
    for idx in order {
        let t = &triplets[idx];
        let is_duplicate = kept
            .iter()
            .any(|&k| shared_finder_count(triplets[k].finder_indices, t.finder_indices) >= 2);
        if !is_duplicate {
            kept.push(idx);
        }
    }
    kept
}

/// One attempt's outcome: either a successful decode (consuming its three
/// finders) or a trace-only failure.
struct AttemptResult {
    trace: DecodeAttemptTrace,
    code: Option<DecodedCode>,
}

/// Run the full per-candidate pipeline for one triplet: dimension
/// cross-checks, alignment, sampling, and `decode_bits`. See the module
/// doc for the stage-by-stage contract. Accumulates elapsed time for each
/// of the three stage buckets into `timings`.
fn attempt_candidate(
    view: &LumaView,
    grid: &TileGrid,
    triplet_index: usize,
    t: &TripletCandidate,
    timings: &mut DecodeTimings,
) -> AttemptResult {
    let dimension_est = t.dimension;

    let version_clock = StageClock::start();
    let mut dimension = t.dimension;
    let mut transform = provisional_transform(t, dimension);

    let mut timing_check = None;
    if dimension <= 41 {
        if let Some(transitions) = count_timing_transitions(view, grid, t, &transform) {
            let timing_dim = transitions + 13;
            timing_check = Some(timing_dim);
            if (timing_dim as i64 - dimension as i64).abs() <= 2 && timing_dim != dimension {
                dimension = timing_dim;
                transform = provisional_transform(t, dimension);
            }
        }
    }

    // Defensive bounds check: see the module doc's "Between the two" note.
    // `version_from_dim` may be 0 here only via the timing check nudging an
    // already-minimal (v1) estimate below its valid range.
    let version_from_dim = dimension.saturating_sub(17) / 4;
    if !(1..=40).contains(&version_from_dim) {
        timings.version_ns += version_clock.elapsed_ns();
        return AttemptResult {
            trace: DecodeAttemptTrace {
                triplet_index,
                dimension_est,
                dimension_final: dimension,
                timing_check,
                version_bits: None,
                alignment_found: 0,
                alignment_total: 0,
                oob_fraction: 0.0,
                outcome: "invalid_dimension".to_string(),
            },
            code: None,
        };
    }

    let mut version_bits = None;
    if version_from_dim >= 7 {
        if let Some(v) = read_version_bits(view, grid, &transform, dimension, t.inverted) {
            version_bits = Some(v);
            let new_dim = 17 + 4 * v;
            if new_dim != dimension {
                dimension = new_dim;
                transform = provisional_transform(t, dimension);
            }
        }
    }
    timings.version_ns += version_clock.elapsed_ns();

    // Guaranteed valid at this point: either unchanged from the already
    // validated `version_from_dim`, or overridden from the BCH table,
    // which only ever yields versions 7..=40.
    let version = dimension.saturating_sub(17) / 4;

    let alignment_clock = StageClock::start();
    let alignment = locate_alignment_patterns(view, grid, &transform, version, t.inverted);
    timings.alignment_ns += alignment_clock.elapsed_ns();

    let n = alignment.coords.len();
    let alignment_total = (n * n).saturating_sub(3) as u32;
    let alignment_found = alignment
        .found
        .iter()
        .filter(|s| matches!(s, AnchorSlot::Found(_)))
        .count() as u32;

    let corners = [
        transform.map(0.0, 0.0),
        transform.map(1.0, 0.0),
        transform.map(1.0, 1.0),
        transform.map(0.0, 1.0),
    ];

    let sample_decode_clock = StageClock::start();
    let sampled = match sample_grid(view, grid, t, dimension, &alignment) {
        Some(s) => s,
        None => {
            timings.sample_decode_ns += sample_decode_clock.elapsed_ns();
            return AttemptResult {
                trace: DecodeAttemptTrace {
                    triplet_index,
                    dimension_est,
                    dimension_final: dimension,
                    timing_check,
                    version_bits,
                    alignment_found,
                    alignment_total,
                    oob_fraction: 0.0,
                    outcome: "sample_transform_degenerate".to_string(),
                },
                code: None,
            };
        }
    };

    if sampled.oob_fraction > MAX_OOB_FRACTION {
        timings.sample_decode_ns += sample_decode_clock.elapsed_ns();
        return AttemptResult {
            trace: DecodeAttemptTrace {
                triplet_index,
                dimension_est,
                dimension_final: dimension,
                timing_check,
                version_bits,
                alignment_found,
                alignment_total,
                oob_fraction: sampled.oob_fraction,
                outcome: format!(
                    "oob_fraction {:.4} exceeds {:.4}",
                    sampled.oob_fraction, MAX_OOB_FRACTION
                ),
            },
            code: None,
        };
    }

    let decode_result = decode_bits(&sampled.bits);
    timings.sample_decode_ns += sample_decode_clock.elapsed_ns();

    match decode_result {
        Ok(payload) => {
            // Cross-check consistency (Global Constraints, transcribed):
            // decode_bits' own version comes from the bit matrix it was
            // handed, which is exactly `dimension`-sized, so in practice
            // this can never actually disagree — QR dimension<->version is
            // a bijection (`dimension = 17 + 4*version`), and rqrr derives
            // its reported version from the grid size it was given, not by
            // re-reading version-info bits at decode time. Checked anyway,
            // defensively: if it ever did disagree, decode_bits' RS-
            // validated fields are what DecodedCode carries, with the
            // discrepancy recorded in the trace rather than silently
            // dropped.
            let outcome = if payload.version == version {
                "decoded".to_string()
            } else {
                format!(
                    "decoded (dimension mismatch: sampled grid implied v{version}, \
                     decode_bits returned v{})",
                    payload.version
                )
            };
            AttemptResult {
                trace: DecodeAttemptTrace {
                    triplet_index,
                    dimension_est,
                    dimension_final: dimension,
                    timing_check,
                    version_bits,
                    alignment_found,
                    alignment_total,
                    oob_fraction: sampled.oob_fraction,
                    outcome,
                },
                code: Some(DecodedCode {
                    payload: payload.payload,
                    payload_bytes: payload.payload_bytes,
                    version: payload.version,
                    ecc: payload.ecc,
                    mirrored: payload.mirrored,
                    dimension,
                    corners,
                    inverted: t.inverted,
                    finder_indices: t.finder_indices,
                }),
            }
        }
        Err(e) => AttemptResult {
            trace: DecodeAttemptTrace {
                triplet_index,
                dimension_est,
                dimension_final: dimension,
                timing_check,
                version_bits,
                alignment_found,
                alignment_total,
                oob_fraction: sampled.oob_fraction,
                outcome: format!("{e:?}"),
            },
            code: None,
        },
    }
}

/// Decode every plausible QR code in one frame: proximity-dedup `triplets`,
/// attempt each survivor best-first (ascending `snap_error`), and arbitrate
/// so no two returned [`DecodedCode`]s share a finder. `finders` is used
/// only to size the finder-consumption bookkeeping (`finders.len()`) — all
/// of a triplet's own geometry already lives on the [`TripletCandidate`]
/// itself.
///
/// Returns the decoded codes, one [`DecodeAttemptTrace`] per triplet
/// actually attempted (proximity-deduped-away triplets and triplets skipped
/// because their finders were already consumed produce no trace entry —
/// they were never really "attempted"), and the accumulated per-stage
/// timing totals (see [`DecodeTimings`]'s doc for why this is a 3-tuple
/// rather than the brief's literal 2-tuple).
pub(crate) fn decode_candidates(
    view: &LumaView,
    grid: &TileGrid,
    finders: &[FinderCandidate],
    triplets: &[TripletCandidate],
) -> (Vec<DecodedCode>, Vec<DecodeAttemptTrace>, DecodeTimings) {
    let mut timings = DecodeTimings { version_ns: 0, alignment_ns: 0, sample_decode_ns: 0 };
    let mut consumed = vec![false; finders.len()];
    let mut codes = Vec::new();
    let mut attempts = Vec::new();
    let mut attempts_run = 0usize;

    for idx in dedup_triplet_indices(triplets) {
        if attempts_run >= MAX_DECODE_ATTEMPTS {
            break;
        }
        let t = &triplets[idx];
        if t.finder_indices.iter().any(|&i| consumed[i]) {
            continue;
        }
        attempts_run += 1;

        let result = attempt_candidate(view, grid, idx, t, &mut timings);
        if let Some(code) = result.code {
            for &i in &t.finder_indices {
                consumed[i] = true;
            }
            codes.push(code);
        }
        attempts.push(result.trace);
    }

    (codes, attempts, timings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triplet(snap_error: f64, finder_indices: [usize; 3]) -> TripletCandidate {
        TripletCandidate {
            tl: [0.0, 0.0],
            tr: [1.0, 0.0],
            bl: [0.0, 1.0],
            module: 1.0,
            dimension: 21,
            snap_error,
            inverted: false,
            finder_indices,
        }
    }

    // --- Proximity dedup ---

    #[test]
    fn dedup_keeps_lower_snap_error_when_two_finders_shared() {
        let triplets = [
            triplet(0.1, [0, 1, 2]), // kept: nothing before it
            triplet(0.2, [0, 1, 3]), // shares {0,1} with [0]: duplicate, dropped
            triplet(0.3, [4, 5, 6]), // disjoint: kept
        ];
        assert_eq!(dedup_triplet_indices(&triplets), vec![0, 2]);
    }

    #[test]
    fn dedup_keeps_both_when_only_one_finder_shared() {
        let triplets = [
            triplet(0.1, [0, 1, 2]),
            triplet(0.2, [2, 3, 4]), // shares only {2}: not a duplicate
        ];
        assert_eq!(dedup_triplet_indices(&triplets), vec![0, 1]);
    }

    #[test]
    fn dedup_drops_exact_duplicate_regardless_of_order() {
        let triplets = [
            triplet(0.05, [0, 1, 2]),
            triplet(0.10, [1, 2, 0]), // same set, different array order
        ];
        assert_eq!(dedup_triplet_indices(&triplets), vec![0]);
    }

    #[test]
    fn dedup_chain_all_collapse_to_the_first() {
        // A triplet dropped as a duplicate must still count as a
        // comparison target for later ones sharing its indices with the
        // *original* kept survivor — here every later triplet shares >= 2
        // indices directly with index 0, so all collapse to it.
        let triplets = [
            triplet(0.1, [0, 1, 2]),
            triplet(0.2, [0, 1, 9]),
            triplet(0.3, [0, 2, 9]),
            triplet(0.4, [1, 2, 9]),
        ];
        assert_eq!(dedup_triplet_indices(&triplets), vec![0]);
    }

    #[test]
    fn shared_finder_count_counts_set_intersection() {
        assert_eq!(shared_finder_count([0, 1, 2], [0, 1, 2]), 3);
        assert_eq!(shared_finder_count([0, 1, 2], [0, 1, 9]), 2);
        assert_eq!(shared_finder_count([0, 1, 2], [0, 9, 9]), 1);
        assert_eq!(shared_finder_count([0, 1, 2], [7, 8, 9]), 0);
    }

    // --- Full pipeline smoke tests (synthetic renders) ---

    use crate::testpaint::render_module_grid_transformed;
    use crate::PerspectiveTransform;

    fn axis_aligned_quad(dim: usize, scale: f64, quiet: f64) -> ([[f64; 2]; 4], usize) {
        let img_side = ((dim as f64 + 2.0 * quiet) * scale).round() as usize;
        let x0 = quiet * scale;
        let x1 = x0 + dim as f64 * scale;
        ([[x0, x0], [x1, x0], [x1, x1], [x0, x1]], img_side)
    }

    fn triplet_from_transform(transform: &PerspectiveTransform, dim: usize, dimension_est: u32) -> TripletCandidate {
        let dimf = dim as f64;
        TripletCandidate {
            tl: transform.map(3.5 / dimf, 3.5 / dimf),
            tr: transform.map((dimf - 3.5) / dimf, 3.5 / dimf),
            bl: transform.map(3.5 / dimf, (dimf - 3.5) / dimf),
            module: 4.0,
            dimension: dimension_est,
            snap_error: 0.0,
            inverted: false,
            finder_indices: [0, 1, 2],
        }
    }

    fn dummy_finders(n: usize) -> Vec<FinderCandidate> {
        (0..n)
            .map(|_| FinderCandidate { x: 0.0, y: 0.0, module: 4.0, inverted: false, hits: 3 })
            .collect()
    }

    #[test]
    fn decodes_a_synthetic_v1_code_with_exact_dimension_estimate() {
        let version = 1i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"HELLO", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        assert_eq!(code.width(), dim);
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let t = triplet_from_transform(&transform, dim, dim as u32);
        let (codes, attempts, _timings) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t));
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].outcome, "decoded");
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].payload, "HELLO");
        assert_eq!(codes[0].version, 1);
        assert!(!codes[0].mirrored);
        assert_eq!(codes[0].finder_indices, [0, 1, 2]);
    }

    #[test]
    fn version_bits_override_recovers_from_a_wrong_dimension_estimate() {
        // Reproduces the recorded "ver_12_v40" finding (Plan 2 Task 6 follow-
        // ups, transcribed in the plan doc): a real v40 triplet's own
        // dimension estimate landed at 173 (v39's dimension), one version
        // step short of the true 177 (v40) — the geometric estimate is
        // close but not exact, and only the version-info bits can correct
        // it. (A much larger, multi-version gap is NOT representative: at
        // that scale the wrong assumed dimension distorts the whole-grid
        // transform enough that the version-info blocks themselves get
        // sampled from the wrong pixels — this test's one-version gap
        // matches the actual recorded failure mode instead.)
        let version = 40i16;
        let dim = 17 + 4 * version as usize; // 177
        let scale = 2.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"VERSION40TEST", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        assert_eq!(code.width(), dim);
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let wrong_est_dim = dim - 4; // v39's dimension (173), one version step short of the true v40 (177).
        let t = triplet_from_transform(&transform, dim, wrong_est_dim as u32);
        let (codes, attempts, _timings) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t));
        assert_eq!(attempts.len(), 1, "{attempts:?}");
        assert_eq!(attempts[0].dimension_est, wrong_est_dim as u32);
        assert_eq!(attempts[0].dimension_final, dim as u32);
        assert_eq!(attempts[0].version_bits, Some(40));
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].payload, "VERSION40TEST");
        assert_eq!(codes[0].version, 40);
    }

    #[test]
    fn arbitration_consumes_finders_and_skips_a_duplicate_triplet() {
        let version = 1i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"ARBIT", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        // Two triplets sharing all three finder indices (a bogus "second
        // grouping" of the same three finders) but with different
        // snap_error, out of order in the input slice — the lower-error one
        // is index 1 here, so proximity dedup must keep index 1 and drop
        // index 0's *duplicate*, but since both point at the same real
        // image content, the actually-attempted one still decodes and
        // consumes {0,1,2}.
        let mut worse = triplet_from_transform(&transform, dim, dim as u32);
        worse.snap_error = 0.5;
        let mut better = triplet_from_transform(&transform, dim, dim as u32);
        better.snap_error = 0.1;
        let triplets = [worse, better];

        let (codes, attempts, _timings) =
            decode_candidates(&view, &grid, &dummy_finders(3), &triplets);
        assert_eq!(codes.len(), 1, "expected exactly one decode, no spurious duplicate");
        assert_eq!(attempts.len(), 1, "the duplicate must be deduped away before attempting");
        assert_eq!(attempts[0].triplet_index, 1, "the lower-snap_error (index 1) survivor must be the one attempted");
    }

    #[test]
    fn oob_candidate_is_rejected_before_decode() {
        let version = 2i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"OOBTEST", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        // Crop the frame's own width in half, same trick sample.rs's own
        // OOB test uses: the triplet still reflects the true (pre-crop)
        // geometry, so a large fraction of modules now sample outside the
        // narrower view.
        let cropped_width = img_side / 2;
        let view = LumaView::new(&img, cropped_width, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let t = triplet_from_transform(&transform, dim, dim as u32);
        let (codes, attempts, _timings) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t));
        assert!(codes.is_empty());
        assert_eq!(attempts.len(), 1);
        assert!(attempts[0].outcome.starts_with("oob_fraction"), "{}", attempts[0].outcome);
    }

    #[test]
    fn garbage_triplet_fails_cleanly_without_panicking() {
        // A triplet whose geometry doesn't correspond to any real code in
        // the (flat) image: every stage must fail gracefully, never panic.
        let img = vec![128u8; 64 * 64];
        let view = LumaView::new(&img, 64, 64, 64).unwrap();
        let grid = TileGrid::build(&view);
        let t = TripletCandidate {
            tl: [10.0, 10.0],
            tr: [50.0, 10.0],
            bl: [10.0, 50.0],
            module: 4.0,
            dimension: 21,
            snap_error: 0.0,
            inverted: false,
            finder_indices: [0, 1, 2],
        };
        let (codes, attempts, _timings) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t));
        assert!(codes.is_empty());
        assert_eq!(attempts.len(), 1);
        assert_ne!(attempts[0].outcome, "decoded");
    }

    #[test]
    fn attempt_cap_limits_total_attempts_even_with_many_disjoint_candidates() {
        // MAX_DECODE_ATTEMPTS + a handful more disjoint (no shared finders)
        // bogus triplets over a flat image: dedup keeps all of them (no
        // finder overlap), but the cap must stop attempts at
        // MAX_DECODE_ATTEMPTS regardless.
        let img = vec![128u8; 64 * 64];
        let view = LumaView::new(&img, 64, 64, 64).unwrap();
        let grid = TileGrid::build(&view);
        let extra = 5;
        let total = MAX_DECODE_ATTEMPTS + extra;
        let triplets: Vec<TripletCandidate> = (0..total)
            .map(|i| TripletCandidate {
                tl: [10.0, 10.0],
                tr: [50.0, 10.0],
                bl: [10.0, 50.0],
                module: 4.0,
                dimension: 21,
                snap_error: i as f64,
                inverted: false,
                finder_indices: [i * 3, i * 3 + 1, i * 3 + 2],
            })
            .collect();
        let (codes, attempts, _timings) =
            decode_candidates(&view, &grid, &dummy_finders(total * 3), &triplets);
        assert!(codes.is_empty());
        assert_eq!(attempts.len(), MAX_DECODE_ATTEMPTS);
    }
}
