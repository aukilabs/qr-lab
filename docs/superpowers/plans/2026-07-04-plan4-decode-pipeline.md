# Plan 4: Decode Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `qr-lab-core` decodes QR payloads end to end — version estimation with cross-checks, alignment-pattern location, perspective grid sampling, rqrr decode with mirrored retry, and cross-code arbitration — gated on payload-exact decode of all 81 golden fixtures AND the two real captures, with every new stage visible as a debug-UI overlay.

**Architecture:** After triplet grouping, candidates are proximity-deduped and attempted best-first. Per candidate: dimension is fixed by cross-checks (timing-transition counting for small versions, BCH version-info bits for v≥7 — authoritative over the triplet estimate, per the recorded ver_12_v40 requirement); alignment patterns are located by parallelogram prediction + concentric re-centering; module centers are sampled through perspective transforms (single transform for v1/fallback, alignment-anchored regions otherwise) against tile thresholds, polarity-aware; the bit matrix feeds rqrr's `BitGrid` decode with a `MirroredGrid` retry. A successful decode consumes its three finders, killing the spurious cross-code triplets seen in multi scenes. Trace gains four stages, the envelope snapshot regenerates, and four new overlays land in the debug UI.

**Tech Stack:** Rust; **new required qr-lab-core deps: `rqrr` (default-features = false → pulls only `g2p`, `lru`)** — the spec-planned decode engine; dev-deps: `qrcode` (test matrix generation), `png` (reading real-capture PNGs in gate tests). TS/debug-ui: no new deps.

## Global Constraints

- **Dependency change is deliberate and recorded:** qr-lab-core drops "zero required runtime deps" (Plan 2 wording) in favor of the spec §3.7 decision: `rqrr = { version = "0.10", default-features = false }`. License MIT/Apache-2.0 + ISC. No other new runtime deps; `serde`/`js-sys` stay optional/target-gated. `#![forbid(unsafe_code)]` stays.
- **No overfitting (standing user directive):** fixtures verify, never tune. Every new constant carries a principled derivation (ISO 18004, zxing/zxing-cpp practice, or an explicit noise model). Gate-failure protocol from Plan 2 applies verbatim (exact miss lists, DONE_WITH_CONCERNS, controller decision recorded here).
- Pinned constants (in `consts.rs`, each with a provenance comment):
  - Version-info BCH(18,6): correct ≤ 3 bit errors (min distance 8, ISO 18004) — reject at ≥4.
  - Timing-transition cross-check applies to estimated dimension ≤ 41 (v≤6 has no version bits; above that, version bits are authoritative).
  - Alignment re-centering probe: ±2.25 modules (zxing-cpp AP probe half-width).
  - Candidate cap: `MAX_DECODE_ATTEMPTS = 24` (≈ 4 codes/frame worst case × 3 triplet permutations × 2 headroom); attempted in `snap_error` order after proximity dedup.
  - Proximity dedup: two triplets sharing ≥2 finder candidates (by index) are duplicates — keep the lower `snap_error`.
  - Sample-out-of-image tolerance: a candidate whose sampling grid would read >2% of modules outside the image is rejected before decode (border clamp hides failures otherwise; 2% ≈ one clipped quiet-zone row on a v1).
- **Decode gates:**
  1. All 81 golden fixtures: every ground-truth code decodes with payload exactly equal to `codes[].payload`, correct `version`, correct `mirrored` flag.
  2. Real captures (from PNG via the `png` dev-dep, downscaled to max-dim 1280 with the production NN formula): `real_1` decodes ≥2 codes including payloads `HTTPS://R8.HR/O9MKM1ZO3W5` and `HTTPS://R8.HR/YLXFAP2B1A8`; `real_2` decodes exactly `HTTPS://R8.HR/YLXFAP2B1A8`. (Case-exact.)
  3. Arbitration: on `multi_*` fixtures the decoded-code count equals the ground-truth code count (no spurious extra decodes), and every finder candidate is consumed by at most one decoded code.
- **Trace compactness (final-review recommendation):** the trace must stay serializable per frame in video mode. Bit matrices serialize as `Vec<u32>` packed rows + dims (not `Vec<bool>`); sampling geometry serializes as the per-region transforms + alignment points (few), NOT per-module points — the debug UI reconstructs grids via its existing TS homography.
- Envelope snapshot regenerates once (UPDATE_SNAPSHOT=1) when the trace shape lands; TS types extend to match (snapshot wins on naming).
- Commits end with the repo's Claude co-author trailer.

## File Structure

```
crates/qr-lab-core/src/
  version.rs      timing-transition count + BCH version-info decode (+ tables)
  alignment.rs    Annex E coordinate table + prediction + concentric re-centering
  sample.rs       provisional transform, alignment-anchored region transforms, module sampling → BitMatrix
  bitmatrix.rs    packed bit matrix (u32 rows), rqrr BitGrid impl
  decode.rs       per-candidate pipeline + arbitration + DecodedCode
  scanner.rs      detect() gains stages 4-7 + timings fields
  trace.rs        new stage records
  consts.rs       new pinned constants
crates/qr-lab-core/tests/decode_gate.rs      gates 1-3
crates/qr-lab-wasm/tests/envelope_snapshot.rs (regenerated snapshot)
debug-ui/src/overlays/layers/{alignment,samplegrid,bits,decoded}.ts (+ tests)
debug-ui/src/scanner/types.ts             extended
```

---

### Task 1: Packed bit matrix + rqrr BitGrid adapter + decode smoke

**Files:** Create `crates/qr-lab-core/src/bitmatrix.rs`; modify `Cargo.toml` (rqrr required; qrcode+png dev), `lib.rs`.

**Interfaces:**
- `pub struct BitMatrix { pub dim: usize /* rows == cols */, words: Vec<u32> /* row-major, ceil(dim/32) words per row */ }` with `new(dim)`, `get(x, y) -> bool`, `set(x, y, v)`, `words(&self) -> &[u32]` (trace serialization), `words_per_row()`.
- `impl rqrr::BitGrid for &BitMatrix` (size, bit) — the spec's ~10-line glue.
- `pub fn decode_bits(m: &BitMatrix) -> Result<DecodedPayload, DecodeFailure>` where `DecodedPayload { payload: String, payload_bytes: Vec<u8>, version: u32, ecc: char, mirrored: bool }` — tries rqrr straight, then via transpose (rqrr's MirroredGrid semantics: implement by wrapping BitGrid with x/y swapped — verify against rqrr's own `MirroredGrid` if public, else a local `Transposed<'_>` wrapper); `mirrored = true` when only the transposed read succeeds. ECC level from rqrr metadata if exposed; if rqrr does not expose it, store `'?'` and record that in the report (do not fake it).
- `DecodeFailure` enum: `Format`, `Version`, `Ecc`, `Content` — mapped from rqrr's error type for trace visibility.

**Test-first (verbatim):**
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn qr_matrix(payload: &str, version: i16, ecc: qrcode::EcLevel) -> BitMatrix {
        let code = qrcode::QrCode::with_version(
            payload.as_bytes(), qrcode::Version::Normal(version), ecc).unwrap();
        let dim = code.width();
        let mut m = BitMatrix::new(dim);
        for y in 0..dim {
            for x in 0..dim {
                m.set(x, y, code[(x, y)] == qrcode::Color::Dark);
            }
        }
        m
    }

    #[test]
    fn packing_round_trips() {
        let mut m = BitMatrix::new(41); // v6: crosses a u32 word boundary
        m.set(0, 0, true); m.set(31, 3, true); m.set(32, 3, true); m.set(40, 40, true);
        assert!(m.get(0, 0) && m.get(31, 3) && m.get(32, 3) && m.get(40, 40));
        assert!(!m.get(1, 0) && !m.get(33, 3));
    }

    #[test]
    fn decodes_v1_and_v7_and_v40() {
        for (v, payload) in [(1i16, "Q:test:1"), (7, "HTTPS://R8.HR/O9MKM1ZO3W5"),
                             (40, &"x".repeat(1000) as &str)] {
            let m = qr_matrix(payload, v, qrcode::EcLevel::M);
            let d = decode_bits(&m).unwrap_or_else(|e| panic!("v{v}: {e:?}"));
            assert_eq!(d.payload, payload, "v{v}");
            assert_eq!(d.version as i16, v);
            assert!(!d.mirrored);
        }
    }

    #[test]
    fn decodes_mirrored() {
        let m = qr_matrix("Q:mirror:1", 1, qrcode::EcLevel::M);
        let mut t = BitMatrix::new(m.dim);
        for y in 0..m.dim { for x in 0..m.dim { t.set(y, x, m.get(x, y)); } }
        let d = decode_bits(&t).unwrap();
        assert_eq!(d.payload, "Q:mirror:1");
        assert!(d.mirrored);
    }

    #[test]
    fn garbage_fails_cleanly() {
        let mut m = BitMatrix::new(21);
        for y in 0..21 { for x in 0..21 { m.set(x, y, (x * 31 + y * 17) % 3 == 0); } }
        assert!(decode_bits(&m).is_err()); // no panic
    }
}
```

**Steps:** deps → tests fail → implement → green; `cargo tree -p qr-lab-core -e normal` shows exactly rqrr+g2p+lru (record output in report); workspace green; commit.

---

### Task 2: Version cross-checks (`version.rs`)

**Interfaces:**
- `pub(crate) fn count_timing_transitions(view, grid, t: &TripletCandidate) -> Option<u32>` — walk the timing row (between finder inner edges, at module row/col 6) via the provisional transform (Task 4 exposes it; for this task take a `&PerspectiveTransform` argument), binarize polarity-aware, count transitions; `dimension_check = transitions + 13` (timing spans dimension−14 modules alternating). Returns None when the walk leaves the image.
- `pub(crate) fn read_version_bits(view, grid, transform, dimension_est, inverted) -> Option<u32>` — sample the two 6×3 version-info blocks (positions per ISO: near TR and BL finders) in both bit orders; BCH(18,6) decode with ≤3 errors; returns the decoded version when either block/orientation agrees.
- BCH implementation: `pub(crate) fn bch_decode_version(bits: u32) -> Option<u32>` — compare against the 34 precomputed VERSION_DECODE_INFO codewords (v7..40, zxing `Version.java` table — transcribe all 34 constants), accept min-Hamming-distance ≤ 3, unique.

**Tests (verbatim for the BCH core):**
```rust
    #[test]
    fn bch_exact_codewords_decode() {
        assert_eq!(bch_decode_version(0x07C94), Some(7));
        assert_eq!(bch_decode_version(0x0C762), Some(12));
        assert_eq!(bch_decode_version(0x28C69), Some(40));
    }

    #[test]
    fn bch_three_errors_ok_four_rejected() {
        let w = 0x07C94u32;
        assert_eq!(bch_decode_version(w ^ 0b111), Some(7));           // 3 flips
        assert_eq!(bch_decode_version(w ^ 0b1111), None);             // 4 flips
    }
```
Plus integration-style tests using synthetic renders: rasterize a v7 and a v20 code axis-aligned via the `qrcode` dev-dep into a luma buffer (module scale 4 px, quiet zone, 25/235 levels — a ~20-line test rasterizer in tests support), build the transform from known corners, assert `read_version_bits` returns the right version and `count_timing_transitions` returns dim−13 for a v3 render.

---

### Task 3: Alignment patterns (`alignment.rs`)

**Interfaces:**
- `pub(crate) fn alignment_coords(version: u32) -> &'static [u8]` — ISO 18004 Annex E row/col center table (transcribe zxing `Version.java` ALIGNMENT_PATTERN_POSITIONS for v1..40; v1 = empty).
- `pub(crate) struct AlignmentGrid { pub coords: Vec<u8>, pub found: Vec<Option<[f64; 2]>> /* (coords.len())² minus the 3 finder corners */ }`
- `pub(crate) fn locate_alignment_patterns(view, grid, provisional: &PerspectiveTransform, version, inverted) -> AlignmentGrid` — iterate grid nodes in raster order skipping the 3 finder corners; predict each position by the parallelogram rule from already-found neighbors (`AP(i−1,j) + AP(i,j−1) − AP(i−1,j−1)`, falling back to the provisional transform when neighbors are missing); re-center with the ±2.25-module concentric probe: scan the binarized 5-module neighborhood for the 1:1:1 dark-light-dark cross-section in both axes centered on a dark module (an alignment pattern is 5×5: dark ring, light ring, single dark center) — record the refined center or None.

**Tests:** table spot checks (v2 → [6,18]; v7 → [6,22,38]; v32 → [6,34,60,86,112] — the irregular one; v40 → [6,30,58,86,114,142,170]); synthetic-render test: rasterize a v7 (has 6 APs minus... coords 3² −3 = 6 patterns) axis-aligned, locate with the exact transform, assert all found within 0.5 module of analytic positions; prediction fallback test: feed a provisional transform with a deliberate 1-module bias and assert re-centering still lands within 0.5 module (the probe's job).

---

### Task 4: Grid sampling (`sample.rs`)

**Interfaces:**
- `pub(crate) fn provisional_transform(t: &TripletCandidate, dimension: u32) -> PerspectiveTransform` — zxing `createTransform`: map module-space finder centers (3.5,3.5), (dim−3.5, 3.5), (3.5, dim−3.5) plus the estimated fourth point (dim−3.5, dim−3.5) extrapolated by parallelogram (no alignment yet) to image px; use `PerspectiveTransform::square_to_quad` on module-space normalized coords (extend homography.rs with `quad_to_quad(src, dst) -> Option<Self>` = `square_to_quad(dst) ∘ square_to_quad(src)⁻¹` — add with unit tests: 4 exact correspondences + round-trip).
- `pub(crate) fn sample_grid(view, grid, t: &TripletCandidate, dimension, alignment: &AlignmentGrid) -> Option<SampledGrid>` where `SampledGrid { bits: BitMatrix, regions: Vec<SampleRegion>, oob_fraction: f64 }`, `SampleRegion { module_rect: [u32; 4], transform: PerspectiveTransform }`:
  - v1 / no found APs: one region, transform anchored on the 3 finder centers + BR extrapolation (when the BR-nearest AP was found, use it as the 4th anchor — zxing Java behavior).
  - Otherwise (zxing-cpp GridSampler ROI approach): tile module space by the alignment coordinate intervals; each cell's transform maps its module-space corner quad to the 4 anchor positions (found AP or predicted/parallelogram position); sample each module center (+0.5) through its region's transform; polarity-aware bit = `(pixel < threshold) != inverted`; count out-of-image samples (clamped reads count as OOB).
- Reject when `oob_fraction > 0.02`.

**Tests:** synthetic renders again (axis-aligned + a 30°-rotated + a perspective-warped v2 and v7 produced by rasterizing with the test rasterizer through a known homography): assert the sampled BitMatrix equals the `qrcode` crate's matrix bit-for-bit (this is the strongest possible unit gate and needs no fixtures). Plus an OOB test: a code half outside the frame → rejected.

---

### Task 5: Decode orchestration + arbitration (`decode.rs`, scanner wiring)

**Interfaces:**
- `pub struct DecodedCode { pub payload: String, pub payload_bytes: Vec<u8>, pub version: u32, pub ecc: char, pub mirrored: bool, pub dimension: u32, pub corners: [[f64; 2]; 4] /* TL,TR,BR,BL of the module region via the final transform */, pub inverted: bool, pub finder_indices: [usize; 3] }`
- `pub(crate) fn decode_candidates(view, grid, finders, triplets) -> (Vec<DecodedCode>, Vec<DecodeAttemptTrace>)`:
  1. Proximity-dedup triplets (shared ≥2 finder indices → keep lower snap_error). Requires `TripletCandidate` to carry its three finder indices — add `pub finder_indices: [usize; 3]` in triplet.rs (small recorded change; gates unchanged).
  2. Attempt in snap_error order, cap `MAX_DECODE_ATTEMPTS`, skipping triplets whose finders were consumed by an earlier success.
  3. Per attempt: dimension from triplet → timing cross-check (≤41) adjusts within ±2 → provisional transform → version bits (est ≥ 7) override dimension (rebuild transform on change — the zxing-cpp trick and the ver_12_v40 fix) → alignment → sample → decode_bits (+ mirrored retry inside) → on success consume finders.
  4. Every attempt records a `DecodeAttemptTrace { triplet_index, dimension_est, dimension_final, timing_check: Option<u32>, version_bits: Option<u32>, alignment_found: u32, alignment_total: u32, oob_fraction: f64, outcome: String /* "decoded" | failure reason */ }`.
- `detect()`/`detect_with` gain the stages; `Detections` gains `codes: Vec<DecodedCode>`; `StageTimings` gains `version_ns, alignment_ns, sample_decode_ns`.

**Gate test `crates/qr-lab-core/tests/decode_gate.rs` (structure verbatim, assertions per the Global Constraints gates):** iterate `common::load_all()`, `detect`, assert per-code payload/version/mirrored exact and per-fixture decoded count == ground-truth count; then the real-capture section: decode `fixtures/real/real_{1,2}.png` via the `png` dev-dep (grayscale or RGB→luma via `luma_from_rgba`), NN-downscale to 1280 with a test-local copy of the production formula, assert the pinned payloads. Report per-prefix decode stats.

---

### Task 5b (recorded amendment after the first gate run): image-derived 4th corner for v1/no-AP candidates

The first gate run failed 15/81 fixtures and real_1 — all v1 codes under genuine perspective, where the provisional transform's affine parallelogram BR estimate diverges (Task 4's documented limitation, now shown to bite the product's primary scenario: small stickers photographed obliquely). Recorded decision: estimate the 4th corner from the image instead (zxing-cpp edge-tracing practice; also the original GPU scanner's `improve_corners` concept — perpendicular boundary probing + line fit + intersection — applied at detection precision now; Plan 5 upgrades it to full-res gradient subpixel).

`refine_fourth_corner(view, grid, t, dimension, provisional) -> Option<[f64; 2]>`:
- Probe the module-region **bottom edge**: at ~8 positions along modules `x ∈ [7, dim−7]` (avoiding finders/corner rounding), walk perpendicular (±1.5 modules around the expected boundary from `provisional`) on the binarized image to locate the last ink→background transition; likewise the **right edge**. Polarity-aware.
- Require ≥5 accepted points per edge; least-squares line fit each; intersect → BR outer corner (module-space `(dim, dim)`).
- On success: rebuild the sampling transform from mixed anchors [(3.5,3.5), (dim−3.5,3.5), (dim,dim)→BR, (3.5,dim−3.5)] via `quad_to_quad` (4 arbitrary correspondences form valid quads). On failure: parallelogram fallback (previous behavior).
- Applied when no Found BR alignment anchor exists. Constants carry provenance; gates unchanged (this is an algorithm fix, not tuning).

### Task 6: Trace + envelope + debug-UI overlays

- `Trace` gains `attempts: Vec<DecodeAttemptTrace>`, `alignment: Vec<AlignmentTraceEntry { predicted: [f64;2], found: Option<[f64;2]> }>` (per last attempted candidate), `sample_regions: Vec<SampleRegionTrace { module_rect, quad: [[f64;2];4] }>`, `bits: Option<BitsTrace { dim, words: Vec<u32> }>`, and `Detections.codes` flows through existing serialization. Regenerate snapshot (UPDATE_SNAPSHOT=1); extend `types.ts` + `parseScanResult` (snapshot wins on names); envelope tests extended.
- New overlays (with fake-canvas tests, registered in App): `alignment` (predicted × vs found ● markers), `samplegrid` (region quads via existing TS homography — draw region borders + every 4th module line to keep draw counts sane at v40), `bits` (semi-transparent module fill from packed words — only when zoomed in: skip drawing when `view.scale * module < 4` px, documented), `decoded` (payload text + version/ecc/mirrored badge at code center; red badge + failure reason for failed attempts from `attempts`).
- Timings panel rows for the three new stages (data-driven if the panel hardcodes rows — adjust).

---

### Task 7: QA + docs + wrap

- Re-run the scripted headless-Chrome QA additions: near_00 (payload badge "Q:near_00:0"), ver_12_v40 (badge shows v40 — the dimension-override fix visible), mirror_00 (mirrored badge), multi_07 (exactly 4 decoded, no spurious), real/real_2 @1280 (R8.HR payload ON the sticker), invtrans_02 (inverted+transparent decodes). Screenshots to scratchpad; per-item PASS/FAIL table.
- Host perf check: `scan_fixture` example prints new stage timings; record @720p totals in the report (budget awareness for Plan 5 — decode stages should be well under 1 ms/code).
- Update debug-ui README (new layers), root README (pipeline status), and the plan's follow-ups section with anything deferred.

---

## Self-review notes

- **Spec coverage (milestone 4 / spec §3 stages 4–7):** version cross-checks ✓ (Task 2, incl. the both-orientations version-bit read), alignment Annex E + parallelogram + re-centering ✓ (Task 3), piecewise sampling with single-transform fallback ✓ (Task 4), rqrr BitGrid + mirrored retry ✓ (Task 1), arbitration + candidate cap ✓ (Task 5, closes the Plan-2/3 recorded findings), trace/overlays/snapshot ✓ (Task 6), real-capture payload regression ✓ (Task 5 gate 2 — the payloads OpenCV confirmed).
- **Type consistency:** `BitMatrix`/`DecodedCode`/`DecodeAttemptTrace`/`SampledGrid` names used consistently across Tasks 1–6; `TripletCandidate.finder_indices` addition recorded in Task 5 and consumed by arbitration.
- **Known risks:** rqrr's public API surface for external grids (BitGrid is public per research; ECC-level exposure uncertain — Task 1 handles honestly); synthetic-render test rasterizer duplication with testpaint.rs (Task 2 may extend testpaint rather than write anew — implementer's call, noted); trace size at v40 in video mode (mitigated by packed words + region-not-point serialization + the bits-layer zoom gate); real-capture gate depends on the two hardcoded payloads (verified twice: OpenCV decode + visual).

## Post-merge follow-ups (recorded at final review, 2026-07-04)

Items surfaced during final review of the decode pipeline that are deliberately deferred rather than blocking this merge — none regress a gated behavior, all are scoped forward (mostly to Plan 5) or recorded as accepted, monitored risk.

a. **decimate-detect / full-res-sample redesign.** `IMG_4832` evidence: far codes in that frame *detect* at the 1280 working resolution (3 triplets found) but can't be sampled/decoded there — at that distance the modules land at roughly 2px/module, below what perspective sampling can resolve; the same frame decodes successfully once the working resolution is raised to 2560. The fix is a resolution-decoupled pipeline — detect at a small, fast working resolution, then re-sample the located candidate at (or near) the source resolution — which is squarely Plan 5 scope (subpixel corner refinement already lives there) and should absorb this finding, including the subpixel-refinement work already planned for corners.
b. **v40 `sample_decode` cost.** Measured ~24.5ms host-side for a v40 candidate's sample+decode stage alone. Not a regression (no budget was set for this in Plan 4), but worth a dedicated Plan 5 performance pass — v40's per-module sampling cost dominates at that dimension (177×177 modules) and is the natural next target once correctness work stabilizes.
c. **Attempt cap should count rounds, not attempts.** `MAX_DECODE_ATTEMPTS` (`consts.rs`, currently 24) counts *attempts* (one triplet at one corner-rotation), but since Task 5b a single attempt can now spend up to 3 sample+decode *rounds* (parallelogram + up to two edge-fit refinement retries — see `DecodeAttemptTrace::rounds`). The cap's original "≈4 codes/frame × 3 rotations × 2 headroom" budget reasoned in attempts, not rounds, so the real worst-case round count per frame is now up to 3x higher than that budget assumed. Not a correctness bug — every round is still RS-validated before anything is trusted — but the cap should be re-derived (or re-expressed) in rounds so its stated provenance stays accurate; left for a follow-up pass rather than re-deriving the constant under final-review time pressure.
d. **Rotation-retry starvation of the 9th+ candidate on cluttered frames.** `decode_candidates`' corner-role rotation retry (see `rotated_corner_roles_still_decode_via_rotation_retry`) can, on a sufficiently cluttered frame with many candidate triplets, consume enough of the attempt cap retrying mis-assigned corner roles that a legitimate 9th-or-later candidate never gets attempted. Acceptable within this project's stated 4-codes-per-frame envelope (the cap's own headroom comfortably covers it there); flagged to monitor if real-world frame counts grow past that envelope, not to fix now.
e. **Dead timing-adopt branch.** The dimension-refinement pipeline's timing cross-check (module doc, step 1) has a code path for adopting the timing-derived dimension that, per the current gate suite and real-capture corpus, is never actually exercised as live/reachable in practice (the version-info-bits check or the geometric estimate already agree in every observed case that reaches it). Recorded to either gain a `debug_assert!` documenting the invariant that makes it unreachable, or be removed outright, once someone re-verifies which — left as a follow-up rather than deleting code whose necessity hasn't been re-confirmed at final review.
f. **EXIF orientation mismatch, host vs. browser.** The browser path's `createImageBitmap` (see `debug-ui/src/App.tsx` and `useImageSource.ts`) honors EXIF orientation tags automatically; the Rust host-side examples (e.g. `crates/qr-lab-core/examples/decode_photo.rs`) do not perform any EXIF-orientation correction. This means a real photo with a non-identity EXIF orientation tag can present a *different* pixel frame (rotated/flipped) to the host pipeline than to the browser pipeline for the exact same source file. Not a bug in either path individually — recorded so a future cross-checked host/browser comparison isn't misattributed to a decode regression when it's actually this orientation mismatch.
g. **Timing exact-agreement + alignment-probe noise robustness.** The timing cross-check's "adopt within ±2 modules" rule and the alignment concentric re-centering probe's tolerances are both derived from the synthetic fixture suite plus the two real captures on hand; as the real-capture corpus grows, both should be revisited against a wider noise distribution (sensor noise, compression artifacts, motion blur) than what two captures can characterize. No action needed now — recorded so this isn't forgotten once more real photos are available.
h. **`samplegrid` overlay draws region borders only.** Task 6's spec called for the sample-grid overlay to draw "region borders + every 4th module line" for a visual sense of the sampled grid's density. The shipped `debug-ui/src/overlays/layers/samplegrid.ts` deliberately draws only the region-border quads (see that file's own header comment) — expanding `SampleRegionTrace`'s corner-quad-only serialization back into a per-module grid client-side was judged not worth the added draw cost at v40 (up to 36 regions). Recorded as an intentional deviation from the original task description, not an oversight.
