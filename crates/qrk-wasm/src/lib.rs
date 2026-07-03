//! WebAssembly bindings for `qrk-core`, exposing a single `scan_rgba`
//! entry point that the debug UI (Plan 3) calls with a captured camera
//! frame and gets back detections (and, optionally, the full per-stage
//! trace) as a plain JS object.

use qrk_core::{detect, detect_traced, luma_from_rgba, Detections, LumaView, Trace};
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
/// as a `JsValue`. `rgba` must be exactly `width * height * 4` bytes,
/// tightly packed (row stride == width). When `with_trace` is set, the
/// result carries the full per-stage `Trace` (tiles/finders/triplets);
/// otherwise `trace` is `None`.
#[wasm_bindgen]
pub fn scan_rgba(rgba: &[u8], width: u32, height: u32, with_trace: bool) -> JsValue {
    let width = width as usize;
    let height = height as usize;
    let luma = luma_from_rgba(rgba, width, height);
    let view = LumaView::new(&luma, width, height, width)
        .expect("scan_rgba: invalid width/height for the given buffer");

    let (detections, trace) = if with_trace {
        let mut trace = Trace::new();
        let detections = detect_traced(&view, &mut trace);
        (detections, Some(trace))
    } else {
        (detect(&view), None)
    };

    serde_wasm_bindgen::to_value(&WasmResult { detections, trace })
        .expect("scan_rgba: WasmResult is always serializable")
}

#[cfg(test)]
mod tests {
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
}
