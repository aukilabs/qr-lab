use crate::consts::{CONTRAST_FLOOR, TILE};
use crate::LumaView;

/// Binarization parameters for [`TileGrid::build_with`] (Plan 6). The
/// default is EXACTLY the historical [`TileGrid::build`] behavior — same
/// integer arithmetic, same skip rule — so every existing entry point stays
/// bit-identical. The robustness ladder perturbs these to re-scan a frame
/// without touching any pixels:
///
/// - `threshold_offset`: added to each tile's computed threshold, clamped to
///   `0..=255`. ±8 (~3% of the 8-bit range, the amplitude of typical print
///   dot-gain / JPEG ringing) recovers codes whose ink bleeds dark (`-8`) or
///   washes light (`+8`) relative to the local midpoint.
/// - `contrast_floor`: replaces [`CONTRAST_FLOOR`] (12 = 6σ of the σ≈2
///   sensor-noise model). The ladder's low-contrast rung drops it to 6 (3σ)
///   to reach glare-washed codes the default floor skips entirely — at the
///   cost of more noise-triggered candidate work, which is why it is a
///   recovery rung and not the default.
/// - `sauvola`: use a tile-granular Sauvola threshold surface
///   (`t = m·(1 + k·(s/R − 1))`, k=0.2, R=128 — Sauvola's published
///   normalization constants) computed from 3×3-dilated tile mean/std
///   instead of the min/max midpoint. The std term pulls thresholds toward
///   background in low-variance regions, so quiet zones stay clean where a
///   midpoint threshold speckles — the DIBCO-transfer rung for noisy,
///   unevenly lit frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BinarizeSpec {
    pub threshold_offset: i16,
    pub contrast_floor: u8,
    pub sauvola: bool,
}

impl Default for BinarizeSpec {
    fn default() -> Self {
        Self {
            threshold_offset: 0,
            contrast_floor: CONTRAST_FLOOR,
            sauvola: false,
        }
    }
}

pub struct TileGrid {
    pub tiles_x: usize,
    pub tiles_y: usize,
    threshold: Vec<u8>,
    skip: Vec<bool>,
}

impl TileGrid {
    pub fn build(view: &LumaView) -> TileGrid {
        Self::build_with(view, BinarizeSpec::default())
    }

    /// [`TileGrid::build`] with explicit [`BinarizeSpec`] parameters. With
    /// `BinarizeSpec::default()` this is bit-identical to the historical
    /// `build` (same loops, same integer ops — the default-path pins in
    /// `decode_gate.rs` continue to hold).
    pub fn build_with(view: &LumaView, spec: BinarizeSpec) -> TileGrid {
        let (w, h) = (view.width(), view.height());
        let tiles_x = w.div_ceil(TILE);
        let tiles_y = h.div_ceil(TILE);
        let mut mins = vec![255u8; tiles_x * tiles_y];
        let mut maxs = vec![0u8; tiles_x * tiles_y];
        // Sauvola-only tile statistics; untouched (empty) on the default
        // path so the historical path allocates nothing extra beyond two
        // empty Vecs.
        let mut sums = Vec::new();
        let mut sumsqs = Vec::new();
        let mut counts = Vec::new();
        if spec.sauvola {
            sums = vec![0u32; tiles_x * tiles_y];
            sumsqs = vec![0u64; tiles_x * tiles_y];
            counts = vec![0u32; tiles_x * tiles_y];
        }
        for ty in 0..tiles_y {
            for y in ty * TILE..((ty + 1) * TILE).min(h) {
                let row = view.row(y);
                for tx in 0..tiles_x {
                    let x0 = tx * TILE;
                    let x1 = (x0 + TILE).min(w);
                    let s = &row[x0..x1];
                    let idx = ty * tiles_x + tx;
                    // Full TILE-wide segment: NEON min/max (+ optional
                    // sum/sumsq) is bit-identical to the scalar fold and
                    // is the unconditional full-frame cost of every scan.
                    #[cfg(target_arch = "aarch64")]
                    {
                        if s.len() == TILE {
                            let arr: &[u8; TILE] = s.try_into().expect("TILE bytes");
                            let (lo, hi) = crate::neon::min_max_u8x16(arr);
                            mins[idx] = mins[idx].min(lo);
                            maxs[idx] = maxs[idx].max(hi);
                            if spec.sauvola {
                                sums[idx] += crate::neon::sum_u8x16(arr);
                                sumsqs[idx] += crate::neon::sumsq_u8x16(arr);
                                counts[idx] += TILE as u32;
                            }
                            continue;
                        }
                    }
                    for &p in s {
                        mins[idx] = mins[idx].min(p);
                        maxs[idx] = maxs[idx].max(p);
                    }
                    if spec.sauvola {
                        for &p in s {
                            sums[idx] += p as u32;
                            sumsqs[idx] += (p as u64) * (p as u64);
                        }
                        counts[idx] += s.len() as u32;
                    }
                }
            }
        }
        let mut threshold = vec![0u8; tiles_x * tiles_y];
        let mut skip = vec![false; tiles_x * tiles_y];
        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let (mut lo, mut hi) = (255u8, 0u8);
                let (mut sum, mut sumsq, mut count) = (0u64, 0u64, 0u64);
                for ny in ty.saturating_sub(1)..=(ty + 1).min(tiles_y - 1) {
                    for nx in tx.saturating_sub(1)..=(tx + 1).min(tiles_x - 1) {
                        lo = lo.min(mins[ny * tiles_x + nx]);
                        hi = hi.max(maxs[ny * tiles_x + nx]);
                        if spec.sauvola {
                            sum += sums[ny * tiles_x + nx] as u64;
                            sumsq += sumsqs[ny * tiles_x + nx];
                            count += counts[ny * tiles_x + nx] as u64;
                        }
                    }
                }
                let idx = ty * tiles_x + tx;
                let base = if spec.sauvola && count > 0 {
                    // Sauvola over the dilated neighborhood: m·(1 + k·(s/R − 1))
                    // with k=0.2, R=128, in f64 (identical on every host —
                    // IEEE-754, no fast-math).
                    let m = sum as f64 / count as f64;
                    let var = (sumsq as f64 / count as f64) - m * m;
                    let s = var.max(0.0).sqrt();
                    (m * (1.0 + 0.2 * (s / 128.0 - 1.0)))
                        .round()
                        .clamp(0.0, 255.0) as i16
                } else {
                    ((lo as u16 + hi as u16) / 2) as i16
                };
                threshold[idx] = (base + spec.threshold_offset).clamp(0, 255) as u8;
                skip[idx] = hi - lo < spec.contrast_floor;
            }
        }
        TileGrid {
            tiles_x,
            tiles_y,
            threshold,
            skip,
        }
    }

    /// # Panics
    /// Panics (via out-of-bounds slice indexing) if `x` or `y` is
    /// outside the image this grid was built from, i.e.
    /// `x >= tiles_x * TILE` or `y >= tiles_y * TILE`.
    #[inline]
    pub fn threshold_at(&self, x: usize, y: usize) -> u8 {
        self.threshold[(y / TILE) * self.tiles_x + x / TILE]
    }

    /// # Panics
    /// Panics (via out-of-bounds slice indexing) if `x` or `y` is
    /// outside the image this grid was built from, i.e.
    /// `x >= tiles_x * TILE` or `y >= tiles_y * TILE`.
    #[inline]
    pub fn is_skip(&self, x: usize, y: usize) -> bool {
        self.skip[(y / TILE) * self.tiles_x + x / TILE]
    }

    /// # Panics
    /// Panics (via out-of-bounds slice indexing) if `y` is outside the
    /// image this grid was built from, i.e. `y >= tiles_y * TILE`.
    pub fn row_all_skip(&self, y: usize) -> bool {
        let base = (y / TILE) * self.tiles_x;
        self.skip[base..base + self.tiles_x].iter().all(|&s| s)
    }

    /// Export this grid's per-tile thresholds/skip mask for [`crate::Trace`].
    /// `pub(crate)` rather than making `threshold`/`skip` public fields —
    /// only the debug-trace path needs read access to them.
    #[cfg(feature = "debug-trace")]
    pub(crate) fn to_trace(&self) -> crate::trace::TileTrace {
        crate::trace::TileTrace {
            tiles_x: self.tiles_x,
            tiles_y: self.tiles_y,
            thresholds: self.threshold.clone(),
            skip: self.skip.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LumaView;

    fn img(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        (0..w * h).map(|i| f(i % w, i / w)).collect()
    }

    #[test]
    fn flat_image_is_all_skip() {
        let d = img(64, 48, |_, _| 128);
        let g = TileGrid::build(&LumaView::new(&d, 64, 48, 64).unwrap());
        assert_eq!((g.tiles_x, g.tiles_y), (4, 3));
        for y in (0..48).step_by(7) {
            assert!(g.row_all_skip(y));
            for x in (0..64).step_by(7) {
                assert!(g.is_skip(x, y));
                assert_eq!(g.threshold_at(x, y), 128);
            }
        }
    }

    #[test]
    fn contrast_tile_thresholds_midpoint_and_dilates() {
        // 64x48: left half black(20), right half white(220).
        let d = img(64, 48, |x, _| if x < 32 { 20 } else { 220 });
        let g = TileGrid::build(&LumaView::new(&d, 64, 48, 64).unwrap());
        // The boundary column of tiles sees both -> not skip, threshold 120.
        assert!(!g.is_skip(32, 24));
        assert_eq!(g.threshold_at(32, 24), 120);
        // One tile AWAY from the boundary: dilation pulls the extrema
        // across, so threshold is still 120 and not skip.
        assert!(!g.is_skip(16, 24));
        assert_eq!(g.threshold_at(16, 24), 120);
        // NOTE (corrected vs. brief, see task-3-report.md "Test brief
        // inconsistency" for the full numeric derivation): tiles are 16px,
        // tiles_x = 64/16 = 4 -> tile indices 0,1,2,3 with tile1=[16,32)
        // black, tile2=[32,48) white. x=48 is tile index 3, the LAST
        // column, an edge tile. Its 3x3 neighborhood clamps to nx in
        // {2,3} only (no tile 4 exists) -> both tile2 and tile3 are
        // uniformly white (220,220) -> dilated max-min = 0 < CONTRAST_FLOOR
        // (12) -> skip = true. This is the exact mirror of tile 0 below
        // (neighbors {0,1}, both uniformly black) which the brief already
        // asserts is skip. The brief's original assertion here read
        // `assert!(!g.is_skip(48, 24));` (not skip), which contradicts the
        // implementation given verbatim in the same brief and breaks the
        // left/right symmetry of this fixture.
        assert!(g.is_skip(48, 24));
        // Two tiles away (x=0..15 is 2 tiles from the boundary col at 32?
        // tiles are 16 wide: tile0 [0,16), tile1 [16,32), tile2 [32,48).
        // tile0 is adjacent to tile1 which touches the boundary via
        // dilation from tile2... tile0's 3x3 neighborhood = tiles 0,1 ->
        // all black -> skip.
        assert!(g.is_skip(0, 24));
    }

    #[test]
    fn ragged_edges_are_handled() {
        // 70x30 -> 5x2 tiles, last column 6px wide, last row 14px tall.
        let d = img(70, 30, |x, y| if x >= 64 && y >= 16 { 200 } else { 50 });
        let g = TileGrid::build(&LumaView::new(&d, 70, 30, 70).unwrap());
        assert_eq!((g.tiles_x, g.tiles_y), (5, 2));
        assert!(!g.is_skip(69, 29));
        assert_eq!(g.threshold_at(69, 29), 125);
    }
}
