mod common;

// `LumaView` is unused by name here (`fx.view()` returns it, but the type
// is never spelled out) — the import list is verbatim from the task-5
// brief; silence rather than trim it so the test text matches the brief.
#[allow(unused_imports)]
use qrk_core::{find_finders, LumaView, PerspectiveTransform, TileGrid};

use common::expected_finder_centers as expected_centers;

#[test]
fn every_ground_truth_finder_is_detected() {
    let mut missed: Vec<String> = Vec::new();
    let mut worst_fp = 0usize;
    for fx in common::load_all() {
        let view = fx.view();
        let grid = TileGrid::build(&view);
        let found = find_finders(&view, &grid);
        worst_fp = worst_fp.max(found.len());
        for c in &fx.codes {
            let tol = c.module_size_px.max(2.0);
            for (k, e) in expected_centers(c).iter().enumerate() {
                let best = found
                    .iter()
                    .map(|f| ((f.x - e[0]).powi(2) + (f.y - e[1]).powi(2)).sqrt())
                    .fold(f64::INFINITY, f64::min);
                if best > tol {
                    missed.push(format!(
                        "{} code v{} finder {k}: nearest {best:.2}px (tol {tol:.2})",
                        fx.name, c.version));
                }
            }
        }
    }
    assert!(missed.is_empty(), "missed finders:\n{}", missed.join("\n"));
    // Candidate-explosion guard, not a precision gate.
    assert!(worst_fp < 600, "candidate explosion: {worst_fp}");
}
