use crate::consts::{CONTRAST_FLOOR, TILE};
use crate::LumaView;

pub struct TileGrid {
    pub tiles_x: usize,
    pub tiles_y: usize,
    threshold: Vec<u8>,
    skip: Vec<bool>,
}

impl TileGrid {
    pub fn build(view: &LumaView) -> TileGrid {
        let (w, h) = (view.width(), view.height());
        let tiles_x = w.div_ceil(TILE);
        let tiles_y = h.div_ceil(TILE);
        let mut mins = vec![255u8; tiles_x * tiles_y];
        let mut maxs = vec![0u8; tiles_x * tiles_y];
        for ty in 0..tiles_y {
            for y in ty * TILE..((ty + 1) * TILE).min(h) {
                let row = view.row(y);
                for tx in 0..tiles_x {
                    let s = &row[tx * TILE..((tx + 1) * TILE).min(w)];
                    let idx = ty * tiles_x + tx;
                    for &p in s {
                        mins[idx] = mins[idx].min(p);
                        maxs[idx] = maxs[idx].max(p);
                    }
                }
            }
        }
        let mut threshold = vec![0u8; tiles_x * tiles_y];
        let mut skip = vec![false; tiles_x * tiles_y];
        for ty in 0..tiles_y {
            for tx in 0..tiles_x {
                let (mut lo, mut hi) = (255u8, 0u8);
                for ny in ty.saturating_sub(1)..=(ty + 1).min(tiles_y - 1) {
                    for nx in tx.saturating_sub(1)..=(tx + 1).min(tiles_x - 1) {
                        lo = lo.min(mins[ny * tiles_x + nx]);
                        hi = hi.max(maxs[ny * tiles_x + nx]);
                    }
                }
                let idx = ty * tiles_x + tx;
                threshold[idx] = ((lo as u16 + hi as u16) / 2) as u8;
                skip[idx] = hi - lo < CONTRAST_FLOOR;
            }
        }
        TileGrid { tiles_x, tiles_y, threshold, skip }
    }

    #[inline]
    pub fn threshold_at(&self, x: usize, y: usize) -> u8 {
        self.threshold[(y / TILE) * self.tiles_x + x / TILE]
    }

    #[inline]
    pub fn is_skip(&self, x: usize, y: usize) -> bool {
        self.skip[(y / TILE) * self.tiles_x + x / TILE]
    }

    pub fn row_all_skip(&self, y: usize) -> bool {
        let base = (y / TILE) * self.tiles_x;
        self.skip[base..base + self.tiles_x].iter().all(|&s| s)
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
