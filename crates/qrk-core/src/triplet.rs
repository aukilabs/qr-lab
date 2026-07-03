//! Groups verified finder-pattern candidates (Task 5's `FinderCandidate`)
//! into triplets that plausibly form one QR code's three finder corners,
//! and estimates the code's module dimension from the triplet's geometry.
//! Every tolerance below is derived from QR geometry or the fixture
//! suite's perspective envelope — see the Plan 2 Task 6 algorithm contract
//! (task-6-brief.md) and task-6-report.md for the full derivations.

use crate::finder::FinderCandidate;

/// A grouped triplet, canonically ordered `tl`/`tr`/`bl` (cross product
/// `(tr-tl)x(bl-tl) > 0` in y-down image coordinates — normal reading
/// order), plus the dimension estimate derived from the triplet's leg
/// lengths.
#[derive(Clone, Copy, Debug)]
pub struct TripletCandidate {
    pub tl: [f64; 2],
    pub tr: [f64; 2],
    pub bl: [f64; 2],
    pub module: f64,
    pub dimension: u32,
    pub snap_error: f64,
    pub inverted: bool,
}

/// `|cos|` of the between-leg angle at a candidate corner must be at most
/// this to accept the corner as (near) a right angle. `arccos(0.4) ≈
/// 66.4°`, so the admitted included angle spans `[66.4°, 113.6°]` — i.e.
/// ±23.6° either side of 90°. The fixture suite's steepest scenario
/// (`tilt45_*`) tilts the code plane up to 45° from fronto-parallel; a
/// camera-plane right angle projected through a 45° tilt about an in-plane
/// axis foreshortens one leg but does not, by itself, rotate the *angle*
/// between the two legs anywhere near that far off 90° (foreshortening is
/// captured separately by the leg-balance check below) — the ±23.6°
/// budget here is headroom for the combined effect of tilt plus whatever
/// in-plane rotation and detector jitter perturb the corner's measured
/// vertex position. Comfortably excludes collinear/near-collinear triples
/// (`|cos|` near 1).
const MAX_ABS_COS: f64 = 0.4;

/// Leg balance: the shorter leg must be at least this fraction of the
/// longer one (`|1 - shorter/longer| ≤ 0.5` ⇔ `shorter/longer ≥ 0.5`). A
/// 45° perspective tilt foreshortens the far leg by a factor of
/// `cos(45°) ≈ 0.707` relative to the near one, i.e. `|1-0.707| ≈ 0.293`;
/// 0.5 keeps that margin plus headroom for combined tilt + in-plane
/// rotation without admitting a leg pair skewed enough to indicate a
/// wrong-corner pairing.
const MAX_LEG_IMBALANCE: f64 = 0.5;

/// Pairwise finder module-size ratio must be at most this. A single QR
/// code's three finders share one physical module size; under perspective
/// the nearer finder can read larger than the farther one, but the
/// fixture suite's steepest single-code perspective does not spread a
/// genuine triple's module estimates beyond 1.5x. A larger spread is
/// treated as evidence the three candidates come from different codes
/// (cross-code noise in multi-code frames).
const MAX_MODULE_RATIO: f64 = 1.5;

/// The two per-leg dimension estimates (`round(leg/module) + 7`) from one
/// triplet must agree within this many modules, or the legs are not both
/// measuring the same code's side (mismatched/degenerate grouping).
const MAX_DIM_DISAGREEMENT: i64 = 4;

/// QR Model 2 dimension range: version 1..=40 -> `21 + 4*(v-1)` spans
/// `[21, 177]` (the QR spec).
const MIN_DIMENSION: i64 = 21;
const MAX_DIMENSION: i64 = 177;

/// Output cap: bounds worst-case downstream decode work per frame.
const MAX_TRIPLETS: usize = 64;

/// `a`/`b` as the larger-over-smaller ratio (always ≥ 1), so one threshold
/// works regardless of which of `a`/`b` is bigger.
fn ratio_ge1(a: f64, b: f64) -> f64 {
    if a > b {
        a / b
    } else {
        b / a
    }
}

/// `|cos|` of the angle between legs `corner->p` and `corner->q`; `1.0`
/// (treated as maximally non-perpendicular, i.e. rejected downstream) if
/// either leg is degenerate (zero length).
fn abs_cos_angle(corner: [f64; 2], p: [f64; 2], q: [f64; 2]) -> f64 {
    let v1 = [p[0] - corner[0], p[1] - corner[1]];
    let v2 = [q[0] - corner[0], q[1] - corner[1]];
    let dot = v1[0] * v2[0] + v1[1] * v2[1];
    let n1 = (v1[0] * v1[0] + v1[1] * v1[1]).sqrt();
    let n2 = (v2[0] * v2[0] + v2[1] * v2[1]).sqrt();
    if n1 == 0.0 || n2 == 0.0 {
        return 1.0;
    }
    (dot / (n1 * n2)).abs()
}

/// Legs `|corner->p|`, `|corner->q|` as shorter-over-longer (`<= 1.0`), so
/// `1.0 - ratio` is the fractional imbalance regardless of which leg is
/// longer.
fn leg_balance(corner: [f64; 2], p: [f64; 2], q: [f64; 2]) -> f64 {
    let a = ((p[0] - corner[0]).powi(2) + (p[1] - corner[1]).powi(2)).sqrt();
    let b = ((q[0] - corner[0]).powi(2) + (q[1] - corner[1]).powi(2)).sqrt();
    if a > b {
        b / a
    } else {
        a / b
    }
}

/// Snap `mean` to the nearest integer that is `≡ 1 (mod 4)` — the QR
/// dimension congruence `21, 25, 29, ..., 177` — returning
/// `(snapped, |mean - snapped|)`.
fn snap_dimension(mean: f64) -> (i64, f64) {
    let base = ((mean - 1.0) / 4.0).round() as i64;
    let snapped = base * 4 + 1;
    (snapped, (mean - snapped as f64).abs())
}

/// Evaluate one unordered candidate triple: polarity + module-ratio gate,
/// corner identification, angle + leg-balance gate, canonical ordering,
/// then the dimension estimate + its gates. `None` if any gate fails.
fn try_group(a: FinderCandidate, b: FinderCandidate, c: FinderCandidate) -> Option<TripletCandidate> {
    if a.inverted != b.inverted || b.inverted != c.inverted {
        return None;
    }
    if ratio_ge1(a.module, b.module) > MAX_MODULE_RATIO
        || ratio_ge1(b.module, c.module) > MAX_MODULE_RATIO
        || ratio_ge1(a.module, c.module) > MAX_MODULE_RATIO
    {
        return None;
    }

    let pts = [[a.x, a.y], [b.x, b.y], [c.x, c.y]];

    // Corner = the finder whose two legs have the smallest |cos| between
    // them (closest to perpendicular), chosen once and independent of the
    // leg-balance/angle thresholds below.
    let mut corner_idx = 0usize;
    let mut best_cos = f64::INFINITY;
    for i in 0..3 {
        let (p, q) = match i {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let cos = abs_cos_angle(pts[i], pts[p], pts[q]);
        if cos < best_cos {
            best_cos = cos;
            corner_idx = i;
        }
    }
    if best_cos > MAX_ABS_COS {
        return None;
    }

    let (p_idx, q_idx) = match corner_idx {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    if leg_balance(pts[corner_idx], pts[p_idx], pts[q_idx]) < 1.0 - MAX_LEG_IMBALANCE {
        return None;
    }

    let tl = pts[corner_idx];
    let (mut tr, mut bl) = (pts[p_idx], pts[q_idx]);
    let cross = (tr[0] - tl[0]) * (bl[1] - tl[1]) - (tr[1] - tl[1]) * (bl[0] - tl[0]);
    // cross == 0.0 (collinear) cannot reach here: a collinear corner has
    // |cos| == 1, already rejected by the MAX_ABS_COS gate above.
    if cross < 0.0 {
        std::mem::swap(&mut tr, &mut bl);
    }

    let module = (a.module + b.module + c.module) / 3.0;
    let leg_tr = ((tr[0] - tl[0]).powi(2) + (tr[1] - tl[1]).powi(2)).sqrt();
    let leg_bl = ((bl[0] - tl[0]).powi(2) + (bl[1] - tl[1]).powi(2)).sqrt();
    let dim_tr = (leg_tr / module).round() as i64 + 7;
    let dim_bl = (leg_bl / module).round() as i64 + 7;
    if (dim_tr - dim_bl).abs() > MAX_DIM_DISAGREEMENT {
        return None;
    }
    let mean = (dim_tr + dim_bl) as f64 / 2.0;
    let (snapped, snap_error) = snap_dimension(mean);
    if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&snapped) {
        return None;
    }

    Some(TripletCandidate {
        tl,
        tr,
        bl,
        module,
        dimension: snapped as u32,
        snap_error,
        inverted: a.inverted,
    })
}

/// Groups `finders` into every plausible finder triplet: tests each
/// unordered triple against [`try_group`]'s gates, then sorts survivors by
/// ascending `snap_error` and caps the result at [`MAX_TRIPLETS`].
pub fn group_triplets(finders: &[FinderCandidate]) -> Vec<TripletCandidate> {
    let n = finders.len();
    let mut out = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                if let Some(t) = try_group(finders[i], finders[j], finders[k]) {
                    out.push(t);
                }
            }
        }
    }
    out.sort_by(|x, y| x.snap_error.partial_cmp(&y.snap_error).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(MAX_TRIPLETS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finder::FinderCandidate;

    fn f(x: f64, y: f64) -> FinderCandidate {
        FinderCandidate { x, y, module: 4.0, inverted: false, hits: 3 }
    }

    #[test]
    fn groups_axis_aligned_v1() {
        // v1: n=21, leg = 14 modules = 56px.
        let t = group_triplets(&[f(100.0, 100.0), f(156.0, 100.0), f(100.0, 156.0)]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].dimension, 21);
        assert_eq!(t[0].tl, [100.0, 100.0]);
        assert_eq!(t[0].tr, [156.0, 100.0]);
        assert_eq!(t[0].bl, [100.0, 156.0]);
        assert!(t[0].snap_error < 0.6);
    }

    #[test]
    fn canonicalizes_mirrored_order() {
        // Same three points fed so the natural order is mirrored;
        // output must still satisfy the cross-product convention.
        let t = group_triplets(&[f(100.0, 100.0), f(100.0, 156.0), f(156.0, 100.0)]);
        assert_eq!(t.len(), 1);
        let (tl, tr, bl) = (t[0].tl, t[0].tr, t[0].bl);
        let cross = (tr[0] - tl[0]) * (bl[1] - tl[1])
            - (tr[1] - tl[1]) * (bl[0] - tl[0]);
        assert!(cross > 0.0);
    }

    #[test]
    fn rejects_mixed_polarity_and_bad_geometry() {
        let mut inv = f(156.0, 100.0);
        inv.inverted = true;
        assert!(group_triplets(&[f(100.0, 100.0), inv, f(100.0, 156.0)]).is_empty());
        // Collinear points: no ~90° corner.
        assert!(group_triplets(&[f(0.0, 0.0), f(50.0, 0.0), f(100.0, 0.0)]).is_empty());
        // Legs too unbalanced (14 vs 40 modules).
        assert!(group_triplets(&[f(0.0, 0.0), f(56.0, 0.0), f(0.0, 160.0)]).is_empty());
    }

    #[test]
    fn rotated_45_still_groups() {
        let t = group_triplets(&[
            f(200.0, 200.0),
            f(200.0 + 39.6, 200.0 + 39.6),
            f(200.0 - 39.6, 200.0 + 39.6),
        ]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].dimension, 21);
    }
}
