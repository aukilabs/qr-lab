mod common;

// `LumaView` is unused by name (`fx.view()` returns it without the type
// ever being spelled out) — the import list is verbatim from the
// task-6 brief; silence rather than trim it so the test text matches
// the brief (same treatment as `finder_gate.rs`'s identical import).
#[allow(unused_imports)]
use qrk_core::{find_finders, group_triplets, LumaView, TileGrid};

#[test]
fn every_code_yields_a_matching_triplet() {
    let mut missed = Vec::new();
    for fx in common::load_all() {
        let view = fx.view();
        let grid = TileGrid::build(&view);
        let trips = group_triplets(&find_finders(&view, &grid));
        for c in &fx.codes {
            let exp = common::expected_finder_centers(c);
            let tol = c.module_size_px.max(2.0);
            let n = 4 * c.version + 17;
            let ok = trips.iter().any(|t| {
                t.inverted == c.inverted
                    && (t.dimension as i64 - n as i64).abs() <= 4
                    && [t.tl, t.tr, t.bl].iter().all(|p| {
                        exp.iter().any(|e| {
                            ((p[0] - e[0]).powi(2) + (p[1] - e[1]).powi(2)).sqrt() <= tol
                        })
                    })
            });
            if !ok {
                missed.push(format!("{} v{}: no matching triplet ({} cands)",
                                    fx.name, c.version, trips.len()));
            }
        }
    }
    assert!(missed.is_empty(), "unmatched codes:\n{}", missed.join("\n"));
}
