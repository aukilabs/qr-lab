//! WebAssembly bindings for `qrk-core`, exposing a single `scan_rgba`
//! entry point that the debug UI (Plan 3) calls with a captured camera
//! frame and gets back detections (and, optionally, the full per-stage
//! trace) as a plain JS object.
//!
//! Plan 5 Task 1: `scan_rgba` gained `max_dim`/`refine` and now owns the
//! source→working downscale in Rust via `qrk_core::scan`/`scan_traced`,
//! rather than `detect`/`detect_traced` directly on an already-downscaled
//! view — the debug UI's TS worker used to downscale before calling this
//! function; it now hands over the full SOURCE frame instead (see
//! `debug-ui/src/scanner/worker.ts`'s doc comment). `WasmResult` gained
//! `scan_width`/`scan_height` so the worker can learn the working
//! resolution `scan` picked without re-deriving it itself.

use qrk_core::{downscaled_dims, luma_from_rgba, scan, scan_traced, Detections, LumaView, ScanOptions, Trace};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Serializable envelope returned to JS: the detection result, the working
/// (post-downscale) resolution `scan` actually detected on, plus the
/// optional debug trace when the caller asked for it.
///
/// `pub` (rather than private or `pub(crate)`) and `#[doc(hidden)]` so
/// `tests/envelope_snapshot.rs` — a separate integration-test crate that
/// only sees this crate's public API — can construct and serialize this
/// *exact* struct instead of hand-rolling a lookalike. That keeps the
/// debug UI's cross-language contract snapshot (Plan 3 Task 1) from ever
/// diverging from what `scan_rgba` actually sends over the wire: if this
/// struct's shape changes, both the wasm binding and the snapshot test
/// change together.
#[derive(Serialize)]
#[doc(hidden)]
pub struct WasmResult {
    pub detections: Detections,
    pub trace: Option<Trace>,
    /// The working view's width/height in px — what `Detections`' own
    /// working-px geometry (and the displayed overlay coordinates) is
    /// relative to. Equal to `width`/`height` (the SOURCE frame passed in)
    /// whenever `detections.source_scale == 1.0`; otherwise the downscaled
    /// dimensions `qrk_core::downscale_luma` computed for `max_dim`.
    pub scan_width: u32,
    pub scan_height: u32,
}

/// Scan one SOURCE-resolution RGBA frame and return a `WasmResult` (via
/// `serde-wasm-bindgen`) as a `JsValue`, or `Err` with a descriptive
/// message on invalid input (a panic would poison the wasm instance for
/// every later call, so all input checks happen up front and nothing on
/// this path can panic). `rgba` must be exactly `width * height * 4`
/// bytes, tightly packed (row stride == width).
///
/// `max_dim` caps the WORKING view's longest side (`0` = no cap — detect
/// directly on the full source, like `qrk_core::detect`); the downscale
/// (when one is needed) happens here, in Rust, via `qrk_core::scan` — see
/// that function's and `ScanOptions`'s doc comments for the exact NN
/// formula and the `source_scale` conversion it produces. `refine` is
/// plumbing-only as of this task (see `ScanOptions::refine`): threaded
/// through so this signature and the debug UI's wire protocol don't need a
/// second breaking change once Plan 5 Task 3 lands refinement.
///
/// When `with_trace` is set, the result carries the full per-stage `Trace`
/// (tiles/finders/triplets); otherwise `trace` is `None` and the
/// trace-recording cost is skipped entirely. Stage timings are real
/// elapsed time on wasm too — `qrk_core::StageClock` backs them with
/// `js_sys::Date::now()`, millisecond-resolution rather than the
/// nanosecond resolution `Instant` gives on native targets, so a fast
/// stage can still read as 0ns. JS-side wall time (e.g. `performance.now()`
/// around the `scan_rgba` call) complements these per-stage numbers with
/// sub-ms precision for the call as a whole; it isn't the only source.
#[wasm_bindgen]
pub fn scan_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    max_dim: u32,
    with_trace: bool,
    refine: bool,
) -> Result<JsValue, JsValue> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 {
        return Err(JsValue::from_str(&format!(
            "scan_rgba: width and height must be non-zero (got {width}x{height})"
        )));
    }
    let expected_len = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| {
            JsValue::from_str(&format!(
                "scan_rgba: width * height * 4 overflows usize ({width}x{height})"
            ))
        })?;
    if rgba.len() != expected_len {
        return Err(JsValue::from_str(&format!(
            "scan_rgba: rgba buffer is {} bytes, expected width * height * 4 = {} \
             ({width}x{height})",
            rgba.len(),
            expected_len
        )));
    }

    let luma = luma_from_rgba(rgba, width, height);
    let view = LumaView::new(&luma, width, height, width)
        .map_err(|e| JsValue::from_str(&format!("scan_rgba: invalid luma view: {e:?}")))?;

    let opts = ScanOptions { max_working_dim: max_dim, refine };

    // Branch on `with_trace` (rather than always calling `scan_traced` with
    // a throwaway `Trace`) so the no-trace path stays exactly as cheap as
    // `scan`'s own doc comment promises: `qrk_core`'s internals never call
    // any `record_*` (with its `to_vec()` clones) unless a trace was
    // actually asked for.
    let mut trace = with_trace.then(Trace::new);
    let detections = match trace.as_mut() {
        Some(t) => scan_traced(&view, &opts, t),
        None => scan(&view, &opts),
    };
    let (scan_width, scan_height) =
        downscaled_dims(width, height, max_dim).unwrap_or((width, height));

    serde_wasm_bindgen::to_value(&WasmResult {
        detections,
        trace,
        scan_width: scan_width as u32,
        scan_height: scan_height as u32,
    })
    .map_err(JsValue::from)
}

#[cfg(test)]
mod tests {
    use super::WasmResult;

    const SIDE: usize = 32;

    fn flat_luma() -> Vec<u8> {
        let rgba = vec![128u8; SIDE * SIDE * 4];
        qrk_core::luma_from_rgba(&rgba, SIDE, SIDE)
    }

    #[test]
    fn scan_result_shape_is_serializable() {
        // Round-trip the result struct through serde_json natively to pin
        // the field names the debug UI will consume — via `qrk_core::scan`
        // (Plan 5 Task 1's entry point, what `scan_rgba` now calls), not
        // the older `detect`, so this test exercises the same shape
        // `source_scale` included.
        let d = vec![128u8; 32 * 32 * 4];
        let luma = qrk_core::luma_from_rgba(&d, 32, 32);
        let view = qrk_core::LumaView::new(&luma, 32, 32, 32).unwrap();
        let opts = qrk_core::ScanOptions { max_working_dim: 0, refine: false };
        let det = qrk_core::scan(&view, &opts);
        let json = serde_json::to_value(&det).unwrap();
        assert!(json.get("finders").is_some());
        assert!(json.get("triplets").is_some());
        assert!(json.get("timings").is_some());
        assert_eq!(json.get("source_scale").and_then(|v| v.as_f64()), Some(1.0));
    }

    #[test]
    fn wasm_result_envelope_without_trace_pins_field_names() {
        let luma = flat_luma();
        let view = qrk_core::LumaView::new(&luma, SIDE, SIDE, SIDE).unwrap();

        let opts = qrk_core::ScanOptions { max_working_dim: 0, refine: false };
        let result = WasmResult {
            detections: qrk_core::scan(&view, &opts),
            trace: None,
            scan_width: SIDE as u32,
            scan_height: SIDE as u32,
        };
        let json = serde_json::to_value(&result).unwrap();

        let det = json.get("detections").expect("detections key");
        assert!(det.get("finders").is_some());
        assert!(det.get("triplets").is_some());
        assert!(det.get("timings").is_some());
        assert!(det.get("source_scale").is_some());
        assert!(json.get("trace").expect("trace key").is_null());
        assert_eq!(json.get("scan_width").and_then(|v| v.as_u64()), Some(SIDE as u64));
        assert_eq!(json.get("scan_height").and_then(|v| v.as_u64()), Some(SIDE as u64));
    }

    #[test]
    fn wasm_result_envelope_with_trace_pins_field_names() {
        let luma = flat_luma();
        let view = qrk_core::LumaView::new(&luma, SIDE, SIDE, SIDE).unwrap();

        let mut trace = qrk_core::Trace::new();
        let opts = qrk_core::ScanOptions { max_working_dim: 0, refine: false };
        let detections = qrk_core::scan_traced(&view, &opts, &mut trace);
        let result = WasmResult {
            detections,
            trace: Some(trace),
            scan_width: SIDE as u32,
            scan_height: SIDE as u32,
        };
        let json = serde_json::to_value(&result).unwrap();

        let det = json.get("detections").expect("detections key");
        assert!(det.get("finders").is_some());
        assert!(det.get("triplets").is_some());
        assert!(det.get("timings").is_some());

        let trace_json = json.get("trace").expect("trace key");
        assert!(trace_json.is_object());
        assert!(trace_json.get("tiles").is_some());
        assert!(trace_json.get("finders").is_some());
        assert!(trace_json.get("triplets").is_some());
    }

    #[test]
    fn scan_none_and_some_trace_agree_on_a_real_fixture() {
        // `find_finders`'s row scan on flat/synthetic images short-circuits
        // too early to exercise most of the detection pipeline, so this
        // loads a real golden fixture (qrk-wasm has no fixture loader of
        // its own — see `tests/common/mod.rs` in qrk-core — so read the
        // raw bytes directly) and checks `scan` (the `scan_rgba` no-trace
        // path) against `scan_traced` (the with-trace path) to confirm the
        // shared orchestration produces identical detections either way.
        // `max_working_dim: 1280` mirrors the debug UI's default working
        // resolution — near_00 is already exactly 1280 wide, so this
        // exercises the no-downscale (`source_scale == 1.0`) branch of
        // `scan`, same as `scan_rgba` would hit for this fixture.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/near_00.luma");
        let luma = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (width, height) = (1280usize, 720usize);
        assert_eq!(luma.len(), width * height, "near_00.luma: unexpected size");
        let view = qrk_core::LumaView::new(&luma, width, height, width).unwrap();

        let opts = qrk_core::ScanOptions { max_working_dim: 1280, refine: false };
        let a = qrk_core::scan(&view, &opts);
        let mut trace = qrk_core::Trace::new();
        let b = qrk_core::scan_traced(&view, &opts, &mut trace);

        assert_eq!(a.finders.len(), b.finders.len());
        assert_eq!(a.triplets.len(), b.triplets.len());
        assert_eq!(a.source_scale, 1.0);
        assert_eq!(b.source_scale, 1.0);
        assert!(!a.finders.is_empty(), "near_00 should yield finder candidates");
    }
}
