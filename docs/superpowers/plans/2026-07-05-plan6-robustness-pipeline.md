# Plan 6 — Robustness pipeline: adaptive escalation ladder + fixture matrix expansion

Status: implemented + experiments merged (2026-07-05). Results in §8.
Driving evidence: real video frames (`fixtures/real/domain-data-mp4/`, `video_f167.png`)
fail under (1) motion blur, (2) partial shadow / uneven illumination, (3) low
apparent resolution. Goal: CPU-only robustness improvements behind opt-in scanner
config, measured by an expanded fixture matrix, structured as a staged recovery
ladder — never an exhaustive preprocessing sweep.

Constraints carried forward (see design spec + project memory):

- Frame budget: ≤5 ms/frame @~720p on a Pixel 7 big core (spec:11) — extrapolated,
  not yet gated. Exceeding it does not disqualify an experiment, but each rung's
  cost must be measured and the default ladder must stay near budget.
- Baseline behavior unchanged by default: `scan(max_working_dim:0, refine:false)`
  stays bit-identical to `detect()` (pinned by decode_gate.rs:385-416). All
  robustness features are opt-in booleans that cost literally nothing when off.
- No overfitting to fixtures: every constant needs a principled derivation
  (QR geometry, sampling theory, sensor physics, or cited prior art). Gates
  verify, never tune.
- Determinism: identical input → identical output bytes. Therefore the ladder's
  cost cap is **work-based (variant count), not wall-clock-based** — a ms budget
  would make output depend on host speed.
- `#![forbid(unsafe_code)]`, no new heavy deps, single-threaded.

## 1. Research summary

Nine parallel research/mapping passes (zxing-cpp, AprilTag, ArUco, BoofCV,
Dynamsoft, DIBCO/document binarization, QR-deblurring literature, Nyquist-limit
QR study) converge on a small set of load-bearing facts:

1. **The codebase already implements the right skeleton.** "Detect at working
   resolution, sample+refine at source resolution" (scan.rs → SourceView) is
   exactly AprilTag's `quad_decimate` + full-res decode and zxing-cpp's pyramid
   + rescaled positions. The ladder generalizes this from one (working, source)
   pair to per-variant provenance.
2. **The binarizer is the AprilTag tile min/max scheme** (16px tiles, 3×3
   dilation, midpoint threshold, CONTRAST_FLOOR=12 skip). It is already
   locally adaptive — so whole-frame contrast enhancement (CLAHE, gamma,
   histogram stretch, local contrast normalization, Laplacian pyramids) is
   **redundant by construction**: a local threshold is invariant to smooth
   monotonic illumination inside its window. The high-leverage contrast knobs
   live *inside* the threshold formula (offset, contrast floor, flat-tile
   policy), not in front of it, and cost zero pixel passes.
3. **Detection and decode fail at different degradation levels and need
   different treatment.** Finder run-ratio matching (~±50% tolerance) survives
   blur up to ~1–1.5 modules and is polarity/scale tolerant; 1-module data
   cells corrupt much earlier. So: run detection on raw or mildly processed
   buffers; reserve aggressive recovery for decode, where the ROI, orientation
   and module pitch are already known and every constant can be expressed in
   module units.
4. **Cost ordering discipline:** module-space operations (decode-round retries,
   the existing Fix-B sharpening round, mirrored retry) are O(modules) ≈ µs;
   pixel-space rungs are O(pixels) ≈ ms — a ~1000× gap. Exhaust all module-space
   retries on existing candidates before firing any pixel rung. ROI-scoped
   pixel work (~200×200) is ~20–50× cheaper than full-frame.
5. **NN downscale aliases QR patterns.** A QR is a near-Nyquist checkerboard;
   nearest-neighbor decimation can delete or double entire 1-module runs.
   Pyramid levels for detection must be 2×2 box-averaged (also denoises: σ/2
   per octave). The existing `downscale_luma` NN path is pinned to the debug-UI
   formula and stays for parity; pyramid rungs get a new box kernel.
6. **Octave scale steps are principled; finer steps are waste.** A run-ratio
   detector responds over >2 octaves of px/module, so factor-2 levels
   (1.0×/0.5×/0.25×, stop near max-dim ~400–500px — zxing's field-tested
   threshold) give overlapping coverage. The scale ladder subsumes ArUco-style
   threshold-window sweeps (same coverage, geometric instead of linear cost).
7. **Upscaling helps only via gray edge information.** At <3 px/module,
   ±0.5px run quantization exceeds the 1:1:3:1:1 tolerance; the missing
   information survives in anti-aliased edge pixels. Bilinear 2× (monotone,
   ring-free) converts it into measurable runs; Lanczos/bicubic ring on step
   edges; pixel-art/edge-directed upscalers actively vandalize checkerboard
   data regions. The long-term principled fix is sub-pixel run measurement
   (interpolated threshold crossings — mathematically equivalent to bilinear
   2× at ~1/10 the cost); the ladder rung ships the 2× buffer first because it
   reuses the pipeline unmodified.
8. **For shadows, the hard case is a sharp shadow edge crossing the symbol**,
   not a smooth gradient (tiles already handle those). Averaging-based
   normalizers (Retinex, CLAHE) halo exactly at that edge; envelope/windowed
   estimators degrade gracefully. If normalizing, **divide** by the background
   estimate (illumination is multiplicative) with a floored denominator —
   never subtract.
9. **For motion blur**, direction is estimable almost free (structure-tensor
   minor axis, ~0.1–0.3ms/ROI, reliable exactly when blur is strong), and
   extent from the 20–80% edge-rise width of a located finder border
   (L = rise/0.6, exact for box blur). Directional 1-D sharpening along θ gets
   ~2× the restoration per unit noise vs isotropic. FFT Wiener is affordable
   at ROI scale (1–5ms @256²) as a deep rung with 3-candidate PSF sweep +
   decode-checksum-as-oracle (Sörös). Full blind deconvolution: 100ms–1s,
   out of budget, skip.
10. **In video, temporal strategies change the cost model**: per-frame variant
    rotation makes a tall ladder affordable at constant per-frame cost; ROI
    tracking + frame-sharpness gating beat heavy restoration. These need a
    multi-frame session API — deferred to a follow-up plan, but the ladder's
    variant records are designed to support rotation.

### Verdict table

| Technique | Verdict | Why |
|---|---|---|
| Box-filter detection pyramid (1.0/0.5/0.25×) | **likely useful** | zxing/AprilTag-proven; ≤1.33× cost; fixes large-module + noisy frames |
| Threshold offset / contrast-floor sweep on shared tile stats | **likely useful** | The cheap realization of the whole contrast axis; no pixel passes |
| Bilinear 2× upscale rung (frame if small, else ROI) | **likely useful** | Only rung reaching <3 px/module codes; ring-free |
| Background division (box-blur estimate, floored divide) | **likely useful** | The shadow-edge fix; O(N) via running-sum boxes |
| Unsharp mask (fixed-point, detection buffer only) | **likely useful** | Only rung reaching mild blur; must stay off noisy rungs |
| Structure-tensor blur direction + edge-rise extent gating | **likely useful** | ~free; routes the whole blur tier |
| Directional 1-D sharpen along θ | **likely useful** | Encodes the 1-D line-PSF physics at convolution cost |
| Sauvola/Wolf threshold surface | worth experimenting | May be redundant with tile dilation + pyramid; DIBCO transfer |
| zxing flat-tile threshold inheritance (vs skip) | worth experimenting | Alternative flat-region policy; A/B vs CONTRAST_FLOOR skip |
| ROI Wiener deconvolution (3 candidate PSFs) | worth experimenting | 1–5ms/ROI; the only true multi-module-smear inverter |
| Richardson-Lucy (1-D spatial) | worth experimenting | Only after Wiener output rings; 3–18ms/ROI |
| Bicubic (vs bilinear) upscale | worth experimenting | Expect a wash; cheap A/B |
| Sub-pixel run measurement (threshold crossings) | worth experimenting (phase 2) | Dominates upscaling in principle; invasive detector refactor |
| Temporal accumulation / ROI tracking / variant rotation | worth experimenting (phase 2) | Needs ScanSession API + video fixtures |
| CLAHE / adaptive histogram equalization | **not worth it** | Redundant with tile threshold; halos + dark-noise speckle |
| Gamma correction / global contrast stretch / LCN | **not worth it** | Monotonic global remaps ≈ no-ops for local-order-based decisions |
| Laplacian-pyramid contrast enhancement | **not worth it** | 5–10× cost of a threshold sweep for the same codes |
| Lanczos upscale | **not worth it** | Rings on binary content; strictly dominated by bilinear |
| Edge-directed (NEDI/ICBI) & pixel-art (hqx/xBR) upscalers | **not worth it** | Hallucinate diagonal connections in checkerboard data |
| Blind deconvolution (Sörös-style full loop) | **not worth it** | 100ms–1s/ROI; in video, skipping to a sharper frame is cheaper |
| Morphological closing background (detect-time) | **shipped (E3)** | "unknown module size" objection resolved by sizing the SE from baseline finder evidence (10 modules); beats the box-blur estimate: no halo at sharp shadow/glare edges, recovers narrow shadow bands, cheaper (4 O(N) van Herk passes) |

## 2. Pipeline architecture — the escalation ladder

One new entry point (existing entry points untouched):

```
scan_robust(source: &LumaView, opts: &ScanOptions, cfg: &ScanConfig) -> RobustDetections
```

Ladder (each stage gated by its config flag; a disabled stage costs nothing):

- **Stage 0 — baseline**: exactly today's `scan(source, opts)`. If it decodes
  ≥1 code and leaves no undecoded-triplet evidence, return (early exit).
  *Escalation trigger*: `codes.is_empty()` OR undecoded triplets remain
  (triplets are positive evidence a code is present — the strongest signal).
- **Stage 1 — multi-scale** (`enable_multi_scale`): 2×2 box-averaged pyramid
  levels at 0.5× (and 0.25× while max-dim > ~400px) of the working view.
  Detection re-runs per level; corners map back through the level scale;
  sampling + refinement read the true SOURCE via SourceView. Targets: noisy
  frames (box averaging halves σ per octave) and large-module codes
  (threshold-window mismatch).
- **Stage 2 — contrast / threshold recovery** (`enable_contrast_normalization`):
  re-binarize + re-scan the *existing* working view with perturbed tile-grid
  parameters — threshold offset ±8 gray (~3% of range ≈ print dot-gain /
  JPEG ringing amplitude) and a lowered contrast floor (6 = 3σ sensor noise
  vs the default 12 = 6σ) for glare-washed / low-contrast codes. Zero new
  pixel buffers; tile stats are shared.
- **Stage 3 — shadow recovery** (`enable_shadow_normalization` /
  `enable_adaptive_thresholding`): geometry-preserving luma rewrite:
  background estimate B via a grayscale morphological CLOSING (van Herk
  separable rect SE, O(N) independent of SE size; SE = 10× the largest
  baseline finder-candidate module — the widest solid ink structure is the
  finder ring, 7 modules, ≤7√2 axis-aligned under tilt — falling back to
  max(w,h)/6 with no evidence), then `out = clamp(I·200 / max(B, 8))`
  (divide, never subtract; floor = 2–3× sensor noise). The envelope does
  not smear across sharp shadow edges (E3: the earlier 3×-box-blur estimate
  haloed exactly there and yielded zero first-decodes) and tracks shadow
  BANDS narrower than a fixed window. Re-scan the normalized buffer as its
  own source (geometry unchanged, so corners are valid source px). `enable_adaptive_thresholding`
  additionally tries a Sauvola-style threshold surface (integral images,
  k=0.2, R=128, window = width/8) as an alternative binarization.
- **Stage 4 — sharpness recovery** (`enable_sharpening`): 5-tap separable
  fixed-point unsharp ([1,4,6,4,1]/16, amount 0.6–1.0) on the working view,
  detection + binarization only — module sampling still reads the raw source
  (ringing must never touch sampling). Never combined with pyramid levels
  (downscale already low-passed what sharpening restores; opposite failure
  classes).
- **Stage 5 — low-res recovery** (`enable_low_res_upscaling`): fixed-2×
  bilinear upscale (constant weights, u16 fixed point) of the working view
  when estimated px/module of undecoded triplets < 3 (Nyquist-study floor
  3–3.5 px/module, not fixture-tuned) or when nothing detected and
  max-dim ≤ ~800. Corners /2 back; sampling reads original source.
- **Stage 6 — deblur** (`enable_deblur`): per undecoded-triplet ROI:
  structure-tensor θ (fire only when anisotropy confidence > 0.3), 1-D
  directional unsharp along θ with support < module/2; retry decode. Wiener
  (ROI FFT, candidate lengths {0.5m, 1m, 1.5m}) is a phase-2 experiment
  behind the same flag.

Cross-cutting rules:

- **Early exit** (`enable_early_exit`, default on for robust presets): stop as
  soon as every triplet-evidence code has decoded, or nothing suggests a code
  remains. `max_variants_per_frame` caps total variant scans (work-based
  budget, deterministic).
- **Dedup**: a decoded code is new iff no already-accepted code has the same
  payload with center distance < half its module span (multi-code frames can
  repeat payloads at different positions). First (cheapest) variant wins;
  later duplicates only update provenance stats.
- **Reuse**: the working view, tile stats per (buffer), and the pyramid chain
  are computed once and shared across rungs. Buffers come from per-call Vecs
  (no globals) following the downscale_luma ownership pattern.
- **Provenance**: every variant scan appends a `VariantRecord { stage, variant
  (params), timings, finders/triplets/codes counts, new_codes }`; every
  accepted code carries `origin: VariantId`. `RobustDetections` exposes
  `variants: Vec<VariantRecord>`, `early_exited: bool`, and total timings —
  everything the benchmark needs to rank rungs by marginal yield per ms.

## 3. Multi-dimensional pyramid design

Axes and their *realization* (materialize as little as possible):

- **Resolution axis** (real buffers): source → working (existing NN cap for
  parity) → box 0.5× → box 0.25×; plus bilinear 2.0× as a recovery level.
  Levels are built lazily and cached for the frame (mip-chain arena).
- **Illumination axis** (parameters, not buffers, wherever possible):
  {default tile midpoint, offset −8, offset +8, low contrast-floor,
  Sauvola surface} — all reuse per-level tile/integral stats. Only
  background-division materializes a buffer (it must rewrite pixels).
- **Sharpness axis** (buffers, gated): {raw, unsharp, directional-θ (ROI),
  Wiener candidate (ROI, phase 2)}.

Combination policy (what is generated vs avoided):

- Threshold variants compose with every resolution level (nearly free).
- Sharpening composes only with levels ≥1.0× (never below — mechanism
  conflict) and never feeds module sampling.
- Upscale composes only with the *source/working* level and only under the
  px/module<3 trigger; never upscale an already-sharpened buffer (stacked
  overshoot).
- Background division runs at working resolution only; its output may feed
  the threshold-variant sweep but not the sharpening rung in v1 (bounded
  matrix).
- Enumeration order = expected marginal-yield per ms, cheapest first;
  `max_variants_per_frame` truncates the tail deterministically.

Worst-case default `robust_fast` ladder at 720p (host-scalar estimates):
pyramid build ~0.3ms + 0.5× scan ~2.5ms + two threshold variants ~2×(rebin+scan)
+ background-divide ~2ms + rescan — bounded by `max_variants_per_frame = 8`.
Expected cost on decodable frames ≈ baseline (early exit at stage 0).

## 4. Scanner config

Rust style, following ScanOptions conventions (Clone+Copy+Debug, pub fields,
0-disables sentinels, per-field cost documented; serde-Serialize feature-gated
on the *output* types only):

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScanConfig {
    pub enable_multi_scale: bool,            // stage 1: box pyramid detection rungs
    pub enable_contrast_normalization: bool, // stage 2: threshold offset / contrast-floor sweep
    pub enable_shadow_normalization: bool,   // stage 3a: background division rewrite
    pub enable_adaptive_thresholding: bool,  // stage 3b: Sauvola-style threshold surface
    pub enable_sharpening: bool,             // stage 4: unsharp (detection-only buffer)
    pub enable_deblur: bool,                 // stage 6: directional sharpen (ROI), Wiener (phase 2)
    pub enable_low_res_upscaling: bool,      // stage 5: 2x bilinear recovery level
    pub max_variants_per_frame: u32,         // 0 = unlimited; work-based budget (determinism)
    pub enable_early_exit: bool,             // stop when triplet evidence is exhausted
}
```

`ScanConfig::default()` = all false ⇒ `scan_robust` ≡ `scan` + provenance
wrapper (stage-0 only), preserving baseline behavior. Presets:

- `ScanConfig::BASELINE` — all off.
- `ScanConfig::ROBUST_FAST` — multi_scale + contrast + shadow + adaptive_threshold
  + sharpening + low_res_upscaling, `max_variants_per_frame: 8`, early_exit on.
- `ScanConfig::ROBUST_FULL_BENCHMARK` — everything on incl. deblur,
  `max_variants_per_frame: 64`, early_exit **off** (measure every rung).

Benchmarking/debug output (the user-proposed `enableVariantBenchmarking` /
`enableFixtureDebugOutput`) live in the *harness binary*, not the core config:
the core always records provenance (cheap), and the harness decides what to
emit. WASM: `scan_rgba` keeps its flat-arg surface; a follow-up adds a
`scan_rgba_robust` taking a serde-deserialized config object (new JS→wasm
pattern, snapshot-pinned).

`min_confidence_for_early_exit` from the proposal is deliberately dropped in
v1: the scanner has no calibrated per-code confidence scalar (rqrr does not
surface corrected-bit counts); the principled exit signal is structural —
undecoded triplet evidence. If a confidence scalar lands later (ECC margin),
the field can be added without breaking Copy.

## 5. Fixture generation matrix expansion

All new degradations are **post-render image-space ops** in the existing hook
(generate.py:41-45), driven by new `FixtureSpec` fields, deterministic via the
existing `_rng_for(seed, name)` scheme. New `Degradations` dataclass (all
optional, default off — existing fixtures stay byte-identical):

- `motion_blur: (length_px, angle_deg, curve)` — line-PSF convolution;
  curve ∈ {0 (linear), small quadratic bend} via 3-segment polyline kernel.
- `defocus: radius_px` — disc kernel.
- `shadow: (strength ∈ [0,1], softness_px, angle_deg, shape ∈ {half-plane, band, blob})`
  — multiplicative illumination field (divide-compatible ground truth).
- `illum_gradient: (min_gain, max_gain, angle)`; `glare: (center, radius, gain)`;
  `contrast_scale: float` (compress around mean for low-contrast).
- `resolution: (downscale_factor, method ∈ {area, nearest}, restore: bool)` —
  downscale (and optionally re-upscale to original dims to model soft low-res).
- `jpeg_quality: int` — encode/decode round-trip.
- `noise_sigma` (existing) + `shot_noise: bool` (signal-dependent variant).
- `occlusion: (target ∈ {finder, data}, fraction, gray)` — rectangle over a
  chosen region, positioned from ground-truth corners.

New scenario families (~56 fixtures, names follow `<family>_<NN>`):

| family | sweep | expect |
|---|---|---|
| `mblur_00..07` | length {4,8,12,16}px × angle {0,30,60,90}° | detect ok; decode fails at ≥ ~1.5 module smear |
| `defocus_00..03` | radius {2,4,6,8}px | decode fails when radius > module/2 |
| `shadow_00..07` | strength {0.45,0.75} × softness {2,32}px × {half-plane, band} crossing the symbol | detect ok; decode hard at strong+sharp |
| `illum_00..05` | gradient 4:1, glare hotspot, contrast-compressed (floor probe) | low-contrast cases fail baseline |
| `lowres_00..07` | render at 5px/module then degrade to {2.5,2.0,1.7,1.4} px/module (×2 resampling methods) | detection dies < ~2px/module baseline |
| `jpeg_00..03` | quality {40,25,15,10} | decode degrades with blocking |
| `occl_00..05` | finder-corner clip {10,25%}, data-region patch {5,15,30%} | ECC-level dependent |
| `combo2_00..07` | lowres+blur, shadow+tilt45, jpeg+lowcontrast, blur+shadow+lowres, … | the real-video failure profile |

Per-fixture JSON gains: `degradations` (exact params), and per-code
`expect_detect: bool`, `expect_decode: bool`, `difficulty: 0..3`.
Expectations are set from *physics* (module-units thresholds above), then
verified: a fixture whose expectation proves wrong under the baseline scanner
is evidence, not a knob — flag, don't silently tune.

Gate impact: `decode_gate`/`refine_gate`/`fixtures_smoke` currently assert
`fixtures.len() == 81` and 100% decode. New degraded families are **not** part
of the golden 100% gate: gates filter to `expect_decode == true` (absent field
= true, so the existing 81 keep exact semantics), assert the expected subset
exactly, and the robustness families get their own reporting + a ratchet gate
(decode rate per family must not regress vs the committed baseline JSON).

## 6. Metrics (benchmark harness)

New `qrk-bench` binary (workspace member, depends on qrk-core; JSON output):
runs {fixture × config-preset × working-dim} and emits machine-readable
records: per-fixture (name, family, difficulty, degradation params, apparent
px/module) × per-config: detected/decoded per code, corner error vs truth,
false positives, per-stage timings, variants evaluated, early-exit flag, and
per-variant marginal yield (which stage/variant produced each code). Aggregates:
detection rate, decode rate, FP rate, mean corner error, p50/p95 frame time,
success by degradation type / px-size bucket / config / stage, early-exit rate,
most-useful-variant per family. Headline questions: decode under motion blur,
decode under partial shadow, detection at low resolution, CPU cost vs the 5ms
budget.

## 7. Implementation plan

Branch `plan6-robustness`. Files:

1. **qrk-core**: `enhance.rs` (box_downscale_2x, bilinear_upscale_2x,
   box_blur_running / background_divide, unsharp_5tap, structure_tensor_theta,
   directional_unsharp — all pure fixed-point, unit-tested against small
   analytic images); `tiles.rs` (+ `build_with(view, offset, contrast_floor)`,
   delegating default); `ladder.rs` (`ScanConfig`, `VariantRecord`,
   `RobustDetections`, `scan_robust`, dedup, provenance); `scanner.rs`
   (thread threshold params through `detect_with_source` — default path
   bit-identical); `sample.rs` (relax the `<=1` doc on SourceView scales for
   the 2× rung); consts.rs (new constants with derivations).
2. **tools/fixtures**: `degrade.py` (op library + unit tests), `scenarios.py`
   (new families + Degradations on FixtureSpec), `generate.py` (apply ops,
   record params + expectations), `test_*` updates; regenerate fixtures.
3. **tests**: gates updated for expectations + count; new `robustness_gate.rs`
   (ratchet vs committed baseline results JSON); keep the bit-identical
   baseline pin green.
4. **qrk-bench**: batch runner + JSON emitter (+ optional real-frame mode:
   PNG dir).
5. **Experiments** (parallel worktrees on top of the infra branch): E1 multi-
   scale, E2 contrast/threshold sweep, E3 shadow normalization + Sauvola,
   E4 sharpening + directional deblur, E5 2× upscale. Each reports
   detection/decode deltas per family at working dims {0 (=720p native), 960,
   640} and per-config combos, plus per-rung timings vs the 5ms budget.
   Winners merge; losers are recorded with numbers.

First experiment: E1+E2 (cheapest, broadest). Second: E3 (shadow) and E5
(low-res) — the two named failure modes. E4/Wiener runs last (deep rung).

Phase-2 backlog (separate plans): sub-pixel run measurement in finder.rs,
ROI Wiener/RL, ScanSession (variant rotation, ROI tracking, temporal
accumulation), WASM config surface, NEON.


## 8. Results (2026-07-05, host M-series release build; all five experiment
branches merged — see exp/e1..e5 branches and
docs/superpowers/plans/2026-07-05-exp-e2-threshold-params.md)

Fixture suite = 154 codes across 137+5 fixtures (81 golden + 61 degraded
incl. the E4 mblur2 deep-smear family, which is 0/5 by design — past the
box-MTF sign flip, documented Wiener-tier headroom for phase 2).

| working dim | config | detected | decoded | degraded dec | mean ms | p95 ms | early exit |
|---|---|---|---|---|---|---|---|
| native (1280×720) | baseline | 138 | 130 | 37/61 | 4.8 | 6.4 | — |
| native | robust-fast | **146** | **140** | **47/61** | 10.5 | 46 | 89% |
| native | robust-full | 146 | 140 | 47/61 | 58 | 105 | — |
| 960 | baseline | 125 | 115 | 29/61 | 3.1 | 4.4 | — |
| 960 | robust-fast | 136 | 131 | 42/61 | 8.0 | 28 | 85% |
| 640 | baseline | 120 | 111 | 29/61 | 1.5 | 2.7 | — |
| 640 | robust-fast | 136 | 132 | 44/61 | **3.9** | 13 | 87% |

Real failure recordings (240 frames @2fps from fixtures/real/domain-data-mp4,
max-dim 1280): baseline decodes on 14 frames; robust-fast 22 (+57%);
robust-full 24. Rung credits on real frames: Upscaled2x/UpscaledRoi,
Pyramid 0.5×, ThresholdOffset+8.

Headline per-failure-mode outcomes:
- Low resolution: detection 4/8 → 8/8 (down to 1.4 px/module), decode 4/8 →
  7/8; the remaining 1.4 px/module decode is at the Nyquist information
  limit (it detects, which still yields an AR track point). ROI-scoped
  upscale runs at ~1ms vs ~12ms full-frame; 3× tail decodes sub-Nyquist
  codes at reduced working dims.
- Partial shadow: 8/8 at every working dim (E3's evidence-sized van Herk
  closing background estimate — SE = 10 × measured finder module — recovers
  the sharp shadow BAND crossing the symbol that fixed-window estimators
  bridge over).
- Motion blur: baseline already tolerates ~1.8–2.1-module smears; the Van
  Cittert tier extends the decodable band to ~2.1–2.4 modules (pinned by
  gate3c); 2.5+-module smears need sign-inverting (Wiener-class)
  deconvolution — phase 2, with fixtures and estimators already in place.
- Frame budget: early-exit frames pay baseline cost (~4.8ms @720p host).
  robust-fast at 640 working dim sits INSIDE the 5ms budget mean (3.9ms)
  while beating native-dim baseline decode (132 vs 130). Worst-case
  (codeless/hard frames) is 28–46ms p95 — the per-frame variant-rotation
  work (phase 2) converts that additive cost to a constant.

Negative results worth keeping (E2, full doc on the branch): threshold
offsets beyond ±8, contrast floor 4, Sauvola k ≥ 0.3, and running the
threshold sweep on pyramid levels are all zero-sum or sub-noise-margin —
the shipped principled values stand. E1's falsification test confirmed
half-octave pyramid levels reach nothing octaves miss; the 0.25× level is
now gated on octave-coverage geometry (DEEP_LEVEL_MIN_PARENT_DIM = 1392px).

Known follow-ups (measured, not speculative): robust-fast's early exit has
a multi-code blind spot at reduced dims (codes invisible at working res
leave no triplet evidence to keep the ladder alive — 15/20 multi at 640 vs
robust-full's 20/20); NN working-downscale (max_working_dim path) aliases
sub-3px/module codes at 960 (box-filter the ladder's working view — the
parity pin binds only the default path); combo2 lowres+blur needs a
COMPOSED rung (upscale-then-sharpen) or sub-pixel run measurement.

## 9. Cumulative evidence pipeline (2026-07-05, branch plan6-robustness)

The independent-rung ladder of §2 was rewritten into a CUMULATIVE EVIDENCE
PIPELINE after four diagnosis passes on 368 real video frames
(dmt_recording 11-34-54, 1920×1440; "diagnosis D1..D4" in code comments):
finders that only co-appear across DIFFERENT variants could never group
(grouping ran per-variant — 19 frames hold a coherent pooled trio no single
variant forms, 61 more hold coherent pairs); the deblur tier fired on
253/368 frames with zero evidence for zero decodes at 47% of frame cost;
ShadowNormalized was the largest zero-yield line item; and the NN working
downscale destroyed marginal finder runs the box kernel preserves.

Design as shipped:

- **Shared candidate pool.** Every detect pass ADDS its finder candidates
  (source px, 3.5-module/polarity dedup, earliest wins) to one pool — the
  source of truth behind `RobustDetections::finders`. Enhancement passes
  detect on a BOX (area-average) working view (`enhance::area_downscale`,
  bit-exact separable integer sweep) instead of the pinned NN buffer; a
  new `VariantKind::BoxWorking` sanity pass runs first among enhancements
  when a downscale happened. Module sampling keeps reading the pristine
  source (a prefiltered sampling path measured net −4 decodes, D4).
- **Pooled group+decode after every stage batch** (µs-scale, no budget
  slot): the pool maps onto the box view, `group_triplets` runs over the
  union, fresh triples (order-normalized index registry, covered-skip) go
  straight through `decode_candidates`; codes carry
  `VariantKind::CrossVariant` (stage 7); undecoded fresh triples join the
  evidence set and drive ROI recovery like variant-native triplets.
- **Evidence-scoped, resolution-independent recovery.** Evidence units are
  uncovered triplets AND coherent pooled pairs (same polarity, module
  ratio ≤1.5, separation ∈ [14, 170·√2] modules; ≤4/frame, smallest
  separation first). Each unit gets one SOURCE-px ROI; the factor comes
  from the CANDIDATE's pitch, never the frame size: ≥3.5 px/module → 1:1
  rescan (escalating once to 2× if unexplained — measured on illum_02),
  1.75–3.5 → 2×, <1.75 → 3×. ROIs are tile-phase-aligned (+1 tile ring) so
  interior thresholds are byte-identical to a full-frame pass — the
  unaligned crop's phase-shifted tile grid alone flipped f0309's
  5.35 px/module decode. `MAX_UPSCALE_INPUT_DIM` no longer gates ROIs; the
  whole-frame 2×/3× DETECTION-STARVED fallback (no triplet evidence)
  survives unchanged, plus the pre-E5 post-ROI whole-frame 2× when
  evidence stays uncovered (zero-regression construction; f0310).
- **No-evidence economics.** ShadowNormalized and Sharpened run only when
  the pool is non-empty; the deblur tier is evidence-ROI-scoped (tensor,
  edge-rise, directional unsharp and Van Cittert all measured/applied ON
  the ROI — the ROI-local θ is the physically correct smear direction,
  D3), with a last-resort single-finder unit class admitted only at
  ≥3.5 px/module with a measured smear in the recoverable band
  [max(7 px, 1·m), 2.4·m] (this class is what keeps gate3c green).
  The whole-working-frame DirectionalSharpened pass and its global-tensor
  gate are gone.
- `ScanConfig::ROBUST_FAST` cap raised 8 → 16: the full-frame stages fill
  ≤8 slots and the recovery tail (≤8 ROI passes at ~1 ms each — 0.83 ms
  mean measured in D1) would otherwise be cut off exactly on evidence
  frames.

Measured on the 368-frame recording (host M-series release; bench luma
now matches `luma_from_rgba` rounding, so "before" differs from D1's
truncating-luma numbers):

| dim | config | decoded frames | ≥1 unified-triplet frames | mean ms | p95 ms |
|---|---|---|---|---|---|
| 1280 | baseline | 15 → 15 | 30 → 30 | 3.5 → 3.6 | 4.3 → 4.1 |
| 1280 | robust-fast | 33 → **39** | 54 → **85** | 43.2 → 40.2 | 48.3 → 61.6 |
| 1280 | robust-full | 34 → **39** | 54 → **85** | 68.4 → **47.3** | 108.4 → **69.2** |
| full | baseline | 23 → 23 | 42 → 42 | 5.9 → 5.9 | 6.7 → 6.6 |
| full | robust-fast | 37 → **38** | 66 → **86** | 70.8 → **55.1** | 80.5 → 89.0 |
| full | robust-full | 37 → **38** | 68 → **86** | 142.1 → **66.5** | 220.4 → 96.9 |

ZERO frames decode before-but-not-after in any config at either dim.
Gained @1280: f0068, f0210, f0220, f0309, f0310, f0357 (f0210/f0220 are
D1/D2's structurally-undetectable pooling cases; f0068 came from the
tile-aligned rescan). First-decode credit shifted off the whole-frame 2×
(8 → 1, the 1 via the post-ROI fallback on f0310) onto BoxWorking (6) and
CrossVariant pooled decodes (6). The remaining pooling candidates
(f0013/f0062/f0070/f0123/f0258/f0259/f0355) now all hold ≥3 unified
finders and a unified triplet — detection evidence, pinned for f0070 by
`robust_gate::gate6` on the committed `fixtures/real/video_f0070.png` —
but stay decode-fragile at 4.3–5.1 px/module handheld (D2's half-LSB
margin). Degraded-fixture ratchet unchanged at 47/53 (mblur2_00 decoded
briefly through an unaligned crop and flipped back with tile alignment —
a single rounding-margin code traded for the two real-video frames).

### 9.1 Deblur-consumer gate on the empty-pool blind upscale (2026-07-05)

The initial rewrite kept the detection-starved whole-frame 2× firing on
every zero-triplet frame (~12 ms), which dominated robust-fast's cost on
the codeless-heavy video (154/368 frames reach it with an EMPTY pool).
Measurement showed that pass has **zero direct recall on a zero-finder
frame** — on real video it decoded nothing across all 154, and the lowres
fixtures that decode via whole-frame 2× (lowres_04/05) all reach the
fallback with a NON-empty pool. Its only demonstrated value on a truly
empty pool is surfacing a coarse candidate for the DEBLUR tier to refine
(gate3c: a 2-module-smeared code with no finder at any binarization
decodes only as whole-frame-2× → Van Cittert).

So the blind pass on a zero-finder frame now runs iff `!pool.is_empty()
|| enable_deblur` — production `ROBUST_FAST` (deblur off) skips it; the
benchmark config (deblur on) retains the gate3c recovery path. Measured
effect (11-34-54, max-dim 1280): robust-fast mean **40.2 → 33.2 ms** with
decoded frames (39) and triplet-evidence frames (85) UNCHANGED — a pure
~17% latency cut at zero recall cost. All gates green (gate3c retained).
The remaining robust-fast cost (threshold family, pool-gated
shadow+sharpen on the 134 evidence-bearing frames, box substrate) is the
irreducible full-ladder-per-frame floor that per-frame variant rotation
(the ScanSession API, §9.2) amortizes.

### 9.2 ScanSession: temporal amortization for video (2026-07-05)

`ScanSession` (public, `qrk-core`) is the stateful multi-frame entry point
for a continuous camera stream, sitting above single-frame `scan_robust`
(unchanged; still the right call for stills). Two deterministic mechanisms
(the frame counter drives everything — a session replayed on identical
frames produces identical output):

1. **Rung rotation** (`SessionConfig::rotation_period`, default 3): the
   three always-on-cost detect groups — box+pyramid substrate, threshold
   sweep, Sauvola — are round-robined across frames (group `g` runs on
   frame `f` iff `f % period == g % period`) instead of all running every
   frame. Baseline, the box-view build, and pooled group+decode still run
   every frame.
2. **Cross-frame seed pooling** (`pool_ttl_frames`, default 4): each frame's
   finder candidates are kept for a few frames and injected into later
   frames' candidate pool, so a code whose finders land under passes on
   DIFFERENT frames still groups — the temporal analog of the in-frame
   cross-variant pooling win. Seeds are grouped on the CURRENT frame's
   pixels (`group_triplets`' leg-module walk re-validates them), so a stale
   seed over a region the code has moved off simply fails to group: pooling
   is self-correcting and cannot fabricate a code (verified: zero spurious
   payloads over 368 real frames).

Measured (11-34-54, 368 frames, robust-fast, max-dim 1280; `qrk-bench
--session`): mean **31.9 → 22.5 ms/frame (−30 %)**, triplet-evidence frames
**85 → 96 (+13 %** — more AR track-points), and every distinct payload still
captured (5/5; the 3 frames that dropped a single-frame decode each recover
it within ±3 frames — 0.3 s at 10 fps — so a real scanning dwell loses
nothing). `rotation_period: 1` disables rotation (full ladder every frame,
cross-frame pooling still active). Pinned by `robust_gate::gate7` (f0070
replayed through a rotating session forms the triplet within one rotation
cycle) and the ladder unit tests (rotation schedule, period-1 parity with
`scan_robust`).

Remaining lever (documented, not built): ROI tracking — once a code is
decoded, seed the next frame's search with a full-resolution ROI around its
last quad and demote full-frame scanning to a periodic rescan. It reuses
the same session state and would make the steady-state (phone held at a
code) ~10× cheaper, but early exit already makes those frames ~4 ms, so the
codeless/searching-frame cost this session work targets was the higher
priority.
