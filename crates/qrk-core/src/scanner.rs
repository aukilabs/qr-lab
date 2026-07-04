//! Orchestrates the detection + decode stages — tile thresholding, finder
//! scanning, triplet grouping, then decode orchestration/arbitration —
//! behind one `detect`/`detect_traced` entry point, and measures per-stage
//! wall time with [`StageClock`]. The first three stages are measured from
//! here, not inside the stage functions themselves, so they stay
//! measurement-free and reusable standalone; `decode.rs`'s own three
//! sub-stages (version cross-check, alignment, sample+decode) are
//! interleaved per candidate rather than run as whole contiguous phases, so
//! `decode_candidates` accumulates their `StageClock` totals itself and
//! hands them back for `StageTimings` — see `decode::DecodeTimings`'s doc
//! for why that's a deliberate deviation from this file's own convention.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::decode::{decode_candidates, DecodedCode};
use crate::finder::{find_finders, FinderCandidate};
use crate::sample::SourceView;
use crate::tiles::TileGrid;
use crate::trace::Trace;
use crate::triplet::{group_triplets, TripletCandidate};
use crate::LumaView;

/// Per-stage wall-clock time in nanoseconds, measured around each of the
/// `detect_traced` stage calls.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StageTimings {
    pub tiles_ns: u64,
    pub finders_ns: u64,
    pub triplets_ns: u64,
    /// Timing cross-check + version-info-bits reading, summed across every
    /// decode attempt in the frame.
    pub version_ns: u64,
    /// Alignment-pattern location, summed across every decode attempt.
    pub alignment_ns: u64,
    /// Grid sampling + `decode_bits`, summed across every decode attempt.
    pub sample_decode_ns: u64,
    /// Subpixel corner refinement (Plan 5 Task 3), summed across every
    /// decoded candidate this frame — `0` whenever `ScanOptions::refine`
    /// is `false` (the default, including every `detect`/`detect_with`/
    /// `detect_traced` call, which never enables it).
    pub refine_ns: u64,
}

/// One frame's detection result: every finder candidate, every grouped
/// triplet, every successfully decoded code, and the timings that produced
/// them.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Detections {
    pub finders: Vec<FinderCandidate>,
    pub triplets: Vec<TripletCandidate>,
    pub codes: Vec<DecodedCode>,
    pub timings: StageTimings,
    /// Working-resolution ÷ source-resolution scale (Plan 5 Task 1):
    /// `working_dim / source_dim`, always `<= 1` — the working view IS the
    /// source, or a downscaled copy of it, never an upscaled one. `1.0` for
    /// every `detect`/`detect_with`/`detect_traced` call (they never
    /// downscale) and for any `scan`/`scan_traced` call whose
    /// `ScanOptions::max_working_dim` didn't require a downscale.
    ///
    /// Direction, stated unambiguously since "scale" alone is ambiguous:
    /// every OTHER field on this struct (and `DecodedCode::corners`) is in
    /// WORKING px. To convert one of those coordinates to SOURCE px,
    /// DIVIDE by this field: `source_px = working_px / source_scale`.
    /// Equivalently, `working_px = source_px * source_scale`.
    ///
    /// Computed from the WIDTH axis specifically (`working_width /
    /// source_width`) — `scan`'s downscale rounds width and height
    /// independently (see `downscale_luma`'s doc comment), so for a
    /// non-square image the height axis's own ratio can differ from this
    /// by up to half a source pixel of rounding. This is the same
    /// approximation the debug UI's pre-Plan-5 `workingScaleFor` already
    /// made (`scanWidth / sourceWidth`, width only) — carried forward
    /// rather than introduced here.
    ///
    /// Note this width-pinning is a PUBLIC-scalar approximation only:
    /// internally, `scan`'s source-resolution module sampling (Plan 5
    /// Task 2) lifts working→source through exact PER-AXIS ratios (see
    /// `sample::SourceView`), so sampling precision does not inherit this
    /// field's height-axis rounding slack.
    pub source_scale: f64,
}

/// Per-stage wall-clock timer behind [`StageTimings`].
///
/// On `wasm32` targets `std::time::Instant::now()` compiles but panics at
/// runtime ("time not implemented on this platform" on
/// wasm32-unknown-unknown), so there the clock instead reads
/// `js_sys::Date::now()` — a millisecond-resolution `f64` timestamp backed
/// by JS `Date.now()`, available in both window and worker global scopes
/// without reaching for `web-sys::Performance` (which would need
/// target-specific plumbing to fetch the right global in each context).
/// This keeps the wasm dependency surface to one tiny crate (`js-sys`,
/// already pulled in transitively by `wasm-bindgen`) at the cost of
/// precision: `Date.now()` only resolves to ~1ms, so fast stages (the
/// triplets stage in particular, often sub-millisecond) commonly report
/// 0ns on wasm even though real work happened. Recorded decision: if
/// µs-scale precision is later needed, swap this arm to
/// `web-sys::Performance::now()` (sub-ms resolution) — that requires
/// resolving the global scope in both window and worker contexts, which is
/// why it wasn't the first choice here. Everywhere else this wraps
/// `Instant`.
#[derive(Clone, Copy, Debug)]
pub struct StageClock {
    #[cfg(not(target_arch = "wasm32"))]
    start: Instant,
    #[cfg(target_arch = "wasm32")]
    start_ms: f64,
}

impl StageClock {
    /// Start timing a stage.
    #[must_use]
    pub fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: Instant::now(),
            #[cfg(target_arch = "wasm32")]
            start_ms: js_sys::Date::now(),
        }
    }

    /// Nanoseconds since [`StageClock::start`] (ms-resolution on `wasm32` —
    /// see the [`StageClock`] doc comment).
    #[must_use]
    pub fn elapsed_ns(self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_nanos() as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            let delta_ms = js_sys::Date::now() - self.start_ms;
            // Guard against a negative delta (clock adjustments, or a
            // same-millisecond read landing a float epsilon below start) —
            // elapsed time is never negative, so clamp to 0 rather than
            // wrapping through `as u64`.
            if delta_ms <= 0.0 {
                0
            } else {
                (delta_ms * 1e6) as u64
            }
        }
    }
}

/// The single detection orchestration every entry point (`detect`,
/// `detect_traced`, and `qrk-wasm`'s `scan_rgba`) funnels through: runs
/// the three stages in order and, when `trace` is `Some`, records each
/// stage's output into it. When `trace` is `None` no `record_*` call is
/// made at all — not merely a cheap no-op — so a caller that does not
/// want a trace never pays `Trace::record_finders`/`record_triplets`'s
/// `to_vec()` clone cost even when this crate is compiled with the
/// `debug-trace` feature (as `qrk-wasm` does, to keep the feature
/// available for its own optional trace output without taxing the
/// common no-trace path).
///
/// Thin wrapper over [`detect_with_source`] with `source: None, refine:
/// false` — kept as its own function (signature unchanged from before Plan
/// 5 Task 2) so every existing `detect`/`detect_with`/`detect_traced`
/// caller is unaffected by the source-resolution sampling or subpixel
/// refinement plumbing.
pub fn detect_with(view: &LumaView, trace: Option<&mut Trace>) -> Detections {
    detect_with_source(view, None, trace, false)
}

/// [`detect_with`]'s real body, additionally threading an optional SOURCE
/// view through to [`decode_candidates`] (Plan 5 Task 2): `scan.rs`'s
/// `scan_with` calls this directly (not `detect_with`) with
/// `Some(SourceView { view: source, sx, sy })` whenever it downscaled `view`
/// from `source`, so module sampling
/// can read the SOURCE image instead of the (lossier, at ~2 working
/// px/module) working `view` — see `sample::sample_grid`'s doc for exactly
/// what changes. Every other stage below (tiles/finders/triplets, plus
/// decode's own timing-check/version-bits/alignment sub-stages) still runs
/// on `view` — the WORKING resolution — regardless of `source`, per the
/// plan's scope note: only module sampling moves to source resolution this
/// task. `pub(crate)`, not `pub`: this is `scan.rs`'s own internal seam, not
/// part of the crate's public entry-point surface (`scan`/`scan_traced`
/// already are). `refine` (Plan 5 Task 3) enables subpixel corner
/// refinement on every decoded candidate — against `source.view` when
/// `source` is `Some`, else against `view` itself (source == working, no
/// downscale happened — refinement still runs; see `decode::attempt_candidate`'s
/// doc).
pub(crate) fn detect_with_source(
    view: &LumaView,
    source: Option<SourceView>,
    mut trace: Option<&mut Trace>,
    refine: bool,
) -> Detections {
    let tiles_clock = StageClock::start();
    let grid = TileGrid::build(view);
    let tiles_ns = tiles_clock.elapsed_ns();
    if let Some(t) = &mut trace {
        t.record_tiles(&grid);
    }

    let finders_clock = StageClock::start();
    let finders = find_finders(view, &grid);
    let finders_ns = finders_clock.elapsed_ns();
    if let Some(t) = &mut trace {
        t.record_finders(&finders);
    }

    let triplets_clock = StageClock::start();
    let triplets = group_triplets(view, &grid, &finders);
    let triplets_ns = triplets_clock.elapsed_ns();
    if let Some(t) = &mut trace {
        t.record_triplets(&triplets);
    }

    // `decode_candidates` accumulates its own three-way stage-timing split
    // (see the module doc) since its sub-stages are interleaved per
    // candidate rather than run as whole phases; `attempts` and the Task 6
    // trace data (`decode_trace`) are recorded the same way
    // `record_finders`/`record_triplets` are above — a copy only when a
    // trace was actually requested.
    let (codes, attempts, decode_timings, decode_trace) =
        decode_candidates(view, &grid, &finders, &triplets, trace.is_some(), source, refine);
    if let Some(t) = &mut trace {
        t.record_attempts(&attempts);
        t.record_alignment(&decode_trace.alignment);
        t.record_sample_regions(&decode_trace.sample_regions);
        if let Some(bits) = decode_trace.bits {
            t.record_bits(bits);
        }
        if let Some(refine_trace) = decode_trace.refine {
            t.record_refine(refine_trace);
        }
    }

    Detections {
        finders,
        triplets,
        codes,
        timings: StageTimings {
            tiles_ns,
            finders_ns,
            triplets_ns,
            version_ns: decode_timings.version_ns,
            alignment_ns: decode_timings.alignment_ns,
            sample_decode_ns: decode_timings.sample_decode_ns,
            refine_ns: decode_timings.refine_ns,
        },
        // `detect`/`detect_with`/`detect_traced` never downscale — `view`
        // IS the working view, so working == source. `scan`/`scan_traced`
        // (`scan.rs`) call this same function on their own working view,
        // then overwrite this with the real ratio when a downscale
        // happened — see `scan_with`.
        source_scale: 1.0,
    }
}

/// Run the full detection pipeline on `view`, discarding trace data.
pub fn detect(view: &LumaView) -> Detections {
    detect_with(view, None)
}

/// Run the full detection pipeline on `view`, recording each stage's
/// output into `trace` (a no-op without the `debug-trace` feature).
pub fn detect_traced(view: &LumaView, trace: &mut Trace) -> Detections {
    detect_with(view, Some(trace))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LumaView;

    #[test]
    fn detect_on_flat_image_is_empty_and_fast_path() {
        let d = vec![128u8; 320 * 240];
        let view = LumaView::new(&d, 320, 240, 320).unwrap();
        let det = detect(&view);
        assert!(det.finders.is_empty());
        assert!(det.triplets.is_empty());
    }

    #[cfg(feature = "debug-trace")]
    #[test]
    fn trace_records_stages() {
        let d = vec![128u8; 64 * 64];
        let view = LumaView::new(&d, 64, 64, 64).unwrap();
        let mut tr = Trace::new();
        let _ = detect_traced(&view, &mut tr);
        let tiles = tr.tiles.as_ref().expect("tiles recorded");
        assert_eq!(tiles.thresholds.len(), 4 * 4);
    }
}
