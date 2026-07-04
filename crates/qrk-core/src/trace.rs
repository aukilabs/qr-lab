//! Optional per-stage instrumentation for [`crate::detect_traced`].
//!
//! `Trace` always exists so `detect_traced` compiles with or without the
//! `debug-trace` feature. Without the feature it is a zero-field unit
//! struct with `#[inline]` no-op record methods — the optimizer erases
//! every call site, so `detect` paying for `detect_traced`'s plumbing
//! costs nothing in the default build. With the feature it accumulates
//! the tile grid, finder candidates, triplet candidates, and (Plan 4 Task
//! 6) the decode-stage trace data from one `detect_traced` call for
//! debugging/visualization.
//!
//! The decode-stage trace *types* below (`AlignmentTraceEntry`,
//! `SampleRegionTrace`, `BitsTrace`) are, unlike `TileTrace`, always
//! compiled regardless of the `debug-trace` feature — matching
//! `decode::DecodeAttemptTrace`'s own precedent (that struct has lived
//! ungated since Task 5, since `decode_candidates` always builds its
//! `Vec<DecodeAttemptTrace>` as part of its normal return contract; only
//! `Trace` itself, the feature-gated *container*, decides whether to keep
//! a copy). `decode::decode_candidates` builds all four of these
//! unconditionally too (see its own module doc) — the data is small
//! (bounded by the alignment lattice's `<= 7x7` nodes and `<= 6x6` sample
//! regions per attempt, and a single packed bit matrix per successful
//! decode), so there is no separate runtime "want trace" flag to thread;
//! `detect_with` is simply the one place that decides whether to keep any
//! of it, via these `record_*` calls.

use crate::decode::DecodeAttemptTrace;
use crate::finder::FinderCandidate;
use crate::tiles::TileGrid;
use crate::triplet::TripletCandidate;

/// Per-tile thresholds and skip mask, exported from a built [`TileGrid`]
/// via its `pub(crate)` `to_trace` accessor (the grid keeps its own
/// fields private).
#[cfg(feature = "debug-trace")]
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TileTrace {
    pub tiles_x: usize,
    pub tiles_y: usize,
    pub thresholds: Vec<u8>,
    pub skip: Vec<bool>,
}

/// One alignment-pattern lattice slot's predicted-vs-found position (Plan 4
/// Task 6), for every slot [`crate::alignment::locate_alignment_patterns`]
/// actually searched — the three finder-corner slots are excluded (see
/// `AlignmentGrid::to_trace_entries`): they're never searched, only
/// projected through the provisional transform as a parallelogram-
/// prediction input, so `predicted == found` there would be a no-op that
/// adds nothing to a debug overlay. Recorded for the LAST candidate
/// `decode_candidates` actually attempted in the frame (success or
/// failure) — see [`Trace::record_alignment`].
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AlignmentTraceEntry {
    /// Predicted image-pixel position (parallelogram rule from already-
    /// resolved neighbors, or the provisional transform's direct
    /// projection — see `alignment::predict_position`).
    pub predicted: [f64; 2],
    /// The concentric probe's re-centered position, or `None` when the
    /// probe found no matching dark-light-dark cross-section.
    pub found: Option<[f64; 2]>,
}

/// One sample region's module rectangle and the image-pixel quad its four
/// corners map to through the region's own transform (Plan 4 Task 6) —
/// per the plan's trace-compactness constraint this is region corner
/// quads, NOT per-module points; the debug UI reconstructs the module
/// grid from `BitsTrace`'s packed words via its own TS homography port.
/// Recorded for the LAST candidate attempted in the frame, same as
/// [`AlignmentTraceEntry`] — see [`Trace::record_sample_regions`].
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleRegionTrace {
    pub module_rect: [u32; 4],
    pub quad: [[f64; 2]; 4],
}

/// The LAST successfully decoded candidate's sampled bit matrix in the
/// frame, packed the same way [`crate::bitmatrix::BitMatrix`] stores it
/// (row-major `u32` words, `ceil(dim/32)` per row) — per the plan's
/// trace-compactness constraint (`Vec<u32>` packed rows, not `Vec<bool>`).
/// `None` when no candidate decoded this frame.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BitsTrace {
    pub dim: u32,
    pub words: Vec<u32>,
}

#[cfg(feature = "debug-trace")]
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Trace {
    pub tiles: Option<TileTrace>,
    pub finders: Vec<FinderCandidate>,
    pub triplets: Vec<TripletCandidate>,
    /// One entry per decode attempt actually run this frame (see
    /// [`crate::decode::decode_candidates`]'s own doc for what counts as
    /// "attempted" — proximity-deduped-away and already-consumed-finder
    /// triplets produce no entry).
    pub attempts: Vec<DecodeAttemptTrace>,
    /// The last attempted candidate's alignment-pattern search results
    /// (empty for a v1 candidate, which has no alignment patterns at all —
    /// see [`AlignmentTraceEntry`]'s doc). "Last attempted" per the plan's
    /// literal contract — NOT necessarily the same candidate `bits` (below)
    /// or `Detections.codes` describe: in a multi-triplet frame the LAST
    /// attempt run can be a different, unrelated, and possibly-FAILED
    /// candidate from whichever one(s) actually decoded (e.g. a spurious
    /// finder-noise triplet attempted after the real code already decoded).
    /// A debug-UI consumer overlaying `alignment`/`sample_regions` next to
    /// `bits`/decoded-payload data should not assume they describe the same
    /// physical code — see `debug-ui/src/overlays/layers/{alignment,
    /// samplegrid}.ts`'s own doc comments for the same caveat. (A narrower,
    /// fixed footgun: an attempt that bails out at the `invalid_dimension`
    /// check — before ever reaching alignment/sampling — is excluded from
    /// updating this field, so it can no longer blank an earlier real
    /// decode's trace down to empty; see
    /// `decode::AttemptResult::reached_geometry_stage`.)
    pub alignment: Vec<AlignmentTraceEntry>,
    /// The last attempted candidate's sample regions — same "last
    /// attempted, not last decoded" caveat as `alignment` above (they
    /// always describe the SAME candidate as each other, just not
    /// necessarily the same one as `bits`).
    pub sample_regions: Vec<SampleRegionTrace>,
    /// The last successfully decoded candidate's sampled bit matrix (`None`
    /// if nothing decoded this frame) — unlike `alignment`/`sample_regions`
    /// above, this one tracks "last DECODED", so it always agrees with
    /// `Detections.codes`' own last entry (see
    /// `debug-ui/src/overlays/layers/bits.ts`'s doc for how the debug UI
    /// relies on that specific agreement).
    pub bits: Option<BitsTrace>,
}

#[cfg(feature = "debug-trace")]
impl Trace {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn record_tiles(&mut self, grid: &TileGrid) {
        self.tiles = Some(grid.to_trace());
    }

    #[inline]
    pub fn record_finders(&mut self, finders: &[FinderCandidate]) {
        self.finders = finders.to_vec();
    }

    #[inline]
    pub fn record_triplets(&mut self, triplets: &[TripletCandidate]) {
        self.triplets = triplets.to_vec();
    }

    #[inline]
    pub fn record_attempts(&mut self, attempts: &[DecodeAttemptTrace]) {
        self.attempts = attempts.to_vec();
    }

    #[inline]
    pub fn record_alignment(&mut self, entries: &[AlignmentTraceEntry]) {
        self.alignment = entries.to_vec();
    }

    #[inline]
    pub fn record_sample_regions(&mut self, regions: &[SampleRegionTrace]) {
        self.sample_regions = regions.to_vec();
    }

    #[inline]
    pub fn record_bits(&mut self, bits: BitsTrace) {
        self.bits = Some(bits);
    }
}

/// Zero-field unit struct: `debug-trace` off. No-op inline methods let
/// `detect_traced`'s call sites compile identically to the traced build,
/// so the optimizer can erase them entirely.
#[cfg(not(feature = "debug-trace"))]
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Trace;

#[cfg(not(feature = "debug-trace"))]
impl Trace {
    #[inline]
    pub fn new() -> Self {
        Trace
    }

    #[inline]
    pub fn record_tiles(&mut self, _grid: &TileGrid) {}

    #[inline]
    pub fn record_finders(&mut self, _finders: &[FinderCandidate]) {}

    #[inline]
    pub fn record_triplets(&mut self, _triplets: &[TripletCandidate]) {}

    #[inline]
    pub fn record_attempts(&mut self, _attempts: &[DecodeAttemptTrace]) {}

    #[inline]
    pub fn record_alignment(&mut self, _entries: &[AlignmentTraceEntry]) {}

    #[inline]
    pub fn record_sample_regions(&mut self, _regions: &[SampleRegionTrace]) {}

    #[inline]
    pub fn record_bits(&mut self, _bits: BitsTrace) {}
}
