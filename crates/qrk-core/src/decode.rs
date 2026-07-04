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
//!    instead, see step 2): a pure cross-check, never an override —
//!    [`DecodeAttemptTrace::timing_check`] records the raw transition-count
//!    reading for diagnostics, but `dimension` is never reassigned from it.
//!    (Plan 5 Task 6: this stage used to attempt to *adopt* the
//!    lattice-snapped reading when within +/-2 modules of the current
//!    estimate, but that branch was provably dead — see the inline
//!    derivation at the check — and was removed in favor of a
//!    `debug_assert` of the same lattice invariant.)
//! 2. **Version-info bits** (only once the dimension implies `version >=
//!    7`): [`crate::version::read_version_bits`] BCH-decodes the redundant
//!    version-info blocks; per the plan's recorded `ver_12_v40` finding this
//!    is authoritative over the geometric estimate when it decodes, so a
//!    disagreement here always wins — the ONLY way `dimension` ever changes
//!    from the triplet's own estimate.
//!
//! After step 2, a defensive bounds check rejects a dimension whose implied
//! version would fall outside `1..=40` — [`crate::alignment::alignment_coords`]
//! panics on an out-of-range version. Step 2's BCH table only ever yields
//! `7..=40`, and step 1 never changes `dimension` at all, so this guard is
//! purely defensive against future refactors — but silently corrupting
//! `dimension` into something `alignment_coords` panics on would take down
//! the whole scan, so an out-of-range value is still rejected here as a
//! normal (traced) failed attempt rather than assumed impossible.
//!
//! After the dimension settles: alignment patterns are located, the grid is
//! sampled, and [`crate::bitmatrix::decode_bits`] runs (with its own
//! internal mirrored retry). When that tile-threshold decode fails on a
//! bit-error class re-thresholding can plausibly fix (Ecc/Content/Format —
//! not Version, which means the grid size itself is wrong), Plan 4B Fix B
//! retries ONCE per round from the SAME sampled grays, re-thresholded
//! against a per-code finder-derived reference value and unsharp-mask
//! sharpened first (see [`crate::bitmatrix::build_reference_threshold_bits`])
//! — real-video-capture robustness against scene-driven tile-threshold
//! drift and inter-module blur crosstalk, no resampling. A successful
//! decode consumes its three finder indices so no later, lower-priority
//! triplet sharing any of them can also decode in the same frame — the
//! arbitration property Gate 3 checks.

use crate::alignment::{locate_alignment_patterns, AnchorSlot};
use crate::bitmatrix::{build_reference_threshold_bits, decode_bits, BitMatrix, DecodeFailure};
use crate::consts::{MAX_DECODE_ROUNDS, MAX_OOB_FRACTION};
use crate::finder::FinderCandidate;
use crate::refine::{refine_corners, RefinedCorners};
use crate::sample::{
    needs_refined_br, provisional_transform, refine_fourth_corner, sample_grid, EdgeFitMode,
    SourceView,
};
use crate::scanner::StageClock;
use crate::tiles::TileGrid;
use crate::trace::{AlignmentTraceEntry, BitsTrace, RefineTrace, SampleRegionTrace};
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
    /// TL, TR, BR, BL image-pixel corners of the module region — module-space
    /// `(0,0)`, `(dimension,0)`, `(dimension,dimension)`, `(0,dimension)`
    /// respectively. Each corner is mapped through the FINAL sample
    /// region (from the round that actually decoded) whose `module_rect`
    /// touches it, at that region's own unit-square point (`(0,0)`,
    /// `(1,0)`, `(1,1)`, or `(0,1)`). For a single-region candidate (v1..6,
    /// or any candidate whose sampling used one located/refined 4th
    /// anchor for the whole grid) all four corners come from that same
    /// whole-grid transform, so this is unchanged from the pre-multi-region
    /// behavior. For a multi-region candidate (v7+, with interior
    /// alignment patterns actually located and tiled — see
    /// [`crate::sample::sample_grid`]), each corner instead comes from
    /// ITS OWN corner region's transform — e.g. the BR corner is anchored
    /// by the alignment-pattern evidence nearest the bottom-right, not
    /// extrapolated from the whole-grid finder-only parallelogram, which
    /// under keystone perspective would otherwise be off by whole modules
    /// at exactly that corner.
    pub corners: [[f64; 2]; 4],
    /// `true` when the code's polarity is inverted (light-on-dark).
    pub inverted: bool,
    /// The three [`FinderCandidate`] indices this decode consumed —
    /// identical to the winning [`TripletCandidate::finder_indices`].
    pub finder_indices: [usize; 3],
    /// Subpixel-refined TL/TR/BR/BL module-region corners, in SOURCE px
    /// (Plan 5 Task 3) — `None` when `ScanOptions::refine` was `false`, or
    /// `true` but [`crate::refine::refine_corners`] produced fewer than 2
    /// valid edge lines (see that function's doc). `corners` above always
    /// stays WORKING px regardless; convert it to source px yourself
    /// (`working_px / source_scale`, per-axis via `Detections::source_scale`'s
    /// doc) if you need to compare the two spaces directly — this field is
    /// never a scaled copy of `corners`, it's an independent, more precise
    /// estimate traced directly against the source image.
    pub refined_corners: Option<[[f64; 2]; 4]>,
    /// Per-corner refinement provenance for `refined_corners`, `[TL, TR,
    /// BR, BL]` (final-review carried item: this was previously buried
    /// inside the trace-only `RefineTrace::corner_refined` — `Trace` is
    /// opt-in per call, so a caller reading `refined_corners` without a
    /// `Trace` had no way to tell a genuinely-refined corner from one that
    /// silently fell back to the coarse, source-scaled position — see
    /// `refined_corners`'s own doc for that fallback rule). `[false; 4]`
    /// when `refined_corners` is `None` (refinement didn't run, or ran and
    /// produced fewer than 2 valid edge lines); mirrors
    /// `crate::refine::RefinedCorners::corner_refined` field-for-field when
    /// `refined_corners` is `Some`.
    pub corner_refined: [bool; 4],
}

/// One decode attempt's trace, whether it succeeded or failed — every
/// triplet actually attempted (i.e. not skipped because its finders were
/// already consumed, and not dropped by proximity dedup) gets exactly one
/// of these. `Trace::attempts` (Plan 4 Task 6) is this struct's real
/// consumer: `decode_candidates` always builds its `Vec<DecodeAttemptTrace>`
/// as part of its normal return contract (not gated behind any trace flag —
/// the data is small), and `scanner.rs`'s `detect_with` records a copy into
/// `Trace` when one was requested.
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
    /// The dimension actually used to build the sampled grid (post
    /// version-bits override, the only stage that can ever change it — see
    /// the module doc).
    pub dimension_final: u32,
    /// The timing cross-check's raw reading (`transitions + 13`), if the
    /// walk completed — `None` if it wasn't run (estimate > 41) or the walk
    /// left the image. Diagnostic only: this reading is never adopted into
    /// `dimension_final` (see the module doc), so it's recorded here
    /// unconditionally rather than only on disagreement.
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
    /// `true` when this attempt sampled through an image-derived 4th
    /// corner from [`crate::sample::refine_fourth_corner`] (Plan 4 Task 5b)
    /// instead of the parallelogram extrapolation — i.e. no BR alignment
    /// anchor existed AND the edge-tracing refinement succeeded. Recorded
    /// as its own field (the brief left the choice of "extend outcome or
    /// add a bool" open) so trace consumers can filter on it without
    /// string-parsing `outcome`.
    pub refined_corner: bool,
    /// `"decoded"` on success (see [`decode_candidates`]'s cross-check note
    /// for the one case where it carries an appended discrepancy message
    /// instead of the bare string), else a short failure reason.
    pub outcome: String,
    /// Per-round visibility into this attempt's sample+decode sub-pipeline
    /// (final-review carried item: without this, a failed REFINED round —
    /// one that ran a retry but still didn't decode — was invisible, since
    /// `outcome` only ever reflected the FIRST (parallelogram) round or
    /// whichever round actually decoded, silently dropping every failed
    /// retry's own reason). One entry per round actually run, in order:
    /// always starts with `"parallelogram:<tag>"`, followed by
    /// `"anchor_line:<tag>"` and/or `"outer_hull:<tag>"` when Task 5b's
    /// refined-corner retry ran (see the loop below for exactly when each
    /// mode is skipped). Plan 4B Fix B extends the round NAME (not `<tag>`)
    /// with a `+refbits` suffix — e.g. `"outer_hull+refbits:decoded"` —
    /// whenever that round's tile-threshold `bits` failed rqrr but the
    /// reference-threshold + sharpening retry (same sampled grays, no
    /// resampling — see `bitmatrix::build_reference_threshold_bits`) then
    /// decoded, so the mechanism stays visible in the trace rather than
    /// looking identical to a plain tile-threshold decode. `<tag>` is one
    /// of `"decoded"`, `"oob"`,
    /// `"sample_transform_degenerate"`, or one of `"failed_format"` /
    /// `"failed_version"` / `"failed_ecc"` / `"failed_content"` (one per
    /// `crate::bitmatrix::DecodeFailure` variant — only `failed_ecc` is an
    /// actual Reed-Solomon/error-correction rejection; the other three are
    /// distinct failure classes with distinct root causes, so this trace
    /// keeps them apart rather than collapsing them into one misleadingly
    /// RS-flavored bucket) — see [`round_tag`]. Chosen over embedding this
    /// into `outcome` itself
    /// (the brief's alternative) to keep `outcome` a stable, single-reason
    /// string for existing consumers (this module's own tests match on it
    /// with `starts_with`/`==`) while still giving a trace consumer full
    /// per-round detail as structured data instead of a string to parse.
    pub rounds: Vec<String>,
}

/// Collapse a round's full `outcome` string down to one of
/// [`DecodeAttemptTrace::rounds`]'s four stable tags, so a trace consumer
/// can match on a fixed vocabulary instead of parsing arbitrary prose (the
/// non-decoded outcome strings above are diagnostic text for a human, not a
/// stable contract).
fn round_tag(outcome: &str) -> &'static str {
    if outcome.starts_with("decoded") {
        "decoded"
    } else if outcome.starts_with("oob_fraction") {
        "oob"
    } else if outcome == "sample_transform_degenerate" {
        "sample_transform_degenerate"
    } else {
        // The only remaining source is `format!("{e:?}")` of a
        // `crate::bitmatrix::DecodeFailure`, whose `{:?}` is exactly one of
        // these four variant names (see that enum's doc). Matched by name
        // rather than collapsed into one "failed_rs" bucket (an earlier,
        // less precise version of this function did that): only `Ecc` is
        // an actual Reed-Solomon/error-correction failure — `Format`/
        // `Version` are BCH-protected *metadata* reads (a geometry/masking
        // bug reads as a totally different failure than a real ECC
        // overflow), and `Content` fires only after Reed-Solomon has
        // already succeeded (the bug is in segment/UTF-8 parsing, not
        // error correction at all) — a single "failed_rs" tag would point
        // a trace consumer at the wrong subsystem for 3 of these 4 cases.
        match outcome {
            "Format" => "failed_format",
            "Version" => "failed_version",
            "Ecc" => "failed_ecc",
            "Content" => "failed_content",
            // Defensive: `decode_bits` only returns those four variants
            // today, but a stable fallback beats a silent misclassification
            // if that enum ever grows a new one.
            _ => "failed_other",
        }
    }
}

/// This round's name for [`DecodeAttemptTrace::rounds`] — see
/// `sample::EdgeFitMode`'s own doc for what each mode does.
fn round_name(mode: EdgeFitMode) -> &'static str {
    match mode {
        EdgeFitMode::AnchorLine => "anchor_line",
        EdgeFitMode::OuterHull => "outer_hull",
    }
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
/// [`MAX_DECODE_ROUNDS`] rounds of decode work, so there is no single contiguous
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
    /// Subpixel corner refinement (Plan 5 Task 3), summed across every
    /// attempt that actually ran `refine_corners` (i.e. decoded, with
    /// `ScanOptions::refine == true`) — `0` whenever refinement is
    /// disabled, matching every other field's "only counts attempts that
    /// reached that stage" convention.
    pub refine_ns: u64,
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
/// finders) or a trace-only failure, plus (Plan 4 Task 6) the debug-trace
/// data this attempt produced — built only when `want_trace` is set (see
/// `attempt_candidate`'s own parameter), so a caller that isn't collecting
/// a `Trace` at all (`detect()`, or `qrk-wasm`'s `with_trace: false`) never
/// pays for the extra `Vec`/homography-projection work these three fields
/// cost, on top of not keeping a copy of it.
struct AttemptResult {
    trace: DecodeAttemptTrace,
    code: Option<DecodedCode>,
    /// This attempt's alignment-pattern search results, Task 6 trace form —
    /// empty both when `!want_trace` and, legitimately, for a v1 candidate
    /// (no alignment patterns exist at all) or one that bailed out before
    /// ever locating alignment patterns (the `invalid_dimension` early
    /// return); `decode_candidates` (Plan 4B Fix A) no longer needs to tell
    /// these apart — it selects a trace-record-worthy attempt by POSITION
    /// (first attempt overall, or the attempt that decoded), not by
    /// whether this vec happens to be non-empty, so a trivial empty result
    /// here is simply the honest answer for that attempt.
    alignment_trace: Vec<AlignmentTraceEntry>,
    /// This attempt's sample regions, Task 6 trace form — from whichever
    /// round produced `code`/`corners` (the FINAL round, same rule
    /// `corners` itself already follows; see the module doc), so the two
    /// stay geometrically consistent. Empty when `!want_trace`.
    sample_regions_trace: Vec<SampleRegionTrace>,
    /// `Some` iff this attempt decoded (`code.is_some()`) AND `want_trace`
    /// was set — the sampled bit matrix behind that decode.
    bits_trace: Option<BitsTrace>,
    /// This attempt's subpixel corner refinement diagnostics (Plan 5 Task
    /// 3) — `Some` iff refinement actually ran (`refine == true` AND this
    /// attempt decoded) AND produced a usable result (`refine_corners`
    /// returned `Some`). Unlike `alignment_trace`/`sample_regions_trace`/
    /// `bits_trace` this is populated regardless of `want_trace`: the data
    /// is a fixed 4-element array of small integers/bools (no
    /// `Vec`/homography-projection allocation), so gating it behind
    /// `want_trace` would save essentially nothing while adding a second
    /// code path.
    refine_trace: Option<RefineTrace>,
}

/// Run the full per-candidate pipeline for one triplet: dimension
/// cross-checks, alignment, sampling, and `decode_bits`. See the module
/// doc for the stage-by-stage contract. Accumulates elapsed time for each
/// of the three stage buckets into `timings`. `want_trace` gates the Task 6
/// debug-trace fields on the returned `AttemptResult` (see its doc) — pass
/// `false` from any caller that isn't collecting a `Trace` at all. `source`
/// (Plan 5 Task 2) is `Some` only when `scan()` downscaled the source image
/// to build `view` — see [`sample_grid`]'s doc for exactly how it
/// changes module sampling; every OTHER stage here (timing cross-check,
/// version-info bits, alignment-pattern location) still runs on `view`
/// (the WORKING view) regardless, per the plan's scope note. `refine`
/// (Plan 5 Task 3) enables subpixel corner refinement on a successful
/// decode — against `source.view` when `source` is `Some`, else against
/// `view` itself with `sx = sy = 1.0` (source == working: refinement still
/// runs when no downscale happened, it just has nothing to lift — see
/// `refine::refine_corners`'s doc).
#[allow(clippy::too_many_arguments)]
fn attempt_candidate(
    view: &LumaView,
    grid: &TileGrid,
    triplet_index: usize,
    t: &TripletCandidate,
    timings: &mut DecodeTimings,
    want_trace: bool,
    source: Option<SourceView>,
    refine: bool,
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
            // A raw timing reading is a transition count, unconstrained by
            // the QR dimension lattice (21, 25, 29, ... — always == 1 mod
            // 4), so it must be snapped to the nearest lattice value before
            // it may replace `dimension` (zxing `Detector.computeDimension`
            // does the same snap on its module-counting estimate; adopting
            // a raw 23 would build an invalid 23x23 grid that can never
            // decode — an actual bug caught by the Task 5b gate run on
            // `tilt45_02`, whose timing walk misread 21 as 23). A reading
            // exactly between two lattice values (raw == 3 mod 4, e.g. 23)
            // snaps ambiguously — zxing rejects that case outright, and so
            // does this. Note the arithmetic consequence: on the lattice,
            // any change is a multiple of 4, so the brief's +/-2 adoption
            // tolerance means the snapped reading can only ever *confirm*
            // the estimate — the timing check is a cross-check, and the
            // version-info bits (below) remain the only dimension override,
            // exactly their respective roles in the plan.
            let rem = (timing_dim as i64 - 21).rem_euclid(4);
            let snapped = match rem {
                0 => Some(timing_dim as i64),
                1 => Some(timing_dim as i64 - 1),
                3 => Some(timing_dim as i64 + 1),
                _ => None, // equidistant between two lattice values
            };
            // Plan 5 Task 6 (perf cleanup, provably-dead branch removed):
            // this used to be `if (snapped - dimension).abs() <= 2 &&
            // snapped != dimension { dimension = snapped; ... }` — an
            // "adopt the timing reading" branch that can never actually
            // run. `snapped` (every `Some` arm above) and `dimension`
            // (always `t.dimension`, itself always a QR lattice value from
            // `triplet.rs`'s own snapping) are BOTH `== 1 (mod 4)`, so
            // `snapped - dimension` is always a multiple of 4 — the only
            // multiple of 4 with `abs(..) <= 2` is 0, which the `!=`
            // half of the old condition excluded. So the adoption
            // could only ever confirm `dimension`, never change it,
            // exactly as this function's doc already says ("the brief's
            // +/-2 adoption tolerance means the snapped reading can only
            // ever *confirm* the estimate"). Kept as a debug_assert of
            // that lattice invariant, rather than silent dead code, so a
            // future change to either snapping scheme trips it instead of
            // quietly reintroducing an unreachable branch.
            if let Some(snapped) = snapped {
                debug_assert_eq!(
                    (snapped - dimension as i64).rem_euclid(4),
                    0,
                    "lattice-snapped timing reading {snapped} and dimension estimate {dimension} \
                     should both sit on the QR dimension lattice (== 1 mod 4), so their \
                     difference should always be a multiple of 4",
                );
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
                refined_corner: false,
                outcome: "invalid_dimension".to_string(),
                rounds: Vec::new(),
            },
            code: None,
            alignment_trace: Vec::new(),
            sample_regions_trace: Vec::new(),
            bits_trace: None,
            refine_trace: None,
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
    let alignment =
        locate_alignment_patterns(view, grid, &transform, version, t.inverted, want_trace);
    timings.alignment_ns += alignment_clock.elapsed_ns();

    let n = alignment.coords.len();
    let alignment_total = (n * n).saturating_sub(3) as u32;
    let alignment_found = alignment
        .found
        .iter()
        .filter(|s| matches!(s, AnchorSlot::Found(_)))
        .count() as u32;
    // Task 6: only built when a trace was actually requested (review
    // finding: unconditionally building this — and the homography-heavy
    // `sample_regions_trace`/`bits_trace` below — on every attempt this
    // frame (bounded by [`MAX_DECODE_ROUNDS`]'s per-frame round budget) is
    // real, avoidable work on the untraced hot path
    // `detect()`/`with_trace: false` take).
    let alignment_trace =
        if want_trace { alignment.to_trace_entries() } else { Vec::new() };

    let sample_decode_clock = StageClock::start();

    // One sample+decode round's full result, including its Task 6 trace
    // data (also always built — see `AttemptResult`'s doc).
    struct RoundOutcome {
        outcome: String,
        oob_fraction: f64,
        corners: [[f64; 2]; 4],
        payload: Option<crate::bitmatrix::DecodedPayload>,
        sample_regions_trace: Vec<SampleRegionTrace>,
        bits_trace: Option<BitsTrace>,
        /// The actual bit matrix behind a successful `payload` (`None`
        /// otherwise) — Plan 5 Task 3's `refine_corners` needs the real
        /// [`BitMatrix`], not just its packed `BitsTrace` form, and
        /// unconditionally (regardless of `want_trace`), so this is a
        /// separate field rather than reusing `bits_trace`.
        bits: Option<BitMatrix>,
        /// `true` iff this round's `payload` came from Plan 4B Fix B's
        /// reference-threshold + sharpening retry (below) rather than the
        /// primary tile-threshold `bits` — the caller extends this round's
        /// tag with `+refbits` when set, so the mechanism stays visible in
        /// `DecodeAttemptTrace::rounds` (e.g. `"outer_hull+refbits:decoded"`).
        used_refbits: bool,
    }

    // One sample+decode round for a given (optional) refined BR corner.
    // `corners`/`sample_regions_trace` come from the FINAL sampled grid:
    // each corner through the region whose `module_rect` touches it (see
    // `DecodedCode::corners`'s doc) — the single region's own transform for
    // every corner when sampling ran through one region (v1/no-AP, the
    // Task 5b refined rebuild, and the classic single-AP case), else each
    // corner's own AP-anchored tile for the multi-region case.
    let run_round = |refined_br: Option<[f64; 2]>| -> RoundOutcome {
        let fallback_corners = [
            transform.map(0.0, 0.0),
            transform.map(1.0, 0.0),
            transform.map(1.0, 1.0),
            transform.map(0.0, 1.0),
        ];
        let sampled = match sample_grid(view, grid, t, dimension, &alignment, refined_br, source) {
            Some(s) => s,
            None => {
                return RoundOutcome {
                    outcome: "sample_transform_degenerate".to_string(),
                    oob_fraction: 0.0,
                    corners: fallback_corners,
                    payload: None,
                    sample_regions_trace: Vec::new(),
                    bits_trace: None,
                    bits: None,
                    used_refbits: false,
                }
            }
        };
        // Each grid corner (module-space `0`/`dimension` on each axis) is
        // mapped through the SPECIFIC region whose `module_rect` touches
        // it, never through the whole-grid finder-only provisional. The
        // regions tile `[0,dimension) x [0,dimension)` with no gaps or
        // overlaps (see `sample_grid`'s tiling loop, which stretches the
        // outermost interval on each axis to reach `0`/`dimension`), so
        // exactly one region's `module_rect` touches any given grid
        // corner. For a single-region candidate that region spans the
        // whole grid, so this reduces to the previous whole-grid mapping;
        // for the multi-region tiling (v7+, real interior alignment
        // patterns) it instead uses the corner's OWN nearest AP-anchored
        // region — critically, the BR corner then reflects the alignment
        // pattern(s) located near it, not a whole-grid parallelogram
        // extrapolation that silently discards that evidence and, under
        // keystone perspective, is off by whole modules there.
        let corner_via_region = |mx: u32, my: u32| -> [f64; 2] {
            let region = sampled
                .regions
                .iter()
                .find(|r| {
                    let [x0, y0, x1, y1] = r.module_rect;
                    (x0 == mx || x1 == mx) && (y0 == my || y1 == my)
                })
                .expect(
                    "sample_grid's regions tile [0,dimension) x [0,dimension) with no \
                     gaps, so every grid corner touches at least one region's module_rect",
                );
            region.transform.map(mx as f64 / dimension as f64, my as f64 / dimension as f64)
        };
        let corners = [
            corner_via_region(0, 0),
            corner_via_region(dimension, 0),
            corner_via_region(dimension, dimension),
            corner_via_region(0, dimension),
        ];
        let sample_regions_trace: Vec<SampleRegionTrace> = if want_trace {
            sampled.regions.iter().map(|r| r.to_trace(dimension)).collect()
        } else {
            Vec::new()
        };
        if sampled.oob_fraction > MAX_OOB_FRACTION {
            return RoundOutcome {
                outcome: format!("oob_fraction {:.4} exceeds {:.4}", sampled.oob_fraction, MAX_OOB_FRACTION),
                oob_fraction: sampled.oob_fraction,
                corners,
                payload: None,
                sample_regions_trace,
                bits_trace: None,
                bits: None,
                used_refbits: false,
            };
        }
        // Build one round's `RoundOutcome` from a decoded (or failed)
        // `bitmatrix::decode_bits` result — shared between the primary
        // tile-threshold attempt and Fix B's reference-threshold retry
        // below, so the version cross-check / trace-building logic isn't
        // duplicated.
        let outcome_from = |bits: &crate::bitmatrix::BitMatrix,
                             result: Result<crate::bitmatrix::DecodedPayload, DecodeFailure>,
                             used_refbits: bool| {
            match result {
                Ok(payload) => {
                    // Cross-check consistency (Global Constraints,
                    // transcribed): decode_bits' own version comes from the
                    // bit matrix it was handed, which is exactly
                    // `dimension`-sized, so in practice this can never
                    // actually disagree — QR dimension<->version is a
                    // bijection (`dimension = 17 + 4*version`), and rqrr
                    // derives its reported version from the grid size it
                    // was given, not by re-reading version-info bits at
                    // decode time. Checked anyway, defensively: if it ever
                    // did disagree, decode_bits' RS-validated fields are
                    // what DecodedCode carries, with the discrepancy
                    // recorded in the trace rather than silently dropped.
                    let outcome = if payload.version == version {
                        "decoded".to_string()
                    } else {
                        format!(
                            "decoded (dimension mismatch: sampled grid implied v{version}, \
                             decode_bits returned v{})",
                            payload.version
                        )
                    };
                    let bits_trace = want_trace
                        .then(|| BitsTrace { dim: bits.dim as u32, words: bits.words().to_vec() });
                    RoundOutcome {
                        outcome,
                        oob_fraction: sampled.oob_fraction,
                        corners,
                        payload: Some(payload),
                        sample_regions_trace: sample_regions_trace.clone(),
                        bits_trace,
                        bits: Some(bits.clone()),
                        used_refbits,
                    }
                }
                Err(e) => RoundOutcome {
                    outcome: format!("{e:?}"),
                    oob_fraction: sampled.oob_fraction,
                    corners,
                    payload: None,
                    sample_regions_trace: sample_regions_trace.clone(),
                    bits_trace: None,
                    bits: None,
                    used_refbits: false,
                },
            }
        };

        let primary = decode_bits(&sampled.bits);
        // Plan 4B Fix B: the tile-threshold `bits` failed rqrr on one of
        // the three classes re-thresholding can plausibly fix — `Version`
        // means the dimension/grid size itself is wrong, which no amount
        // of re-thresholding the SAME grid corrects, so it's excluded.
        // Exactly one extra `decode_bits` call, no resampling: same
        // `sampled.grays` this round already captured, just a different
        // bit matrix built from them (see `build_reference_threshold_bits`).
        if let Err(e) = &primary {
            if matches!(e, DecodeFailure::Ecc | DecodeFailure::Content | DecodeFailure::Format) {
                if let Some(refbits) =
                    build_reference_threshold_bits(&sampled.grays, dimension as usize, t.inverted)
                {
                    let retry = decode_bits(&refbits);
                    if retry.is_ok() {
                        return outcome_from(&refbits, retry, true);
                    }
                }
            }
        }
        outcome_from(&sampled.bits, primary, false)
    };

    // Task 6: per-round outcome visibility (see `DecodeAttemptTrace::rounds`'s
    // doc) — a round's tag is recorded regardless of whether it goes on to
    // win the attempt below. Plan 4B Fix B: a `+refbits` suffix on the
    // round NAME (not the outcome tag) makes the reference-threshold +
    // sharpening retry visible whenever it's the one that actually decoded
    // (e.g. `"outer_hull+refbits:decoded"`). Defined before `first` is
    // destructured below so it can still borrow `first` whole.
    let round_entry = |name: &str, r: &RoundOutcome| -> String {
        format!("{}{}:{}", name, if r.used_refbits { "+refbits" } else { "" }, round_tag(&r.outcome))
    };

    // First round: the Task 5 behavior — parallelogram / single-AP
    // anchored sampling, exactly as previously gated.
    let first = run_round(None);
    let mut rounds = vec![round_entry("parallelogram", &first)];
    let mut outcome = first.outcome;
    let mut oob_fraction = first.oob_fraction;
    let mut corners = first.corners;
    let mut payload = first.payload;
    let mut sample_regions_trace = first.sample_regions_trace;
    let mut bits_trace = first.bits_trace;
    let mut winning_bits = first.bits;
    let mut refined_corner = false;

    // Task 5b: only when that round fails Reed-Solomon decode AND the
    // alignment search produced no BR anchor at all (v1 always; higher
    // versions whose probes all missed), estimate the 4th corner from the
    // image and retry once. Trying the parallelogram FIRST is deliberate:
    // (a) it preserves the previously gated behavior wherever it already
    // worked — the edge-fit refinement carries its own localization noise
    // (measured 0.1-0.5 modules of BR-corner error across the golden
    // fixtures) and must not displace a near-exact parallelogram on the
    // frontal cases it was never needed for — and (b) `decode_bits` is
    // RS-validated, so retry-on-failure can never turn a good decode into
    // a wrong one. Runs under the sample_decode timing bucket (it is
    // sampling-geometry work, interleaved per attempt like the rest), and
    // only on the failure path, so the common already-decoding path never
    // pays for it.
    if payload.is_none() && needs_refined_br(&alignment) {
        // Two robust edge-fit estimators with complementary failure
        // domains (see `sample::EdgeFitMode`), each RS-validated — a wrong
        // corner estimate can only fail the retry, never mis-decode.
        let mut tried: Option<[f64; 2]> = None;
        for mode in [EdgeFitMode::AnchorLine, EdgeFitMode::OuterHull] {
            let Some(rc) = refine_fourth_corner(view, grid, t, dimension, &transform, mode) else {
                continue;
            };
            // Skip the second decode when both estimators agree (within a
            // tenth of a module — far below any error that could flip a
            // sampled bit, so the retry would be byte-identical).
            if let Some(prev) = tried {
                let d2 = (rc[0] - prev[0]).powi(2) + (rc[1] - prev[1]).powi(2);
                if d2.sqrt() < 0.1 * t.module {
                    continue;
                }
            }
            tried = Some(rc);
            let r = run_round(Some(rc));
            rounds.push(round_entry(round_name(mode), &r));
            if r.payload.is_some() {
                outcome = r.outcome;
                oob_fraction = r.oob_fraction;
                corners = r.corners;
                payload = r.payload;
                sample_regions_trace = r.sample_regions_trace;
                bits_trace = r.bits_trace;
                winning_bits = r.bits;
                refined_corner = true;
                break;
            }
        }
    }
    timings.sample_decode_ns += sample_decode_clock.elapsed_ns();

    // Plan 5 Task 3: subpixel corner refinement, against the SOURCE image
    // when a downscale happened, else `view` itself (source == working,
    // `sx = sy = 1.0` — refinement still runs; see this function's doc).
    // Only attempted on an actual decode (`winning_bits` is `Some` only
    // then — see `RoundOutcome::bits`'s doc), and timed separately from
    // `sample_decode_ns` (its own `StageTimings` row).
    let refine_result = if refine {
        winning_bits.as_ref().and_then(|bits| {
            let (refine_source, sx, sy) = match source {
                Some(sv) => (sv.view, sv.sx, sv.sy),
                None => (view, 1.0, 1.0),
            };
            let refine_clock = StageClock::start();
            let result = refine_corners(refine_source, sx, sy, &corners, bits, t.inverted);
            timings.refine_ns += refine_clock.elapsed_ns();
            result
        })
    } else {
        None
    };
    let refined_corners = refine_result.as_ref().map(|r| r.corners);
    let corner_refined = corner_refined_flags(refine_result.as_ref());
    let refine_trace = refine_result.as_ref().map(|r| r.to_trace());

    AttemptResult {
        trace: DecodeAttemptTrace {
            triplet_index,
            dimension_est,
            dimension_final: dimension,
            timing_check,
            version_bits,
            alignment_found,
            alignment_total,
            oob_fraction,
            refined_corner,
            outcome,
            rounds,
        },
        code: payload.map(|p| DecodedCode {
            payload: p.payload,
            payload_bytes: p.payload_bytes,
            version: p.version,
            ecc: p.ecc,
            mirrored: p.mirrored,
            dimension,
            corners,
            inverted: t.inverted,
            finder_indices: t.finder_indices,
            refined_corners,
            corner_refined,
        }),
        alignment_trace,
        sample_regions_trace,
        bits_trace,
        refine_trace,
    }
}

/// `DecodedCode::corner_refined`'s plumbing: `[false; 4]` when refinement
/// didn't run or produced `None` (see `DecodedCode::refined_corners`'s doc
/// for both cases), else a field-for-field copy of the successful
/// [`RefinedCorners::corner_refined`] — never re-derived here, so this can
/// never disagree with the per-corner fallback [`refine_round`] itself
/// already decided. Pulled out as its own function (rather than inlined at
/// the `DecodedCode` literal) so it has a unit-testable seam independent of
/// running the full image pipeline — see the `corner_refined_flags_*` tests
/// below.
fn corner_refined_flags(refine_result: Option<&RefinedCorners>) -> [bool; 4] {
    refine_result.map(|r| r.corner_refined).unwrap_or([false; 4])
}

/// Decode every plausible QR code in one frame: proximity-dedup `triplets`,
/// attempt each survivor best-first (ascending `snap_error`), and arbitrate
/// so no two returned [`DecodedCode`]s share a finder. `finders` is used
/// only to size the finder-consumption bookkeeping (`finders.len()`) — all
/// of a triplet's own geometry already lives on the [`TripletCandidate`]
/// itself.
///
/// Returns the decoded codes, one [`DecodeAttemptTrace`] per attempt
/// actually run — a triplet contributes up to 3 consecutive entries, one
/// per corner-role rotation tried (all sharing its `triplet_index`, in
/// rotation order 0..3; see the rotation note in the loop below), while
/// proximity-deduped-away triplets and triplets skipped because their
/// finders were already consumed produce no trace entry at all — the
/// accumulated per-stage timing totals (see [`DecodeTimings`]'s doc for
/// why this is a 3-tuple element rather than the brief's literal 2-tuple);
/// and (Plan 4 Task 6) the frame-level trace data in [`DecodeTraceData`],
/// built only when `want_trace` is set (pass `trace.is_some()` from
/// `detect_with` — see [`AttemptResult`]'s doc for why this matters: the
/// per-attempt trace data is otherwise-avoidable homography/allocation work
/// on every attempt this frame runs, bounded by [`MAX_DECODE_ROUNDS`]'s
/// round budget — see the loop below). `source` (Plan 5
/// Task 2) is threaded straight through to every [`attempt_candidate`] call
/// unchanged — see that function's doc; so is `refine` (Plan 5 Task 3).
pub(crate) fn decode_candidates(
    view: &LumaView,
    grid: &TileGrid,
    finders: &[FinderCandidate],
    triplets: &[TripletCandidate],
    want_trace: bool,
    source: Option<SourceView>,
    refine: bool,
) -> (Vec<DecodedCode>, Vec<DecodeAttemptTrace>, DecodeTimings, DecodeTraceData) {
    let mut timings =
        DecodeTimings { version_ns: 0, alignment_ns: 0, sample_decode_ns: 0, refine_ns: 0 };
    let mut consumed = vec![false; finders.len()];
    let mut codes = Vec::new();
    let mut attempts = Vec::new();
    // Plan 5 Task 6: budgeted in ROUNDS (one `sample_grid`+`decode_bits`
    // cycle each — see [`MAX_DECODE_ROUNDS`]'s doc), not attempts. Checked
    // once per attempt (rotation), before it runs, so an attempt already
    // under way always completes — the worst-case overrun is bounded by
    // the most expensive single attempt (3 rounds), not unbounded.
    let mut rounds_run = 0usize;
    // Plan 4B Fix A (trace honesty): two independent captures, resolved
    // into the actual `DecodeTraceData` after the loop (see the doc there
    // for the full rule). `first_*`: the FIRST attempt run this frame —
    // rotation 0 of the first (lowest-`snap_error`) candidate actually
    // attempted, i.e. canonical (unrotated) corner roles — captured once
    // and never touched again, so a later candidate's or a rotation
    // retry's geometry can never displace it. `decoded_*`: the most recent
    // attempt that actually decoded (mirrors the pre-fix `bits` contract,
    // now extended to `alignment`/`sample_regions` too, so all three always
    // describe the SAME candidate on success).
    let mut first_alignment: Vec<AlignmentTraceEntry> = Vec::new();
    let mut first_sample_regions: Vec<SampleRegionTrace> = Vec::new();
    let mut first_attempt_seen = false;
    // Explicit "did anything decode this frame" selector for the branch
    // after the loop — deliberately NOT inferred from `decoded_bits`'s
    // `Option` state, which conflates "nothing decoded" with "decoded but
    // `want_trace` was false so no `BitsTrace` was ever built".
    let mut any_decoded = false;
    let mut decoded_alignment: Vec<AlignmentTraceEntry> = Vec::new();
    let mut decoded_sample_regions: Vec<SampleRegionTrace> = Vec::new();
    let mut decoded_bits: Option<BitsTrace> = None;
    let mut decoded_refine: Option<RefineTrace> = None;

    'candidates: for idx in dedup_triplet_indices(triplets) {
        let t = &triplets[idx];
        if t.finder_indices.iter().any(|&i| consumed[i]) {
            continue;
        }

        // Attempt the candidate's three cyclic corner-role rotations, best
        // (detected) assignment first. `triplet::try_group` picks the TL
        // corner as the most-perpendicular *image-space* angle — a
        // heuristic that steep perspective can fool (observed on
        // `tilt45_04`: the detected roles were one cyclic rotation off,
        // making the sampled matrix a 90-degree-rotated read that can
        // never decode). Each rotation is a full, traced attempt, itself
        // costing 1-3 rounds against the [`MAX_DECODE_ROUNDS`] budget below
        // — see that constant's doc for the "3 triplet permutations * 3
        // geometry rounds" provenance. Rotations after the first only run
        // when the previous one failed; `decode_bits` is RS-validated, so
        // a wrong rotation can never decode into a wrong payload. Cyclic
        // rotations preserve the tl/tr/bl cross-product orientation
        // convention, so the rotated candidate is still canonical.
        for rotation in 0..3 {
            if rounds_run >= MAX_DECODE_ROUNDS {
                break 'candidates;
            }
            let rotated = if rotation == 0 {
                *t
            } else {
                let mut r = *t;
                let (p, i) = ([t.tl, t.tr, t.bl], t.finder_indices);
                let k = rotation; // shift roles: new_tl = old[(0+k)%3], ...
                r.tl = p[k % 3];
                r.tr = p[(k + 1) % 3];
                r.bl = p[(k + 2) % 3];
                r.finder_indices = [i[k % 3], i[(k + 1) % 3], i[(k + 2) % 3]];
                r
            };

            let result =
                attempt_candidate(view, grid, idx, &rotated, &mut timings, want_trace, source, refine);
            // Each entry in `result.trace.rounds` is one `run_round` call
            // (`sample_grid`+`decode_bits`, optionally with Fix B's
            // same-grays refbits retry folded in — see
            // `MAX_DECODE_ROUNDS`'s doc for why that retry doesn't count as
            // its own round) — the actual, honest cost of this attempt.
            // `.max(1)`: the `invalid_dimension` early return (before any
            // `run_round` call — see `attempt_candidate`'s bounds check)
            // is the one path with a genuinely empty `rounds`; floored at 1
            // so a frame full of such trivially-rejected candidates still
            // makes bounded, budget-consuming progress rather than
            // bypassing the cap entirely.
            rounds_run += result.trace.rounds.len().max(1);
            let decoded = result.code.is_some();
            // Fix A: the very first attempt run this frame, full stop —
            // canonical (rotation 0) corner roles, captured once and never
            // overwritten (not by a later candidate, not by THIS
            // candidate's own rotation retries).
            if !first_attempt_seen {
                first_attempt_seen = true;
                first_alignment = result.alignment_trace.clone();
                first_sample_regions = result.sample_regions_trace.clone();
            }
            // Fix A: the most recently DECODED attempt — mirrors the
            // pre-fix `bits` contract, now also driving `alignment`/
            // `sample_regions` so all three agree on the same candidate
            // whenever anything decodes this frame (see `DecodeTraceData`).
            if decoded {
                any_decoded = true;
                decoded_alignment = result.alignment_trace.clone();
                decoded_sample_regions = result.sample_regions_trace.clone();
                decoded_bits = result.bits_trace.clone();
                decoded_refine = result.refine_trace.clone();
            }
            if let Some(code) = result.code {
                for &i in &rotated.finder_indices {
                    consumed[i] = true;
                }
                codes.push(code);
            }
            attempts.push(result.trace);
            if decoded {
                break;
            }
        }
    }

    // Fix A's selection rule: a decoded candidate's own geometry wins
    // outright (all three fields then describe that SAME candidate); with
    // no decode anywhere this frame, fall back to the first attempt's
    // canonical geometry rather than whatever the last (possibly a
    // wrong-role rotation retry pointing away from the code) attempt
    // happened to leave behind — see `DecodeTraceData`'s doc.
    let trace_data = if any_decoded {
        DecodeTraceData {
            alignment: decoded_alignment,
            sample_regions: decoded_sample_regions,
            bits: decoded_bits,
            refine: decoded_refine,
        }
    } else {
        DecodeTraceData {
            alignment: first_alignment,
            sample_regions: first_sample_regions,
            bits: None,
            refine: None,
        }
    };
    (codes, attempts, timings, trace_data)
}

/// Frame-level Plan 4 Task 6 trace data [`decode_candidates`] hands back
/// alongside its usual `(codes, attempts, timings)` — kept as a separate
/// return element (not folded into `DecodeTimings`, which is purely
/// numeric) so `scanner.rs`'s `detect_with` can feed each field straight
/// into the matching `Trace::record_*` call.
///
/// # Plan 4B Fix A: selection rule (trace honesty)
/// Investigation finding: the pre-fix rule recorded `alignment`/
/// `sample_regions` from the LAST attempted candidate, which — after a
/// failed candidate's corner-role rotation retries (see
/// [`decode_candidates`]'s rotation loop) — is frequently a wrong-role
/// attempt whose sampled parallelogram points AWAY from the real code,
/// misleading the debug UI on every failed frame. The fix:
///
/// - **Any decode succeeded this frame**: all three fields come from THAT
///   decoded attempt (the most recent one, if more than one code decoded) —
///   `alignment`/`sample_regions` now always agree with `bits` and with
///   `Detections.codes`' own last entry, never an unrelated candidate.
/// - **Nothing decoded**: `alignment`/`sample_regions` come from the FIRST
///   attempt run this frame — rotation 0 (canonical, unrotated corner
///   roles) of the first (lowest-`snap_error`) candidate attempted, NEVER
///   a rotation retry and NEVER a later candidate — "the geometry a person
///   debugging wants to see" even when it's sparse (e.g. v1's legitimately
///   empty `alignment`) rather than whichever later attempt happened to
///   run last. `bits` stays `None` (nothing decoded, so there is no
///   sampled matrix to show).
pub(crate) struct DecodeTraceData {
    /// See this struct's "Plan 4B Fix A" doc section for the exact
    /// selection rule.
    pub alignment: Vec<AlignmentTraceEntry>,
    /// Same selection rule as `alignment` (always the same candidate as
    /// it).
    pub sample_regions: Vec<SampleRegionTrace>,
    /// The most recently decoded candidate's sampled bit matrix, `None` if
    /// nothing decoded this frame — unchanged from the pre-Fix-A contract.
    pub bits: Option<BitsTrace>,
    /// The most recently decoded candidate's subpixel corner refinement
    /// diagnostics (Plan 5 Task 3) — same selection rule as `bits`: `None`
    /// if nothing decoded this frame, refinement was disabled, or
    /// refinement ran but produced fewer than 2 valid edge lines.
    pub refine: Option<RefineTrace>,
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
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t), false, None, false);
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
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t), false, None, false);
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

        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), &triplets, false, None, false);
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
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t), false, None, false);
        assert!(codes.is_empty());
        // One attempt per corner-role rotation (all fail), each rejected
        // by the OOB gate before decode.
        assert_eq!(attempts.len(), 3);
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
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t), false, None, false);
        assert!(codes.is_empty());
        // One attempt per corner-role rotation, all failing cleanly. Each
        // costs exactly 1 round (confirmed via `rounds`: no alignment
        // pattern at v1 makes `needs_refined_br` true, but on this flat
        // 128-gray image `refine_fourth_corner` finds no usable edge
        // transition to probe at all, so the Task 5b retry loop never
        // actually runs a second `run_round` call) — the companion
        // round-budget test below relies on this same cost.
        assert_eq!(attempts.len(), 3);
        assert!(attempts.iter().all(|a| a.rounds.len() == 1), "{attempts:?}");
        assert!(attempts.iter().all(|a| a.outcome != "decoded"));
    }

    #[test]
    fn round_budget_limits_total_rounds_even_with_many_disjoint_candidates() {
        // MAX_DECODE_ROUNDS + a handful more disjoint (no shared finders)
        // bogus triplets over a flat image: dedup keeps all of them (no
        // finder overlap), but the round budget must stop total decode
        // work at MAX_DECODE_ROUNDS regardless of how many more candidates
        // are supplied (Plan 5 Task 6: this test used to size `total` off
        // `MAX_DECODE_ATTEMPTS` and assert `attempts.len() ==
        // MAX_DECODE_ATTEMPTS`; the cap is now denominated in rounds, not
        // attempts — see that constant's doc). This exact geometry costs
        // 1 round/attempt (pinned by
        // `garbage_triplet_fails_cleanly_without_panicking` just above),
        // so the round budget and the attempt count coincide here,
        // letting this test still assert an exact number.
        let img = vec![128u8; 64 * 64];
        let view = LumaView::new(&img, 64, 64, 64).unwrap();
        let grid = TileGrid::build(&view);
        let extra = 5;
        let total = MAX_DECODE_ROUNDS + extra;
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
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(total * 3), &triplets, false, None, false);
        assert!(codes.is_empty());
        assert_eq!(attempts.len(), MAX_DECODE_ROUNDS);
        let rounds_run: usize = attempts.iter().map(|a| a.rounds.len().max(1)).sum();
        assert_eq!(rounds_run, MAX_DECODE_ROUNDS, "budget must bind exactly at 1 round/attempt on this geometry");
    }

    // --- Task 5b: image-derived 4th corner + corner-role rotation ---

    /// The keystone (trapezoid) quad `sample.rs`'s own AP-superiority test
    /// established: top edge narrowed to `narrow` of the bottom edge's
    /// width. At `narrow = 0.90` a v1's finder-only parallelogram
    /// reconstruction measurably fails bit-for-bit (recorded in
    /// `sample.rs`'s test docs since Task 4), making it exactly the
    /// synthetic case Task 5b's refinement must rescue.
    fn keystone_quad(dim: usize, scale: f64, narrow: f64, img_side: usize) -> [[f64; 2]; 4] {
        let side = dim as f64 * scale;
        let margin = (img_side as f64 - side) / 2.0;
        let cx = img_side as f64 / 2.0;
        let top_half = side / 2.0 * narrow;
        let bottom_half = side / 2.0;
        let (y0, y1) = (margin, margin + side);
        [
            [cx - top_half, y0],
            [cx + top_half, y0],
            [cx + bottom_half, y1],
            [cx - bottom_half, y1],
        ]
    }

    #[test]
    fn refined_corner_rescues_v1_keystone_that_defeats_the_parallelogram() {
        let version = 1i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let img_side = ((dim as f64 * scale) * std::f64::consts::SQRT_2 + 40.0).ceil() as usize;
        let quad = keystone_quad(dim, scale, 0.90, img_side);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"KEYSTONE", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        assert_eq!(code.width(), dim);
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let t = triplet_from_transform(&transform, dim, dim as u32);
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t), false, None, false);
        assert_eq!(codes.len(), 1, "attempts: {attempts:?}");
        assert_eq!(codes[0].payload, "KEYSTONE");
        // The decode must have come through the image-derived corner — the
        // whole point of Task 5b — not the parallelogram fallback (which
        // this keystone is pinned to defeat).
        let decoded_attempt = attempts.iter().find(|a| a.outcome.starts_with("decoded")).unwrap();
        assert!(
            decoded_attempt.refined_corner,
            "expected the refined-corner path, got: {decoded_attempt:?}"
        );
        // Task 6's carried-item fix: the FAILED parallelogram round (this
        // keystone is pinned to defeat it) must stay visible in `rounds`
        // instead of being silently overwritten by whichever refined round
        // actually decoded — this is exactly the "failed refined rounds are
        // invisible" gap the plan called out.
        assert!(
            decoded_attempt.rounds.len() >= 2,
            "expected the failed parallelogram round plus at least one refined round, got: {:?}",
            decoded_attempt.rounds
        );
        assert!(
            decoded_attempt.rounds[0].starts_with("parallelogram:") && decoded_attempt.rounds[0] != "parallelogram:decoded",
            "expected the first round to be a FAILED parallelogram attempt, got: {:?}",
            decoded_attempt.rounds
        );
        assert!(
            decoded_attempt.rounds.last().unwrap().ends_with(":decoded"),
            "expected the last recorded round to be the one that actually decoded, got: {:?}",
            decoded_attempt.rounds
        );
    }

    // --- Final-review fix: per-region corners for multi-region decodes ---

    #[test]
    fn multi_region_decode_reports_per_region_anchored_corners_under_keystone() {
        // Pins the final-review fix to `DecodedCode::corners`: a
        // multi-region decode (v7+, real interior alignment patterns
        // located and tiled — see `sample::sample_grid`) must report each
        // corner from the region whose `module_rect` touches it, not from
        // the whole-grid finder-only parallelogram transform. The BR
        // corner is the one that actually moves: under this same
        // narrow=0.90 keystone `sample.rs`'s own AP-superiority test
        // already proves the AP-anchored regions sample bit-for-bit
        // correctly for v7 — this test instead checks that the CORNER
        // OUTPUT reflects that same AP-anchored evidence, comparing
        // against the true render homography (not a parallelogram
        // approximation of it).
        let version = 7i16;
        let dim = 17 + 4 * version as usize; // 45
        let scale = 4.0;
        let img_side = ((dim as f64 * scale) * std::f64::consts::SQRT_2 + 40.0).ceil() as usize;
        let quad = keystone_quad(dim, scale, 0.90, img_side);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"MULTIREGIONKEYSTONE", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        assert_eq!(code.width(), dim);
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let t = triplet_from_transform(&transform, dim, dim as u32);
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t), false, None, false);
        assert_eq!(codes.len(), 1, "attempts: {attempts:?}");
        assert_eq!(codes[0].payload, "MULTIREGIONKEYSTONE");

        // Ground truth: the render homography itself (`transform`) is the
        // TRUE perspective transform the image was rendered through, not
        // a parallelogram approximation of it — its own unit-square
        // corners are exactly the module-grid corners' true image
        // positions, the same convention `DecodedCode::corners` documents
        // (`(0,0)`, `(1,0)`, `(1,1)`, `(0,1)` -> TL, TR, BR, BL).
        let ground_truth = [
            transform.map(0.0, 0.0),
            transform.map(1.0, 0.0),
            transform.map(1.0, 1.0),
            transform.map(0.0, 1.0),
        ];
        let tol = (2.0f64).max(scale); // max(2.0px, 1 module in px)
        let labels = ["TL", "TR", "BR", "BL"];
        for (i, label) in labels.iter().enumerate() {
            let got = codes[0].corners[i];
            let want = ground_truth[i];
            let err = ((got[0] - want[0]).powi(2) + (got[1] - want[1]).powi(2)).sqrt();
            assert!(
                err <= tol,
                "{label} corner error {err:.3}px exceeds tolerance {tol:.3}px: got {got:?}, want {want:?}",
            );
        }
    }

    #[test]
    fn rotated_corner_roles_still_decode_via_rotation_retry() {
        // A clean v1 render, but the triplet handed in with its corner
        // roles cyclically rotated (tl<-tr<-bl<-tl) — the mis-assignment
        // `triplet::try_group`'s most-perpendicular-corner heuristic
        // produces under steep perspective (observed on the `tilt45_04`
        // fixture). The rotation retry in `decode_candidates` must recover
        // it, at the cost of extra traced attempts for the failed
        // rotation(s).
        let version = 1i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"ROTROLE", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let good = triplet_from_transform(&transform, dim, dim as u32);
        let mut rotated = good;
        // One cyclic rotation off: what try_group would emit if it picked
        // the wrong corner as TL (orientation-preserving, so still a
        // canonically valid triplet).
        rotated.tl = good.tr;
        rotated.tr = good.bl;
        rotated.bl = good.tl;
        rotated.finder_indices = [1, 2, 0];

        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&rotated), false, None, false);
        assert_eq!(codes.len(), 1, "attempts: {attempts:?}");
        assert_eq!(codes[0].payload, "ROTROLE");
        assert!(
            attempts.len() > 1,
            "the mis-assigned rotation must have produced at least one failed attempt first"
        );
        assert_eq!(attempts.last().unwrap().outcome, "decoded");
    }

    // --- Plan 4B Fix A: trace-selection honesty ---

    #[test]
    fn total_failure_trace_uses_first_candidates_canonical_attempt_not_last_or_retry() {
        // No decode succeeds anywhere in this frame (garbage geometry over
        // a flat image) — Fix A's "otherwise" branch must apply:
        // `DecodeTraceData` comes from the FIRST attempt (rotation 0,
        // canonical/unrotated corner roles) of the FIRST candidate actually
        // attempted (lowest `snap_error`), never a later candidate and
        // never one of that candidate's own rotation retries — the
        // root-cause finding this fix addresses ("last attempted" could be
        // an unrelated wrong-role rotation retry pointing away from the
        // real code, misleading the debug UI on every failed frame).
        let img = vec![128u8; 64 * 64];
        let view = LumaView::new(&img, 64, 64, 64).unwrap();
        let grid = TileGrid::build(&view);

        // Two garbage v1 candidates with distinct geometry (so their
        // sample-region quads are distinguishable) and disjoint finder
        // indices (so proximity dedup keeps both, and both get attempted).
        let a = TripletCandidate {
            tl: [10.0, 10.0], tr: [50.0, 10.0], bl: [10.0, 50.0],
            module: 4.0, dimension: 21, snap_error: 0.1, inverted: false,
            finder_indices: [0, 1, 2],
        };
        let b = TripletCandidate {
            tl: [5.0, 5.0], tr: [55.0, 8.0], bl: [8.0, 55.0],
            module: 4.0, dimension: 21, snap_error: 0.2, inverted: false,
            finder_indices: [3, 4, 5],
        };

        // Ground truth: exactly what `decode_candidates`'s own first
        // attempt (candidate A, rotation 0 — i.e. `a` unrotated) produces,
        // computed directly via `attempt_candidate` so this test doesn't
        // need to hand-derive the sampled quad geometry itself.
        let mut timings = DecodeTimings { version_ns: 0, alignment_ns: 0, sample_decode_ns: 0, refine_ns: 0 };
        let expected = attempt_candidate(&view, &grid, 0, &a, &mut timings, true, None, false);
        assert!(expected.code.is_none(), "test setup: candidate A must not decode");
        assert!(
            !expected.sample_regions_trace.is_empty(),
            "test setup: a v1 candidate must still produce one whole-grid sample region"
        );

        let (codes, attempts, _timings, trace_data) =
            decode_candidates(&view, &grid, &dummy_finders(6), &[a, b], true, None, false);
        assert!(codes.is_empty(), "test setup: nothing in this frame should decode");
        assert!(
            attempts.len() > 1,
            "expected multiple attempts (A's rotation retries and/or B): {attempts:?}"
        );

        assert_eq!(trace_data.sample_regions.len(), expected.sample_regions_trace.len());
        for (got, want) in trace_data.sample_regions.iter().zip(expected.sample_regions_trace.iter()) {
            assert_eq!(got.module_rect, want.module_rect);
            for k in 0..4 {
                assert!(
                    (got.quad[k][0] - want.quad[k][0]).abs() < 1e-9
                        && (got.quad[k][1] - want.quad[k][1]).abs() < 1e-9,
                    "sample_regions corner {k} mismatch: got {:?}, want {:?}",
                    got.quad[k], want.quad[k],
                );
            }
        }
        assert!(trace_data.bits.is_none(), "nothing decoded, so bits must stay None");
    }

    #[test]
    fn decoded_frame_trace_matches_the_decoding_attempt_not_a_later_failed_one() {
        // Fix A's "any decode succeeds" branch: a real decode followed by a
        // later, unrelated failed candidate must NOT blank the decoded
        // candidate's alignment/sample_regions/bits back down — all three
        // must still describe the code that actually decoded.
        let version = 1i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"FIXAORDER", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let good = triplet_from_transform(&transform, dim, dim as u32);
        // A second, garbage candidate with disjoint finder indices and a
        // lower snap_error than... no: attempted in dedup (snap_error)
        // order, so give it a HIGHER snap_error to guarantee it's attempted
        // strictly after the real decode.
        let mut garbage = good;
        garbage.tl = [1.0, 1.0];
        garbage.tr = [3.0, 1.0];
        garbage.bl = [1.0, 3.0];
        garbage.snap_error = 1.0;
        garbage.finder_indices = [3, 4, 5];

        let (codes, attempts, _timings, trace_data) = decode_candidates(
            &view, &grid, &dummy_finders(6), &[good, garbage], true, None, false,
        );
        assert_eq!(codes.len(), 1, "attempts: {attempts:?}");
        assert_eq!(codes[0].payload, "FIXAORDER");
        assert!(
            attempts.last().unwrap().outcome != "decoded",
            "test setup: the LAST attempt run must be the garbage candidate's failure: {attempts:?}"
        );
        // v1: no alignment patterns, so an empty vec is the correct
        // "decoded" shape too — the real assertion is `bits`, which can
        // only be populated by the actual decode.
        assert_eq!(trace_data.sample_regions.len(), 1, "v1 samples through one whole-grid region");
        let bits = trace_data.bits.expect("the frame's one decode must populate bits");
        assert_eq!(bits.dim, dim as u32);
    }

    #[test]
    fn later_decode_overrides_earlier_failed_candidates_first_attempt_trace() {
        // The mirror-image Fix A scenario of the two tests above: the
        // FIRST candidate attempted (lowest snap_error) is garbage and
        // fails ALL THREE corner-role rotations; a LATER candidate then
        // decodes. The recorded regions/bits must be the DECODED
        // candidate's — the "any decode wins" branch must override the
        // already-captured first attempt's canonical geometry, not keep it.
        let version = 1i16;
        let dim = 17 + 4 * version as usize;
        let scale = 4.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"LATERWINS", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        let img = render_module_grid_transformed(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let mut good = triplet_from_transform(&transform, dim, dim as u32);
        good.snap_error = 0.5; // attempted SECOND
        good.finder_indices = [3, 4, 5];
        // Garbage geometry in a corner of the image away from the real
        // code's finder centers, with the LOWER snap_error so dedup order
        // attempts it (and its rotation retries) first.
        let garbage = TripletCandidate {
            tl: [2.0, 2.0],
            tr: [14.0, 2.0],
            bl: [2.0, 14.0],
            module: 4.0,
            dimension: 21,
            snap_error: 0.05,
            inverted: false,
            finder_indices: [0, 1, 2],
        };

        // Ground truth: the decoded attempt's own trace, computed directly
        // (same technique as the total-failure test above).
        let mut timings = DecodeTimings { version_ns: 0, alignment_ns: 0, sample_decode_ns: 0, refine_ns: 0 };
        let expected = attempt_candidate(&view, &grid, 1, &good, &mut timings, true, None, false);
        assert!(expected.code.is_some(), "test setup: the good candidate must decode on its own");

        let (codes, attempts, _timings, trace_data) =
            decode_candidates(&view, &grid, &dummy_finders(6), &[garbage, good], true, None, false);
        assert_eq!(codes.len(), 1, "attempts: {attempts:?}");
        assert_eq!(codes[0].payload, "LATERWINS");
        assert!(
            attempts.len() >= 4,
            "test setup: garbage must burn all 3 rotations before the decode: {attempts:?}"
        );
        assert!(
            attempts[..3].iter().all(|a| a.outcome != "decoded"),
            "test setup: the first three attempts must be the garbage candidate failing: {attempts:?}"
        );

        // The recorded regions must be the DECODED candidate's, not the
        // garbage first attempt's canonical geometry.
        assert_eq!(trace_data.sample_regions.len(), expected.sample_regions_trace.len());
        for (got, want) in trace_data.sample_regions.iter().zip(expected.sample_regions_trace.iter()) {
            assert_eq!(got.module_rect, want.module_rect);
            for k in 0..4 {
                assert!(
                    (got.quad[k][0] - want.quad[k][0]).abs() < 1e-9
                        && (got.quad[k][1] - want.quad[k][1]).abs() < 1e-9,
                    "sample_regions corner {k} mismatch: got {:?}, want {:?} (garbage geometry leaked?)",
                    got.quad[k], want.quad[k],
                );
            }
        }
        let bits = trace_data.bits.expect("the decode must populate bits");
        let want_bits = expected.bits_trace.expect("expected attempt decoded, so it carries bits");
        assert_eq!(bits.dim, want_bits.dim);
        assert_eq!(bits.words, want_bits.words, "bits must be the decoded candidate's matrix");
    }

    // --- `DecodedCode::corner_refined` plumbing (final-review carried item) ---

    #[test]
    fn corner_refined_flags_is_all_false_when_refinement_did_not_run() {
        assert_eq!(corner_refined_flags(None), [false; 4]);
    }

    #[test]
    fn corner_refined_flags_plumbs_a_hand_built_partial_array_unchanged() {
        // Hand-built directly (no image, no real `refine_corners` call) with
        // a PARTIAL flags array — two corners refined, two fell back to the
        // coarse position — so this can't pass by accident the way an
        // always-true or always-`[false; 4]` result would: it specifically
        // pins that `corner_refined_flags` copies `RefinedCorners`'s array
        // through verbatim rather than, say, collapsing it to "any true" or
        // re-deriving it some other way.
        let partial = [true, false, true, false];
        let rc = RefinedCorners {
            corners: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            edge_stats: [crate::refine::EdgeStat::default(); 4],
            corner_refined: partial,
        };
        assert_eq!(corner_refined_flags(Some(&rc)), partial);
    }

    #[test]
    fn decode_candidates_populates_corner_refined_all_true_on_a_clean_frontal_render() {
        // Golden-fixture check (review-flagged gap: `corner_refined` was
        // otherwise never asserted on ANY decoded code, real or synthetic —
        // only `refined_corners.is_some()` was, which as `refine_round`'s
        // per-corner fallback shows is not the same claim). A clean,
        // antialiased, frontal v1 render is the easy case every edge is
        // expected to fit in: this pins the happy path all the way through
        // `attempt_candidate` -> `corner_refined_flags` -> `DecodedCode`,
        // independent of the hand-built unit test above (which never
        // exercises real image sampling or `refine_round`'s own is-this-
        // corner-refined decision, only the copy-through afterward).
        let version = 1i16;
        let dim = 17 + 4 * version as usize;
        let scale = 8.0;
        let (quad, img_side) = axis_aligned_quad(dim, scale, 4.0);
        let transform = PerspectiveTransform::square_to_quad(quad).unwrap();
        let code = qrcode::QrCode::with_version(
            b"CORNERFLAGS", qrcode::Version::Normal(version), qrcode::EcLevel::M,
        )
        .unwrap();
        assert_eq!(code.width(), dim);
        let img = crate::testpaint::render_module_grid_transformed_antialiased(
            dim, |x, y| code[(x, y)] == qrcode::Color::Dark, 25, 235, &transform, img_side, img_side, 8,
        );
        let view = LumaView::new(&img, img_side, img_side, img_side).unwrap();
        let grid = TileGrid::build(&view);

        let t = triplet_from_transform(&transform, dim, dim as u32);
        let (codes, attempts, _timings, ..) =
            decode_candidates(&view, &grid, &dummy_finders(3), std::slice::from_ref(&t), false, None, true);
        assert_eq!(codes.len(), 1, "attempts: {attempts:?}");
        assert_eq!(codes[0].payload, "CORNERFLAGS");
        assert!(
            codes[0].refined_corners.is_some(),
            "test setup: refinement must succeed on this clean render"
        );
        assert_eq!(
            codes[0].corner_refined,
            [true, true, true, true],
            "expected every corner refined on a clean frontal render, got {:?}",
            codes[0].corner_refined
        );
    }
}
