mod common;

#[test]
fn suite_loads_and_ground_truth_is_sane() {
    let fixtures = common::load_all();
    assert!(fixtures.len() >= 81, "got {}", fixtures.len());

    let mut multi_max = 0;
    for f in &fixtures {
        let v = f.view();
        assert_eq!(v.width(), f.width);
        assert_eq!(v.height(), f.height);
        assert!(!f.codes.is_empty());
        multi_max = multi_max.max(f.codes.len());
        for c in &f.codes {
            assert!((1..=40).contains(&c.version));
            for [x, y] in c.corners_px {
                assert!(x > 0.0 && x < f.width as f64 - 1.0);
                assert!(y > 0.0 && y < f.height as f64 - 1.0);
            }
            if f.degraded {
                // Plan 6 degraded families deliberately go below the nominal
                // module floor (lowres_ targets 1.4 px/module — sub-Nyquist
                // headroom probes); only pin that the value is physical.
                assert!(c.module_size_px > 1.0, "{}: {}", f.name, c.module_size_px);
            } else {
                assert!(c.module_size_px > 1.5, "{}: {}", f.name, c.module_size_px);
            }
        }
    }
    assert_eq!(multi_max, 4, "multi_ scenarios must reach 4 codes");
}

#[test]
fn luma_pixels_match_ground_truth_ink() {
    // For every code: probe 0.5*module Euclidean along the TL->BR diagonal
    // (0.35 modules per axis) — inside is dark finder ink, outside is the
    // light quiet zone. (1.5*module would overshoot the finder's 1-module
    // dark outer ring into the white second ring.)
    // Golden fixtures only: Plan 6 degradations (occlusion over the TL
    // finder, deep shadow, glare) legitimately break these probes — that is
    // what they are for.
    for f in common::load_golden() {
        let v = f.view();
        for c in &f.codes {
            let tl = c.corners_px[0];
            let br = c.corners_px[2];
            let len = ((br[0] - tl[0]).powi(2) + (br[1] - tl[1]).powi(2)).sqrt();
            let dir = [(br[0] - tl[0]) / len, (br[1] - tl[1]) / len];
            let m = c.module_size_px * 0.5;
            let inside = v.get(
                (tl[0] + dir[0] * m).round() as usize,
                (tl[1] + dir[1] * m).round() as usize,
            );
            let outside = v.get(
                (tl[0] - dir[0] * m).round() as usize,
                (tl[1] - dir[1] * m).round() as usize,
            );
            // Non-inverted keeps the proven <110 bound (far_* modules are
            // ~3.6 px; blur bleed rules out a tighter one). Inverted
            // scenarios are all near (modules >=5.5 px), so >170 is safe.
            if c.inverted {
                assert!(inside > 170, "{}: inside={}", f.name, inside);
            } else {
                assert!(inside < 110, "{}: inside={}", f.name, inside);
            }
            match (c.opaque_plate, c.inverted) {
                (true, false) => assert!(outside > 150, "{}: out={}", f.name, outside),
                (true, true) => assert!(outside < 110, "{}: out={}", f.name, outside),
                (false, _) => assert!(outside > 95 && outside < 165, "{}: out={}", f.name, outside),
            }
        }
    }
}
