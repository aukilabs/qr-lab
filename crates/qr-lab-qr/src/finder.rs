//! 1:1:3:1:1 run-length pattern matcher: scans a boolean "is dark" stream
//! (one row or column of thresholded pixels) for finder-pattern
//! cross-sections in either polarity. Task 5 builds `find_finders` on top
//! of `match_row` in this same file.

use crate::consts::{ROW_STEP, TILE};
use crate::tiles::TileGrid;
use crate::LumaView;

/// A 5-run window that satisfies [`pattern_fits`].
///
/// `center` is the pixel-center coordinate of the middle run's midpoint;
/// `module` is `total / 7` (the estimated module width); `inverted` is
/// true when the middle run is light (a normal finder's middle run is
/// dark); `start..end` is the pixel span of the whole 5-run window.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RunHit {
    pub center: f64,
    pub module: f64,
    pub inverted: bool,
    // `find_finders` (Task 5) verifies hits from `center`/`module`/`inverted`
    // alone; `start`/`end` are kept for Debug output and future consumers
    // (e.g. Task 6 triplet grouping) that may want the whole-window span.
    #[allow(dead_code)]
    pub start: usize,
    #[allow(dead_code)]
    pub end: usize,
}

/// Pinned 1:1:3:1:1 variance rules (see Plan 2 "No overfitting" — every
/// tolerance here is a fraction of the estimated module width, not a value
/// tuned against a fixture).
pub(crate) fn pattern_fits(runs: &[f64; 5]) -> bool {
    let total: f64 = runs.iter().sum();
    if total < 7.0 {
        return false;
    }
    let unit = total / 7.0;
    let max_var = unit / 2.0;
    (runs[0] - unit).abs() < max_var
        && (runs[1] - unit).abs() < max_var
        && (runs[2] - 3.0 * unit).abs() < 3.0 * max_var
        && (runs[3] - unit).abs() < max_var
        && (runs[4] - unit).abs() < max_var
        && runs.iter().all(|&r| r >= 1.0)
}

/// Run-length encodes `bits` (`true` = dark per pixel) and slides a 5-run
/// window over the result, advancing run-by-run so overlapping windows —
/// e.g. two finders sharing the light gap run between them — both get a
/// chance to fire. Every window whose lengths satisfy [`pattern_fits`]
/// (checked in both polarities, since the rule is symmetric in the run
/// values) produces one [`RunHit`] appended to `out`.
///
/// `scratch` is caller-owned run-length storage: cleared at the top of
/// this call and otherwise reused, so a caller scanning many rows (e.g.
/// `find_finders`' row loop) can pass the same `Vec` every call instead
/// of paying a fresh heap allocation per row.
#[cfg(test)]
pub(crate) fn match_row(
    bits: impl Iterator<Item = bool>,
    scratch: &mut Vec<(bool, usize, usize)>,
    out: &mut Vec<RunHit>,
) {
    match_row_at(bits, 0, scratch, out);
}

/// The `match_row` test helper generalized for a horizontal sub-span whose first pixel is at
/// `x_offset` in the source row. Keeping the offset in the matcher avoids
/// materializing skipped tile spans merely to preserve source coordinates.
fn match_row_at(
    bits: impl Iterator<Item = bool>,
    x_offset: usize,
    scratch: &mut Vec<(bool, usize, usize)>,
    out: &mut Vec<RunHit>,
) {
    scratch.clear();
    for (x, v) in bits.enumerate() {
        let x = x + x_offset;
        match scratch.last_mut() {
            Some(last) if last.0 == v => last.2 += 1,
            _ => scratch.push((v, x, 1)),
        }
    }
    if scratch.len() < 5 {
        return;
    }
    for w in scratch.windows(5) {
        let lens = [
            w[0].2 as f64,
            w[1].2 as f64,
            w[2].2 as f64,
            w[3].2 as f64,
            w[4].2 as f64,
        ];
        if pattern_fits(&lens) {
            let middle = w[2];
            out.push(RunHit {
                center: middle.1 as f64 + middle.2 as f64 / 2.0 - 0.5,
                module: lens.iter().sum::<f64>() / 7.0,
                inverted: !middle.0,
                start: w[0].1,
                end: w[4].1 + w[4].2,
            });
        }
    }
}

/// A verified, possibly-merged finder-pattern candidate.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FinderCandidate {
    /// Finder center X in working-image pixel-center coordinates.
    pub x: f64,
    /// Finder center Y in working-image pixel-center coordinates.
    pub y: f64,
    /// Estimated module width in pixels.
    pub module: f64,
    /// `true` when the finder has inverted polarity (light center run).
    pub inverted: bool,
    /// Number of independent row detections merged into this candidate.
    pub hits: u32,
}

/// Bundles the read-only inputs every cross-check sample needs, so the
/// walking helpers below take one reference instead of four positional
/// arguments apiece.
struct Scan<'a, 'b> {
    view: &'a LumaView<'b>,
    grid: &'a TileGrid,
    w: usize,
    h: usize,
}

impl Scan<'_, '_> {
    /// Sample the binarized polarity at `(x, y)`, or `None` off-image (the
    /// caller treats a border as ending — clamping — the current run).
    fn sample(&self, x: isize, y: isize) -> Option<bool> {
        if x < 0 || y < 0 {
            return None;
        }
        let (xu, yu) = (x as usize, y as usize);
        if xu >= self.w || yu >= self.h {
            return None;
        }
        Some(self.view.get(xu, yu) < self.grid.threshold_at(xu, yu))
    }

    /// Walk from `(x, y)` in unit steps `(dx, dy)`, counting consecutive
    /// pixels whose polarity equals `target`, up to `cap` steps. Stops (the
    /// run "ends there") on a polarity change or the image border.
    fn walk_while(
        &self,
        mut x: isize,
        mut y: isize,
        dx: isize,
        dy: isize,
        target: bool,
        cap: isize,
    ) -> isize {
        let mut n = 0isize;
        while n < cap {
            match self.sample(x, y) {
                Some(c) if c == target => {
                    n += 1;
                    x += dx;
                    y += dy;
                }
                _ => break,
            }
        }
        n
    }

    /// Build the 5-run window centered on the pixel `(x0, y0)` along axis
    /// `(dx, dy)` (vertical: `(0, 1)`; diagonal: `(±1, ±1)`), mirroring
    /// `match_row`'s run construction but walking outward from a seed
    /// pixel instead of scanning a whole line. Returns the run lengths
    /// plus the signed offset (in steps along the axis from `(x0, y0)`) of
    /// the middle run's pixel-center, using the same `start + len/2 - 0.5`
    /// convention as `match_row`'s `RunHit::center`. `None` only if
    /// `(x0, y0)` itself is off-image.
    fn axis_cross_check(
        &self,
        x0: isize,
        y0: isize,
        dx: isize,
        dy: isize,
        cap: isize,
    ) -> Option<([f64; 5], f64)> {
        let center_color = self.sample(x0, y0)?;

        // Middle run: half extending outward in +direction, half in
        // -direction (the seed pixel itself belongs to whichever half is
        // counted first).
        let mid_pos = self.walk_while(x0 + dx, y0 + dy, dx, dy, center_color, cap);
        let mid_neg = self.walk_while(x0 - dx, y0 - dy, -dx, -dy, center_color, cap);

        // +direction: run3 (opposite color) then run4 (center color).
        let (p3x, p3y) = (x0 + dx * (1 + mid_pos), y0 + dy * (1 + mid_pos));
        let run3 = self.walk_while(p3x, p3y, dx, dy, !center_color, cap);
        let (p4x, p4y) = (p3x + dx * run3, p3y + dy * run3);
        let run4 = self.walk_while(p4x, p4y, dx, dy, center_color, cap);

        // -direction: run1 (opposite color) then run0 (center color).
        let (n1x, n1y) = (x0 - dx * (1 + mid_neg), y0 - dy * (1 + mid_neg));
        let run1 = self.walk_while(n1x, n1y, -dx, -dy, !center_color, cap);
        let (n0x, n0y) = (n1x - dx * run1, n1y - dy * run1);
        let run0 = self.walk_while(n0x, n0y, -dx, -dy, center_color, cap);

        let mid_total = (mid_pos + mid_neg + 1) as f64;
        let runs = [
            run0 as f64,
            run1 as f64,
            mid_total,
            run3 as f64,
            run4 as f64,
        ];
        let t_center = -(mid_neg as f64) + mid_total / 2.0 - 0.5;
        Some((runs, t_center))
    }

    /// Verify one horizontal [`RunHit`] (found on scan row `y`) with a
    /// vertical then diagonal cross-check, per the Plan 2 Task 5 algorithm
    /// contract. Returns a single-hit [`FinderCandidate`] on success.
    fn verify_hit(&self, hit: &RunHit, y: usize) -> Option<FinderCandidate> {
        let x0 = hit.center.round() as isize;
        let y0 = y as isize;
        // Per-run walk cap: the farthest a genuine run can extend from
        // the seed pixel is the 3-module core's 1.5-module half, plus
        // the 1-module separator, plus the 1-module ring = 3.5 modules;
        // 4.0 adds slack for measurement jitter.
        let cap = ((4.0 * hit.module).ceil() as isize).max(1);

        let (v_runs, t) = self.axis_cross_check(x0, y0, 0, 1, cap)?;
        if !pattern_fits(&v_runs) {
            return None;
        }
        let cy = y0 as f64 + t;
        let v_module = v_runs.iter().sum::<f64>() / 7.0;

        let cyr = cy.round() as isize;
        let diag_ok = |dx: isize, dy: isize| {
            self.axis_cross_check(x0, cyr, dx, dy, cap)
                .is_some_and(|(runs, _)| pattern_fits(&runs))
        };
        if !(diag_ok(1, 1) || diag_ok(1, -1)) {
            return None;
        }

        Some(FinderCandidate {
            x: hit.center,
            y: cy,
            module: (hit.module + v_module) / 2.0,
            inverted: hit.inverted,
            hits: 1,
        })
    }
}

/// Merge `new` into `merged` per the pinned rule: same polarity, centers
/// within `module` (average of the two estimates) on both axes, module
/// ratio ≤ 1.3 → weighted average by hit count. Otherwise append as a new
/// candidate.
fn merge_candidate(merged: &mut Vec<FinderCandidate>, new: FinderCandidate) {
    for c in merged.iter_mut() {
        if c.inverted != new.inverted {
            continue;
        }
        let thresh = (c.module + new.module) / 2.0;
        if (c.x - new.x).abs() >= thresh || (c.y - new.y).abs() >= thresh {
            continue;
        }
        // 1.3, not the cross-finder 1.5 perspective bound (see
        // triplet.rs's MAX_MODULE_RATIO derivation): `c` and `new` are
        // two observations of the SAME finder, so they differ only by
        // run-quantization jitter, not perspective foreshortening — a
        // tighter bound is safe, and 1.3 sits between the two.
        let ratio = if c.module > new.module {
            c.module / new.module
        } else {
            new.module / c.module
        };
        if ratio > 1.3 {
            continue;
        }
        let total = c.hits + new.hits;
        let (ch, nh) = (c.hits as f64, new.hits as f64);
        c.x = (c.x * ch + new.x * nh) / total as f64;
        c.y = (c.y * ch + new.y * nh) / total as f64;
        c.module = (c.module * ch + new.module * nh) / total as f64;
        c.hits = total;
        return;
    }
    merged.push(new);
}

/// Scan `view` for finder-pattern candidates: horizontal scanlines every
/// [`ROW_STEP`] rows, each hit verified by a vertical then diagonal
/// cross-check, merged across rows, and finally filtered to `hits >= 2`
/// (seen on at least two independent scan rows).
///
/// Hot path: visit only contiguous spans of non-skip tiles, binarize each
/// active span tile-by-tile (constant threshold per 16px tile; aarch64 uses
/// NEON compares), then RLE-match the span in source coordinates. A tile is
/// marked active from a 3×3 neighborhood of tile statistics, so every
/// contrast-bearing tile has one full tile of active padding. Consequently
/// a 1:1:3:1:1 window cannot be cut by a skipped span: any window containing
/// both polarities makes its own tile or a neighbor active. Resetting the RLE
/// at skipped spans also prevents low-contrast texture from manufacturing
/// cross-span finder candidates.
pub fn find_finders(view: &LumaView, grid: &TileGrid) -> Vec<FinderCandidate> {
    let (w, h) = (view.width(), view.height());
    let scan = Scan { view, grid, w, h };
    let mut merged: Vec<FinderCandidate> = Vec::new();
    let mut row_hits: Vec<RunHit> = Vec::new();
    // Scratch run-length buffer, hoisted out of the loop and reused
    // (cleared inside `match_row`) across all ~h/ROW_STEP scanned rows —
    // otherwise `match_row` would allocate a fresh `Vec` per row (~360
    // times per 720p frame at ROW_STEP=2).
    let mut runs_scratch: Vec<(bool, usize, usize)> = Vec::new();
    // Active-span dark mask; sized to width once and reused. Only the prefix
    // matching the current span is written or read.
    let mut dark = vec![false; w];
    for y in (0..h).step_by(ROW_STEP) {
        row_hits.clear();
        let row = view.row(y);
        let skipped = (0..grid.tiles_x)
            .filter(|&tx| grid.is_skip(tx * TILE, y))
            .count();
        if skipped == grid.tiles_x {
            continue;
        }
        // On highly textured rows, enumerating several short active spans
        // costs more than the pixels it avoids. Keep the dense SIMD-friendly
        // path until at least one quarter of the row is provably flat; the
        // sparse path then skips >=4 pixels of RLE+binarization work per tile
        // lookup even before SIMD width is considered.
        if skipped * 4 < grid.tiles_x {
            binarize_row_span(row, grid, y, 0, w, &mut dark);
            match_row_at(dark.iter().copied(), 0, &mut runs_scratch, &mut row_hits);
        } else {
            let mut tx = 0;
            while tx < grid.tiles_x {
                while tx < grid.tiles_x && grid.is_skip(tx * TILE, y) {
                    tx += 1;
                }
                let start_tx = tx;
                while tx < grid.tiles_x && !grid.is_skip(tx * TILE, y) {
                    tx += 1;
                }
                let x0 = start_tx * TILE;
                let x1 = (tx * TILE).min(w);
                if x1.saturating_sub(x0) < 7 {
                    continue;
                }
                let len = x1 - x0;
                binarize_row_span(row, grid, y, x0, x1, &mut dark[..len]);
                match_row_at(
                    dark[..len].iter().copied(),
                    x0,
                    &mut runs_scratch,
                    &mut row_hits,
                );
            }
        }
        for hit in &row_hits {
            if let Some(cand) = scan.verify_hit(hit, y) {
                merge_candidate(&mut merged, cand);
            }
        }
    }
    merged.retain(|c| c.hits >= 2);
    merged
}

/// Fill `dark` for source-row span `x0..x1` with
/// `row[x] < threshold_at(x, y)`. The span is tile-aligned except for the
/// image's ragged right edge. One threshold lookup per tile; full tiles use
/// NEON lane compares on aarch64.
fn binarize_row_span(
    row: &[u8],
    grid: &TileGrid,
    y: usize,
    span_x0: usize,
    span_x1: usize,
    dark: &mut [bool],
) {
    debug_assert!(span_x0 <= span_x1 && span_x1 <= row.len());
    debug_assert_eq!(dark.len(), span_x1 - span_x0);
    for tx in (span_x0 / TILE)..span_x1.div_ceil(TILE) {
        let tile_x0 = tx * TILE;
        let tile_x1 = (tile_x0 + TILE).min(span_x1);
        let out0 = tile_x0 - span_x0;
        let thr = grid.threshold_at(tile_x0, y);
        let seg = &row[tile_x0..tile_x1];
        #[cfg(target_arch = "aarch64")]
        {
            if seg.len() == TILE {
                let arr: &[u8; TILE] = seg.try_into().expect("TILE bytes");
                let mask = crate::neon::dark_mask_u8x16(arr, thr);
                dark[out0..out0 + TILE].copy_from_slice(&mask);
                continue;
            }
        }
        for (i, &p) in seg.iter().enumerate() {
            dark[out0 + i] = p < thr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(spec: &[(bool, usize)]) -> Vec<bool> {
        spec.iter()
            .flat_map(|&(v, n)| std::iter::repeat_n(v, n))
            .collect()
    }

    #[test]
    fn pattern_fits_accepts_ideal_and_tolerant() {
        assert!(pattern_fits(&[4.0, 4.0, 12.0, 4.0, 4.0]));
        assert!(pattern_fits(&[3.0, 4.0, 13.0, 4.0, 5.0]));
        assert!(!pattern_fits(&[4.0, 4.0, 4.0, 4.0, 4.0])); // middle not 3x
        assert!(!pattern_fits(&[1.0, 8.0, 12.0, 4.0, 4.0])); // outer off
    }

    #[test]
    fn match_row_finds_dark_finder() {
        // light(10) dark(4) light(4) DARK(12) light(4) dark(4) light(10)
        let row = bits(&[
            (false, 10),
            (true, 4),
            (false, 4),
            (true, 12),
            (false, 4),
            (true, 4),
            (false, 10),
        ]);
        let mut hits = Vec::new();
        let mut scratch = Vec::new();
        match_row(row.iter().copied(), &mut scratch, &mut hits);
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert!(!h.inverted);
        assert!((h.module - 4.0).abs() < 1e-9);
        // middle run spans x in [18, 30) -> pixel-center 23.5.
        assert!((h.center - 23.5).abs() <= 0.5, "center={}", h.center);
    }

    #[test]
    fn match_row_finds_inverted_finder() {
        // Same geometry, polarity flipped: middle run is LIGHT.
        let row = bits(&[
            (true, 10),
            (false, 4),
            (true, 4),
            (false, 12),
            (true, 4),
            (false, 4),
            (true, 10),
        ]);
        let mut hits = Vec::new();
        let mut scratch = Vec::new();
        match_row(row.iter().copied(), &mut scratch, &mut hits);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].inverted);
    }

    #[test]
    fn match_row_at_preserves_source_coordinates() {
        let finder = bits(&[(false, 4), (true, 4), (false, 12), (true, 4), (false, 4)]);
        let mut scratch = Vec::new();
        let mut hits = Vec::new();
        match_row_at(finder.iter().copied(), 32, &mut scratch, &mut hits);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].start, 32);
        assert_eq!(hits[0].end, 60);
        assert!((hits[0].center - 45.5).abs() < f64::EPSILON);
    }

    #[test]
    fn match_row_rejects_noise() {
        let row = bits(&[
            (false, 3),
            (true, 2),
            (false, 9),
            (true, 1),
            (false, 5),
            (true, 7),
            (false, 2),
        ]);
        let mut hits = Vec::new();
        let mut scratch = Vec::new();
        match_row(row.iter().copied(), &mut scratch, &mut hits);
        assert!(hits.is_empty());
    }

    #[test]
    fn match_row_finds_two_adjacent_finders() {
        let one = [(true, 4), (false, 4), (true, 12), (false, 4), (true, 4)];
        let mut spec = vec![(false, 8)];
        spec.extend_from_slice(&one);
        spec.push((false, 20));
        spec.extend_from_slice(&one);
        spec.push((false, 8));
        let mut hits = Vec::new();
        let mut scratch = Vec::new();
        match_row(bits(&spec).iter().copied(), &mut scratch, &mut hits);
        // NOTE (corrected vs. brief, see task-4-report.md for the full
        // numeric derivation): the brief asserted `hits.len() == 2`, but
        // `pattern_fits`'s own verbatim tolerance — outer runs within
        // +/-0.5*unit, middle run within +/-1.5*unit of 3*unit — also
        // accepts the 20px light gap between the two finders as a
        // (spurious, inverted-polarity) middle run: window lengths
        // [4,4,20,4,4] give unit=36/7=5.142857, and |20-3*unit|=4.571429
        // < 3*max_var=7.714286, so pattern_fits(&[4,4,20,4,4]) == true.
        // This is a real consequence of the pinned tolerance, not an
        // implementation bug: `match_row` and `pattern_fits` are unchanged
        // from the brief. The two genuine finders are still found and are
        // distinguishable as the two non-inverted hits.
        assert_eq!(hits.len(), 3);
        let normal: Vec<_> = hits.iter().filter(|h| !h.inverted).collect();
        assert_eq!(normal.len(), 2);
    }

    // Painting helper promoted to a shared cfg(test) module when the
    // triplet.rs accept-path tests started needing it too.
    use crate::testpaint::paint_finder;

    #[test]
    fn detects_synthetic_finder_both_polarities() {
        let (w, h) = (160, 160);
        for inverted in [false, true] {
            let (ink, bg) = if inverted { (230, 30) } else { (25, 225) };
            let mut img = vec![bg; w * h];
            paint_finder(&mut img, w, 40, 56, 6, ink, bg);
            let view = crate::LumaView::new(&img, w, h, w).unwrap();
            let grid = crate::tiles::TileGrid::build(&view);
            let f = find_finders(&view, &grid);
            assert_eq!(f.len(), 1, "inverted={inverted}: {f:?}");
            assert_eq!(f[0].inverted, inverted);
            // center = (40 + 3.5*6, 56 + 3.5*6) = (61, 77) at pixel centers
            assert!((f[0].x - 60.5).abs() < 1.5, "{:?}", f[0]);
            assert!((f[0].y - 76.5).abs() < 1.5, "{:?}", f[0]);
            assert!((f[0].module - 6.0).abs() < 1.0);
        }
    }
}
