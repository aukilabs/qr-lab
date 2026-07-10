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
//! resolution `scan` picked without re-deriving it itself. Plan 5 Task 3:
//! `refine` now enables real subpixel corner refinement (previously
//! plumbing-only) — no signature change needed here, since the field
//! already existed. Plan 5 Task 5: `qrgen` (feature `qr-gen`, off by
//! default) adds `generate_qr` — the debug UI's 3D-scene mode uses it to
//! texture a plane with a real, decodable QR — see that module's doc.

use qrk_core::{
    downscaled_dims, luma_from_rgba, scan, scan_robust, scan_robust_debug, scan_traced, Detections,
    LumaView, RobustDetections, ScanConfig, ScanOptions, Trace, VariantKind,
};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[cfg(feature = "qr-gen")]
mod qrgen;
#[cfg(feature = "qr-gen")]
pub use qrgen::generate_qr;

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
/// formula and the `source_scale` conversion it produces. `refine` enables
/// subpixel corner refinement (Plan 5 Task 3, see `ScanOptions::refine`'s
/// doc): each decoded code's `refined_corners` is then populated (source
/// px) instead of staying `None`, at the cost of the extra per-code edge
/// probing/fitting work (visible in `StageTimings::refine_ns`).
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

    let opts = ScanOptions {
        max_working_dim: max_dim,
        refine,
    };

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

/// JS-side robustness config (Plan 6): the camelCase mirror of
/// `qrk_core::ScanConfig`, deserialized from the plain object the debug UI
/// sends (`serde_wasm_bindgen::from_value` — the first JS→wasm struct in
/// this crate; everything before Plan 6 crossed as flat scalars).
/// `#[serde(default)]` on the container means every omitted field is the
/// all-off default, so `{}` (or `undefined`, handled in `scan_rgba_robust`)
/// is exactly `ScanConfig::default()` — baseline behavior.
///
/// Serialize is also derived so [`robust_presets`] can hand the SAME shape
/// back to JS — the UI reads preset flag values from Rust instead of
/// duplicating them (no cross-language drift).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[doc(hidden)]
pub struct RobustConfig {
    pub enable_multi_scale: bool,
    pub enable_contrast_normalization: bool,
    pub enable_shadow_normalization: bool,
    pub enable_adaptive_thresholding: bool,
    pub enable_sharpening: bool,
    pub enable_deblur: bool,
    pub enable_low_res_upscaling: bool,
    pub max_variants_per_frame: u32,
    pub enable_early_exit: bool,
}

impl RobustConfig {
    fn to_core(self) -> ScanConfig {
        ScanConfig {
            enable_multi_scale: self.enable_multi_scale,
            enable_contrast_normalization: self.enable_contrast_normalization,
            enable_shadow_normalization: self.enable_shadow_normalization,
            enable_adaptive_thresholding: self.enable_adaptive_thresholding,
            enable_sharpening: self.enable_sharpening,
            enable_deblur: self.enable_deblur,
            enable_low_res_upscaling: self.enable_low_res_upscaling,
            max_variants_per_frame: self.max_variants_per_frame,
            enable_early_exit: self.enable_early_exit,
        }
    }

    fn from_core(c: ScanConfig) -> Self {
        RobustConfig {
            enable_multi_scale: c.enable_multi_scale,
            enable_contrast_normalization: c.enable_contrast_normalization,
            enable_shadow_normalization: c.enable_shadow_normalization,
            enable_adaptive_thresholding: c.enable_adaptive_thresholding,
            enable_sharpening: c.enable_sharpening,
            enable_deblur: c.enable_deblur,
            enable_low_res_upscaling: c.enable_low_res_upscaling,
            max_variants_per_frame: c.max_variants_per_frame,
            enable_early_exit: c.enable_early_exit,
        }
    }
}

/// The named `ScanConfig` presets, exposed so the debug UI's preset picker
/// reads the authoritative Rust values (`qrk_core::ScanConfig::{BASELINE,
/// ROBUST_FAST, ROBUST_FULL_BENCHMARK}`) instead of hard-coding copies that
/// would silently drift.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[doc(hidden)]
pub struct RobustPresets {
    pub baseline: RobustConfig,
    pub robust_fast: RobustConfig,
    pub robust_full_benchmark: RobustConfig,
}

/// Returns [`RobustPresets`] as a plain JS object (see its doc).
#[wasm_bindgen]
pub fn robust_presets() -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&RobustPresets {
        baseline: RobustConfig::from_core(ScanConfig::BASELINE),
        robust_fast: RobustConfig::from_core(ScanConfig::ROBUST_FAST),
        robust_full_benchmark: RobustConfig::from_core(ScanConfig::ROBUST_FULL_BENCHMARK),
    })
    .map_err(JsValue::from)
}

/// One ladder variant's buffer thumbnail (Plan 6 debug capture): what the
/// scanner actually saw at that rung, box-averaged to ≤320 px on the longest
/// side by `qrk_core::scan_robust_debug`. `luma` crosses to JS as a single
/// `Uint8Array` (`serde_bytes`), tightly packed, `width * height` bytes.
#[derive(Serialize)]
#[doc(hidden)]
pub struct WasmSnapshot {
    /// Same serde shape as `RobustDetections`' own `VariantKind` values
    /// (externally tagged enum: `"Baseline"` or `{ "Pyramid": { "level": 1 } }`)
    /// so the UI needs exactly one kind-parser for records, codes, and
    /// snapshots.
    pub kind: VariantKind,
    pub stage: u8,
    pub width: u32,
    pub height: u32,
    #[serde(with = "serde_bytes")]
    pub luma: Vec<u8>,
}

/// Serializable envelope for [`scan_rgba_robust`] — the Plan 6 counterpart
/// of [`WasmResult`], pinned the same way (`tests/envelope_snapshot.rs`;
/// `pub` + `#[doc(hidden)]` for the same reason as [`WasmResult`]).
///
/// Coordinate spaces, stated for the UI's benefit: `robust.codes[i]
/// .corners_source` / `.refined_corners_source` / `robust.triplet_evidence`
/// are SOURCE px (the full frame handed to `scan_rgba_robust`); each
/// `robust.codes[i].code.corners` stays in that code's own VARIANT working
/// px (do not draw those directly). `baseline`'s geometry is baseline
/// working px — `scan_width`/`scan_height`, same convention as
/// [`WasmResult`].
#[derive(Serialize)]
#[doc(hidden)]
pub struct WasmRobustResult {
    /// The unified pipeline result — SAME shape and conventions as
    /// [`WasmResult`]'s `detections` (finders/triplets/codes in WORKING px,
    /// `source_scale`, baseline stage timings), assembled from the ladder's
    /// cross-variant union so every classic consumer (overlays, timings)
    /// works identically in robust mode. Robust mode is one pipeline with
    /// optional features: turning flags on shows up here as MORE
    /// finders/triplets/codes, nothing else changes. Per-code
    /// `refined_corners` stay SOURCE px (the [`Detections`] contract);
    /// `finder_indices` on codes/triplets reference their ORIGIN variant's
    /// candidate list, not `finders` here — provenance only.
    pub detections: Detections,
    /// The ladder metadata: per-variant records, per-code provenance +
    /// SOURCE-px geometry, triplet evidence, early-exit/budget flags.
    pub robust: RobustDetections,
    /// The ladder filmstrip — present only when `capture` was requested;
    /// one entry per `robust.variants` record, same order.
    pub snapshots: Option<Vec<WasmSnapshot>>,
    pub scan_width: u32,
    pub scan_height: u32,
}

/// Assemble the unified [`Detections`] from a ladder result (see
/// [`WasmRobustResult::detections`]'s doc): the robust union's SOURCE-px
/// geometry multiplied into baseline-working px per axis, codes' coarse
/// corners likewise (refined corners are already source px and pass
/// through), stage timings from the baseline variant. `pub` +
/// `#[doc(hidden)]` for the same snapshot-test reason as [`WasmResult`].
#[doc(hidden)]
pub fn unified_detections(robust: &RobustDetections, sx: f64, sy: f64) -> Detections {
    Detections {
        finders: robust
            .finders
            .iter()
            .map(|f| qrk_core::FinderCandidate {
                x: f.x * sx,
                y: f.y * sy,
                module: f.module * sx,
                inverted: f.inverted,
                hits: f.hits,
            })
            .collect(),
        triplets: robust
            .triplets
            .iter()
            .map(|t| qrk_core::TripletCandidate {
                tl: [t.tl[0] * sx, t.tl[1] * sy],
                tr: [t.tr[0] * sx, t.tr[1] * sy],
                bl: [t.bl[0] * sx, t.bl[1] * sy],
                module: t.module * sx,
                dimension: t.dimension,
                snap_error: t.snap_error,
                inverted: t.inverted,
                finder_indices: t.finder_indices,
            })
            .collect(),
        codes: robust
            .codes
            .iter()
            .map(|c| qrk_core::DecodedCode {
                corners: c.corners_source.map(|p| [p[0] * sx, p[1] * sy]),
                refined_corners: c.refined_corners_source,
                ..c.code.clone()
            })
            .collect(),
        timings: robust
            .variants
            .first()
            .map(|v| v.timings)
            .unwrap_or_default(),
        source_scale: sx,
    }
}

/// Scan one SOURCE-resolution RGBA frame through the Plan 6 adaptive
/// escalation ladder (`qrk_core::scan_robust`) and return a
/// [`WasmRobustResult`]. `rgba`/`width`/`height`/`max_dim`/`refine` behave
/// exactly as in [`scan_rgba`] (same validation, same panic-free error
/// path). `config` is a plain JS object matching [`RobustConfig`]
/// (camelCase; missing fields default to off — pass
/// `robust_presets().robustFast` etc. for a preset); `undefined`/`null` is
/// accepted as "all defaults".
///
/// `capture: true` switches to `qrk_core::scan_robust_debug` — identical
/// detection results, plus the per-variant `snapshots` filmstrip (one
/// box-downscale chain + ~58 KB pixel payload per variant; leave it off
/// for video-rate scanning — everything else, including the unified
/// `detections`, is always present).
#[wasm_bindgen]
pub fn scan_rgba_robust(
    rgba: &[u8],
    width: u32,
    height: u32,
    max_dim: u32,
    refine: bool,
    config: JsValue,
    capture: bool,
) -> Result<JsValue, JsValue> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 {
        return Err(JsValue::from_str(&format!(
            "scan_rgba_robust: width and height must be non-zero (got {width}x{height})"
        )));
    }
    let expected_len = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| {
            JsValue::from_str(&format!(
                "scan_rgba_robust: width * height * 4 overflows usize ({width}x{height})"
            ))
        })?;
    if rgba.len() != expected_len {
        return Err(JsValue::from_str(&format!(
            "scan_rgba_robust: rgba buffer is {} bytes, expected width * height * 4 = {} \
             ({width}x{height})",
            rgba.len(),
            expected_len
        )));
    }
    let cfg = if config.is_undefined() || config.is_null() {
        RobustConfig::default()
    } else {
        serde_wasm_bindgen::from_value(config)
            .map_err(|e| JsValue::from_str(&format!("scan_rgba_robust: invalid config: {e}")))?
    }
    .to_core();

    let luma = luma_from_rgba(rgba, width, height);
    let view = LumaView::new(&luma, width, height, width)
        .map_err(|e| JsValue::from_str(&format!("scan_rgba_robust: invalid luma view: {e:?}")))?;
    let opts = ScanOptions {
        max_working_dim: max_dim,
        refine,
    };

    let (robust, snapshots) = if capture {
        let debug = scan_robust_debug(&view, &opts, &cfg);
        let snapshots = debug
            .snapshots
            .into_iter()
            .map(|s| WasmSnapshot {
                kind: s.kind,
                stage: s.stage,
                width: s.width as u32,
                height: s.height as u32,
                luma: s.luma,
            })
            .collect();
        (debug.detections, Some(snapshots))
    } else {
        (scan_robust(&view, &opts, &cfg), None)
    };

    build_robust_envelope(robust, snapshots, width, height, max_dim)
}

/// Shared tail of [`scan_rgba_robust`] and [`WasmScanSession::scan_frame_rgba`]:
/// derive the working dims, assemble the unified [`Detections`], and
/// serialize the [`WasmRobustResult`] envelope.
fn build_robust_envelope(
    robust: RobustDetections,
    snapshots: Option<Vec<WasmSnapshot>>,
    width: usize,
    height: usize,
    max_dim: u32,
) -> Result<JsValue, JsValue> {
    let (scan_width, scan_height) =
        downscaled_dims(width, height, max_dim).unwrap_or((width, height));
    let detections = unified_detections(
        &robust,
        scan_width as f64 / width as f64,
        scan_height as f64 / height as f64,
    );
    serde_wasm_bindgen::to_value(&WasmRobustResult {
        detections,
        robust,
        snapshots,
        scan_width: scan_width as u32,
        scan_height: scan_height as u32,
    })
    .map_err(JsValue::from)
}

/// Validate an RGBA frame and build its borrowed [`LumaView`]'s backing
/// luma, or return a descriptive `Err` (never panics — a panic poisons the
/// wasm instance). Shared by the single-frame and session paths.
fn prepare_luma(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, JsValue> {
    if width == 0 || height == 0 {
        return Err(JsValue::from_str(&format!(
            "width and height must be non-zero (got {width}x{height})"
        )));
    }
    let expected_len = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| JsValue::from_str(&format!("width * height * 4 overflows ({width}x{height})")))?;
    if rgba.len() != expected_len {
        return Err(JsValue::from_str(&format!(
            "rgba buffer is {} bytes, expected width * height * 4 = {expected_len} ({width}x{height})",
            rgba.len()
        )));
    }
    Ok(luma_from_rgba(rgba, width, height))
}

/// A stateful multi-frame scanner for VIDEO, wrapping
/// [`qrk_core::ScanSession`]: hold one across a camera stream and feed it
/// frames via [`scan_frame_rgba`](WasmScanSession::scan_frame_rgba). It
/// amortizes the robustness ladder across near-duplicate frames (rung
/// rotation + cross-frame candidate pooling), so per-frame cost stays near
/// baseline while detection recall is preserved temporally — the debug UI's
/// video path uses it to keep playback lean. Result envelope is identical
/// to [`scan_rgba_robust`]'s (minus `snapshots` — capture is off on the
/// video path). Call [`reset`](WasmScanSession::reset) on a source change
/// or seek so stale candidates from another scene cannot seed the new one.
#[wasm_bindgen]
pub struct WasmScanSession {
    inner: qrk_core::ScanSession,
}

#[wasm_bindgen]
impl WasmScanSession {
    /// Build a session. `config` is the same camelCase [`RobustConfig`]
    /// object [`scan_rgba_robust`] takes (`undefined`/`null` ⇒ all-off
    /// default); `rotation_period` (frames) and `pool_ttl_frames` tune the
    /// temporal amortization (`0`/omitted ⇒ the `SessionConfig` defaults 3
    /// and 4). See [`qrk_core::SessionConfig`].
    #[wasm_bindgen(constructor)]
    pub fn new(
        config: JsValue,
        rotation_period: u32,
        pool_ttl_frames: u32,
    ) -> Result<WasmScanSession, JsValue> {
        let cfg = if config.is_undefined() || config.is_null() {
            RobustConfig::default()
        } else {
            serde_wasm_bindgen::from_value(config)
                .map_err(|e| JsValue::from_str(&format!("WasmScanSession: invalid config: {e}")))?
        }
        .to_core();
        let defaults = qrk_core::SessionConfig::default();
        let session = qrk_core::SessionConfig {
            rotation_period: if rotation_period == 0 {
                defaults.rotation_period
            } else {
                rotation_period
            },
            pool_ttl_frames: if pool_ttl_frames == 0 {
                defaults.pool_ttl_frames
            } else {
                pool_ttl_frames as u64
            },
        };
        Ok(WasmScanSession {
            inner: qrk_core::ScanSession::new(cfg, session),
        })
    }

    /// Scan the next video frame; same `rgba`/`width`/`height`/`max_dim`/
    /// `refine` contract and result envelope as [`scan_rgba_robust`] (no
    /// `snapshots`). Advances the session's internal frame counter.
    #[wasm_bindgen]
    pub fn scan_frame_rgba(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
        max_dim: u32,
        refine: bool,
    ) -> Result<JsValue, JsValue> {
        let (w, h) = (width as usize, height as usize);
        let luma = prepare_luma(rgba, w, h).map_err(|e| {
            JsValue::from_str(&format!("scan_frame_rgba: {}", e.as_string().unwrap_or_default()))
        })?;
        let view = LumaView::new(&luma, w, h, w)
            .map_err(|e| JsValue::from_str(&format!("scan_frame_rgba: invalid luma view: {e:?}")))?;
        let opts = ScanOptions {
            max_working_dim: max_dim,
            refine,
        };
        let det = self.inner.scan_frame(&view, &opts);
        build_robust_envelope(det, None, w, h, max_dim)
    }

    /// Drop all cross-frame state and restart the frame counter — call on a
    /// source change or seek.
    #[wasm_bindgen]
    pub fn reset(&mut self) {
        self.inner.reset();
    }

    /// Frames scanned since construction or the last [`reset`](Self::reset).
    #[wasm_bindgen(getter)]
    pub fn frame_index(&self) -> u32 {
        self.inner.frame_index() as u32
    }
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
        let opts = qrk_core::ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
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

        let opts = qrk_core::ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
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
        assert_eq!(
            json.get("scan_width").and_then(|v| v.as_u64()),
            Some(SIDE as u64)
        );
        assert_eq!(
            json.get("scan_height").and_then(|v| v.as_u64()),
            Some(SIDE as u64)
        );
    }

    #[test]
    fn wasm_result_envelope_with_trace_pins_field_names() {
        let luma = flat_luma();
        let view = qrk_core::LumaView::new(&luma, SIDE, SIDE, SIDE).unwrap();

        let mut trace = qrk_core::Trace::new();
        let opts = qrk_core::ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
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
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/near_00.luma");
        let luma = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (width, height) = (1280usize, 720usize);
        assert_eq!(luma.len(), width * height, "near_00.luma: unexpected size");
        let view = qrk_core::LumaView::new(&luma, width, height, width).unwrap();

        let opts = qrk_core::ScanOptions {
            max_working_dim: 1280,
            refine: false,
        };
        let a = qrk_core::scan(&view, &opts);
        let mut trace = qrk_core::Trace::new();
        let b = qrk_core::scan_traced(&view, &opts, &mut trace);

        assert_eq!(a.finders.len(), b.finders.len());
        assert_eq!(a.triplets.len(), b.triplets.len());
        assert_eq!(a.source_scale, 1.0);
        assert_eq!(b.source_scale, 1.0);
        assert!(
            !a.finders.is_empty(),
            "near_00 should yield finder candidates"
        );
    }

    #[test]
    fn robust_envelope_field_names_are_pinned() {
        // Shape pin for the Plan 6 robust envelope (capture on): field
        // names here are what debug-ui/src/scanner/types.ts parses — the
        // committed cross-language snapshot (envelope_snapshot.rs) pins the
        // capture-OFF variant; this test covers the capture-only fields
        // with a tiny in-memory frame so no multi-hundred-KB pixel dump
        // needs committing.
        let luma = flat_luma();
        let view = qrk_core::LumaView::new(&luma, SIDE, SIDE, SIDE).unwrap();
        let opts = qrk_core::ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let cfg = qrk_core::ScanConfig::ROBUST_FULL_BENCHMARK;
        let debug = qrk_core::scan_robust_debug(&view, &opts, &cfg);
        let snapshots: Vec<super::WasmSnapshot> = debug
            .snapshots
            .into_iter()
            .map(|s| super::WasmSnapshot {
                kind: s.kind,
                stage: s.stage,
                width: s.width as u32,
                height: s.height as u32,
                luma: s.luma,
            })
            .collect();
        let detections = super::unified_detections(&debug.detections, 1.0, 1.0);
        let result = super::WasmRobustResult {
            detections,
            robust: debug.detections,
            snapshots: Some(snapshots),
            scan_width: SIDE as u32,
            scan_height: SIDE as u32,
        };
        let json = serde_json::to_value(&result).unwrap();

        let robust = json.get("robust").expect("robust key");
        for key in [
            "codes",
            "variants",
            "early_exited",
            "budget_exhausted",
            "total_ns",
            "triplet_evidence",
        ] {
            assert!(robust.get(key).is_some(), "robust.{key}");
        }
        let variants = robust.get("variants").unwrap().as_array().unwrap();
        assert!(!variants.is_empty());
        for key in [
            "kind",
            "stage",
            "timings",
            "total_ns",
            "finders",
            "triplets",
            "codes",
            "new_codes",
        ] {
            assert!(variants[0].get(key).is_some(), "variants[0].{key}");
        }
        // Baseline kind serializes as the bare tag string.
        assert_eq!(variants[0].get("kind").unwrap().as_str(), Some("Baseline"));

        // The unified pipeline result: same field names as WasmResult's
        // `detections`, assembled from the ladder union (one-pipeline
        // contract — robust flags just mean more entries here).
        let detections = json.get("detections").expect("detections key");
        for key in ["finders", "triplets", "codes", "timings", "source_scale"] {
            assert!(detections.get(key).is_some(), "detections.{key}");
        }
        // The robust metadata carries the union too (source px).
        assert!(robust.get("finders").is_some());
        assert!(robust.get("triplets").is_some());

        let snaps = json.get("snapshots").unwrap().as_array().unwrap();
        assert_eq!(snaps.len(), variants.len(), "one snapshot per variant");
        for key in ["kind", "stage", "width", "height", "luma"] {
            assert!(snaps[0].get(key).is_some(), "snapshots[0].{key}");
        }
        let w = snaps[0].get("width").unwrap().as_u64().unwrap();
        let h = snaps[0].get("height").unwrap().as_u64().unwrap();
        assert_eq!(
            snaps[0].get("luma").unwrap().as_array().unwrap().len() as u64,
            w * h,
            "luma is width*height bytes"
        );
    }

    #[test]
    fn robust_config_roundtrip_and_presets_are_camel_case() {
        // {} deserializes to the all-off default...
        let cfg: super::RobustConfig = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(cfg.to_core(), qrk_core::ScanConfig::default());
        // ...and camelCase keys land on the right fields.
        let cfg: super::RobustConfig = serde_json::from_value(serde_json::json!({
            "enableMultiScale": true,
            "maxVariantsPerFrame": 8,
            "enableEarlyExit": true,
        }))
        .unwrap();
        let core = cfg.to_core();
        assert!(core.enable_multi_scale);
        assert_eq!(core.max_variants_per_frame, 8);
        assert!(core.enable_early_exit);
        assert!(!core.enable_deblur);
        // Presets round-trip through the same camelCase shape.
        let presets = super::RobustPresets {
            baseline: super::RobustConfig::from_core(qrk_core::ScanConfig::BASELINE),
            robust_fast: super::RobustConfig::from_core(qrk_core::ScanConfig::ROBUST_FAST),
            robust_full_benchmark: super::RobustConfig::from_core(
                qrk_core::ScanConfig::ROBUST_FULL_BENCHMARK,
            ),
        };
        let json = serde_json::to_value(&presets).unwrap();
        assert!(json.get("robustFast").is_some());
        assert_eq!(
            json.pointer("/robustFast/maxVariantsPerFrame")
                .and_then(|v| v.as_u64()),
            Some(qrk_core::ScanConfig::ROBUST_FAST.max_variants_per_frame as u64)
        );
        let back: super::RobustConfig =
            serde_json::from_value(json.get("robustFullBenchmark").unwrap().clone()).unwrap();
        assert_eq!(back.to_core(), qrk_core::ScanConfig::ROBUST_FULL_BENCHMARK);
    }
}
