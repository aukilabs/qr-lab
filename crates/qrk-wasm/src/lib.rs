//! WebAssembly bindings for `qrk-core`, exposing a single `scan_rgba`
//! entry point that the debug UI (Plan 3) calls with a captured camera
//! frame and gets back detections (and, optionally, the full per-stage
//! trace) as a plain JS object.

use qrk_core::{detect_with, luma_from_rgba, Detections, LumaView, Trace};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Serializable envelope returned to JS: the detection result, plus the
/// optional debug trace when the caller asked for it.
#[derive(Serialize)]
struct WasmResult {
    detections: Detections,
    trace: Option<Trace>,
}

/// Scan one RGBA frame and return a `WasmResult` (via `serde-wasm-bindgen`)
/// as a `JsValue`, or `Err` with a descriptive message on invalid input
/// (a panic would poison the wasm instance for every later call, so all
/// input checks happen up front and nothing on this path can panic).
/// `rgba` must be exactly `width * height * 4` bytes, tightly packed (row
/// stride == width). When `with_trace` is set, the result carries the full
/// per-stage `Trace` (tiles/finders/triplets); otherwise `trace` is `None`
/// and the trace-recording cost is skipped entirely. Stage timings are
/// zero on wasm (see `qrk_core::StageClock`) — measure wall time JS-side.
#[wasm_bindgen]
pub fn scan_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
    with_trace: bool,
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

    // `qrk_core::detect_with` is the single orchestration both `detect`
    // and `detect_traced` funnel through; it only pays `Trace`'s
    // record/clone cost when `trace` is `Some`, so passing `None` here
    // (this crate compiles `qrk-core` with `debug-trace`) is as cheap as
    // the old crate-local `detect_untraced` duplicate used to be —
    // without needing that duplicate.
    let mut trace = with_trace.then(Trace::new);
    let detections = detect_with(&view, trace.as_mut());

    serde_wasm_bindgen::to_value(&WasmResult { detections, trace }).map_err(JsValue::from)
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
        // the field names the debug UI will consume.
        let d = vec![128u8; 32 * 32 * 4];
        let luma = qrk_core::luma_from_rgba(&d, 32, 32);
        let view = qrk_core::LumaView::new(&luma, 32, 32, 32).unwrap();
        let det = qrk_core::detect(&view);
        let json = serde_json::to_value(&det).unwrap();
        assert!(json.get("finders").is_some());
        assert!(json.get("triplets").is_some());
        assert!(json.get("timings").is_some());
    }

    #[test]
    fn wasm_result_envelope_without_trace_pins_field_names() {
        let luma = flat_luma();
        let view = qrk_core::LumaView::new(&luma, SIDE, SIDE, SIDE).unwrap();

        let result = WasmResult {
            detections: qrk_core::detect_with(&view, None),
            trace: None,
        };
        let json = serde_json::to_value(&result).unwrap();

        let det = json.get("detections").expect("detections key");
        assert!(det.get("finders").is_some());
        assert!(det.get("triplets").is_some());
        assert!(det.get("timings").is_some());
        assert!(json.get("trace").expect("trace key").is_null());
    }

    #[test]
    fn wasm_result_envelope_with_trace_pins_field_names() {
        let luma = flat_luma();
        let view = qrk_core::LumaView::new(&luma, SIDE, SIDE, SIDE).unwrap();

        let mut trace = qrk_core::Trace::new();
        let detections = qrk_core::detect_traced(&view, &mut trace);
        let result = WasmResult {
            detections,
            trace: Some(trace),
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
    fn detect_with_none_and_some_trace_agree_on_a_real_fixture() {
        // `find_finders`'s row scan on flat/synthetic images short-circuits
        // too early to exercise most of the detection pipeline, so this
        // loads a real golden fixture (qrk-wasm has no fixture loader of
        // its own — see `tests/common/mod.rs` in qrk-core — so read the
        // raw bytes directly) and checks `detect_with(None)` (the
        // `scan_rgba` no-trace path) against `detect_with(Some(..))` (the
        // with-trace path) to confirm the shared orchestration produces
        // identical detections either way.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/near_00.luma");
        let luma = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (width, height) = (1280usize, 720usize);
        assert_eq!(luma.len(), width * height, "near_00.luma: unexpected size");
        let view = qrk_core::LumaView::new(&luma, width, height, width).unwrap();

        let a = qrk_core::detect_with(&view, None);
        let mut trace = qrk_core::Trace::new();
        let b = qrk_core::detect_with(&view, Some(&mut trace));

        assert_eq!(a.finders.len(), b.finders.len());
        assert_eq!(a.triplets.len(), b.triplets.len());
        assert!(!a.finders.is_empty(), "near_00 should yield finder candidates");
    }
}
