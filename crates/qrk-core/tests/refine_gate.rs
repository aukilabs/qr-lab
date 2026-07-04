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
//! # Gates
//!
//! - HARD (Plan 5 Global Constraints gate 2): per-prefix mean corner error
//!   <= 0.10px on `near_`, `rot_`, `ver_`. A matched code with
//!   `refined_corners == None` counts as an outright failure on ANY prefix
//!   (not just these three) — a `None` cannot satisfy any finite bound, so
//!   exempting the loose-gated prefixes from this would just let a total
//!   refinement failure hide inside a wide sanity ceiling.
//! - MEASURED-THEN-LOCKED (first green run): `far_`, `tilt45_`, `combo_`,
//!   `trans_`, `inv_`, `invtrans_`, `mirror_`, `multi_` are only asserted
//!   against a loose 0.5px mean sanity ceiling for this commit — see
//!   TODO(locked-values) below. The full measured table is printed and
//!   reported to the controller, which locks the real per-prefix bars in a
//!   follow-up commit once reviewed.

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

/// HARD-gated prefixes: nominal poses with no pathology the plan expects
/// to defeat refinement (frontal-ish `near_`, in-plane-only `rot_`, mild
/// `ver_` tilt sweep across every version).
const HARD_GATED_PREFIXES: [&str; 3] = ["near", "rot", "ver"];

/// Loose sanity ceiling for every other prefix, for THIS commit only —
/// TODO(locked-values): replace with the measured-then-locked per-prefix
/// bars once the controller reviews this run's printed table (Plan 5
/// Global Constraints gate 2's measured-then-locked protocol).
const SANITY_CEILING_PX: f64 = 0.5;
const HARD_CEILING_PX: f64 = 0.10;

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

    for prefix in HARD_GATED_PREFIXES {
        let Some(s) = stats.get(prefix) else {
            hard_failures.push(format!("{prefix}: no fixtures found for a HARD-gated prefix"));
            continue;
        };
        let mean = s.mean();
        // `mean.is_finite()` guards `NaN` (an empty `errors` set — no code
        // for this prefix ever reached a valid refinement) explicitly
        // failing rather than silently comparing false either way.
        let ok = mean.is_finite() && mean <= HARD_CEILING_PX;
        if !ok {
            hard_failures.push(format!(
                "{prefix}: HARD gate mean {mean:.4}px exceeds {HARD_CEILING_PX}px (max {:.4}px, {} codes)",
                s.max(),
                s.codes
            ));
        }
    }

    for (prefix, s) in &stats {
        if HARD_GATED_PREFIXES.contains(&prefix.as_str()) {
            continue;
        }
        if s.errors.is_empty() {
            continue; // nothing decoded/refined for this prefix at all
        }
        let mean = s.mean();
        let ok = mean.is_finite() && mean <= SANITY_CEILING_PX;
        if !ok {
            hard_failures.push(format!(
                "{prefix}: loose sanity ceiling mean {mean:.4}px exceeds {SANITY_CEILING_PX}px \
                 (max {:.4}px, {} codes) — TODO(locked-values) once reviewed",
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
