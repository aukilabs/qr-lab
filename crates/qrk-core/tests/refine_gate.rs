//! Plan 5's fixture accuracy gate (Global Constraints, gate 2): per-prefix
//! mean/max subpixel-refined-corner error against each golden fixture's
//! `corners_px` ground truth.
//!
//! # Method
//!
//! For every one of the 81 top-level golden fixtures (`common::load_all`,
//! same 81 `fixture_prefix`-summarized set `decode_gate.rs`'s gate 1
//! decodes): `scan(fixture.view(), ScanOptions { max_working_dim: 0, refine:
//! true })`. `max_working_dim: 0` disables the Rust-owned downscale
//! entirely — every fixture is already generated at the pipeline's working
//! resolution (Plan 2/4's scenario matrix), so `source == working` and
//! `Detections::source_scale == 1.0`; `refined_corners` (when present) is
//! therefore directly comparable to `corners_px` with no scale conversion,
//! exactly as the task brief specifies.
//!
//! Decoded codes are matched to ground-truth codes by payload equality
//! (fixture payloads are unique within a fixture — see
//! `tools/fixtures/scenarios.py`'s `_sample_code`/`multi` construction).
//! Fixtures/codes that fail to decode at all are out of this gate's scope
//! (covered by `decode_gate.rs`'s gate 1); this file only measures corner
//! accuracy on codes that DID decode.
//!
//! # Corner-order correspondence (mirrored codes) — IDENTITY, derived and
//! verified below; NOT a TR/BL swap
//!
//! The obvious hypothesis (and the one this file's first draft implemented,
//! then disproved — kept here because it's the natural trap) is that a
//! mirrored code's `corners_px` and `refined_corners` disagree on which
//! physical corner is "TR" vs "BL", requiring a swap before comparing.
//! That hypothesis is WRONG for this pipeline, for a reason specific to how
//! QR finder patterns work:
//!
//! - `corners_px` (ground truth) is `[TL, TR, BR, BL]` in a PURE PHYSICAL/
//!   camera-projected sense — `tools/fixtures/render.py::corners_px` is the
//!   projection of `_plane_corners_m`'s fixed physical-square corners
//!   through the code's camera pose. It takes no `mirrored` argument.
//! - `tools/fixtures/render.py::make_symbol` renders a mirrored code as
//!   `modules = original.T` (a full bit-matrix transpose) BEFORE handing it
//!   to the SAME physical-plane rendering as a non-mirrored code — i.e.
//!   mirroring changes which DATA bits occupy which physical module cell,
//!   never the physical corner positions or the camera pose.
//! - Crucially, a QR finder pattern (the 7x7 nested-square block at each of
//!   the 3 non-BR corners) is a FIXED, content-independent bit pattern —
//!   identical at every corner, in every valid QR code, transposed or not
//!   (ISO 18004's finder pattern is itself symmetric under transpose). So
//!   after `modules = original.T`, the physical TL/TR/BL corners each still
//!   show a generic, identical-looking finder blob — transposing swaps
//!   which corner's underlying DATA/format-info bits originated from
//!   "logical TR" vs "logical BL", but not which physical corners visually
//!   contain a finder pattern, and not the finder blobs' own appearance.
//! - `triplet.rs::try_group` assigns `tl`/`tr`/`bl` from PURE GEOMETRY on
//!   the 3 detected finder positions (right-angle vertex -> `tl`; winding
//!   sign -> `tr`/`bl`) — it has no way to see, and does not need to see,
//!   which corner's finder pattern happens to encode which logical role.
//!   Since finder positions and the winding of the physical TL/TR/BL
//!   triangle never change under a pure content transpose (only the plane's
//!   own tilt/rotation could do that, and mirroring alone does not touch
//!   the camera pose), `try_group`'s geometric labeling lands on the exact
//!   same physical corners whether or not the code is mirrored. Mirroring
//!   only ever matters one stage later, at CONTENT decoding
//!   (`bitmatrix::decode_bits`'s internal transpose retry / `is_mirrored`)
//!   — it is invisible to geometry.
//!
//! So the correspondence is IDENTITY regardless of `mirrored`:
//! `refined[i]` <-> `corners_px[i]` for `i` in `0..4`, always.
//!
//! This is not just argued but MEASURED: running this gate with the naive
//! swap(1,3) hypothesis applied to `mirror_*` fixtures produced a mean
//! error of 132.6px (max 330.2px) — obvious garbage, since `corners_px`'s
//! TR/BL entries are then compared against the wrong physical corners.
//! Reverting to identity brings `mirror_*` in line with every other prefix
//! (mean ~0.01-0.03px, see the printed table) — direct empirical
//! confirmation of the geometric argument above, not a tuned fix.
//!
//! # Gate (LOCKED — controller decision, 2026-07-04)
//!
//! Per-prefix mean corner error <= 0.10px, UNIFORMLY across ALL prefixes.
//!
//! The plan's measured-then-locked protocol ran its course: the first
//! green run's per-prefix table (embedded near the assertion as a
//! reference comment) showed every prefix — including the non-nominal
//! `far_`/`tilt45_`/`combo_`/`trans_`/`inv_`/`invtrans_`/`mirror_`/
//! `multi_` set the plan left to be measured first — landing at
//! 0.008-0.039px mean, i.e. 2.5-12x inside the nominal prefixes' own
//! 0.10px bar. The controller's recorded lock decision: extend the SAME
//! 0.10px pose-quality budget to every prefix (a principled bound shared
//! with the plan's nominal gate), rather than locking the exact measured
//! values (which would over-fit the gate to one generator seed's noise
//! floor).
//!
//! A matched code with `refined_corners == None` counts as an outright
//! failure on ANY prefix — a `None` cannot satisfy any finite bound.

mod common;

use std::collections::BTreeMap;

use qrk_core::{scan, ScanOptions};

/// Same grouping helper as `decode_gate.rs`'s `fixture_prefix` (duplicated,
/// not shared, per that file's own precedent: each integration test binary
/// compiles `common` — and any such small helper — independently).
fn fixture_prefix(name: &str) -> String {
    let mut s = name;
    if let Some(pos) = s.rfind('_') {
        let tail = &s[pos + 1..];
        if tail.len() > 1 && tail.starts_with('v') && tail[1..].bytes().all(|b| b.is_ascii_digit()) {
            s = &s[..pos];
        }
    }
    if let Some(pos) = s.rfind('_') {
        let tail = &s[pos + 1..];
        if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
            s = &s[..pos];
        }
    }
    s.to_string()
}

/// Ground-truth corners in the DECODED pipeline's own corner order — see
/// the module doc's "Corner-order correspondence" section: this is the
/// IDENTITY mapping, `mirrored` notwithstanding (a QR's finder patterns are
/// content-independent, so the pipeline's purely-geometric `tl`/`tr`/`bl`
/// labeling never depends on which corner's data happens to be mirrored).
/// `truth.mirrored` is accepted (not just dropped) to keep this function's
/// signature self-documenting at call sites and available if a future
/// fixture generator ever introduces a TRUE geometric mirror (an actually
/// flipped camera/scene, as opposed to this suite's content-only
/// transpose) that would need a real permutation.
fn permuted_truth_corners(truth: &common::CodeTruth) -> [[f64; 2]; 4] {
    let _ = truth.mirrored;
    truth.corners_px
}

fn corner_error(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The locked, uniform per-prefix mean bound (see the module doc's "Gate
/// (LOCKED)" section for the controller's recorded decision and the
/// measured reference table near the assertion below).
const MEAN_CEILING_PX: f64 = 0.10;

#[derive(Default)]
struct PrefixStats {
    fixtures: usize,
    codes: usize,
    /// Every matched code's 4 corner errors, pooled (not per-code
    /// averaged first) — the per-prefix mean/max the plan's gate asks for.
    errors: Vec<f64>,
    /// `"fixture: payload"` for every matched code whose `refined_corners`
    /// was `None` (refinement produced fewer than 2 valid edge lines).
    refine_failures: Vec<String>,
}

impl PrefixStats {
    fn mean(&self) -> f64 {
        if self.errors.is_empty() {
            f64::NAN
        } else {
            self.errors.iter().sum::<f64>() / self.errors.len() as f64
        }
    }

    fn max(&self) -> f64 {
        self.errors.iter().cloned().fold(0.0, f64::max)
    }
}

#[test]
fn fixture_accuracy_gate() {
    let fixtures = common::load_all();
    assert_eq!(fixtures.len(), 81, "expected exactly 81 golden fixtures in fixtures/");

    let mut stats: BTreeMap<String, PrefixStats> = BTreeMap::new();
    let mut hard_failures: Vec<String> = Vec::new();

    for fx in &fixtures {
        let prefix = fixture_prefix(&fx.name);
        let view = fx.view();
        let det = scan(&view, &ScanOptions { max_working_dim: 0, refine: true });
        assert_eq!(
            det.source_scale, 1.0,
            "{}: expected source_scale == 1.0 (max_working_dim: 0 disables downscale)",
            fx.name
        );

        let entry = stats.entry(prefix.clone()).or_default();
        entry.fixtures += 1;

        for truth in &fx.codes {
            let Some(code) = det.codes.iter().find(|c| c.payload == truth.payload) else {
                // Out of scope here (decode_gate.rs's gate 1 owns decode
                // success); nothing to measure for a code that never
                // decoded.
                continue;
            };
            entry.codes += 1;

            match code.refined_corners {
                None => {
                    entry.refine_failures.push(format!("{}: {:?}", fx.name, truth.payload));
                }
                Some(refined) => {
                    let want = permuted_truth_corners(truth);
                    for i in 0..4 {
                        entry.errors.push(corner_error(refined[i], want[i]));
                    }
                }
            }
        }
    }

    println!("\n=== Plan 5 gate 2: per-prefix refined-corner accuracy (source px) ===");
    println!(
        "{:<10} {:>9} {:>7} {:>10} {:>10} {:>10}",
        "prefix", "fixtures", "codes", "mean_px", "max_px", "none_fail"
    );
    for (prefix, s) in &stats {
        println!(
            "{:<10} {:>9} {:>7} {:>10.4} {:>10.4} {:>10}",
            prefix,
            s.fixtures,
            s.codes,
            s.mean(),
            s.max(),
            s.refine_failures.len()
        );
    }

    // Every `refined_corners == None` among matched codes is an outright
    // failure, on every prefix (see the module doc's rationale).
    for (prefix, s) in &stats {
        for f in &s.refine_failures {
            hard_failures.push(format!("{prefix}: refinement produced None for {f}"));
        }
    }

    // Reference table from the first green run (2026-07-04, the run the
    // controller's uniform-0.10px lock decision was based on) — kept here
    // so future drift is visible in review even while a regression stays
    // under the gate. Source px, all matched codes' 4 corners pooled:
    //
    //   prefix     fixtures   codes    mean_px     max_px  none_fail
    //   combo             4       4     0.0239     0.0725          0
    //   far               8       8     0.0152     0.0541          0
    //   inv               6       6     0.0075     0.0162          0
    //   invtrans          4       4     0.0176     0.0646          0
    //   mirror            4       4     0.0227     0.0646          0
    //   multi             8      20     0.0176     0.1058          0
    //   near              8       8     0.0119     0.0602          0
    //   rot              12      12     0.0218     0.0845          0
    //   tilt45            8       8     0.0388     0.2262          0
    //   trans             6       6     0.0226     0.0744          0
    //   ver              13      13     0.0191     0.0877          0
    for (prefix, s) in &stats {
        let mean = s.mean();
        // `mean.is_finite()` makes `NaN` (an empty `errors` set — no code
        // for this prefix ever reached a valid refinement) explicitly fail
        // rather than silently comparing false either way. Every prefix in
        // the suite has decodable codes (decode_gate.rs gate 1), so an
        // empty set here means refinement collapsed wholesale.
        let ok = mean.is_finite() && mean <= MEAN_CEILING_PX;
        if !ok {
            hard_failures.push(format!(
                "{prefix}: mean {mean:.4}px exceeds the locked {MEAN_CEILING_PX}px bound \
                 (max {:.4}px, {} codes)",
                s.max(),
                s.codes
            ));
        }
    }

    assert!(
        hard_failures.is_empty(),
        "Plan 5 gate 2 FAILED ({} issue(s)):\n{}",
        hard_failures.len(),
        hard_failures.join("\n")
    );
}
