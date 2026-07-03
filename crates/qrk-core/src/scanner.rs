//! Orchestrates the three detection stages — tile thresholding, finder
//! scanning, triplet grouping — behind one `detect`/`detect_traced` entry
//! point, and measures per-stage wall time with `std::time::Instant`
//! (here, not inside the stage functions themselves, so the stages stay
//! measurement-free and reusable standalone).

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

/// Run the full detection pipeline on `view`, discarding trace data.
pub fn detect(view: &LumaView) -> Detections {
    detect_traced(view, &mut Trace::new())
}

/// Run the full detection pipeline on `view`, recording each stage's
/// output into `trace` (a no-op without the `debug-trace` feature).
pub fn detect_traced(view: &LumaView, trace: &mut Trace) -> Detections {
    let tiles_start = Instant::now();
    let grid = TileGrid::build(view);
    let tiles_ns = tiles_start.elapsed().as_nanos() as u64;
    trace.record_tiles(&grid);

    let finders_start = Instant::now();
    let finders = find_finders(view, &grid);
    let finders_ns = finders_start.elapsed().as_nanos() as u64;
    trace.record_finders(&finders);

    let triplets_start = Instant::now();
    let triplets = group_triplets(view, &grid, &finders);
    let triplets_ns = triplets_start.elapsed().as_nanos() as u64;
    trace.record_triplets(&triplets);

    Detections {
        finders,
        triplets,
        timings: StageTimings { tiles_ns, finders_ns, triplets_ns },
    }
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
