//! 1:1:3:1:1 run-length pattern matcher: scans a boolean "is dark" stream
//! (one row or column of thresholded pixels) for finder-pattern
//! cross-sections in either polarity. Task 5 builds `find_finders` on top
//! of `match_row` in this same file.

/// A 5-run window that satisfies [`pattern_fits`].
///
/// `center` is the pixel-center coordinate of the middle run's midpoint;
/// `module` is `total / 7` (the estimated module width); `inverted` is
/// true when the middle run is light (a normal finder's middle run is
/// dark); `start..end` is the pixel span of the whole 5-run window.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // consumed by Task 5's find_finders; remove then.
pub(crate) struct RunHit {
    pub center: f64,
    pub module: f64,
    pub inverted: bool,
    pub start: usize,
    pub end: usize,
}

/// Pinned 1:1:3:1:1 variance rules (see Plan 2 "No overfitting" — every
/// tolerance here is a fraction of the estimated module width, not a value
/// tuned against a fixture).
#[allow(dead_code)] // consumed by Task 5's find_finders; remove then.
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
#[allow(dead_code)] // consumed by Task 5's find_finders; remove then.
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
}
