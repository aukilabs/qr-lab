//! Orchestrates the three detection stages — tile thresholding, finder
//! scanning, triplet grouping — behind one `detect`/`detect_traced` entry
//! point, and measures per-stage wall time with [`StageClock`] (here, not
//! inside the stage functions themselves, so the stages stay
//! measurement-free and reusable standalone).

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::finder::{find_finders, FinderCandidate};
use crate::tiles::TileGrid;
use crate::trace::Trace;
use crate::triplet::{group_triplets, TripletCandidate};
use crate::LumaView;

/// Per-stage wall-clock time in nanoseconds, measured around each of the
/// three `detect_traced` stage calls.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StageTimings {
    pub tiles_ns: u64,
    pub finders_ns: u64,
    pub triplets_ns: u64,
}

/// One frame's detection result: every finder candidate, every grouped
/// triplet, and the timings that produced them.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Detections {
    pub finders: Vec<FinderCandidate>,
    pub triplets: Vec<TripletCandidate>,
    pub timings: StageTimings,
}

/// Per-stage wall-clock timer behind [`StageTimings`].
///
/// On `wasm32` targets `std::time::Instant::now()` compiles but panics at
/// runtime ("time not implemented on this platform" on
/// wasm32-unknown-unknown), so there the clock is a zero-field struct and
/// every stage reports 0 ns — callers measure wall time host-side (the
/// debug UI uses JS `performance.now()`; a follow-up can wire it through
/// web-sys). Everywhere else it wraps `Instant`.
#[derive(Clone, Copy, Debug)]
pub struct StageClock {
    #[cfg(not(target_arch = "wasm32"))]
    start: Instant,
}

impl StageClock {
    /// Start timing a stage.
    #[must_use]
    pub fn start() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: Instant::now(),
        }
    }

    /// Nanoseconds since [`StageClock::start`] (always 0 on `wasm32`).
    #[must_use]
    pub fn elapsed_ns(self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_nanos() as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            0
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
pub fn detect_with(view: &LumaView, mut trace: Option<&mut Trace>) -> Detections {
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

    Detections {
        finders,
        triplets,
        timings: StageTimings { tiles_ns, finders_ns, triplets_ns },
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
