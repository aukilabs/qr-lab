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
//! Detection itself (tiling, finder/triplet grouping, decode) always runs
//! through the same `detect_with_source` orchestration `detect`/
//! `detect_traced` also use, on whichever view (source, or a freshly
//! downscaled working copy) ends up being detected on — `scan` adds no new
//! detection behavior there. `Detections::source_scale` records which case
//! happened and by how much, so module sampling (Plan 5 Task 2) and
//! subpixel corner refinement (Plan 5 Task 3, `ScanOptions::refine`) can
//! convert working-px geometry back to source px; refinement's own
//! `refine_corners` call happens inside `decode.rs`'s `attempt_candidate`,
//! against `source` itself (either the downscaled-from view, or `view`
//! when no downscale happened — see `attempt_candidate`'s doc for exactly
//! how it resolves that).

use crate::downscale::downscale_luma;
use crate::sample::SourceView;
use crate::scanner::{detect_with_source, Detections};
use crate::trace::Trace;
use crate::LumaView;

/// Options controlling `scan`'s working-resolution budget and (from Plan 5
/// Task 3 on) subpixel corner refinement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// Cap on the working view's longest side, in source px. `0` disables
    /// downscaling entirely — `scan` then detects directly on `source`,
    /// exactly like `detect`/`detect_with`. Mirrors `downscale_luma`'s own
    /// `max_dim` parameter verbatim (see its doc comment for the exact NN
    /// formula this uses).
    pub max_working_dim: u32,
    /// Enables subpixel corner refinement (Plan 5 Task 3): when `true`,
    /// every decoded code's `DecodedCode::refined_corners` is computed
    /// against the SOURCE view (traced edge-line intersections, source
    /// px) instead of staying `None` — `DecodedCode::corner_refined`
    /// records which of the four corners actually came from that
    /// intersection versus the coarse (source-scaled) fallback. Refinement
    /// runs against the SOURCE view when a downscale happened, and against
    /// `source` itself (`sx = sy = 1.0`) when it didn't — enabling this
    /// always has an effect regardless of `max_working_dim`. `false` (the
    /// default most callers should use unless they need the refined
    /// geometry) costs nothing extra: no edge probing, no
    /// `StageTimings::refine_ns`.
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
            // Per-axis ratios: `downscale_luma` rounds width and height
            // independently, so on a non-cleanly-scaling source the two
            // genuinely differ (see `SourceView`'s doc for the measured
            // ~0.75 source-px far-edge lift error a shared width scalar
            // causes). The internal sampling lift uses both exactly.
            let sx = w as f64 / source.width() as f64;
            let sy = h as f64 / source.height() as f64;
            // Plan 5 Task 2: hand the SOURCE view (plus the per-axis
            // ratios, `SourceView`'s convention) down through
            // `detect_with_source` so `decode.rs`'s module sampling can read
            // it instead of `working` — the fix for far codes that DETECT
            // fine at working resolution but can't SAMPLE at ~2 working
            // px/module (real-photo evidence: `IMG_4832`). Every stage
            // before module sampling still runs on `working` exactly as
            // before; only the sampling calls deep inside `decode_candidates`
            // see `source` at all.
            let mut detections = detect_with_source(
                &working,
                Some(SourceView {
                    view: source,
                    sx,
                    sy,
                }),
                trace,
                opts.refine,
                crate::tiles::BinarizeSpec::default(),
            );
            // The PUBLIC scalar stays width-pinned per the plan (see
            // `Detections::source_scale`'s doc) even though the internal
            // sampling lift above is per-axis.
            detections.source_scale = sx;
            detections
        }
        // No downscale needed: `source_scale` defaults to `1.0` inside
        // `detect_with_source`, exactly right since working == source
        // here. IMPORTANT (Plan 5 Task 3): this does NOT delegate to
        // `detect_with` (which hardcodes `refine: false`) — refinement
        // still runs when `opts.refine` is `true` even though no downscale
        // happened; `detect_with_source`'s `source: None` branch handles
        // that by refining against `view` itself with `sx = sy = 1.0` (see
        // `decode::attempt_candidate`'s doc). `detect_with_source(source,
        // None, trace, false)` — the `refine: false` case — is exactly
        // what `detect_with` itself calls, so behavior is byte-identical
        // to pre-Task-3 `scan` whenever refinement is off.
        None => detect_with_source(
            source,
            None,
            trace,
            opts.refine,
            crate::tiles::BinarizeSpec::default(),
        ),
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
        let opts = ScanOptions {
            max_working_dim: 64,
            refine: false,
        };
        let det = scan(&view, &opts);
        assert_eq!(det.source_scale, 1.0);
    }

    #[test]
    fn max_working_dim_zero_disables_downscaling_like_detect() {
        let data = vec![128u8; 64 * 32];
        let view = flat_view(&data, 64, 32);
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let det = scan(&view, &opts);
        assert_eq!(det.source_scale, 1.0);
    }

    #[test]
    fn downscale_needed_reports_the_width_ratio_and_still_detects() {
        let data = vec![128u8; 100 * 50];
        let view = flat_view(&data, 100, 50);
        let opts = ScanOptions {
            max_working_dim: 50,
            refine: false,
        };
        let det = scan(&view, &opts);
        // longest=100, max_dim=50 -> dst_w=round(100*50/100)=50 -> ratio 0.5
        assert_eq!(det.source_scale, 0.5);
    }

    #[test]
    fn scan_and_detect_agree_when_no_downscale_happens() {
        let data = vec![128u8; 32 * 32];
        let view = flat_view(&data, 32, 32);
        let scanned = scan(
            &view,
            &ScanOptions {
                max_working_dim: 0,
                refine: false,
            },
        );
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
        let det = scan_traced(
            &view,
            &ScanOptions {
                max_working_dim: 0,
                refine: false,
            },
            &mut trace,
        );
        assert_eq!(det.source_scale, 1.0);
        #[cfg(feature = "debug-trace")]
        assert!(trace.tiles.is_some());
    }

    #[test]
    fn refine_flag_has_no_effect_when_nothing_decodes() {
        // Plan 5 Task 3: `refine` now DOES enable real work (subpixel
        // corner refinement) — see `decode_gate.rs`'s
        // `plan5_scan_refine_populates_refined_corners_on_a_real_fixture`
        // for the end-to-end proof it actually runs. On a flat image with
        // no finders/codes at all, though, there is nothing for refinement
        // to run against (`attempt_candidate`'s refine call only fires on
        // an actual decode), so `codes`/`source_scale` stay identical
        // either way — a narrower, still-true regression pin than this
        // test's pre-Task-3 name implied.
        let data = vec![128u8; 64 * 32];
        let view = flat_view(&data, 64, 32);
        let without = scan(
            &view,
            &ScanOptions {
                max_working_dim: 0,
                refine: false,
            },
        );
        let with = scan(
            &view,
            &ScanOptions {
                max_working_dim: 0,
                refine: true,
            },
        );
        assert_eq!(without.codes.len(), with.codes.len());
        assert_eq!(
            without.codes.len(),
            0,
            "test setup: a flat image must not decode anything"
        );
        assert_eq!(without.source_scale, with.source_scale);
    }
}
