//! Optional per-stage instrumentation for [`crate::detect_traced`].
//!
//! `Trace` always exists so `detect_traced` compiles with or without the
//! `debug-trace` feature. Without the feature it is a zero-field unit
//! struct with `#[inline]` no-op record methods — the optimizer erases
//! every call site, so `detect` paying for `detect_traced`'s plumbing
//! costs nothing in the default build. With the feature it accumulates
//! the tile grid, finder candidates, and triplet candidates from one
//! `detect_traced` call for debugging/visualization.

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

#[cfg(feature = "debug-trace")]
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Trace {
    pub tiles: Option<TileTrace>,
    pub finders: Vec<FinderCandidate>,
    pub triplets: Vec<TripletCandidate>,
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
}
