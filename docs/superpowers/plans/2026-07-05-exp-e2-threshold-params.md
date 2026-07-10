# Experiment E2 — contrast/threshold rung parameterization (results)

Status: complete (2026-07-05). Verdict: **no parameter change ships** — the
shipped ThresholdFamily ({offset −8, offset +8, contrast_floor 6} on the
working view) plus the Sauvola rung (k = 0.2) is confirmed as the right
composition by isolated per-spec sweeps and end-to-end ladder runs. Every
candidate either duplicates existing coverage, trades wins for losses under
the variant budget, or reaches only below the sensor-noise margin. This doc
records the data so the next person does not re-run the sweep.

Method: isolated per-`BinarizeSpec` detect passes (order-independent
leave-one-out yield) over all 149 fixture codes at working dims {native
(≈720p), 960, 640} and 240 real video frames (max-dim 1280), plus
integration runs of the full bench protocol for the one candidate that
showed unique reach. Host-scalar release build; reference numbers reproduced
exactly before measuring (baseline 130/138 dec/det, robust-fast 138/145 @
8.94 ms mean / 41.2 ms p95, robust-full 139/145; real frames 14 baseline /
20 robust-fast / 22 robust-full).

## (a) Additional offsets {±4, ±12, ±16}

Marginal decode yield (isolated, working view) beyond the shipped
{default, ±8, floor 6, Sauvola} union:

| offset | dim0 | dim960 | dim640 | real frames | note |
|---|---|---|---|---|---|
| ±4 | +0 | +0 | +0 | +1 frame (off+4) | frame also decoded by robust-full via a later rung |
| +12 | +0 | +1 (illum_03) | +0 | +1 frame (f038) | illum_03 reached by +8 at dim0 |
| +16 | +1 (illum_02) | +1 (illum_03) | +0 | +0 | illum_02 already decoded by Upscaled2x |
| −12/−16 | +0 | +0 | +0 | +0 | strictly nothing |

Leave-one-out inside the shipped family: `+8` earns 1 unique code at every
dim and 1 unique real frame; `−8` earns 0/0/1 (defocus_02@640) — the
symmetric pair is kept because the dot-gain/ringing mechanism is signed both
ways and `−8` does contribute at reduced working resolution.

Cost per extra offset: one full re-scan of the working view ≈ **2.37 ms
mean @720p** (max 14.4 ms), paid on every frame that escalates past the
pyramid rungs. Under `ROBUST_FAST`'s cap of 8 variants the marginal effect
is dominated by *displacement*, not by the offset's own yield: integrating
`+16` as a 4th family member gave dim0 robust-fast **+1 decode** (illum_02,
mean 8.94→8.23 ms because a cheap threshold scan displaced the expensive
Sharpen/Upscale tail) but **−1 decode at dim960** (shadow_04 — the extra
slot pushed ShadowNormalized past the cap), ±0 at 640 and on real frames.
Zero-sum budget shuffle ⇒ not shipped.

Mechanism found for the large-offset wins: both illum fixtures are glare
discs; where the 3×3-dilated tile window straddles the glare boundary the
min/max midpoint is dragged below the lifted ink level, and the offset
needed grows with the illumination step across the window — i.e. it is
unbounded and resolution-dependent (bigger at smaller working dims, where a
tile covers more source area). That failure axis is owned by the Sauvola /
shadow-division / upscale rungs (upscale halves the effective tile window in
source terms, which is exactly why Upscaled2x already decodes illum_02);
deepening the offset ladder is the wrong tool for it.

## (b) contrast_floor 4 (2σ)

Fixture suite and real frames: floor 4 ≡ floor 6 ≡ default — identical
decode/detect sets, identical finder counts, at all three dims (the suite
has no tile whose dilated range falls in [4, 12)). Synthetic probe
(contrast crush of near_00 around mid-gray):

| ink/paper separation | floor 12 | floor 6 | floor 4 |
|---|---|---|---|
| ≈9.8 gray | skip-all | decodes | decodes |
| ≈6.6 | skip-all | decodes | decodes |
| ≈4.9 | skip-all | skip-all | **decodes** |
| ≈3.3 | skip-all | skip-all | skip-all |

So floor 4 does reach a real class (separation ∈ [4, 6)). Cost probe on
flat frames with deterministic uniform noise: at amplitude ±2 (range 4) the
frame costs 1.07 ms under floor 6 (all tiles skipped) vs **5.16 ms under
floor 4** (~5× churn, 4 false finder candidates); floor 6 pays the same
penalty one step later (±3). Verdict: not shipped — separation < 6 is below
3σ of the crate's σ≈2 sensor-noise model, so the extra reach is exactly the
band where real frames are noise-dominated; 6 = 3σ is the detection-theory
floor and stays.

## (c) Sauvola k ∈ {0.2, 0.3, 0.5}

Overall isolated decode (working view): k=0.2 → 125/115/104 at
dim0/960/640; k=0.3 → 123/111/105; k=0.5 → 119/107/102. On the target
families: all three k recover combo2_01 + shadow_04 at dim0, but k≥0.3
loses defocus_02 (and more elsewhere) — the same threshold depression that
cleans a shadowed quiet zone erodes thin dark modules in well-lit regions.
At dim960 k=0.3/0.5 uniquely reach shadow_04 (k=0.2 misses it there);
k=0.3 also uniquely decodes 1 real frame (11-37-31_f036 — which robust-full
already decodes via another rung). Net: k=0.2 (Sauvola's published
document-ink constant) dominates as the single value; a *second* Sauvola
variant at k=0.3 would buy ~1 mid-res fixture code + 0 net real frames for
3.3 ms and a budget slot ⇒ not shipped.

## (d) Threshold sweep on the pyramid 0.5× level

Rejected decisively. The shipped specs run on the 0.5× box level cost
~0.7 ms each (~3.5× cheaper, as predicted) but the family union collapses:
**misses 17 / 35 / 75 of the work-level recoveries at dim0/960/640**
(all lowres/multi/large-version codes plus the marquee shadow_04 and
combo2_01 Sauvola wins), adding only 1–2 chance-level codes (e.g. occl_01
via off+8@half — a run-quantization fluke on an occluded finder, no
mechanism). Mechanism: threshold recovery targets *decode-limited* codes,
and decode is run-quantization-limited — halving resolution doubles
quantization error, destroying exactly the codes the sweep exists to save.
Threshold variants may *compose* with pyramid levels for detection, but the
recovery value lives at the highest-resolution view only.

## Incidental findings (for E1 / ladder-ordering follow-up)

- `LowContrastFloor` (floor 6) yields zero on the entire fixture suite and
  all 240 real frames at every dim; its only measured value is the
  contrast-crush class (gate3a's synthetic probe, separation ∈ [6, 12)).
  It is kept on principle (the class is real; fixtures verify, never tune)
  but it is the family's weakest slot under the cap. It also costs more
  than the offset variants when it does run (4.55 ms vs 2.37 ms mean —
  unskipped low-contrast tiles create extra scan work).
- `ROBUST_FAST`'s cap of 8 makes Upscaled2x (variant #9 on the
  triplet-evidence path) unreachable at dim0 — that alone is the entire
  robust-fast vs robust-full gap there (illum_02). 12 fixtures
  budget-exhaust at dim0, 19 at 960. Re-ordering / re-budgeting the tail
  (e.g. yield-per-ms order: Sauvola 3 codes/3.3 ms > Upscale 3–4/11.9 >
  off+8 1/2.4 > Shadow ~1/12.6 > off−8, floor6 0) is E1/architecture
  territory, not a threshold-parameter change.
- Per-variant mean cost @720p working (robust-full, dim0): Pyramid 0.51 ms,
  ThresholdOffset 2.37 ms, Sauvola 3.27 ms, LowContrastFloor 4.55 ms,
  Sharpened 7.33 ms, Upscaled2x 11.90 ms, ShadowNormalized 12.56 ms,
  DirectionalSharpened 13.96 ms.

Raw data: isolated sweeps + integration JSONs were produced by an
`#[ignore]`d harness (not committed — this doc and the numbers above are the
artifact); the harness is trivially reconstructable: run
`detect_with_source` per `BinarizeSpec` over `fixtures/` and score against
`corners_px`/`payload` ground truth.
