//! `scan()`: the primary detection entry point (Plan 5 Task 1), sitting
//! above `detect`/`detect_with` rather than replacing them. Where `detect`
//! always runs the pipeline on exactly the [`LumaView`] it's handed, `scan`
//! additionally owns an optional Rust-side NN downscale (see
//! `downscale.rs`) so a caller can hand it a full SOURCE-resolution view
//! plus a working-resolution budget, instead of pre-downscaling itself —
//! the debug UI's TS worker did exactly that pre-downscaling before this
//! task; `scan_rgba` (qrk-wasm) now does it in Rust instead (see that
//! crate's doc comment).
//!
//! This task adds no new detection behavior: `scan` funnels through the
//! exact same `detect_with` orchestration `detect`/`detect_traced` already
//! use, on whichever view (source, or a freshly downscaled working copy)
//! ends up being detected on. The only new output is
//! `Detections::source_scale`, which records which case happened and by
//! how much, so later tasks (source-resolution module sampling; subpixel
//! corner refinement) can convert working-px geometry back to source px.

use crate::downscale::downscale_luma;
use crate::sample::SourceView;
use crate::scanner::{detect_with, detect_with_source, Detections};
use crate::trace::Trace;
use crate::LumaView;

/// Options controlling `scan`'s working-resolution budget and (from Plan 5
/// Task 3 on) subpixel corner refinement.
#[derive(Clone, Copy, Debug)]
pub struct ScanOptions {
    /// Cap on the working view's longest side, in source px. `0` disables
    /// downscaling entirely — `scan` then detects directly on `source`,
    /// exactly like `detect`/`detect_with`. Mirrors `downscale_luma`'s own
    /// `max_dim` parameter verbatim (see its doc comment for the exact NN
    /// formula this uses).
    pub max_working_dim: u32,
    /// Plumbing only as of this task: accepted and threaded through
    /// `scan_rgba`'s wasm signature and the debug UI's wire protocol, but
    /// has NO effect on `scan`'s output yet — there is no refinement stage
    /// to enable. That lands in Plan 5 Task 3 (`refine_corners`), which
    /// will make `scan` compute `DecodedCode::refined_corners` against the
    /// source view when this is `true`. Landing the field now (rather than
    /// widening `ScanOptions` again later) means Task 3 is an additive
    /// change to `scan`'s body, not another breaking signature change to
    /// `scan_rgba`/the worker/client wire protocol.
    pub refine: bool,
}

/// Run the full detection pipeline on `source`, optionally downscaling it
/// to a working resolution first per `opts.max_working_dim`. Detection
/// (tiling, finder/triplet grouping, decode) always runs on the WORKING
/// view — the source view itself when no downscale is needed.
/// `Detections::source_scale` records the working/source relationship (see
/// that field's doc comment for the exact conversion direction).
pub fn scan(source: &LumaView, opts: &ScanOptions) -> Detections {
    scan_with(source, opts, None)
}

/// Like [`scan`], but records each stage's output into `trace` (a no-op
/// without the `debug-trace` feature) — the `scan`-family counterpart to
/// `detect_traced`.
pub fn scan_traced(source: &LumaView, opts: &ScanOptions, trace: &mut Trace) -> Detections {
    scan_with(source, opts, Some(trace))
}

fn scan_with(source: &LumaView, opts: &ScanOptions, trace: Option<&mut Trace>) -> Detections {
    match downscale_luma(source, opts.max_working_dim) {
        Some((buf, w, h)) => {
            let working = LumaView::new(&buf, w, h, w).expect(
                "downscale_luma always returns a non-empty, tightly packed (stride == width) buffer",
            );
            // Width-based ratio — see `Detections::source_scale`'s doc
            // comment for why width (rather than height) is the pinned
            // axis when the two disagree by a rounding hair on a
            // non-square image.
            let scale = w as f64 / source.width() as f64;
            // Plan 5 Task 2: hand the SOURCE view (plus this same ratio,
            // `SourceView::scale`'s convention) down through
            // `detect_with_source` so `decode.rs`'s module sampling can read
            // it instead of `working` — the fix for far codes that DETECT
            // fine at working resolution but can't SAMPLE at ~2 working
            // px/module (real-photo evidence: `IMG_4832`). Every stage
            // before module sampling still runs on `working` exactly as
            // before; only the sampling calls deep inside `decode_candidates`
            // see `source` at all.
            let mut detections =
                detect_with_source(&working, Some(SourceView { view: source, scale }), trace);
            detections.source_scale = scale;
            detections
        }
        // No downscale needed: `detect_with` already defaults
        // `source_scale` to `1.0`, exactly right since working == source
        // here.
        None => detect_with(source, trace),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_view(data: &[u8], w: usize, h: usize) -> LumaView<'_> {
        LumaView::new(data, w, h, w).unwrap()
    }

    #[test]
    fn no_downscale_needed_borrows_source_and_reports_scale_1() {
        let data = vec![128u8; 64 * 32];
        let view = flat_view(&data, 64, 32);
        let opts = ScanOptions { max_working_dim: 64, refine: false };
        let det = scan(&view, &opts);
        assert_eq!(det.source_scale, 1.0);
    }

    #[test]
    fn max_working_dim_zero_disables_downscaling_like_detect() {
        let data = vec![128u8; 64 * 32];
        let view = flat_view(&data, 64, 32);
        let opts = ScanOptions { max_working_dim: 0, refine: false };
        let det = scan(&view, &opts);
        assert_eq!(det.source_scale, 1.0);
    }

    #[test]
    fn downscale_needed_reports_the_width_ratio_and_still_detects() {
        let data = vec![128u8; 100 * 50];
        let view = flat_view(&data, 100, 50);
        let opts = ScanOptions { max_working_dim: 50, refine: false };
        let det = scan(&view, &opts);
        // longest=100, max_dim=50 -> dst_w=round(100*50/100)=50 -> ratio 0.5
        assert_eq!(det.source_scale, 0.5);
    }

    #[test]
    fn scan_and_detect_agree_when_no_downscale_happens() {
        let data = vec![128u8; 32 * 32];
        let view = flat_view(&data, 32, 32);
        let scanned = scan(&view, &ScanOptions { max_working_dim: 0, refine: false });
        let detected = crate::scanner::detect(&view);
        assert_eq!(scanned.finders.len(), detected.finders.len());
        assert_eq!(scanned.triplets.len(), detected.triplets.len());
        assert_eq!(scanned.source_scale, detected.source_scale);
    }

    #[test]
    fn scan_traced_records_the_same_trace_shape_as_detect_traced() {
        let data = vec![128u8; 64 * 64];
        let view = flat_view(&data, 64, 64);
        let mut trace = Trace::new();
        let det = scan_traced(&view, &ScanOptions { max_working_dim: 0, refine: false }, &mut trace);
        assert_eq!(det.source_scale, 1.0);
        #[cfg(feature = "debug-trace")]
        assert!(trace.tiles.is_some());
    }

    #[test]
    fn refine_flag_has_no_effect_yet() {
        let data = vec![128u8; 64 * 32];
        let view = flat_view(&data, 64, 32);
        let without = scan(&view, &ScanOptions { max_working_dim: 0, refine: false });
        let with = scan(&view, &ScanOptions { max_working_dim: 0, refine: true });
        assert_eq!(without.codes.len(), with.codes.len());
        assert_eq!(without.source_scale, with.source_scale);
    }
}
