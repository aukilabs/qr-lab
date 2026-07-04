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
//! UPDATE_SNAPSHOT=1 cargo test -p qrk-wasm --test envelope_snapshot
//! ```
//!
//! Without the env var, the test reads the committed snapshot and asserts
//! freshly generated JSON matches it byte-for-byte (modulo a trailing
//! newline) — silent drift between Rust and TS fails this test.

use qrk_core::{detect_with, LumaView, StageTimings, Trace};
use qrk_wasm::WasmResult;
use std::path::PathBuf;

const WIDTH: usize = 1280;
const HEIGHT: usize = 720;

/// `crates/qrk-wasm` -> workspace root.
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
    // `detect_with(view, Some(&mut trace))` path `scan_rgba(with_trace:
    // true)` takes, so the snapshot carries the *populated* trace variant
    // — near_00 has real detections, so the TS types below get non-empty
    // array/struct shapes to check against, not just nulls.
    let mut trace = Trace::new();
    let detections = detect_with(&view, Some(&mut trace));
    // `StageTimings` is nondeterministic on every target this test could
    // run on: here (host, not wasm32) it's `Instant::now()` deltas, a
    // different number every run; on the real wasm32 target `StageClock`
    // reads `js_sys::Date::now()` instead (see `qrk_core::StageClock` and
    // `scan_rgba`'s doc comment) — real elapsed time, but still a
    // different number every call, and only ms-resolution at that.
    // Either way, comparing timings byte-for-byte would make the drift
    // gate flaky for reasons that have nothing to do with contract shape,
    // so this test zeroes them before serializing — deterministic, and
    // still representative of the envelope's *shape*, which is the only
    // thing this snapshot is meant to gate.
    let detections = qrk_core::Detections {
        timings: StageTimings::default(),
        ..detections
    };
    let result = WasmResult {
        detections,
        trace: Some(trace),
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
             (run `UPDATE_SNAPSHOT=1 cargo test -p qrk-wasm --test envelope_snapshot` to generate it)",
            snapshot_path.display()
        )
    });

    assert_eq!(
        generated.trim_end(),
        committed.trim_end(),
        "wasm envelope shape drifted from the committed snapshot at {} — if this is an \
         intentional change, regenerate with `UPDATE_SNAPSHOT=1 cargo test -p qrk-wasm \
         --test envelope_snapshot` and update debug-ui/src/scanner/types.ts to match",
        snapshot_path.display()
    );
}
