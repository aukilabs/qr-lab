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
//! `SampleRegionTrace`, `BitsTrace`, and (Plan 5C) `DecodedCodeTrace`) are,
//! unlike `TileTrace`, always compiled regardless of the `debug-trace`
//! feature — matching `decode::DecodeAttemptTrace`'s own precedent (that
//! struct has lived ungated since Task 5, since `decode_candidates` always
//! builds its `Vec<DecodeAttemptTrace>` as part of its normal return
//! contract; only `Trace` itself, the feature-gated *container*, decides
//! whether to keep a copy). `decode::decode_candidates` builds all of these
//! unconditionally too (see its own module doc) — the data is small
//! (bounded by the alignment lattice's `<= 7x7` nodes and `<= 6x6` sample
//! regions per attempt, and a single packed bit matrix per successful
//! decode, times however many codes actually decoded — always few), so
//! there is no separate runtime "want trace" flag to thread; `detect_with`
//! is simply the one place that decides whether to keep any of it, via
//! these `record_*` calls.
//!
//! Plan 5C (multi-code trace): a multi-code frame decodes more than one
//! [`crate::decode::DecodedCode`], but the ORIGINAL `alignment`/
//! `sample_regions`/`bits` fields on [`Trace`] only ever tracked a SINGLE
//! candidate's data (the last one decoded — see the Plan 4B Fix A history
//! in each field's own doc). That meant a multi-code scene's samplegrid/
//! bits debug-UI overlays only ever visualized one of the several decoded
//! codes. [`Trace::codes`] fixes this: one [`DecodedCodeTrace`] per DECODED
//! code, so every code's own data is available. The original singular
//! fields are kept, but their scope narrows to the FAILURE-diagnosis case
//! only (nothing decoded this frame) — see each field's own doc for the
//! exact rule.

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
    /// Number of tiles along the X axis.
    pub tiles_x: usize,
    /// Number of tiles along the Y axis.
    pub tiles_y: usize,
    /// Per-tile thresholds in row-major tile order.
    pub thresholds: Vec<u8>,
    /// Per-tile low-contrast skip flags in row-major tile order.
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
    /// Module-space rectangle `[x, y, width, height]`.
    pub module_rect: [u32; 4],
    /// Image-pixel corners of the region in TL/TR/BR/BL order.
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
    /// Module dimension of the sampled square bit matrix.
    pub dim: u32,
    /// Row-major packed words (`ceil(dim/32)` words per row).
    pub words: Vec<u32>,
}

/// One DECODED code's own alignment/sample-region/bits trace data (Plan 5C:
/// multi-code trace), so a frame with N decoded codes carries N of these on
/// [`Trace::codes`] instead of just the last one. Built at the moment
/// [`crate::decode::decode_candidates`] records a successful decode — cheap,
/// since decoded codes are always few (bounded by the finder count / 3) even
/// when the frame's `attempts`/`triplets` are numerous. See [`Trace::codes`]'s
/// doc for how this relates to the legacy singular `alignment`/
/// `sample_regions`/`bits` fields on `Trace` itself.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DecodedCodeTrace {
    /// Index into [`crate::Detections::codes`] (equivalently,
    /// `WasmResult.detections.codes` on the wire) this entry describes — the
    /// debug UI resolves a `DecodedCodeTrace` back to its `DecodedCode` (for
    /// `corners`, `finder_indices`, etc.) through this index rather than the
    /// two arrays needing to already be the same length by construction
    /// (they are, in practice — one entry per decode, in decode order — but
    /// an explicit index is self-describing and survives either side being
    /// filtered/reordered independently in the future).
    pub code_index: usize,
    /// This code's sample regions — same shape/meaning as the legacy
    /// [`Trace::sample_regions`] field, just scoped to this one code instead
    /// of "whichever candidate was selected".
    pub sample_regions: Vec<SampleRegionTrace>,
    /// This code's sampled bit matrix — same shape/meaning as the legacy
    /// [`Trace::bits`] field, minus the `Option` wrapper: every entry in
    /// `Trace::codes` came from an actual decode, so a bit matrix always
    /// exists (unlike the frame-level singular field, which is `None`
    /// whenever nothing decoded at all).
    pub bits: BitsTrace,
    /// This code's alignment-pattern search results — same shape/meaning as
    /// the legacy [`Trace::alignment`] field, just scoped to this one code.
    pub alignment: Vec<AlignmentTraceEntry>,
}

/// One outer module-region edge's subpixel refinement point counts (Plan 5
/// Task 3) — mirrors `refine::EdgeStat`. Kept as a separate DTO (not a
/// direct re-export) so `refine.rs`'s internal representation can evolve
/// without touching the wire contract — the same separation
/// `AlignmentTraceEntry`/`SampleRegionTrace` already establish for their
/// own source modules.
#[derive(Clone, Copy, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EdgeRefineTrace {
    /// Edge samples considered during refinement.
    pub points_probed: u32,
    /// Samples retained for the total-least-squares fit.
    pub points_fit: u32,
    /// Samples rejected as outliers.
    pub dropped_outliers: u32,
    /// Whether this edge produced a usable line.
    pub valid: bool,
}

/// Subpixel corner refinement diagnostics for the LAST decoded candidate
/// this frame (Plan 5 Task 3) — same selection rule as
/// `bits`/`alignment`/`sample_regions` (see `decode::DecodeTraceData`'s
/// doc): `None` when nothing decoded this frame, refinement was disabled
/// (`ScanOptions::refine == false`), or refinement ran but produced fewer
/// than 2 valid edge lines (`refine::refine_corners` returned `None`).
/// `edges` is `[top, right, bottom, left]` (module-space `y=0`, `x=dim`,
/// `y=dim`, `x=0` — see `refine::EDGE_TO_CORNERS`'s doc); `corner_refined`
/// is `[TL, TR, BR, BL]`, `true` where the corner came from intersecting
/// its two adjacent edge lines rather than keeping the caller's unrefined
/// (coarse, source-scaled) corner.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RefineTrace {
    /// Per-edge diagnostics in top/right/bottom/left order.
    pub edges: [EdgeRefineTrace; 4],
    /// Per-corner refinement success flags in TL/TR/BR/BL order.
    pub corner_refined: [bool; 4],
}

/// Accumulated per-stage debug trace for one `detect_traced` / `scan_traced` call.
///
/// Only available when the `debug-trace` feature is enabled. Without that
/// feature, [`Trace`] is a zero-sized no-op type with the same method names.
#[cfg(feature = "debug-trace")]
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Trace {
    /// Tile-threshold surface captured after the tiles stage, if recorded.
    pub tiles: Option<TileTrace>,
    /// Finder candidates from the finders stage.
    pub finders: Vec<FinderCandidate>,
    /// Grouped triplets from the triplets stage.
    pub triplets: Vec<TripletCandidate>,
    /// One entry per decode attempt actually run this frame (see
    /// [`crate::decode::decode_candidates`]'s own doc for what counts as
    /// "attempted" — proximity-deduped-away and already-consumed-finder
    /// triplets produce no entry).
    pub attempts: Vec<DecodeAttemptTrace>,
    /// One entry per DECODED code this frame (Plan 5C: multi-code trace) —
    /// the honest, complete replacement for the legacy singular
    /// `alignment`/`sample_regions`/`bits` fields below in the (common)
    /// case where at least one code decoded: a multi-code frame (e.g.
    /// `multi_07`'s 4 codes) gets 4 entries here, one per
    /// [`crate::Detections::codes`] entry (see [`DecodedCodeTrace::code_index`]),
    /// each carrying that SPECIFIC code's own alignment search, sample
    /// regions, and bit matrix — not just the last one decoded. Empty
    /// whenever nothing decoded this frame (see the legacy fields' doc for
    /// that failure-diagnosis case instead).
    pub codes: Vec<DecodedCodeTrace>,
    /// FAILURE-DIAGNOSIS ONLY (Plan 5C narrowed this field's scope — see
    /// `codes` above for the decode-success case): populated ONLY when
    /// NOTHING decoded this frame, from the FIRST attempt run this frame —
    /// canonical (unrotated) corner roles of the first (lowest-`snap_error`)
    /// candidate attempted, never a corner-role rotation retry and never a
    /// later candidate (same "first attempt" rule Plan 4B Fix A introduced).
    /// Empty (not just "the first candidate's real alignment search", which
    /// can itself legitimately be empty for a v1 candidate — see
    /// [`AlignmentTraceEntry`]'s doc) whenever ANY code decoded this frame:
    /// that data now lives per-code on `codes` instead, so this field isn't
    /// duplicated — a debug-UI consumer should always prefer `codes` and
    /// only fall back to this field when `codes` is empty. See
    /// `debug-ui/src/overlays/layers/{alignment,samplegrid}.ts`'s own doc
    /// comments for the debug-UI-facing version of this same contract.
    pub alignment: Vec<AlignmentTraceEntry>,
    /// FAILURE-DIAGNOSIS ONLY — same rule as `alignment` above (empty
    /// whenever any code decoded this frame; the per-code data lives on
    /// `codes` instead). Populated from the first attempt run this frame
    /// only when nothing decoded.
    pub sample_regions: Vec<SampleRegionTrace>,
    /// FAILURE-DIAGNOSIS ONLY: always `None` now — there is no "last
    /// decoded candidate's bit matrix" to show at the frame level once any
    /// number of codes can decode; see `codes` for the per-code bit
    /// matrices. Kept (rather than removed) so `Trace`'s shape stays a
    /// simple superset for existing failure-path consumers that only ever
    /// checked `is_none()`/`is_some()` here to mean "did anything decode" —
    /// prefer `!codes.is_empty()` for that check going forward.
    pub bits: Option<BitsTrace>,
    /// This frame's subpixel corner refinement diagnostics (Plan 5 Task 3)
    /// — see [`RefineTrace`]'s doc for the selection rule and `None`
    /// cases.
    pub refine: Option<RefineTrace>,
}

#[cfg(feature = "debug-trace")]
impl Trace {
    /// Create an empty trace container.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the tile-threshold grid from the tiles stage.
    #[inline]
    pub fn record_tiles(&mut self, grid: &TileGrid) {
        self.tiles = Some(grid.to_trace());
    }

    /// Record finder candidates from the finders stage.
    #[inline]
    pub fn record_finders(&mut self, finders: &[FinderCandidate]) {
        self.finders = finders.to_vec();
    }

    /// Record triplets from the triplets stage.
    #[inline]
    pub fn record_triplets(&mut self, triplets: &[TripletCandidate]) {
        self.triplets = triplets.to_vec();
    }

    /// Record per-attempt decode traces for this frame.
    #[inline]
    pub fn record_attempts(&mut self, attempts: &[DecodeAttemptTrace]) {
        self.attempts = attempts.to_vec();
    }

    /// Record per-decoded-code alignment/sample/bits traces.
    #[inline]
    pub fn record_codes(&mut self, codes: Vec<DecodedCodeTrace>) {
        self.codes = codes;
    }

    /// Record failure-diagnosis alignment entries (empty when any code decoded).
    #[inline]
    pub fn record_alignment(&mut self, entries: &[AlignmentTraceEntry]) {
        self.alignment = entries.to_vec();
    }

    /// Record failure-diagnosis sample regions (empty when any code decoded).
    #[inline]
    pub fn record_sample_regions(&mut self, regions: &[SampleRegionTrace]) {
        self.sample_regions = regions.to_vec();
    }

    /// Record a frame-level bits payload (legacy; prefer [`Self::codes`]).
    #[inline]
    pub fn record_bits(&mut self, bits: BitsTrace) {
        self.bits = Some(bits);
    }

    /// Record subpixel corner-refinement diagnostics for this frame.
    #[inline]
    pub fn record_refine(&mut self, refine: RefineTrace) {
        self.refine = Some(refine);
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
    /// Create an empty no-op trace sink.
    #[inline]
    pub fn new() -> Self {
        Trace
    }

    /// No-op: tile grid recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_tiles(&mut self, _grid: &TileGrid) {}

    /// No-op: finder recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_finders(&mut self, _finders: &[FinderCandidate]) {}

    /// No-op: triplet recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_triplets(&mut self, _triplets: &[TripletCandidate]) {}

    /// No-op: decode-attempt recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_attempts(&mut self, _attempts: &[DecodeAttemptTrace]) {}

    /// No-op: decoded-code recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_codes(&mut self, _codes: Vec<DecodedCodeTrace>) {}

    /// No-op: alignment recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_alignment(&mut self, _entries: &[AlignmentTraceEntry]) {}

    /// No-op: sample-region recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_sample_regions(&mut self, _regions: &[SampleRegionTrace]) {}

    /// No-op: bit-matrix recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_bits(&mut self, _bits: BitsTrace) {}

    /// No-op: refine recording is compiled out without `debug-trace`.
    #[inline]
    pub fn record_refine(&mut self, _refine: RefineTrace) {}
}
