//! Snapshot / drift-gate test for the wasm envelope contract (Plan 3
//! Task 1): `debug-ui`'s TypeScript types are hand-written against a JSON
//! snapshot generated *by this test* from the real `WasmResult` struct —
//! the very one `scan_rgba` serializes for the debug UI — so the
//! cross-language contract can never diverge from what shipped code
//! actually sends over the wire.
//!
//! Run with `UPDATE_SNAPSHOT=1` to (re)generate the committed snapshot
//! after a deliberate shape change:
//!
//! ```sh
//! UPDATE_SNAPSHOT=1 cargo test -p qr-lab-wasm --test envelope_snapshot
//! ```
//!
//! Without the env var, the test reads the committed snapshot and asserts
//! freshly generated JSON matches it byte-for-byte (modulo a trailing
//! newline) — silent drift between Rust and TS fails this test.

use qr_lab_wasm::WasmResult;
use qr_lab::{scan_traced, LumaView, ScanOptions, StageTimings, Trace};
use std::path::PathBuf;

const WIDTH: usize = 1280;
const HEIGHT: usize = 720;
/// The debug UI's default working-resolution cap (`DEFAULT_RESOLUTION` in
/// `debug-ui/src/panels/SourcePanel.tsx`) — near_00 is already exactly this
/// wide, so `scan` takes the no-downscale branch (`source_scale == 1.0`,
/// `scan_width`/`scan_height` == `WIDTH`/`HEIGHT`) and this snapshot's
/// geometry is unchanged from the pre-Plan-5 `detect_with`-based version;
/// the new top-level fields (`detections.source_scale`, `scan_width`,
/// `scan_height`) are the only shape delta Plan 5 Task 1 introduces here.
const MAX_WORKING_DIM: u32 = 1280;

/// Plan 5 Task 3: `refine: true` (not the field's own `false` default) —
/// deliberately, so the committed snapshot (and hence the TS types read off
/// it) sees the POPULATED `refined_corners`/`refine` trace shapes, not just
/// `null`s. Refinement still runs here despite `MAX_WORKING_DIM` producing
/// NO downscale for this fixture (source == working) — see `scan_with`'s
/// own doc for why that's true by design, not an oversight — so this
/// exercises exactly the source-px-refinement-without-downscale path a
/// real no-downscale scan would take.
const REFINE: bool = true;

/// `crates/qr-lab-wasm` -> workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn envelope_matches_committed_snapshot() {
    let luma_path = workspace_root().join("fixtures/near_00.luma");
    let luma = std::fs::read(&luma_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", luma_path.display()));
    assert_eq!(
        luma.len(),
        WIDTH * HEIGHT,
        "{}: expected {WIDTH}x{HEIGHT} raw luma bytes, got {}",
        luma_path.display(),
        luma.len()
    );

    let view = LumaView::new(&luma, WIDTH, HEIGHT, WIDTH)
        .unwrap_or_else(|e| panic!("building LumaView over {}: {e:?}", luma_path.display()));

    // Populate the trace (tiles/finders/triplets) via the same
    // `scan_traced(view, &opts, &mut trace)` path `scan_rgba(with_trace:
    // true)` takes, so the snapshot carries the *populated* trace variant
    // — near_00 has real detections, so the TS types below get non-empty
    // array/struct shapes to check against, not just nulls.
    let mut trace = Trace::new();
    let opts = ScanOptions {
        max_working_dim: MAX_WORKING_DIM,
        refine: REFINE,
    };
    let detections = scan_traced(&view, &opts, &mut trace);
    // `StageTimings` is nondeterministic on every target this test could
    // run on: here (host, not wasm32) it's `Instant::now()` deltas, a
    // different number every run; on the real wasm32 target `StageClock`
    // reads `js_sys::Date::now()` instead (see `qr_lab::StageClock` and
    // `scan_rgba`'s doc comment) — real elapsed time, but still a
    // different number every call, and only ms-resolution at that.
    // Either way, comparing timings byte-for-byte would make the drift
    // gate flaky for reasons that have nothing to do with contract shape,
    // so this test zeroes them before serializing — deterministic, and
    // still representative of the envelope's *shape*, which is the only
    // thing this snapshot is meant to gate.
    let detections = qr_lab::Detections {
        timings: StageTimings::default(),
        ..detections
    };
    // near_00 is exactly `MAX_WORKING_DIM` wide, so `scan` takes the
    // no-downscale branch — `scan_width`/`scan_height` equal `WIDTH`/
    // `HEIGHT` and `detections.source_scale` is `1.0` (visible in the
    // committed JSON).
    let result = WasmResult {
        detections,
        trace: Some(trace),
        scan_width: WIDTH as u32,
        scan_height: HEIGHT as u32,
    };

    let generated = serde_json::to_string_pretty(&result).expect("serialize WasmResult");

    let snapshot_path =
        workspace_root().join("debug-ui/src/scanner/__snapshots__/envelope.near_00.json");

    if std::env::var("UPDATE_SNAPSHOT").as_deref() == Ok("1") {
        let dir = snapshot_path.parent().expect("snapshot path has a parent");
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("creating {}: {e}", dir.display()));
        // Trailing newline so the committed file ends cleanly.
        std::fs::write(&snapshot_path, format!("{generated}\n"))
            .unwrap_or_else(|e| panic!("writing {}: {e}", snapshot_path.display()));
        eprintln!("wrote {}", snapshot_path.display());
        return;
    }

    let committed = std::fs::read_to_string(&snapshot_path).unwrap_or_else(|e| {
        panic!(
            "reading committed snapshot {}: {e}\n\
             (run `UPDATE_SNAPSHOT=1 cargo test -p qr-lab-wasm --test envelope_snapshot` to generate it)",
            snapshot_path.display()
        )
    });

    assert_eq!(
        generated.trim_end(),
        committed.trim_end(),
        "wasm envelope shape drifted from the committed snapshot at {} — if this is an \
         intentional change, regenerate with `UPDATE_SNAPSHOT=1 cargo test -p qr-lab-wasm \
         --test envelope_snapshot` and update debug-ui/src/scanner/types.ts to match",
        snapshot_path.display()
    );
}

/// Plan 6: the robust envelope's committed snapshot — same drift-gate
/// discipline as [`envelope_matches_committed_snapshot`], for
/// `scan_rgba_robust`'s `WasmRobustResult`. Uses `shadow_04` (a measured
/// baseline failure the ladder recovers through a recovery rung) so the
/// snapshot carries a POPULATED multi-variant ladder — several
/// `VariantKind` shapes, a stage>0 provenance on the decoded code, and
/// non-trivial `triplet_evidence` — rather than a single-baseline shell.
/// Capture is OFF (`snapshots` = `null`): the filmstrip carries pixel
/// dumps too large to commit; its field names are pinned by qr-lab-wasm's
/// in-crate `robust_envelope_field_names_are_pinned` unit test instead.
/// The unified `detections` (one-pipeline contract: same shape as the
/// classic envelope's, assembled from the ladder union) IS committed here
/// — it is the primary thing the UI consumes.
#[test]
fn robust_envelope_matches_committed_snapshot() {
    let luma_path = workspace_root().join("fixtures/shadow_04.luma");
    // The generated degraded-fixture pack is optional and intentionally not
    // stored on main. Its committed JSON snapshot still drives the TypeScript
    // parser tests; regenerate this Rust-side snapshot from the fixture branch
    // (or a locally generated pack) when the envelope deliberately changes.
    if !luma_path.exists() {
        return;
    }
    let luma = std::fs::read(&luma_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", luma_path.display()));
    assert_eq!(
        luma.len(),
        WIDTH * HEIGHT,
        "{}: unexpected size",
        luma_path.display()
    );
    let view = LumaView::new(&luma, WIDTH, HEIGHT, WIDTH).unwrap();

    let opts = ScanOptions {
        max_working_dim: MAX_WORKING_DIM,
        refine: REFINE,
    };
    let mut robust = qr_lab::scan_robust(&view, &opts, &qr_lab::ScanConfig::ROBUST_FULL_BENCHMARK);
    // Zero every wall-clock field for the same determinism reason the
    // classic snapshot zeroes StageTimings (see that test's comment).
    robust.total_ns = 0;
    for v in &mut robust.variants {
        v.timings = StageTimings::default();
        v.total_ns = 0;
    }

    // Same per-axis working/source ratios scan_rgba_robust computes — this
    // fixture is exactly MAX_WORKING_DIM wide, so both are 1.0 (the
    // no-downscale branch) and the unified geometry equals the source-px
    // union.
    let detections = qr_lab_wasm::unified_detections(&robust, 1.0, 1.0);
    let result = qr_lab_wasm::WasmRobustResult {
        detections,
        robust,
        snapshots: None,
        scan_width: WIDTH as u32,
        scan_height: HEIGHT as u32,
    };
    let generated = serde_json::to_string_pretty(&result).expect("serialize WasmRobustResult");

    let snapshot_path =
        workspace_root().join("debug-ui/src/scanner/__snapshots__/envelope.robust.shadow_04.json");
    if std::env::var("UPDATE_SNAPSHOT").as_deref() == Ok("1") {
        std::fs::write(&snapshot_path, format!("{generated}\n"))
            .unwrap_or_else(|e| panic!("writing {}: {e}", snapshot_path.display()));
        eprintln!("wrote {}", snapshot_path.display());
        return;
    }
    let committed = std::fs::read_to_string(&snapshot_path).unwrap_or_else(|e| {
        panic!(
            "reading committed snapshot {}: {e}\n\
             (run `UPDATE_SNAPSHOT=1 cargo test -p qr-lab-wasm --test envelope_snapshot` to generate it)",
            snapshot_path.display()
        )
    });
    assert_eq!(
        generated.trim_end(),
        committed.trim_end(),
        "robust wasm envelope shape drifted from {} — if intentional, regenerate with \
         UPDATE_SNAPSHOT=1 and update debug-ui/src/scanner/types.ts to match",
        snapshot_path.display()
    );
}
