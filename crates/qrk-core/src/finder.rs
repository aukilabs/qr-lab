//! 1:1:3:1:1 run-length pattern matcher: scans a boolean "is dark" stream
//! (one row or column of thresholded pixels) for finder-pattern
//! cross-sections in either polarity. Task 5 builds `find_finders` on top
//! of `match_row` in this same file.

use crate::consts::ROW_STEP;
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
pub(crate) fn match_row(bits: impl Iterator<Item = bool>, out: &mut Vec<RunHit>) {
    let mut runs: Vec<(bool, usize, usize)> = Vec::new();
    for (x, v) in bits.enumerate() {
        match runs.last_mut() {
            Some(last) if last.0 == v => last.2 += 1,
            _ => runs.push((v, x, 1)),
        }
    }
    if runs.len() < 5 {
        return;
    }
    for w in runs.windows(5) {
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
pub struct FinderCandidate {
    pub x: f64,
    pub y: f64,
    pub module: f64,
    pub inverted: bool,
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
    fn walk_while(&self, mut x: isize, mut y: isize, dx: isize, dy: isize, target: bool, cap: isize) -> isize {
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
    /// [`match_row`]'s run construction but walking outward from a seed
    /// pixel instead of scanning a whole line. Returns the run lengths
    /// plus the signed offset (in steps along the axis from `(x0, y0)`) of
    /// the middle run's pixel-center, using the same `start + len/2 - 0.5`
    /// convention as [`match_row`]'s `RunHit::center`. `None` only if
    /// `(x0, y0)` itself is off-image.
    fn axis_cross_check(&self, x0: isize, y0: isize, dx: isize, dy: isize, cap: isize) -> Option<([f64; 5], f64)> {
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
        let runs = [run0 as f64, run1 as f64, mid_total, run3 as f64, run4 as f64];
        let t_center = -(mid_neg as f64) + mid_total / 2.0 - 0.5;
        Some((runs, t_center))
    }

    /// Verify one horizontal [`RunHit`] (found on scan row `y`) with a
    /// vertical then diagonal cross-check, per the Plan 2 Task 5 algorithm
    /// contract. Returns a single-hit [`FinderCandidate`] on success.
    fn verify_hit(&self, hit: &RunHit, y: usize) -> Option<FinderCandidate> {
        let x0 = hit.center.round() as isize;
        let y0 = y as isize;
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
        let ratio = if c.module > new.module { c.module / new.module } else { new.module / c.module };
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
pub fn find_finders(view: &LumaView, grid: &TileGrid) -> Vec<FinderCandidate> {
    let (w, h) = (view.width(), view.height());
    let scan = Scan { view, grid, w, h };
    let mut merged: Vec<FinderCandidate> = Vec::new();
    let mut row_hits: Vec<RunHit> = Vec::new();
    for y in (0..h).step_by(ROW_STEP) {
        if grid.row_all_skip(y) {
            continue;
        }
        row_hits.clear();
        let bits = (0..w).map(|x| view.get(x, y) < grid.threshold_at(x, y));
        match_row(bits, &mut row_hits);
        for hit in &row_hits {
            if let Some(cand) = scan.verify_hit(hit, y) {
                merge_candidate(&mut merged, cand);
            }
        }
    }
    merged.retain(|c| c.hits >= 2);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(spec: &[(bool, usize)]) -> Vec<bool> {
        spec.iter().flat_map(|&(v, n)| std::iter::repeat_n(v, n)).collect()
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
        match_row(row.iter().copied(), &mut hits);
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
        match_row(row.iter().copied(), &mut hits);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].inverted);
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
        match_row(row.iter().copied(), &mut hits);
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
        match_row(bits(&spec).iter().copied(), &mut hits);
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
