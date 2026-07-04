//! Loads a golden fixture (`fixtures/<name>.json` + `.luma`), runs
//! [`qrk_core::scan`] (Plan 5 Task 6: switched from `detect` so the printed
//! timings actually cover the post-Plan-5 pipeline, including the
//! `refine_ns` stage — `detect` never refines, so that field would always
//! read zero), and prints per-stage timings plus every finder and triplet
//! found. Usage:
//!
//! ```sh
//! cargo run -p qrk-core --example scan_fixture -- near_00
//! ```
//!
//! `max_working_dim: 0` (no downscale) + `refine: true`: these fixtures are
//! already at their intended working resolution, so this measures the same
//! detection path `detect` would, plus refinement against the (identical)
//! source view — `source_scale` stays `1.0`, matching `detect`'s implicit
//! behavior, so this is a pure superset of what the example printed before.
//!
//! Examples build with `[dev-dependencies]` available (unlike the library
//! crate itself), so this reuses `serde`/`serde_json` directly rather than
//! sharing code with `tests/common/mod.rs` — that module compiles into
//! each integration-test binary separately and isn't reachable from an
//! example target.

use std::fs;
use std::path::PathBuf;

use qrk_core::{scan, LumaView, ScanOptions};
use serde::Deserialize;

/// Only the fixture-schema fields this example needs (see
/// `tools/fixtures/generate.py` for the full schema).
#[derive(Deserialize)]
struct Meta {
    width: usize,
    height: usize,
}

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: scan_fixture <fixture-name>");
        std::process::exit(2);
    });

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");

    let json_path = dir.join(format!("{name}.json"));
    let json_text = fs::read_to_string(&json_path)
        .unwrap_or_else(|e| panic!("{}: {e}", json_path.display()));
    let meta: Meta = serde_json::from_str(&json_text)
        .unwrap_or_else(|e| panic!("{}: {e}", json_path.display()));

    let luma_path = dir.join(format!("{name}.luma"));
    let luma = fs::read(&luma_path).unwrap_or_else(|e| panic!("{}: {e}", luma_path.display()));
    assert_eq!(luma.len(), meta.width * meta.height, "{name}: luma buffer size mismatch");

    let view = LumaView::new(&luma, meta.width, meta.height, meta.width)
        .unwrap_or_else(|e| panic!("{name}: invalid LumaView: {e:?}"));

    let opts = ScanOptions { max_working_dim: 0, refine: true };
    let det = scan(&view, &opts);

    println!("fixture: {name} ({}x{})", meta.width, meta.height);
    println!(
        "timings (us): tiles={} finders={} triplets={} version={} alignment={} sample_decode={} refine={}",
        det.timings.tiles_ns / 1_000,
        det.timings.finders_ns / 1_000,
        det.timings.triplets_ns / 1_000,
        det.timings.version_ns / 1_000,
        det.timings.alignment_ns / 1_000,
        det.timings.sample_decode_ns / 1_000,
        det.timings.refine_ns / 1_000,
    );
    println!("finders: {}", det.finders.len());
    println!("triplets: {}", det.triplets.len());
    for (i, t) in det.triplets.iter().enumerate() {
        println!(
            "  [{i}] tl=({:.1},{:.1}) tr=({:.1},{:.1}) bl=({:.1},{:.1}) dimension={} inverted={}",
            t.tl[0], t.tl[1], t.tr[0], t.tr[1], t.bl[0], t.bl[1], t.dimension, t.inverted,
        );
    }
    println!("codes: {}", det.codes.len());
    for (i, c) in det.codes.iter().enumerate() {
        println!(
            "  [{i}] payload={:?} version={} ecc={} mirrored={} dimension={} inverted={} refined_corners={}",
            c.payload, c.version, c.ecc, c.mirrored, c.dimension, c.inverted,
            c.refined_corners.is_some(),
        );
    }
}
