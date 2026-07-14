//! `scan_robust()`: the Plan 6 CUMULATIVE EVIDENCE PIPELINE. Sits ABOVE
//! [`crate::scan()`] the way `scan` sits above `detect`: stage 0 is exactly
//! today's `scan` behavior (bit-identical when every [`ScanConfig`] flag is
//! off), and each enabled recovery stage re-runs detection on a cheap
//! variant of the frame ONLY while evidence suggests an undecoded code
//! remains. This is a staged recovery pipeline with early exit, not an
//! exhaustive preprocessing sweep: expected cost on decodable frames is
//! stage-0 cost; worst-case cost is bounded by
//! [`ScanConfig::max_variants_per_frame`] (a WORK-based cap — a wall-clock
//! budget would make output depend on host speed, violating the determinism
//! rule).
//!
//! Pipeline invariants (see the Plan 6 doc §9 for derivations):
//! - **Every pass ADDS to a shared candidate pool** (source-px finder
//!   candidates, deduplicated, earliest-rung wins), and a free (µs-scale)
//!   pooled group+decode step runs after every stage batch: the pool is
//!   mapped onto the anti-aliased box working view, `group_triplets` runs
//!   once over the UNION, and fresh triples are decoded. This closes the
//!   cross-variant pooling gap diagnosis D1/D2 measured: finder candidates
//!   that only co-appear across DIFFERENT variants could never group when
//!   grouping ran per-variant only (19 real video frames hold a coherent
//!   pooled trio no single variant forms; 61 more hold coherent pairs).
//! - **Detection substrate vs sampling substrate**: enhancement rungs
//!   detect on the box-filtered (area-average) working view — diagnosis D4
//!   measured the pinned NN working downscale destroying marginal finder
//!   runs (+12% finder candidates, +4 decoded frames at matched substrate
//!   with the box kernel) — while module sampling and corner refinement
//!   keep reading the pristine source via [`SourceView`] (a prefiltered
//!   sampling path measured net −4 decodes, D4 arm F). Only the
//!   shadow-normalization and Van Cittert deconvolution rungs sample their
//!   own buffers, because their whole point is that the source's modules
//!   are unreadable as-is (shadowed against working-grid thresholds;
//!   smeared across data cells).
//! - Every variant's detections are mapped back to SOURCE px and deduped
//!   against already-accepted codes; the cheapest variant wins, later
//!   duplicates only enrich [`VariantRecord`] stats.
//! - Escalation is driven by *evidence*: a grouped triplet whose geometry
//!   is coherent (`snap_error < 2.0` — an estimate further than 2 from a
//!   valid mod-4 dimension cannot even round unambiguously) but which no
//!   accepted code covers, or a geometrically coherent pooled finder PAIR,
//!   is positive evidence a code is present and undecoded. Recovery is
//!   ROI-scoped and resolution-independent: the upscale factor comes from
//!   the CANDIDATE's pitch (never the frame size), with factor 1 meaning a
//!   pure pristine-source rescan of the ROI (diagnosis D2: at ≥3.5
//!   px/module the 1:1 crop beats the 2×/3× kernels, which smooth marginal
//!   runs away). The deblur tier is likewise ROI-scoped: structure tensor
//!   and edge-rise are measured ON the evidence ROI (diagnosis D3: the
//!   global tensor measures shelf/aisle gradients, >20° wrong for the code
//!   region on 7/12 evidence frames, and whole-frame firing cost 47% of
//!   frame time for zero yield).
//! - **No-evidence economics**: after the cheap stages, a frame whose pool
//!   is empty runs only the detection-starved whole-frame upscale fallback
//!   and stops — the expensive pixel rungs (shadow normalization,
//!   sharpening, deblur) require candidate-pool evidence (diagnosis D1:
//!   91% of real walkthrough frames decode nothing, and those rungs cost
//!   12.9+5.8 ms/frame for zero yield there).

use crate::consts::TILE;
use crate::decode::decode_candidates;
use crate::downscale::downscale_luma;
use crate::enhance::{
    area_downscale, background_divide, bilinear_upscale_2x, bilinear_upscale_3x,
    bilinear_upscale_4x, box_downscale_half, catmull_rom_upscale_2x, directional_unsharp,
    edge_rise_extent, structure_tensor_blur_direction, unsharp_mask, van_cittert_directional,
};
use crate::finder::FinderCandidate;
use crate::sample::SourceView;
use crate::scanner::{detect_with_source, Detections, StageClock, StageTimings};
use crate::tiles::{BinarizeSpec, TileGrid};
use crate::triplet::{group_triplets, TripletCandidate};
use crate::{DecodedCode, LumaView, ScanOptions};

/// Opt-in robustness enhancements for [`scan_robust`]. Every flag defaults
/// to `false`; the all-off default makes `scan_robust` behave exactly like
/// [`crate::scan()`] plus provenance metadata (pinned by this module's tests).
/// Each flag's cost is ZERO when off — no buffers, no passes, no branches
/// beyond the flag check.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScanConfig {
    /// Stage 1: re-run detection on 2×2 box-averaged pyramid levels (0.5×,
    /// then 0.25× while the level stays ≥ [`MIN_PYRAMID_DIM`] AND the 0.5×
    /// level exceeds [`DEEP_LEVEL_MIN_PARENT_DIM`] — below that no code the
    /// 0.5× level cannot binarize fits the frame, so 0.25× would be
    /// redundant coverage burning a budget slot). Targets noisy frames (box
    /// averaging halves noise σ per octave) and large-module codes
    /// (threshold-window mismatch). Cost ≤ ~⅓ of a baseline detection pass
    /// total (geometric series).
    pub enable_multi_scale: bool,
    /// Stage 2: re-binarize + re-scan the working view with threshold
    /// offsets ±8 (print dot-gain / JPEG-ringing amplitude) and a lowered
    /// contrast floor (6 = 3σ vs the default 6σ) — the cheap realization of
    /// the whole contrast/gamma/histogram axis (no pixel rewrites; a local
    /// threshold is already invariant to smooth monotonic illumination).
    pub enable_contrast_normalization: bool,
    /// Stage 3a: background DIVISION (illumination is multiplicative):
    /// morphological-closing illumination envelope (van Herk, SE sized from
    /// baseline finder evidence — 10 modules, the widest solid ink
    /// structure), `I·200/max(B,8)`. The rung for a shadow EDGE or band
    /// crossing the symbol — the case tile thresholds handle worst. This
    /// variant samples its own normalized buffer (see module doc). Runs
    /// only when the candidate pool is non-empty (diagnosis D1: 12.8 ms
    /// mean for zero yield on evidence-less real frames — the single most
    /// expensive zero-yield line item).
    pub enable_shadow_normalization: bool,
    /// Stage 3b: Sauvola tile threshold surface (k=0.2, R=128) — pulls
    /// thresholds toward background in low-variance regions so noisy quiet
    /// zones stay clean where the min/max midpoint speckles.
    pub enable_adaptive_thresholding: bool,
    /// Stage 4: 5-tap unsharp mask (k=1, sub-module support) on the
    /// detection buffer only — mild-blur recovery. Never combined with
    /// pyramid levels (opposite failure classes) and never sampled from.
    /// Runs only when the candidate pool is non-empty (diagnosis D1:
    /// 5.8 ms/frame for one first-decode over 368 real frames, all yield
    /// on frames that already held candidates).
    pub enable_sharpening: bool,
    /// Stage 6 (deepest): motion-deblur tier, ROI-SCOPED on unexplained
    /// evidence (diagnosis D3 rewrite): per still-uncovered evidence ROI,
    /// the blur direction is estimated from the gradient structure tensor
    /// OF THAT ROI (the global tensor measures scene texture — >20° wrong
    /// for the code region on 7/12 evidence frames), then a directional
    /// 1-D unsharp pass and a 1-D Van Cittert deconvolution candidate
    /// sweep run on the ROI only (smear length from the ROI's 20–80%
    /// edge-rise measurement, bracketed ×{1, ½, 1½}, decode checksum as
    /// oracle). Fires only when the ROI's anisotropy confidence exceeds
    /// [`MIN_DIRECTIONAL_CONFIDENCE`] (isotropic blur gives no direction
    /// to sharpen along), and the deconvolution sweep additionally
    /// requires a measured extent past the unsharp rungs' support
    /// ([`MIN_DEBLUR_LEN`]). Evidence-less frames skip the tier entirely
    /// (D3: 227 of 253 whole-frame firings had zero evidence, 7.1 s of
    /// work for zero decodes; ROI-scoping is 20.8× cheaper per firing).
    pub enable_deblur: bool,
    /// Stage 5: resolution recovery — converts the evidence at hand into
    /// the cheapest pass that can resolve it, with the factor chosen from
    /// the CANDIDATE's pitch, never the frame size (see
    /// [`upscale_factor_for_pitch`]'s derivation). Shapes:
    /// - **ROI-scoped** (uncovered triplet evidence OR a coherent pooled
    ///   finder PAIR): a padded SOURCE-px window around each evidence unit
    ///   (≤ [`MAX_UPSCALE_ROIS`] windows each, ~1 ms at typical code
    ///   sizes), rescanned 1:1 when the pitch already clears the decode
    ///   floor (diagnosis D2: the pristine crop beats the upscale kernels
    ///   there) or bilinear-upscaled 2×/3× below it.
    /// - **Whole-frame 2×** (no evidence at all — detection-starved): the
    ///   pre-E5 fallback, unchanged; skipped when the working view exceeds
    ///   [`MAX_UPSCALE_INPUT_DIM`] (a big frame is not resolution-starved
    ///   at frame scale and 4× its pixels would dominate the budget).
    ///   Diagnosis D1: 8 first-decodes on frames with an empty pool.
    /// - **Whole-frame 3× tail** (still no codes AND no evidence after the
    ///   2×, working ≤ [`MAX_UPSCALE3X_INPUT_DIM`]): the sub-Nyquist last
    ///   resort — at 9× the input's pixels the most expensive pass by far.
    pub enable_low_res_upscaling: bool,
    /// Cap on non-baseline variant scans per frame; `0` = unlimited. This is
    /// the ladder's whole-frame work budget (deterministic, host-independent).
    pub max_variants_per_frame: u32,
    /// Stop climbing as soon as ≥1 code decoded AND no coherent triplet
    /// remains uncovered AND no multi-code signal remains (a free
    /// coherent finder pair OR a free singleton finder outside accepted
    /// codes — a second symbol's partial evidence keeps the ladder alive
    /// even when it has not yet formed a triplet). Off = run every
    /// enabled rung (benchmark mode: measures every rung's marginal
    /// yield).
    pub enable_early_exit: bool,
}

impl ScanConfig {
    /// All enhancements off — `scan_robust` ≡ [`crate::scan()`] + provenance.
    pub const BASELINE: ScanConfig = ScanConfig {
        enable_multi_scale: false,
        enable_contrast_normalization: false,
        enable_shadow_normalization: false,
        enable_adaptive_thresholding: false,
        enable_sharpening: false,
        enable_deblur: false,
        enable_low_res_upscaling: false,
        max_variants_per_frame: 0,
        enable_early_exit: true,
    };

    /// Production-lean ladder tuned for ~30 fps mobile: every high
    /// yield-per-ms rung, no deblur, **no full-frame shadow/sharpen**
    /// (domain gold 419 obs-frames: ShadowNormalized 85× for 2 first-
    /// decodes @12.9 ms mean; Sharpened 83× for 7 @6.6 ms — both dominated
    /// p95 while ROI recovery + threshold family carried the 96% recall).
    /// Keep them in [`Self::ROBUST_FULL_BENCHMARK`] for hard stills / offline.
    /// Cap 12: cheap detect family (box + pyramid + 3 threshold + Sauvola
    /// ≤ 7) + ROI tail (≤ 4 pair/triplet) without the two expensive pixel
    /// rewrites. Early exit on multi-code signal still holds the ladder
    /// open for a second symbol.
    pub const ROBUST_FAST: ScanConfig = ScanConfig {
        enable_multi_scale: true,
        enable_contrast_normalization: true,
        enable_shadow_normalization: false,
        enable_adaptive_thresholding: true,
        enable_sharpening: false,
        enable_deblur: false,
        enable_low_res_upscaling: true,
        max_variants_per_frame: 12,
        enable_early_exit: true,
    };

    /// Benchmark ladder: everything on, no early exit, effectively no cap —
    /// measures each rung's marginal yield on every frame.
    pub const ROBUST_FULL_BENCHMARK: ScanConfig = ScanConfig {
        enable_multi_scale: true,
        enable_contrast_normalization: true,
        enable_shadow_normalization: true,
        enable_adaptive_thresholding: true,
        enable_sharpening: true,
        enable_deblur: true,
        enable_low_res_upscaling: true,
        max_variants_per_frame: 64,
        enable_early_exit: false,
    };
}

/// Smallest pyramid level worth scanning: a v1 symbol (21 modules) at the
/// 2 px/module detection floor plus its 4-module quiet zones is
/// `(21 + 8) · 2 = 58` px; a level under ~200 px can only detect codes that
/// filled over a quarter of the previous level, which that level already
/// covers (each level's detector spans >2 octaves of module size).
const MIN_PYRAMID_DIM: usize = 200;

/// Descending past the FIRST pyramid level additionally requires the parent
/// level to be able to contain a code the parent itself cannot binarize.
/// Tile thresholds are min/max extrema over a 3×3-dilated neighborhood of
/// [`TILE`](=16)px tiles, so reliable binarization ends where one module
/// swallows the whole `3·TILE = 48px` reach: interior tiles then see
/// single-polarity pixels, fall under the contrast floor, and are skipped.
/// That over-sized-module class is the ONLY coverage a second octave of
/// downscaling adds — its other mechanism (a further σ/2 of noise averaging)
/// is already delivered by the level above at twice the resolution. The
/// smallest symbol footprint is a v1: 21 modules + two 4-module quiet zones
/// = 29 module widths, so a beyond-ceiling code spans at least
/// `29 · 3 · TILE = 1392px`; a parent level at or under that size cannot
/// host one and the deeper level is provably redundant coverage (E1
/// measurement agrees: 0.25× yielded zero codes in 274 full-ladder runs at
/// 720p/960 working sizes while costing a ladder budget slot). A ≥2.8K
/// working frame (max_working_dim = 0 on a stills-class source) still
/// descends to 0.25×, which is exactly the frame class whose fitting codes
/// can exceed the 0.5× level's module ceiling.
const DEEP_LEVEL_MIN_PARENT_DIM: usize = 29 * 3 * TILE;

/// WHOLE-FRAME 2× upscaling beyond this working size is disallowed: 4× the
/// pixels of a frame above ~1.5K would cost more than every other rung
/// combined, and a code small enough to be resolution-starved in such a
/// frame is served by the ROI-scoped recovery passes (which are
/// resolution-independent and NOT gated by this constant — the pooling
/// rewrite removed it as an ROI gate, per diagnosis D1: at full 1920 res
/// the old gate deleted the whole 14-first-decode upscale family exactly
/// when the source had the most information). 1536 admits the 1280×720 /
/// 1440×1080 video classes this scanner targets while excluding
/// stills-class (4K+) sources.
const MAX_UPSCALE_INPUT_DIM: usize = 1536;

/// Minimum structure-tensor anisotropy (1 − λmin/λmax) for the directional
/// rung to fire: below ~0.3 the gradient field is near-isotropic (defocus or
/// no blur), a direction estimate is a coin flip, and sharpening the wrong
/// axis amplifies noise while leaving the smear.
const MIN_DIRECTIONAL_CONFIDENCE: f64 = 0.3;

/// A triplet whose dimension estimate is ≥2 modules from every valid mod-4
/// dimension cannot round unambiguously — treat it as noise, not as
/// escalation evidence.
const MAX_EVIDENCE_SNAP_ERROR: f64 = 2.0;

/// ROI padding around an uncovered evidence point's finder-center bounding
/// box, in modules of that triplet's measured pitch. QR geometry: the
/// symbol's farthest corner lies 3.5·√2 ≈ 4.95 modules beyond a finder
/// CENTER (worst case: measuring diagonally from a corner finder), plus the
/// 4-module quiet zone (ISO/IEC 18004 §9.1) the run-length detector needs
/// on every side ⇒ 8.95, rounded up to 9. Not fixture-tuned.
const UPSCALE_ROI_PAD_MODULES: f64 = 9.0;

/// Cap on ROI-scoped upscale passes per frame (Plan 6 architecture doc's
/// "cap ~4 ROIs/frame"): evidence points are already deduplicated to one
/// per physical code (7-module merge radius), so 4 ROIs cover every
/// realistic multi-code frame while bounding the rung's worst case.
const MAX_UPSCALE_ROIS: usize = 4;

/// Below this triplet pitch (SOURCE px/module) the ROI upscale uses factor
/// 3 instead of 2: the decode floor is ~3.5 px/module (Nyquist-study bound,
/// see `enhance::bilinear_upscale_2x`), so 2× can only lift codes with
/// pitch ≥ 3.5/2 = 1.75 px/module above it; anything smaller needs 3×
/// (which reaches down to 3.5/3 ≈ 1.17 px/module — near the hard
/// information limit where the runs alias away entirely).
const UPSCALE_3X_MAX_MODULE_PX: f64 = 1.75;

/// Below this pitch, use a 4× evidence ROI instead of 3×. Both factors lift
/// the nominal pitch over the 3.5 px/module decode floor, but 4× supplies an
/// extra integer sampling phase for the deeply aliased 1.4 px/module tail.
/// The pass remains ROI-only; whole-frame escalation is still capped at 3×.
const UPSCALE_4X_MAX_MODULE_PX: f64 = 1.5;

/// At or above this candidate pitch (SOURCE px/module) the ROI recovery
/// pass RESCANS the pristine source crop 1:1 (factor 1) instead of
/// upscaling: 3.5 px/module is the Nyquist decode floor (same bound as
/// [`UPSCALE_3X_MAX_MODULE_PX`]'s derivation), so the source already
/// carries decode-grade resolution and resampling can only hurt —
/// diagnosis D2 measured the 1:1 crop of a 4.7 px/module code finding all
/// 3 finders + a snap-0.00 triplet where the 2×/3× kernels' interpolation
/// smoothed one finder's marginal vertical run away (2 of 3).
const RESCAN_MIN_MODULE_PX: f64 = 3.5;

/// The ROI recovery factor for a candidate of pitch `m` (SOURCE
/// px/module): resolution-independent by construction — derived from the
/// candidate's own pitch versus the ~3.5 px/module Nyquist decode floor,
/// never from the frame size. `1` = pure pristine-source rescan
/// ([`RESCAN_MIN_MODULE_PX`]); `2` lifts `[1.75, 3.5)` px/module over the
/// floor; `3` reaches the sub-Nyquist band below
/// [`UPSCALE_3X_MAX_MODULE_PX`].
fn upscale_factor_for_pitch(m: f64) -> u8 {
    if m >= RESCAN_MIN_MODULE_PX {
        1
    } else if m >= UPSCALE_3X_MAX_MODULE_PX {
        2
    } else if m >= UPSCALE_4X_MAX_MODULE_PX {
        3
    } else {
        4
    }
}

/// Minimum pooled-pair separation in modules of the pair's mean pitch.
/// Axis-aligned, fronto-parallel v1 finders sit `dimension − 7 = 14`
/// modules apart, but under the operating envelope's ≤45° perspective
/// tilt the foreshortened leg compresses by up to `cos 45° ≈ 0.707`, and
/// the scan-measured module is itself rotation-biased (see `triplet.rs`),
/// so a true adjacent pair routinely measures `d/m ≈ 10–13`. Domain
/// multi-code frames that previously missed the second code (e.g. free
/// pairs at 12.0 and 12.9 modules) sit just under a 14-module floor.
/// Floor = `14 · 0.5 = 7` — the same `MAX_LEG_IMBALANCE` the triplet
/// grouper already admits for one-code geometry — so any pair the grouper
/// would accept as two legs of one symbol stays a valid multi-code pair.
/// Duplicates (same finder, two detections) sit at ≪7 modules and stay out.
const PAIR_MIN_SEP_MODULES: f64 = 7.0;

/// Maximum pooled-pair separation in modules: adjacent corners span
/// `dimension − 7 ∈ [14, 170]` modules (dimension ≤ 177), and the
/// tr/bl DIAGONAL pair is √2 larger ⇒ `170·√2`. Beyond that no single QR
/// symbol can own both candidates.
const PAIR_MAX_SEP_MODULES: f64 = 170.0 * std::f64::consts::SQRT_2;

/// Pair-ROI side length = `PAIR_ROI_SPAN · d + 2 ·`
/// [`UPSCALE_ROI_PAD_MODULES`]` · m` for pair separation `d` and mean
/// pitch `m`, centered on the pair midpoint. Two placements are possible
/// for an unlabeled pair: DIAGONAL (tr/bl — the symbol's ink corners all
/// lie within `d·√2/2 ≈ 0.71·d` of the midpoint) and ADJACENT (two
/// corners of one edge, `d = (dim−7)·m` — the symbol body extends up to
/// `d + 3.5·m` perpendicular from the midpoint's line on one unknown
/// side, but its INK relevant to the missing third finder lies within
/// `~0.8·d` of the midpoint on every axis once the center inset is
/// counted). A `±0.8·d` window (side `1.6·d`) covers both placements'
/// finder geometry, and the `2·9·m` term adds the same
/// farthest-corner + 4-module quiet-zone pad per side that the triplet
/// ROI uses ([`UPSCALE_ROI_PAD_MODULES`]).
const PAIR_ROI_SPAN: f64 = 1.6;

/// Half-side, in modules, of the deblur tier's single-finder evidence ROI:
/// one verified finder is the weakest evidence unit (used only when no
/// triplet or pair evidence exists anywhere in the frame), and the
/// symbol's extent from it is unknown. 29 modules covers a v1 symbol
/// entirely — its farthest corner lies `(21 − 3.5)·√2 ≈ 24.7` modules
/// from any corner finder's CENTER, plus the 4-module quiet zone ⇒ 28.7 —
/// regardless of which of the three corners the finder is. The v1 class
/// is the deliberate target: it is the smallest symbol, hence the one a
/// multi-module smear most easily strips down to a single surviving
/// finder; a larger symbol under the same smear keeps proportionally more
/// finder structure and yields a pair or triplet, taking those (tighter)
/// evidence paths instead.
const SINGLE_ROI_HALF_SIDE_MODULES: f64 = 29.0;

/// Full-frame 3× upscale (the detection-starved sub-Nyquist tail, E5b) is
/// allowed only when the working view's longest side is ≤ this: 3× a
/// 1024-px frame pushes ~9.4 MP through detection — already several times
/// the whole ladder's budget — and a LARGER working frame is not
/// resolution-starved at frame scale in the first place (its failures are
/// per-code, which the ROI path serves).
const MAX_UPSCALE3X_INPUT_DIM: usize = 1024;

/// Smallest useful upscaled-ROI size: a v1 symbol (21 modules) plus its
/// two 4-module quiet zones at the 2 px/module detection floor is
/// `(21 + 8) · 2 = 58` px — an upscaled ROI below that cannot contain a
/// detectable code and is skipped (degenerate evidence, e.g. a
/// sub-pixel-pitch triplet at a frame edge).
const MIN_UPSCALE_ROI_DIM: usize = 58;

/// Which interpolation kernel the fixed-2× upscale rungs use. Experiment
/// E5c's A/B seam: [`scan_robust`] always uses `Bilinear` (the shipped
/// kernel — monotone, ring-free; see `enhance::bilinear_upscale_2x`);
/// `CatmullRom` is reachable only through [`scan_robust_with_kernel`]
/// (benchmark/test harnesses) to measure the predicted bicubic wash. The
/// 3× path is always bilinear — the A/B question is 2×-specific.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscaleKernel {
    Bilinear,
    CatmullRom,
}

/// Van Cittert smear-length bracket around the measured 20–80% edge-rise
/// extent L̂: ×1 (the measurement itself — maximum likelihood, tried first
/// so early exit stops there), then ×½ and ×3⁄2. The rise estimator's two
/// bias tails are bounded and opposite — ramps truncated by a neighboring
/// module edge bias LOW (a truncated rise cannot exceed the true extent),
/// merged same-direction edges bias HIGH — and the truncated Neumann
/// inverse tolerates roughly ±30% PSF-length mismatch (the residual error
/// term stays contractive across the pass band), so half-octave brackets
/// cover ~[0.35, 1.95]·L̂ with an in-tolerance candidate somewhere. The
/// decode checksum is the oracle deciding the sweep (Sörös pattern):
/// a wrong length either decodes anyway or fails the Reed-Solomon check —
/// it can never emit a wrong payload.
const VAN_CITTERT_LEN_BRACKET: [(u32, u32); 3] = [(1, 1), (1, 2), (3, 2)];

/// Deconvolution kernel-length clamps. A kernel of length ≤ 5 px lies
/// inside the 5-tap unsharp mask's support: to first order the truncated
/// Neumann inverse of that short a box IS the correction the (far
/// cheaper, already-run) unsharp rungs apply, so the Van Cittert tier
/// only fires when the measured extent reaches 7 px — the first odd
/// kernel length strictly beyond that support. (Sharp frames measure
/// L̂ ≈ 2–5 px of anti-aliasing width and correctly skip the tier.)
/// Above 63 px at the ~720p working resolution this ladder targets
/// (~9% of frame height), under 2 px of stationary exposure remain per
/// module for any in-budget code size — beyond single-frame linear
/// recovery (temporal strategies are Plan 6 phase 2).
const MIN_DEBLUR_LEN: usize = 7;
const MAX_DEBLUR_LEN: usize = 63;

/// Upper edge of the Van Cittert-recoverable smear band, in modules of the
/// evidence unit's pitch: E4 (pinned by gate3c) measured decode recovery
/// up to ~2.1–2.4-module smears; beyond ~2.5 modules the box-blur MTF
/// crosses zero inside the code's pass band (sign flip) and a truncated
/// linear inverse cannot recover it (Wiener-class territory, Plan 6 phase
/// 2). Used to pre-gate the deblur tier's SINGLE-FINDER units: a lone
/// finder whose ROI measures a smear outside `[`[`MIN_DEBLUR_LEN`]` px,
/// 2.4·m]` is either not deblur work at all or past linear recovery — in
/// both cases paying the deconvolution passes is provably wasted
/// (diagnosis D3's whole-frame version of this waste cost 47% of frame
/// time for zero yield).
const VC_MAX_SMEAR_MODULES: f64 = 2.4;

/// Which ladder rung produced a detection — the provenance the benchmark
/// harness ranks rungs by.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum VariantKind {
    Baseline,
    /// Plain detection pass on the BOX (area-average) working view — the
    /// shared-substrate sanity pass every enhancement rung then builds on.
    /// Runs first among the enhancements whenever any of
    /// multi-scale/contrast/adaptive/shadow/sharpen is enabled AND a
    /// downscale actually happened (with no downscale the box view IS the
    /// source, so this pass would be an exact re-run of the baseline).
    /// Diagnosis D4: on real video the box substrate alone recovers 1–2
    /// finder candidates per frame the pinned NN working view aliases away.
    BoxWorking,
    /// 2×2 box pyramid level; `level` halvings from the box working view
    /// (1 = 0.5×, 2 = 0.25×).
    Pyramid {
        level: u8,
    },
    /// Tile threshold offset in gray levels (±8).
    ThresholdOffset {
        offset: i16,
    },
    /// Contrast floor lowered from 12 (6σ) to 6 (3σ).
    LowContrastFloor,
    /// Sauvola tile threshold surface.
    SauvolaThreshold,
    /// Background-division illumination normalization.
    ShadowNormalized,
    /// 5-tap unsharp mask, detection buffer only.
    Sharpened,
    /// Fixed-2× bilinear upscale of the whole working view (fallback when
    /// no triplet evidence localizes the failure).
    Upscaled2x,
    /// Fixed-3× bilinear upscale of the whole working view — the
    /// detection-starved sub-Nyquist tail (E5b); fires only after a fruitless
    /// [`VariantKind::Upscaled2x`] on frames ≤ [`MAX_UPSCALE3X_INPUT_DIM`].
    Upscaled3x,
    /// ROI-scoped recovery pass (E5a, generalized by the pooling rewrite):
    /// a padded SOURCE-pixel rectangle around one evidence unit (uncovered
    /// triplet or coherent pooled pair), processed at `factor`× chosen
    /// from the CANDIDATE's pitch ([`upscale_factor_for_pitch`]).
    /// `factor: 1` means a pure pristine-source RESCAN of the ROI — no
    /// resampling at all (diagnosis D2: at ≥3.5 px/module the 1:1 crop
    /// beats the interpolating kernels); 2/3 are the bilinear upscales.
    UpscaledRoi {
        factor: u8,
    },
    /// 1-D unsharp along the blur direction estimated from ONE evidence
    /// ROI's own structure tensor (degrees, snapped to {0,45,90,135}).
    /// ROI-scoped since the pooling rewrite — diagnosis D3 measured the
    /// whole-frame variant of this pass costing 3.4 s across a real
    /// recording for zero decodes, with the global θ >20° wrong for the
    /// code region on 7/12 evidence frames.
    DirectionalSharpened {
        theta_deg: i16,
    },
    /// 1-D Van Cittert deconvolution along the ROI-local blur direction
    /// with a candidate kernel length (px, ROI = source scale) from the
    /// ROI's edge-rise extent bracket. Unlike the unsharp rungs this
    /// variant SAMPLES its own restored buffer: deconvolution exists to
    /// restore data-cell contrast, and sampling the pristine-but-smeared
    /// source would discard exactly that restoration.
    VanCittert {
        theta_deg: i16,
        len: u16,
    },
    /// Pooled cross-variant group+decode (the pipeline's namesake step):
    /// the code was decoded from a finder triple whose members came from
    /// DIFFERENT detection passes — assembled by running `group_triplets`
    /// over the unified candidate pool on the box working view, sampled
    /// from the pristine source. No single [`VariantRecord`] owns such a
    /// code (the step is free — µs-scale — and consumes no budget slot),
    /// so this kind exists purely as decode provenance.
    CrossVariant,
}

impl VariantKind {
    /// The ladder stage this rung belongs to (0 = baseline … 6 = deblur,
    /// 7 = pooled cross-variant decode), matching the Plan 6 stage
    /// numbering.
    pub fn stage(&self) -> u8 {
        match self {
            VariantKind::Baseline => 0,
            VariantKind::BoxWorking | VariantKind::Pyramid { .. } => 1,
            VariantKind::ThresholdOffset { .. } | VariantKind::LowContrastFloor => 2,
            VariantKind::ShadowNormalized | VariantKind::SauvolaThreshold => 3,
            VariantKind::Sharpened => 4,
            VariantKind::Upscaled2x | VariantKind::Upscaled3x | VariantKind::UpscaledRoi { .. } => {
                5
            }
            VariantKind::DirectionalSharpened { .. } | VariantKind::VanCittert { .. } => 6,
            VariantKind::CrossVariant => 7,
        }
    }
}

/// Per-variant outcome record: what ran, what it cost, what it found, and
/// how much of that was NEW (not already found by a cheaper rung) — exactly
/// the numbers needed to rank rungs by marginal yield per millisecond.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VariantRecord {
    pub kind: VariantKind,
    pub stage: u8,
    pub timings: StageTimings,
    /// Whole-variant wall time (buffer preparation + detection).
    pub total_ns: u64,
    pub finders: usize,
    pub triplets: usize,
    pub codes: usize,
    /// Codes this variant contributed that no earlier variant had found.
    pub new_codes: usize,
}

/// A decoded code plus its ladder provenance, with all geometry lifted to
/// SOURCE pixels regardless of which variant buffer found it.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RobustCode {
    /// The decode as produced by its variant. NOTE: `code.corners` are in
    /// that VARIANT's working pixels — use [`RobustCode::corners_source`]
    /// unless you know which buffer you're in.
    pub code: DecodedCode,
    /// Coarse module-region corners in SOURCE px (variant corners mapped
    /// through the variant's exact scale).
    pub corners_source: [[f64; 2]; 4],
    /// Refined corners in SOURCE px when refinement ran. For variants that
    /// sample the true source these are the refiner's own source-px output;
    /// for the shadow-normalization rung they are refined against the
    /// normalized working buffer and rescaled (still subpixel-meaningful —
    /// the rewrite is geometry-preserving — but measured on the variant
    /// image; provenance tells you which case you have).
    pub refined_corners_source: Option<[[f64; 2]; 4]>,
    pub variant: VariantKind,
    pub stage: u8,
}

/// One frame's ladder result.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RobustDetections {
    /// Accepted (deduplicated) codes, in acceptance order — cheapest rung
    /// first.
    pub codes: Vec<RobustCode>,
    /// Every variant that ran, in execution order (index 0 = baseline).
    pub variants: Vec<VariantRecord>,
    /// `true` when early exit fired before every enabled rung ran.
    pub early_exited: bool,
    /// `true` when `max_variants_per_frame` truncated the ladder.
    pub budget_exhausted: bool,
    /// Whole-call wall time.
    pub total_ns: u64,
    /// Deduplicated coherent-triplet centroids in SOURCE px, from every
    /// variant that ran — the ladder's own detection evidence, exposed so a
    /// benchmark can score DETECTION (a triplet near the truth quad)
    /// separately from DECODE.
    pub triplet_evidence: Vec<[f64; 2]>,
    /// Union of every variant's finder candidates, deduplicated, in SOURCE
    /// px (`x`/`y`/`module` all source-scaled; `module` divides by the
    /// width-axis scale, the same width-pinned approximation as
    /// [`Detections::source_scale`]). Robust mode is ONE pipeline whose
    /// result stands in for [`Detections`] — these are its `finders`, so
    /// the same consumers (overlays, tooling) work identically with more
    /// flags simply meaning more candidates. Dedup: same polarity within
    /// half a finder span (3.5 modules) of an earlier entry — the earlier
    /// (cheaper-rung) candidate wins, mirroring code dedup.
    pub finders: Vec<FinderCandidate>,
    /// Union of every variant's triplets, deduplicated by centroid within
    /// one finder span (7 modules — the same physical-code merge radius
    /// `triplet_evidence` uses), in SOURCE px. CAVEAT: each entry's
    /// `finder_indices` still index its ORIGIN variant's own finder list,
    /// NOT `finders` above — kept for provenance, not for lookup.
    pub triplets: Vec<TripletCandidate>,
}

/// Variant→source coordinate map: `source_px = variant_px / (sx, sy) +
/// (ox, oy)`. Full-frame variants (pyramid levels, threshold sweeps,
/// upscales of the working view, ...) have a zero offset, reducing this to
/// the historical pure per-axis scale (adding `0.0` to a finite f64 is
/// exact, so the baseline path stays bit-identical); ROI-scoped variants
/// carry the ROI's integer source-px origin in `(ox, oy)`.
#[derive(Clone, Copy)]
struct VariantMap {
    /// `variant_px = source_px_rel * s` per axis (rel = minus the origin).
    sx: f64,
    sy: f64,
    /// The variant view's origin in source px (integer-valued for ROIs).
    ox: f64,
    oy: f64,
}

impl VariantMap {
    /// A full-frame map: pure per-axis scale, zero origin.
    fn scaled(sx: f64, sy: f64) -> Self {
        VariantMap {
            sx,
            sy,
            ox: 0.0,
            oy: 0.0,
        }
    }

    /// Map a variant-view pixel to source px.
    fn to_source(self, p: [f64; 2]) -> [f64; 2] {
        [p[0] / self.sx + self.ox, p[1] / self.sy + self.oy]
    }

    /// Map a point the refiner already emitted in the variant's OWN
    /// [`SourceView`] coordinates (for an ROI sub-view: ROI-relative source
    /// px — the sub-view shares the source's scale but not its origin), so
    /// only the origin shift remains.
    fn offset_only(self, p: [f64; 2]) -> [f64; 2] {
        [p[0] + self.ox, p[1] + self.oy]
    }
}

/// One coherent-triplet evidence point: centroid plus the geometry the
/// ROI-scoped upscale rung needs to place a recovery window around it.
/// All fields are in SOURCE px regardless of which variant found it.
struct EvidencePoint {
    /// Finder-center centroid.
    p: [f64; 2],
    /// `true` once an accepted code explains this point.
    covered: bool,
    /// The triplet's measured module pitch.
    module: f64,
    /// Bounding box `[min_x, min_y, max_x, max_y]` of the triplet's three
    /// finder centers PLUS the parallelogram-completed fourth corner
    /// `tr + bl − tl` (the standard 4th-anchor predictor, cf.
    /// `alignment.rs`'s `anchor_of`) — the box of just three centers
    /// under-covers the bottom-right quadrant of any rotated or tilted
    /// symbol (at 45° in-plane rotation the three centers span only half
    /// the symbol's diagonal), and an ROI padded from it would clip the
    /// code. This is the symbol-position estimate the ROI pads out.
    bbox: [f64; 4],
}

/// Uncovered-evidence bookkeeping: coherent triplet centroids (source px)
/// that no accepted code explains yet.
struct Evidence {
    points: Vec<EvidencePoint>,
}

impl Evidence {
    fn new() -> Self {
        Evidence { points: Vec::new() }
    }

    fn add_triplets(
        &mut self,
        triplets: &[TripletCandidate],
        map: &VariantMap,
        codes: &[RobustCode],
    ) {
        for t in triplets {
            if t.snap_error >= MAX_EVIDENCE_SNAP_ERROR {
                continue;
            }
            let tl = map.to_source(t.tl);
            let tr = map.to_source(t.tr);
            let bl = map.to_source(t.bl);
            let p = [(tl[0] + tr[0] + bl[0]) / 3.0, (tl[1] + tr[1] + bl[1]) / 3.0];
            let module = t.module / map.sx;
            // Merge with an existing evidence point if within a finder
            // pattern's span (7 modules) of it — same physical code.
            let near = self.points.iter().any(|e| {
                let (dx, dy) = (e.p[0] - p[0], e.p[1] - p[1]);
                (dx * dx + dy * dy).sqrt() < 7.0 * module
            });
            if !near {
                let covered = codes.iter().any(|c| covers(c, p));
                // Predicted 4th corner (see `EvidencePoint::bbox`'s doc).
                let br = [tr[0] + bl[0] - tl[0], tr[1] + bl[1] - tl[1]];
                let bbox = [
                    tl[0].min(tr[0]).min(bl[0]).min(br[0]),
                    tl[1].min(tr[1]).min(bl[1]).min(br[1]),
                    tl[0].max(tr[0]).max(bl[0]).max(br[0]),
                    tl[1].max(tr[1]).max(bl[1]).max(br[1]),
                ];
                self.points.push(EvidencePoint {
                    p,
                    covered,
                    module,
                    bbox,
                });
            }
        }
    }

    fn cover_with(&mut self, code: &RobustCode) {
        for e in self.points.iter_mut() {
            if !e.covered && covers(code, e.p) {
                e.covered = true;
            }
        }
    }

    fn any_uncovered(&self) -> bool {
        self.points.iter().any(|e| !e.covered)
    }
}

/// One pooled finder candidate in SOURCE px — an entry of the pipeline's
/// shared candidate pool. The pool is the source of truth behind
/// [`RobustDetections::finders`] (identical dedup rule and semantics; the
/// public field is materialized from it at the end of the run) and the
/// input to the pooled cross-variant group+decode step.
#[derive(Clone)]
struct PooledFinder {
    x: f64,
    y: f64,
    /// Module pitch in SOURCE px (width-axis scale, same width-pinned
    /// approximation as [`Detections::source_scale`]).
    module: f64,
    inverted: bool,
    hits: u32,
    /// The pass that first contributed this candidate. Provenance for
    /// diagnostics; not surfaced on the public result yet.
    #[allow(dead_code)]
    origin: VariantKind,
}

/// The pipeline's accumulating state, threaded through every pass: the
/// public result under assembly, the uncovered-evidence bookkeeping, the
/// cross-variant candidate pool, and the pooled-decode attempt registry.
struct Ladder {
    out: RobustDetections,
    evidence: Evidence,
    pool: Vec<PooledFinder>,
    /// Cross-FRAME seed candidates ([`ScanSession`], source px): finders
    /// carried from recent near-duplicate video frames so a rung rotated
    /// away THIS frame still contributes to grouping. Kept SEPARATE from
    /// `pool` so they never enter the public `finders` union (they belong
    /// to earlier frames), but grouped alongside it — a seeded finder over
    /// pixels the code has since moved off simply fails
    /// [`group_triplets`]'s leg-module walk on the current view and is
    /// dropped, so it can never fabricate a code. Empty for every
    /// single-frame `scan_robust` call.
    seed_pool: Vec<PooledFinder>,
    /// Order-normalized pooled-finder index triples already handed to the
    /// pooled group+decode step — pool indices are stable (dedup keeps the
    /// earliest entry and only ever appends), so a triple is retried at
    /// most once no matter how many stage batches follow.
    attempted_triples: Vec<[usize; 3]>,
    /// Pool length at the last pooled group+decode: the pool is
    /// append-only, so an unchanged length means an IDENTICAL pool and the
    /// whole grouping pass would be a no-op — skip it (this keeps the
    /// "pooled decode after every batch" step at its advertised µs cost on
    /// the common batches that contribute nothing new).
    grouped_len: usize,
}

/// What a recovery ROI serves, for the covered-recheck right before the
/// pass runs (an earlier pass's decode may have explained it already —
/// don't pay for it twice).
#[derive(Clone, Copy)]
enum RoiTarget {
    /// Index into `Evidence::points` (an uncovered coherent triplet).
    Evidence(usize),
    /// A pair's midpoint in SOURCE px: covered once any accepted code
    /// covers the point.
    Point([f64; 2]),
    /// A single finder's center in SOURCE px (the deblur tier's
    /// last-resort unit class): covered like [`RoiTarget::Point`], but
    /// additionally pre-gated on a measured deconvolution-class smear
    /// (see the deblur stage) before any pass is paid for.
    Single([f64; 2]),
}

/// One planned evidence-scoped recovery window: the integer source-px
/// rectangle to slice, the resolution factor to process it at (1 = pure
/// rescan), and the pitch estimate driving kernel/length choices.
struct RecoveryRoi {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    factor: u8,
    /// The evidence unit's module pitch in SOURCE px (deblur kernel
    /// support and Van Cittert lengths derive from it).
    module: f64,
    target: RoiTarget,
}

/// Clamp a padded floating-point rectangle to the source frame and snap it
/// OUTWARD to the source's [`TILE`] grid plus one extra tile ring,
/// returning the integer window (or `None` when degenerate).
///
/// The tile-phase alignment is load-bearing for decode: a crop whose
/// origin is not a tile multiple binarizes the SAME pixels against a
/// phase-shifted 16-px tile grid, and near the ~4–5 px/module decode
/// margin that half-tile threshold shift alone flips decodes (measured:
/// real frame f0309's 5.35 px/module code decodes from the full native
/// frame but not from its unaligned evidence crop — same finders, same
/// triplet, snap 0.50 both ways). With the origin tile-aligned and one
/// ring of margin tiles, every tile the symbol touches sees the exact
/// 3×3-dilated min/max neighborhood a full-frame pass computes, so the
/// factor-1 rescan's thresholds are byte-identical to full-frame
/// detection over the symbol.
fn clamp_roi(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    sw: usize,
    sh: usize,
) -> Option<(usize, usize, usize, usize)> {
    let x0 = x0.floor().max(0.0) as usize;
    let y0 = y0.floor().max(0.0) as usize;
    let x1 = (x1.ceil().max(0.0) as usize).min(sw);
    let y1 = (y1.ceil().max(0.0) as usize).min(sh);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    // Snap outward to tile boundaries, one extra ring each side.
    let x0 = (x0 / TILE).saturating_sub(1) * TILE;
    let y0 = (y0 / TILE).saturating_sub(1) * TILE;
    let x1 = ((x1.div_ceil(TILE) + 1) * TILE).min(sw);
    let y1 = ((y1.div_ceil(TILE) + 1) * TILE).min(sh);
    Some((x0, y0, x1 - x0, y1 - y0))
}

/// Plan the ROI-scoped recovery passes for the current uncovered TRIPLET
/// evidence: pad each uncovered point's finder-center bbox by
/// [`UPSCALE_ROI_PAD_MODULES`] of its own pitch, clamp to the source
/// frame, pick the factor from the point's pitch
/// ([`upscale_factor_for_pitch`] — resolution-independent), and keep the
/// first [`MAX_UPSCALE_ROIS`] whose processed size clears
/// [`MIN_UPSCALE_ROI_DIM`] (no upper size gate: factor 1 costs no
/// resample, and the sub-Nyquist factors only ever apply to small
/// symbols). Insertion order (deterministic) is preserved; an empty
/// result combined with no pair evidence means the caller falls back to
/// the whole-frame detection-starved upscale path.
fn plan_upscale_rois(evidence: &Evidence, source: &LumaView) -> Vec<RecoveryRoi> {
    let (sw, sh) = (source.width(), source.height());
    let mut rois = Vec::new();
    for (i, e) in evidence.points.iter().enumerate() {
        if e.covered {
            continue;
        }
        if rois.len() >= MAX_UPSCALE_ROIS {
            break;
        }
        let pad = UPSCALE_ROI_PAD_MODULES * e.module;
        let Some((x0, y0, w, h)) = clamp_roi(
            e.bbox[0] - pad,
            e.bbox[1] - pad,
            e.bbox[2] + pad,
            e.bbox[3] + pad,
            sw,
            sh,
        ) else {
            continue;
        };
        let factor = upscale_factor_for_pitch(e.module);
        if w.max(h) * (factor as usize) < MIN_UPSCALE_ROI_DIM {
            continue;
        }
        rois.push(RecoveryRoi {
            x0,
            y0,
            w,
            h,
            factor,
            module: e.module,
            target: RoiTarget::Evidence(i),
        });
    }
    rois
}

/// `true` when pooled finder `f` is already spoken for by stronger
/// structure: inside an accepted code's cover, or inside (one module
/// beyond) the finder-center bbox of an existing evidence triplet — its
/// symbol already has a triplet-shaped recovery unit.
fn pooled_finder_consumed(f: &PooledFinder, evidence: &Evidence, codes: &[RobustCode]) -> bool {
    codes.iter().any(|c| covers(c, [f.x, f.y]))
        || evidence.points.iter().any(|e| {
            f.x >= e.bbox[0] - e.module
                && f.x <= e.bbox[2] + e.module
                && f.y >= e.bbox[1] - e.module
                && f.y <= e.bbox[3] + e.module
        })
}

/// Plan recovery ROIs from coherent pooled finder PAIRS — the second
/// evidence class the pooling rewrite adds (diagnosis D2: 61 real frames
/// hold a coherent cross-variant pair and nothing stronger). A pair is
/// coherent when both members are unconsumed, share polarity, their scan
/// modules agree within the triplet grouper's own
/// [`crate::triplet`] `MAX_MODULE_RATIO` bound (1.5), and their
/// separation `d` is geometrically possible for one symbol
/// ([`PAIR_MIN_SEP_MODULES`]`·m ≤ d ≤ `[`PAIR_MAX_SEP_MODULES`]`·m`).
/// Pairs are capped at [`MAX_UPSCALE_ROIS`] per frame, smallest
/// separation first (the most plausible single-code hypothesis), and no
/// finder serves two selected pairs (overlapping hypotheses of one
/// symbol collapse to the tightest).
fn plan_pair_rois(
    pool: &[PooledFinder],
    evidence: &Evidence,
    codes: &[RobustCode],
    source: &LumaView,
) -> Vec<RecoveryRoi> {
    let (sw, sh) = (source.width(), source.height());
    let consumed: Vec<bool> = pool
        .iter()
        .map(|f| pooled_finder_consumed(f, evidence, codes))
        .collect();
    // (separation, i, j, mean module) for every admissible pair.
    let mut cand: Vec<(f64, usize, usize, f64)> = Vec::new();
    for i in 0..pool.len() {
        if consumed[i] {
            continue;
        }
        for j in (i + 1)..pool.len() {
            if consumed[j] || pool[i].inverted != pool[j].inverted {
                continue;
            }
            let ratio = if pool[i].module > pool[j].module {
                pool[i].module / pool[j].module
            } else {
                pool[j].module / pool[i].module
            };
            // Same bound as triplet grouping's coarse module pre-filter
            // (MAX_MODULE_RATIO): one symbol's scan modules under ≤45°
            // tilt spread at most ~1.41×.
            if ratio > 1.5 {
                continue;
            }
            let m = (pool[i].module + pool[j].module) / 2.0;
            let (dx, dy) = (pool[i].x - pool[j].x, pool[i].y - pool[j].y);
            let d = (dx * dx + dy * dy).sqrt();
            if d < PAIR_MIN_SEP_MODULES * m || d > PAIR_MAX_SEP_MODULES * m {
                continue;
            }
            cand.push((d, i, j, m));
        }
    }
    cand.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut used = vec![false; pool.len()];
    let mut rois = Vec::new();
    for (d, i, j, m) in cand {
        if rois.len() >= MAX_UPSCALE_ROIS {
            break;
        }
        if used[i] || used[j] {
            continue;
        }
        let mid = [(pool[i].x + pool[j].x) / 2.0, (pool[i].y + pool[j].y) / 2.0];
        let half = (PAIR_ROI_SPAN * d + 2.0 * UPSCALE_ROI_PAD_MODULES * m) / 2.0;
        let Some((x0, y0, w, h)) = clamp_roi(
            mid[0] - half,
            mid[1] - half,
            mid[0] + half,
            mid[1] + half,
            sw,
            sh,
        ) else {
            continue;
        };
        let factor = upscale_factor_for_pitch(m);
        if w.max(h) * (factor as usize) < MIN_UPSCALE_ROI_DIM {
            continue;
        }
        used[i] = true;
        used[j] = true;
        rois.push(RecoveryRoi {
            x0,
            y0,
            w,
            h,
            factor,
            module: m,
            target: RoiTarget::Point(mid),
        });
    }
    rois
}

/// Plan single-finder ROIs for the deblur tier's last-resort evidence
/// class: one unconsumed verified finder (all three cross-checks passed)
/// with NO pair or triplet anywhere in the frame — the signature a deep
/// directional smear leaves on a small symbol (gate3c's fixture class:
/// two of three finders die, one survives). Window per
/// [`SINGLE_ROI_HALF_SIDE_MODULES`]; capped at [`MAX_UPSCALE_ROIS`].
fn plan_single_rois(
    pool: &[PooledFinder],
    evidence: &Evidence,
    codes: &[RobustCode],
    source: &LumaView,
) -> Vec<RecoveryRoi> {
    let (sw, sh) = (source.width(), source.height());
    let mut rois = Vec::new();
    for f in pool {
        if rois.len() >= MAX_UPSCALE_ROIS {
            break;
        }
        if pooled_finder_consumed(f, evidence, codes) {
            continue;
        }
        // Deconvolution restores CONTRAST, not resolution: a lone finder
        // below the ~3.5 px/module decode floor (RESCAN_MIN_MODULE_PX's
        // Nyquist bound) belongs to a code that could not decode even
        // perfectly restored at 1:1, so it is not a deblur unit (on real
        // video this floor alone removes the sub-2 px/module shelf-label
        // noise candidates that would otherwise fire the tier per frame).
        if f.module < RESCAN_MIN_MODULE_PX {
            continue;
        }
        let half = SINGLE_ROI_HALF_SIDE_MODULES * f.module;
        let Some((x0, y0, w, h)) =
            clamp_roi(f.x - half, f.y - half, f.x + half, f.y + half, sw, sh)
        else {
            continue;
        };
        if w.max(h) < MIN_UPSCALE_ROI_DIM {
            continue;
        }
        rois.push(RecoveryRoi {
            x0,
            y0,
            w,
            h,
            factor: 1,
            module: f.module,
            target: RoiTarget::Single([f.x, f.y]),
        });
    }
    rois
}

/// Deblur-tier planning: still-unexplained evidence ROIs first (uncovered
/// triplets, then coherent pairs — the same windows the stage-5 recovery
/// passes used), falling back to single-finder windows only when the
/// frame holds NO pair/triplet evidence at all. Total capped at
/// [`MAX_UPSCALE_ROIS`].
fn plan_deblur_rois(l: &Ladder, source: &LumaView) -> Vec<RecoveryRoi> {
    let mut rois = plan_upscale_rois(&l.evidence, source);
    rois.extend(plan_pair_rois(&l.pool, &l.evidence, &l.out.codes, source));
    if rois.is_empty() {
        rois = plan_single_rois(&l.pool, &l.evidence, &l.out.codes, source);
    }
    rois.truncate(MAX_UPSCALE_ROIS);
    rois
}

/// `true` while `roi`'s evidence unit is still unexplained by any accepted
/// code — re-checked right before each pass so a unit covered by an
/// earlier pass's decode is not paid for twice.
fn roi_still_uncovered(l: &Ladder, roi: &RecoveryRoi) -> bool {
    match roi.target {
        RoiTarget::Evidence(i) => !l.evidence.points[i].covered,
        RoiTarget::Point(p) | RoiTarget::Single(p) => !l.out.codes.iter().any(|c| covers(c, p)),
    }
}

/// A triplet centroid is explained by a code when it lies within the code's
/// circumradius: a square of edge `e` has circumradius `e/√2 ≈ 0.71e`,
/// padded to `0.75e` for perspective skew.
fn covers(code: &RobustCode, p: [f64; 2]) -> bool {
    let c = quad_center(&code.corners_source);
    let e = quad_mean_edge(&code.corners_source);
    let (dx, dy) = (c[0] - p[0], c[1] - p[1]);
    (dx * dx + dy * dy).sqrt() < 0.75 * e
}

/// Minimum scan-module size (source px) for a free finder to count as
/// multi-code evidence. Below ~2 px/module the detector's 1:1:3:1:1
/// tolerance is already at its Nyquist floor; residual "finders" of
/// module ≈1.5 with hits=2 are scene texture, and pairs of them freely
/// satisfy the geometric separation window on any busy frame — which
/// would pin the ladder open after every single-code decode (near_00
/// has 10 such noise candidates). A real second portal in the domain
/// recordings is the same physical size as the first and sits at a
/// comparable distance, so its finders clear this floor whenever the
/// first code did.
const MULTI_CODE_MIN_MODULE_PX: f64 = 2.5;

/// `true` when the pool still holds a coherent finder PAIR that sits
/// OUTSIDE every accepted code — one multi-code early-exit signal.
///
/// Domain gold-standard recordings (GPU scanner `Observations.csv`) show
/// multi-code frames where one symbol decodes at baseline and the second
/// is only recoverable via a later rung: if early-exit keys solely on
/// "≥1 code and no uncovered *triplet*", the second symbol's two free
/// finders never form a triplet before exit and the ladder stops short.
///
/// Pair geometry matches [`plan_pair_rois`] (polarity, module ratio ≤1.5,
/// separation in `[`PAIR_MIN_SEP_MODULES`, `PAIR_MAX_SEP_MODULES`]`).
/// Both members AND the pair midpoint must fail [`covers`] for every
/// accepted code, and each member must clear [`MULTI_CODE_MIN_MODULE_PX`].
/// The module floor alone kills the ~1.5 px noise candidates that clutter
/// single-code fixtures.
fn has_unconsumed_coherent_pair(pool: &[PooledFinder], codes: &[RobustCode]) -> bool {
    if pool.len() < 2 || codes.is_empty() {
        return false;
    }
    for i in 0..pool.len() {
        if pool[i].module < MULTI_CODE_MIN_MODULE_PX {
            continue;
        }
        if codes.iter().any(|c| covers(c, [pool[i].x, pool[i].y])) {
            continue;
        }
        for j in (i + 1)..pool.len() {
            if pool[j].module < MULTI_CODE_MIN_MODULE_PX {
                continue;
            }
            if codes.iter().any(|c| covers(c, [pool[j].x, pool[j].y])) {
                continue;
            }
            if pool[i].inverted != pool[j].inverted {
                continue;
            }
            let ratio = if pool[i].module > pool[j].module {
                pool[i].module / pool[j].module
            } else {
                pool[j].module / pool[i].module
            };
            if ratio > 1.5 {
                continue;
            }
            let m = (pool[i].module + pool[j].module) / 2.0;
            let (dx, dy) = (pool[i].x - pool[j].x, pool[i].y - pool[j].y);
            let d = (dx * dx + dy * dy).sqrt();
            if d < PAIR_MIN_SEP_MODULES * m || d > PAIR_MAX_SEP_MODULES * m {
                continue;
            }
            let mid = [(pool[i].x + pool[j].x) / 2.0, (pool[i].y + pool[j].y) / 2.0];
            // Midpoint outside every accepted code → a distinct second
            // symbol, not a noise pair on the first.
            if codes.iter().all(|c| !covers(c, mid)) {
                return true;
            }
        }
    }
    false
}

/// Max module-size ratio between a free singleton finder and an accepted
/// code's mean edge/module estimate for it to count as multi-code signal.
/// Domain portals are equal physical size; depth variation within one
/// frame is modest, so a free finder at ≫2× (or ≪½×) the decoded code's
/// pitch is almost never a second portal — it is noise or a far/near
/// unrelated symbol outside the recovery budget.
const MULTI_CODE_MODULE_RATIO: f64 = 2.0;

/// `true` when a single free finder outside every accepted code is still
/// a plausible second-code seed: module ≥ [`MULTI_CODE_MIN_MODULE_PX`],
/// pitch within [`MULTI_CODE_MODULE_RATIO`] of some accepted code, and
/// not covered by any code. Complements [`has_unconsumed_coherent_pair`]
/// for the common domain case where the second symbol contributes only
/// ONE verified finder at the moment the first decodes (the other two
/// appear under later rungs). Without this, early-exit fires as soon as
/// the first code lands and the ladder never runs the pass that would
/// complete the second triple.
fn has_unconsumed_singleton_finder(pool: &[PooledFinder], codes: &[RobustCode]) -> bool {
    if codes.is_empty() {
        return false;
    }
    // Accepted-code pitch proxy: mean edge / 21 (v1 body) is a lower
    // bound on module size for any version; using the mean of the three
    // finder modules in the pool that ARE covered is tighter when
    // available, else fall back to edge/21.
    let mut code_modules: Vec<f64> = Vec::with_capacity(codes.len());
    for c in codes {
        let covered: Vec<f64> = pool
            .iter()
            .filter(|f| covers(c, [f.x, f.y]) && f.module > 0.0)
            .map(|f| f.module)
            .collect();
        if !covered.is_empty() {
            code_modules.push(covered.iter().sum::<f64>() / covered.len() as f64);
        } else {
            let e = quad_mean_edge(&c.corners_source);
            if e > 0.0 {
                code_modules.push(e / 21.0);
            }
        }
    }
    if code_modules.is_empty() {
        return false;
    }
    for f in pool {
        if f.module < MULTI_CODE_MIN_MODULE_PX {
            continue;
        }
        if codes.iter().any(|c| covers(c, [f.x, f.y])) {
            continue;
        }
        let compatible = code_modules.iter().any(|&cm| {
            let ratio = if f.module > cm {
                f.module / cm
            } else {
                cm / f.module
            };
            ratio <= MULTI_CODE_MODULE_RATIO
        });
        if compatible {
            return true;
        }
    }
    false
}

/// Combined multi-code early-exit inhibitor: free pair OR free singleton.
#[inline]
fn has_multi_code_signal(pool: &[PooledFinder], codes: &[RobustCode]) -> bool {
    has_unconsumed_coherent_pair(pool, codes) || has_unconsumed_singleton_finder(pool, codes)
}

fn quad_center(q: &[[f64; 2]; 4]) -> [f64; 2] {
    [
        (q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0,
        (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0,
    ]
}

/// θ (radians) snapped to the raster axes {0°,45°,90°,135°} — the
/// reporting twin of `enhance::snap_dir` (same rounding).
fn snap_deg(theta: f64) -> i16 {
    let deg = theta.to_degrees().rem_euclid(180.0);
    (((deg / 45.0).round() as i64).rem_euclid(4) * 45) as i16
}

fn quad_mean_edge(q: &[[f64; 2]; 4]) -> f64 {
    let d = |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    (d(q[0], q[1]) + d(q[1], q[2]) + d(q[2], q[3]) + d(q[3], q[0])) / 4.0
}

/// A downscaled copy of one ladder variant's detection buffer — what the
/// scanner actually SAW at that rung — captured by [`scan_robust_debug`]
/// for visual inspection (the debug UI's ladder filmstrip). Thumbnails are
/// box-averaged to ≤[`SNAPSHOT_MAX_DIM`] on the longest side: box filtering
/// is the same anti-aliased kernel the pyramid rungs use, so the thumbnail
/// faithfully shows module structure instead of aliasing it away.
#[derive(Clone, Debug)]
pub struct VariantSnapshot {
    pub kind: VariantKind,
    pub stage: u8,
    pub width: usize,
    pub height: usize,
    /// Tightly packed (stride == width) grayscale thumbnail.
    pub luma: Vec<u8>,
}

/// Thumbnail cap for [`VariantSnapshot`]: 320 px keeps a 16:9 frame's thumb
/// under 58 KB while leaving ~2.8 px per module for the smallest code the
/// detector can see at 720p working res — coarse but recognizably a QR.
const SNAPSHOT_MAX_DIM: usize = 320;

/// [`scan_robust_debug`]'s return: the ladder result plus the one payload
/// too heavy to always carry — the per-variant buffer thumbnail filmstrip
/// (~58 KB of pixels per executed variant plus a box-downscale chain each;
/// serializing it per video frame measured 592 ms vs 58 ms round trips in
/// the debug UI). Everything else a frontend needs — unioned finders/
/// triplets for the standard overlays, per-code provenance, per-variant
/// records — is on [`RobustDetections`] itself: robust mode is ONE
/// pipeline whose result plays the same role as [`Detections`], not a
/// second pipeline with a bolted-on copy of the first.
#[derive(Clone, Debug)]
pub struct RobustDebug {
    pub detections: RobustDetections,
    /// One snapshot per variant that RAN, in execution order (index 0 =
    /// the baseline working view).
    pub snapshots: Vec<VariantSnapshot>,
}

/// Capture sink threaded through the ladder by [`scan_robust_debug`];
/// `None` (every production call) costs nothing — no thumbnail passes.
struct DebugCapture {
    snapshots: Vec<VariantSnapshot>,
}

/// Box-average `view` down to ≤[`SNAPSHOT_MAX_DIM`] on its longest side
/// (repeated 2×2 halving — the pyramid kernel), copying once when it
/// already fits.
fn snapshot_thumb(view: &LumaView, kind: VariantKind) -> VariantSnapshot {
    let mut cur: Option<(Vec<u8>, usize, usize)> = None;
    loop {
        let v = match &cur {
            Some((buf, w, h)) => LumaView::new(buf, *w, *h, *w).expect("thumb packed"),
            None => *view,
        };
        if v.width().max(v.height()) <= SNAPSHOT_MAX_DIM {
            break;
        }
        match box_downscale_half(&v) {
            Some(next) => cur = Some(next),
            None => break,
        }
    }
    let (luma, width, height) = cur.unwrap_or_else(|| {
        let (w, h) = (view.width(), view.height());
        let mut buf = vec![0u8; w * h];
        for y in 0..h {
            buf[y * w..(y + 1) * w].copy_from_slice(view.row(y));
        }
        (buf, w, h)
    });
    VariantSnapshot {
        kind,
        stage: kind.stage(),
        width,
        height,
        luma,
    }
}

/// [`scan_robust`] plus the thumbnail filmstrip: identical detection
/// behavior (same ladder, bilinear kernel), additionally returning a
/// [`VariantSnapshot`] per executed variant. Built for the debug UI; the
/// thumbnail work (one box-downscale chain per variant) is paid only here,
/// never by [`scan_robust`].
pub fn scan_robust_debug(source: &LumaView, opts: &ScanOptions, cfg: &ScanConfig) -> RobustDebug {
    let mut capture = DebugCapture {
        snapshots: Vec::new(),
    };
    let detections = scan_robust_inner(
        source,
        opts,
        cfg,
        UpscaleKernel::Bilinear,
        Some(&mut capture),
        FrameControl::full(),
    );
    RobustDebug {
        detections,
        snapshots: capture.snapshots,
    }
}

/// Run the full escalation ladder on `source`. `opts` carries the same
/// working-resolution budget and refinement switch as [`crate::scan()`]; `cfg`
/// selects which recovery rungs may run. With `ScanConfig::default()` /
/// [`ScanConfig::BASELINE`] the result's `codes`/`variants[0]` reproduce
/// [`crate::scan()`]'s behavior exactly.
pub fn scan_robust(source: &LumaView, opts: &ScanOptions, cfg: &ScanConfig) -> RobustDetections {
    scan_robust_with_kernel(source, opts, cfg, UpscaleKernel::Bilinear)
}

/// Multi-frame (video) tuning for [`ScanSession`]. Defaults amortize the
/// ladder across ~3 near-duplicate frames at a ~4-frame pooling memory —
/// the regime real handheld footage sits in (diagnosis D1: codes stay in
/// view for many consecutive frames, so a rung run every third frame still
/// detects within ~0.3 s at 10 fps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionConfig {
    /// Rotation period in frames: the three always-on-cost detect groups
    /// (box+pyramid substrate, threshold sweep, Sauvola) each run once per
    /// `rotation_period` frames instead of every frame. `1` disables
    /// rotation (every group every frame — the full single-frame ladder,
    /// still with cross-frame seed pooling). `3` (default) cuts per-frame
    /// enhancement cost ~3× while cross-frame pooling reassembles the full
    /// candidate set within one period.
    pub rotation_period: u32,
    /// How many frames a contributed finder is kept as a cross-frame seed.
    /// Must be ≥ `rotation_period` so a full rotation cycle is always
    /// represented in the pool. Larger values tolerate slower scanning at
    /// the cost of holding staler candidates (harmless — they are
    /// re-validated against the current frame and dropped if the code
    /// moved). Default `4`.
    pub pool_ttl_frames: u64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            rotation_period: 3,
            pool_ttl_frames: 4,
        }
    }
}

/// A finder candidate carried across frames, tagged with the frame it was
/// contributed on (for TTL eviction — see [`ScanSession`]).
#[derive(Clone)]
struct TimedFinder {
    finder: PooledFinder,
    frame: u64,
}

/// A stateful multi-frame scanner for video: it amortizes the robustness
/// ladder across near-duplicate frames so per-frame cost stays near
/// baseline while recall is preserved TEMPORALLY. Two mechanisms, both
/// deterministic in the frame sequence (the frame counter drives
/// everything — no wall-clock, so a session replayed on identical frames
/// produces identical output):
///
/// 1. **Rung rotation** ([`SessionConfig::rotation_period`]): the ladder's
///    always-on-cost detect passes (box substrate, threshold sweep,
///    Sauvola) are spread across frames instead of all run every frame.
/// 2. **Cross-frame seed pooling**: each frame's finder candidates are kept
///    for [`SessionConfig::pool_ttl_frames`] and injected into later
///    frames' candidate pool, so a code whose finders are only visible
///    under passes that landed on DIFFERENT frames still groups and decodes
///    — the temporal analog of the in-frame cross-variant pooling win. The
///    seeds are grouped on the CURRENT frame's pixels
///    ([`group_triplets`]'s leg-module walk), so a stale seed over a region
///    the code has moved off simply fails to group: pooling is
///    self-correcting and cannot fabricate a code.
///
/// The single-frame [`scan_robust`] is unchanged and remains the right
/// entry point for stills; a `ScanSession` is for a continuous frame
/// stream from one camera. Call [`ScanSession::reset`] on a source change
/// or scene cut to drop stale cross-frame state.
///
/// Every enhancement flag still comes from the [`ScanConfig`] the session
/// was built with; rotation only changes WHEN each enabled pass runs,
/// never WHETHER it is enabled.
pub struct ScanSession {
    cfg: ScanConfig,
    session: SessionConfig,
    frame_index: u64,
    recent: Vec<TimedFinder>,
}

impl ScanSession {
    /// Build a session with the given ladder config and multi-frame tuning.
    #[must_use]
    pub fn new(cfg: ScanConfig, session: SessionConfig) -> Self {
        ScanSession {
            cfg,
            // A TTL below the rotation period would evict a group's
            // contribution before the cycle completes — clamp up so pooling
            // always sees a full cycle.
            session: SessionConfig {
                pool_ttl_frames: session.pool_ttl_frames.max(session.rotation_period as u64),
                ..session
            },
            frame_index: 0,
            recent: Vec::new(),
        }
    }

    /// Scan the next video frame. `source` is a full-resolution luma view of
    /// the frame; `opts` is the same per-call working-resolution/refine
    /// budget [`scan_robust`] takes. Advances the internal frame counter and
    /// updates the cross-frame pool. Returns the same [`RobustDetections`]
    /// shape as `scan_robust` (codes/finders/triplets/timings), computed
    /// from THIS frame's pixels — corners are always current-frame accurate.
    pub fn scan_frame(&mut self, source: &LumaView, opts: &ScanOptions) -> RobustDetections {
        // Seeds = recent finders still within the TTL window. `saturating`
        // guards the first frames where `frame_index < ttl`.
        let cutoff = self
            .frame_index
            .saturating_sub(self.session.pool_ttl_frames);
        let seed: Vec<PooledFinder> = self
            .recent
            .iter()
            .filter(|t| t.frame >= cutoff)
            .map(|t| t.finder.clone())
            .collect();
        let active = RotationSet::for_frame(self.session.rotation_period, self.frame_index);

        let det = scan_robust_inner(
            source,
            opts,
            &self.cfg,
            UpscaleKernel::Bilinear,
            None,
            FrameControl {
                active,
                seed: &seed,
            },
        );

        // Fold this frame's OWN finders (source px, seeds excluded by
        // construction) into the cross-frame pool, then evict the stale.
        for f in &det.finders {
            self.recent.push(TimedFinder {
                finder: PooledFinder {
                    x: f.x,
                    y: f.y,
                    module: f.module,
                    inverted: f.inverted,
                    hits: f.hits,
                    origin: VariantKind::Baseline,
                },
                frame: self.frame_index,
            });
        }
        let cutoff = self
            .frame_index
            .saturating_sub(self.session.pool_ttl_frames);
        self.recent.retain(|t| t.frame >= cutoff);
        self.frame_index = self.frame_index.wrapping_add(1);
        det
    }

    /// Drop all cross-frame state and restart the frame counter — call on a
    /// source change or scene cut so stale candidates from an unrelated
    /// scene cannot seed the new one.
    pub fn reset(&mut self) {
        self.recent.clear();
        self.frame_index = 0;
    }

    /// The number of frames scanned since construction or the last
    /// [`reset`](ScanSession::reset).
    #[must_use]
    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }
}

/// [`scan_robust`] with an explicit 2× upscale kernel — the E5c A/B seam
/// for benchmark/test harnesses only (hence `#[doc(hidden)]`): production
/// callers use [`scan_robust`], which pins [`UpscaleKernel::Bilinear`].
/// Semantics, cost, and edge behavior are identical apart from the odd
/// upscale samples' interpolation weights.
#[doc(hidden)]
pub fn scan_robust_with_kernel(
    source: &LumaView,
    opts: &ScanOptions,
    cfg: &ScanConfig,
    kernel: UpscaleKernel,
) -> RobustDetections {
    scan_robust_inner(source, opts, cfg, kernel, None, FrameControl::full())
}

/// Which rotatable enhancement-DETECT groups run on a given frame. The
/// three groups are the ladder's always-on-cost passes (the
/// evidence-gated recovery tiers already amortize themselves): the box
/// substrate + pyramid detect (`substrate`), the threshold sweep
/// (`threshold`), and the Sauvola surface (`sauvola`). Stage 0 baseline,
/// the box-view build, and the pooled group+decode always run.
/// [`RotationSet::ALL`] (every group) is the single-frame default;
/// [`ScanSession`] rotates the groups across frames so per-frame cost
/// stays near baseline while cross-frame seed pooling preserves recall.
#[derive(Clone, Copy)]
struct RotationSet {
    substrate: bool,
    threshold: bool,
    sauvola: bool,
}

impl RotationSet {
    const ALL: RotationSet = RotationSet {
        substrate: true,
        threshold: true,
        sauvola: true,
    };

    /// Round-robin assignment: group `g` (0 = substrate, 1 = threshold,
    /// 2 = sauvola) runs on frame `f` iff `f % period == g % period`.
    /// `period ≤ 1` ⇒ every group every frame (= [`RotationSet::ALL`], the
    /// no-rotation contract). At `period == 3` each group runs every third
    /// frame; at `period == 2` the third group shares the even slot.
    fn for_frame(period: u32, frame: u64) -> RotationSet {
        if period <= 1 {
            return Self::ALL;
        }
        let p = period as u64;
        let slot = frame % p;
        RotationSet {
            substrate: slot == 0 % p,
            threshold: slot == 1 % p,
            sauvola: slot == 2 % p,
        }
    }
}

/// Per-frame control threaded into [`scan_robust_inner`]: the rotation mask
/// and the cross-frame seed candidates (source px). [`FrameControl::full`]
/// — every group active, no seed — makes the inner body byte-identical to
/// the pre-session behavior, so `scan_robust`/`scan_robust_debug` are
/// unaffected.
struct FrameControl<'a> {
    active: RotationSet,
    seed: &'a [PooledFinder],
}

impl FrameControl<'static> {
    fn full() -> FrameControl<'static> {
        FrameControl {
            active: RotationSet::ALL,
            seed: &[],
        }
    }
}

/// The pipeline body behind [`scan_robust`]/[`scan_robust_with_kernel`]/
/// [`scan_robust_debug`]: `snap` is the optional debug-capture sink — every
/// capture site is `if let Some(..)`-gated, so the `None` path does no
/// extra work at all.
///
/// Execution shape (the cumulative evidence pipeline — see the module doc):
/// stage 0 baseline (NN working view, byte-identical to [`crate::scan()`]) →
/// pooled group+decode → cheap box-substrate batch (box sanity pass,
/// pyramid, threshold family, Sauvola; pooled group+decode after each) →
/// shadow + sharpen ONLY when the pool holds candidates → evidence-scoped
/// recovery ROIs (factor from candidate pitch; whole-frame 2×/3× fallback
/// only when no evidence unit exists) → ROI-scoped deblur on
/// still-unexplained evidence.
fn scan_robust_inner(
    source: &LumaView,
    opts: &ScanOptions,
    cfg: &ScanConfig,
    kernel: UpscaleKernel,
    mut snap: Option<&mut DebugCapture>,
    frame: FrameControl<'_>,
) -> RobustDetections {
    let call_clock = StageClock::start();
    let (sw, sh) = (source.width() as f64, source.height() as f64);

    // ---- Stage 0: baseline (replicates scan_with, keeping the NN working
    // buffer for the whole-frame detection-starved fallback to reuse).
    let working_owned = downscale_luma(source, opts.max_working_dim);
    let working: LumaView = match &working_owned {
        Some((buf, w, h)) => LumaView::new(buf, *w, *h, *w)
            .expect("downscale_luma returns a non-empty tightly packed buffer"),
        None => *source,
    };
    let w_scale = (working.width() as f64 / sw, working.height() as f64 / sh);
    let w_map = VariantMap::scaled(w_scale.0, w_scale.1);
    let downscaled = working_owned.is_some();

    let mut l = Ladder {
        out: RobustDetections {
            codes: Vec::new(),
            variants: Vec::new(),
            early_exited: false,
            budget_exhausted: false,
            total_ns: 0,
            triplet_evidence: Vec::new(),
            finders: Vec::new(),
            triplets: Vec::new(),
        },
        evidence: Evidence::new(),
        pool: Vec::new(),
        seed_pool: frame.seed.to_vec(),
        attempted_triples: Vec::new(),
        grouped_len: 0,
    };

    let baseline_clock = StageClock::start();
    let baseline = detect_with_source(
        &working,
        downscaled.then_some(SourceView {
            view: source,
            sx: w_scale.0,
            sy: w_scale.1,
        }),
        None,
        opts.refine,
        BinarizeSpec::default(),
    );
    let baseline_ns = baseline_clock.elapsed_ns();
    accept(
        &mut l,
        &baseline.codes,
        VariantKind::Baseline,
        &w_map,
        downscaled,
    );
    record(&mut l.out, VariantKind::Baseline, &baseline, baseline_ns);
    pool_finders(
        &mut l.pool,
        &baseline.finders,
        &w_map,
        VariantKind::Baseline,
    );
    union_triplets(&mut l.out.triplets, &baseline.triplets, &w_map);
    if let Some(cap) = snap.as_deref_mut() {
        cap.snapshots
            .push(snapshot_thumb(&working, VariantKind::Baseline));
    }
    l.evidence
        .add_triplets(&baseline.triplets, &w_map, &l.out.codes);

    // Early-exit condition: ≥1 code accepted, every coherent triplet
    // explained, AND no multi-code signal remains (free coherent pair OR
    // free singleton finder outside accepted codes — see
    // [`has_multi_code_signal`]). Without this a second code whose
    // finders have not yet formed a triplet is abandoned the moment the
    // first code lands. `out.early_exited` is set the first time an
    // enabled rung is actually skipped because of it.
    let done = |l: &Ladder| {
        cfg.enable_early_exit
            && !l.out.codes.is_empty()
            && !l.evidence.any_uncovered()
            && !has_multi_code_signal(&l.pool, &l.out.codes)
    };

    let budget = cfg.max_variants_per_frame;
    let mut variants_run: u32 = 0;
    let mut exhausted = false;

    // Gate + budget accounting for one rung. The body only runs (and only
    // pays for its buffer preparation) when the rung is enabled, the ladder
    // has not exited, and the variant budget has room.
    macro_rules! rung {
        ($enabled:expr, $body:expr) => {
            if $enabled && !exhausted {
                if done(&l) {
                    l.out.early_exited = true;
                } else if budget != 0 && variants_run >= budget {
                    exhausted = true;
                } else {
                    variants_run += 1;
                    #[allow(clippy::redundant_closure_call)]
                    ($body)();
                }
            }
        };
    }

    // ---- The shared BOX working view: the enhancement rungs' detection
    // substrate and the pooled group+decode's grouping substrate (module
    // doc: D4 measured the pinned NN downscale destroying marginal finder
    // runs the area kernel preserves). Same dims as the NN working view,
    // so `w_map` is its coordinate map too; when no downscale happened the
    // box view IS the source (no copy). Skipped entirely — no allocation —
    // when no flag needs it or the baseline already early-exits.
    let any_substrate_rung = cfg.enable_multi_scale
        || cfg.enable_contrast_normalization
        || cfg.enable_shadow_normalization
        || cfg.enable_adaptive_thresholding
        || cfg.enable_sharpening;
    let needs_box = any_substrate_rung || cfg.enable_low_res_upscaling || cfg.enable_deblur;
    let box_owned: Option<(Vec<u8>, usize, usize)> = if needs_box && downscaled && !done(&l) {
        Some(area_downscale(source, working.width(), working.height()))
    } else {
        None
    };
    let box_view: LumaView = match &box_owned {
        Some((buf, w, h)) => LumaView::new(buf, *w, *h, *w).expect("box working view packed"),
        // Not downscaled (box == source), no flags, or early exit — in the
        // latter two cases nothing below reads this view.
        None => *source,
    };
    // Tile grid for the pooled group+decode, built lazily on the first
    // pool that can group (≥3 candidates) and reused for the whole frame.
    let mut box_grid: Option<TileGrid> = None;
    macro_rules! pool_decode {
        () => {
            // Group across the CURRENT pool plus any cross-frame seeds — a
            // seed-only trio can decode if the code is roughly stationary
            // (grouping re-validates it on this frame's pixels).
            if needs_box
                && !done(&l)
                && l.seed_pool.len() + l.pool.len() >= 3
                && l.seed_pool.len() + l.pool.len() != l.grouped_len
            {
                if box_grid.is_none() {
                    box_grid = Some(TileGrid::build_with(&box_view, BinarizeSpec::default()));
                }
                pool_group_and_decode(
                    &mut l,
                    &box_view,
                    box_grid.as_ref().expect("just built"),
                    &w_map,
                    source,
                    opts.refine,
                );
            }
        };
    }

    // ---- Pooled group+decode after stage 0: the baseline may itself hold
    // 3+ finders whose per-variant grouping failed on the NN substrate —
    // the anti-aliased box substrate gets a free shot before any rung pays
    // for pixels.
    pool_decode!();

    // ---- Stage 1: box-substrate sanity pass, then pyramid levels chained
    // by box-halving the box view (cheapest rungs: level k costs 4^-k of a
    // baseline pass).
    rung!(
        any_substrate_rung && downscaled && frame.active.substrate,
        || {
            run_variant(
                &mut l,
                &box_view,
                Some(SourceView {
                    view: source,
                    sx: w_scale.0,
                    sy: w_scale.1,
                }),
                w_map,
                true,
                BinarizeSpec::default(),
                VariantKind::BoxWorking,
                opts.refine,
                0,
                snap.as_deref_mut(),
            );
        }
    );
    if cfg.enable_multi_scale && frame.active.substrate {
        let mut level_owned: Option<(Vec<u8>, usize, usize)> = None;
        for level in 1u8..=2 {
            if exhausted || done(&l) {
                // Let `rung!` book the skip/exit state without building
                // the level buffer first.
                rung!(true, || {});
                break;
            }
            let parent: LumaView = match &level_owned {
                Some((buf, w, h)) => LumaView::new(buf, *w, *h, *w).expect("box level packed"),
                None => box_view,
            };
            if parent.width().max(parent.height()) / 2 < MIN_PYRAMID_DIM {
                break;
            }
            // Octave-coverage gate: only descend past 0.5× when the parent
            // could contain a beyond-ceiling code (see
            // DEEP_LEVEL_MIN_PARENT_DIM's derivation).
            if level >= 2 && parent.width().max(parent.height()) <= DEEP_LEVEL_MIN_PARENT_DIM {
                break;
            }
            let Some(next) = box_downscale_half(&parent) else {
                break;
            };
            level_owned = Some(next);
            let (buf, lw, lh) = level_owned.as_ref().unwrap();
            let lview = LumaView::new(buf, *lw, *lh, *lw).expect("box level packed");
            let scale = (*lw as f64 / sw, *lh as f64 / sh);
            rung!(true, || {
                run_variant(
                    &mut l,
                    &lview,
                    Some(SourceView {
                        view: source,
                        sx: scale.0,
                        sy: scale.1,
                    }),
                    VariantMap::scaled(scale.0, scale.1),
                    true,
                    BinarizeSpec::default(),
                    VariantKind::Pyramid { level },
                    opts.refine,
                    0,
                    snap.as_deref_mut(),
                );
            });
        }
        pool_decode!();
    }

    // ---- Stage 2: threshold variants on the box view (no pixel work,
    // full re-scan each). Offsets ±8 always run (domain: 30 first-decodes).
    // LowContrastFloor is the glare/crush arm (gate3a) but domain gold
    // measured 0 first-decodes at 3.3 ms × 107 firings — so it only runs
    // when we still have no accepted code (or multi-code signal remains).
    // That keeps the crush recovery path while skipping it on the common
    // "baseline already decoded, ladder held open for a second symbol" case.
    if cfg.enable_contrast_normalization && frame.active.threshold {
        for (spec, kind) in [
            (
                BinarizeSpec {
                    threshold_offset: -8,
                    ..BinarizeSpec::default()
                },
                VariantKind::ThresholdOffset { offset: -8 },
            ),
            (
                BinarizeSpec {
                    threshold_offset: 8,
                    ..BinarizeSpec::default()
                },
                VariantKind::ThresholdOffset { offset: 8 },
            ),
        ] {
            rung!(true, || {
                run_variant(
                    &mut l,
                    &box_view,
                    Some(SourceView {
                        view: source,
                        sx: w_scale.0,
                        sy: w_scale.1,
                    }),
                    w_map,
                    true,
                    spec,
                    kind,
                    opts.refine,
                    0,
                    snap.as_deref_mut(),
                );
            });
        }
        pool_decode!();
        // Only when still blind: once ≥1 code is accepted the crush-recovery
        // arm cannot help the multi-code tail (which needs geometry rungs,
        // not a lower contrast floor).
        let want_low_floor = l.out.codes.is_empty();
        rung!(want_low_floor, || {
            run_variant(
                &mut l,
                &box_view,
                Some(SourceView {
                    view: source,
                    sx: w_scale.0,
                    sy: w_scale.1,
                }),
                w_map,
                true,
                BinarizeSpec {
                    contrast_floor: 6,
                    ..BinarizeSpec::default()
                },
                VariantKind::LowContrastFloor,
                opts.refine,
                0,
                snap.as_deref_mut(),
            );
        });
        if want_low_floor {
            pool_decode!();
        }
    }

    // ---- Stage 3b: Sauvola threshold surface on the box view. Diagnosis
    // D1 credits it 2 first-decodes on real video, and gate3b's shadow
    // fixture recovery rides on it — so it is its own rotation group
    // rather than a rung skipped for cost.
    if cfg.enable_adaptive_thresholding && frame.active.sauvola {
        rung!(true, || {
            run_variant(
                &mut l,
                &box_view,
                Some(SourceView {
                    view: source,
                    sx: w_scale.0,
                    sy: w_scale.1,
                }),
                w_map,
                true,
                BinarizeSpec {
                    sauvola: true,
                    ..BinarizeSpec::default()
                },
                VariantKind::SauvolaThreshold,
                opts.refine,
                0,
                snap.as_deref_mut(),
            );
        });
        pool_decode!();
    }

    // ---- Stages 3a + 4: shadow normalization and unsharp — the two most
    // expensive full-frame rungs (~12 + 6 ms). Domain measurement: firing
    // on every non-empty pool burned p95 on noise-finder frames. Gate on
    // coherent recovery signal instead:
    //   • uncovered triplet evidence, OR
    //   • multi-code free pair/singleton, OR
    //   • ≥3 pool finders with no codes yet (a full finder set that has
    //     not grouped — the shadow_0x / hard-illumination case where
    //     binarization must rewrite pixels before a triple forms).
    // Empty-pool and lone-noise-finder frames skip both rungs entirely.
    let needs_pixel_recovery = l.evidence.any_uncovered()
        || has_multi_code_signal(&l.pool, &l.out.codes)
        || (l.out.codes.is_empty() && l.pool.len() >= 3);
    if cfg.enable_shadow_normalization && needs_pixel_recovery {
        rung!(true, || {
            let prep_clock = StageClock::start();
            // Structuring element from baseline scale evidence: the
            // widest solid dark INK structure in a QR symbol is the
            // finder ring — 7 modules across, ≤ 7√2 ≈ 9.9 modules
            // axis-aligned under tilt — so 10 modules is the
            // smallest SE the closing can use without ever
            // swallowing evidenced ink; and every pixel above that
            // minimum raises the narrowest recoverable shadow width
            // 1:1, so it is also the optimum. The LARGEST candidate
            // module is used (conservative when evidence spans
            // scales). Lower clamp 20 px = 10 modules at the 2
            // px/module decode floor (a smaller SE cannot protect
            // any decodable code's ink). With no finder evidence
            // there is no ink scale to protect against: fall back
            // to max(w,h)/6, which protects codes up to module =
            // dim/60 (a v2 symbol spanning ~40 % of the frame) —
            // any larger code virtually guarantees at least one
            // baseline finder candidate and takes the evidence
            // path.
            let m_max = baseline
                .finders
                .iter()
                .map(|f| f.module)
                .fold(0.0f64, f64::max);
            let se = if m_max > 0.0 {
                ((10.0 * m_max).round() as usize).max(20)
            } else {
                box_view.width().max(box_view.height()) / 6
            };
            let (buf, nw, nh) = background_divide(&box_view, se);
            let prep_ns = prep_clock.elapsed_ns();
            let nview = LumaView::new(&buf, nw, nh, nw).expect("normalized buffer packed");
            // Samples its OWN buffer (None source view): the
            // normalized image is the one whose modules are readable
            // against its thresholds.
            run_variant(
                &mut l,
                &nview,
                None,
                w_map,
                false,
                BinarizeSpec::default(),
                VariantKind::ShadowNormalized,
                opts.refine,
                prep_ns,
                snap.as_deref_mut(),
            );
        });
    }
    if cfg.enable_sharpening && needs_pixel_recovery {
        rung!(true, || {
            let prep_clock = StageClock::start();
            let (buf, uw, uh) = unsharp_mask(&box_view);
            let prep_ns = prep_clock.elapsed_ns();
            let uview = LumaView::new(&buf, uw, uh, uw).expect("sharpened buffer packed");
            run_variant(
                &mut l,
                &uview,
                Some(SourceView {
                    view: source,
                    sx: w_scale.0,
                    sy: w_scale.1,
                }),
                w_map,
                true,
                BinarizeSpec::default(),
                VariantKind::Sharpened,
                opts.refine,
                prep_ns,
                snap.as_deref_mut(),
            );
        });
    }
    if cfg.enable_shadow_normalization || cfg.enable_sharpening {
        pool_decode!();
    }

    // ---- Stage 5: evidence-scoped recovery. Each evidence unit (an
    // uncovered coherent triplet, or a coherent pooled finder pair) gets
    // one SOURCE-px ROI processed at the factor its own pitch demands
    // (factor 1 = pure rescan, escalating once to 2× if unexplained).
    // Only when NO TRIPLET evidence exists — the pre-rewrite
    // detection-starved condition; a pair is too weak a signal to
    // suppress a fallback diagnosis D1 credits 8 first-decodes — does the
    // whole-frame upscale fallback run (unchanged, including its
    // frame-size gates); anything it surfaces is pooled and re-planned
    // into ROI units.
    if cfg.enable_low_res_upscaling && !exhausted {
        // A helper closure would capture `l` mutably and conflict with
        // `rung!`'s own captures, so the ROI pass body is a macro. One
        // expansion = one detect pass over `$roi`'s window at `$factor`,
        // followed by the free pooled group+decode (an ROI pass's finders
        // may complete a cross-variant triple even when the ROI's own
        // grouping fails).
        macro_rules! roi_pass {
            ($roi:expr, $factor:expr) => {
                rung!(true, || {
                    let roi: &RecoveryRoi = $roi;
                    let factor: u8 = $factor;
                    let Some(roi_view) = source.sub_view(roi.x0, roi.y0, roi.w, roi.h) else {
                        return;
                    };
                    let f = factor as f64;
                    let prep_clock = StageClock::start();
                    // Factor 1 = rescan: detection runs on the pristine
                    // crop itself, no resample buffer at all.
                    let up_owned: Option<(Vec<u8>, usize, usize)> = match (factor, kernel) {
                        (1, _) => None,
                        (4, _) => Some(bilinear_upscale_4x(&roi_view)),
                        (3, _) => Some(bilinear_upscale_3x(&roi_view)),
                        (_, UpscaleKernel::CatmullRom) => Some(catmull_rom_upscale_2x(&roi_view)),
                        _ => Some(bilinear_upscale_2x(&roi_view)),
                    };
                    let prep_ns = prep_clock.elapsed_ns();
                    let uview: LumaView = match &up_owned {
                        Some((buf, uw, uh)) => {
                            LumaView::new(buf, *uw, *uh, *uw).expect("upscaled ROI packed")
                        }
                        None => roi_view,
                    };
                    run_variant(
                        &mut l,
                        &uview,
                        Some(SourceView {
                            view: &roi_view,
                            sx: f,
                            sy: f,
                        }),
                        VariantMap {
                            sx: f,
                            sy: f,
                            ox: roi.x0 as f64,
                            oy: roi.y0 as f64,
                        },
                        true,
                        BinarizeSpec::default(),
                        VariantKind::UpscaledRoi { factor },
                        opts.refine,
                        prep_ns,
                        snap.as_deref_mut(),
                    );
                });
                pool_decode!();
            };
        }
        macro_rules! roi_passes {
            ($rois:expr) => {
                for roi in $rois {
                    if exhausted {
                        break;
                    }
                    // An earlier pass's decode may have covered this unit
                    // already — don't pay for it twice.
                    if !roi_still_uncovered(&l, &roi) {
                        continue;
                    }
                    let factor = roi.factor;
                    roi_pass!(&roi, factor);
                    // Rescan escalation: a factor-1 unit whose 1:1 rescan
                    // leaves it unexplained gets ONE 2× pass of the same
                    // window. Above the decode floor the residual failure
                    // is alignment/grid-geometry quantization, which the
                    // co-sited upscale refines (detection geometry runs
                    // on the view; module sampling reads the pristine
                    // source either way) — measured: the illum_02
                    // gradient fixture detects a snap-0.00 triplet at 8.6
                    // px/module in every native-resolution pass yet only
                    // decodes through the 2× ROI.
                    if factor == 1 && !exhausted && roi_still_uncovered(&l, &roi) {
                        roi_pass!(&roi, 2);
                    }
                }
            };
        }

        // Whole-frame 2× of the NN WORKING view — the pre-pooling rung,
        // byte-for-byte: its NN-decimated (aliased, hence artificially
        // steepened) edges are a genuinely different detection substrate
        // from both the source and the box view, and diagnosis D1 credits
        // it 8 first-decodes on empty-pool frames.
        macro_rules! full_frame_2x {
            () => {
                rung!(
                    working.width().max(working.height()) <= MAX_UPSCALE_INPUT_DIM,
                    || {
                        let prep_clock = StageClock::start();
                        let (buf, uw, uh) = match kernel {
                            UpscaleKernel::Bilinear => bilinear_upscale_2x(&working),
                            UpscaleKernel::CatmullRom => catmull_rom_upscale_2x(&working),
                        };
                        let prep_ns = prep_clock.elapsed_ns();
                        let uview =
                            LumaView::new(&buf, uw, uh, uw).expect("upscaled buffer packed");
                        let scale = (uw as f64 / sw, uh as f64 / sh);
                        run_variant(
                            &mut l,
                            &uview,
                            Some(SourceView {
                                view: source,
                                sx: scale.0,
                                sy: scale.1,
                            }),
                            VariantMap::scaled(scale.0, scale.1),
                            true,
                            BinarizeSpec::default(),
                            VariantKind::Upscaled2x,
                            opts.refine,
                            prep_ns,
                            snap.as_deref_mut(),
                        );
                    }
                );
                pool_decode!();
            };
        }

        let trip_rois = plan_upscale_rois(&l.evidence, source);
        let detection_starved = trip_rois.is_empty();
        let mut rois = trip_rois;
        rois.extend(plan_pair_rois(&l.pool, &l.evidence, &l.out.codes, source));
        roi_passes!(rois);
        // Whole-frame detection-starved 2× (~13 ms). Gating:
        // - Empty pool: only when deblur is on (gate3c feeder; production
        //   ROBUST_FAST skips — D1 measured 0 direct recall on codeless).
        // - Non-empty pool: only when SOME candidate is resolution-starved
        //   (module < RESCAN_MIN_MODULE_PX). The lowres fixtures that
        //   decode via whole-frame 2× all have sub-floor modules; domain
        //   multi-code frames with m≈4–5 previously paid 45×13 ms for
        //   zero first-decodes.
        let run_starved_2x = if l.pool.is_empty() {
            cfg.enable_deblur
        } else {
            l.pool
                .iter()
                .any(|f| f.module > 0.0 && f.module < RESCAN_MIN_MODULE_PX)
        };
        if detection_starved && run_starved_2x && !exhausted {
            full_frame_2x!();
            // Whole-frame 3× (E5b): the sub-Nyquist last resort, only for
            // frames still completely blind — no codes AND no evidence —
            // and only within the 3× size budget (9× pixels ⇒ the
            // priciest pass anywhere).
            rung!(
                l.out.codes.is_empty()
                    && l.evidence.points.is_empty()
                    && working.width().max(working.height()) <= MAX_UPSCALE3X_INPUT_DIM,
                || {
                    let prep_clock = StageClock::start();
                    let (buf, uw, uh) = bilinear_upscale_3x(&working);
                    let prep_ns = prep_clock.elapsed_ns();
                    let uview = LumaView::new(&buf, uw, uh, uw).expect("upscaled buffer packed");
                    let scale = (uw as f64 / sw, uh as f64 / sh);
                    run_variant(
                        &mut l,
                        &uview,
                        Some(SourceView {
                            view: source,
                            sx: scale.0,
                            sy: scale.1,
                        }),
                        VariantMap::scaled(scale.0, scale.1),
                        true,
                        BinarizeSpec::default(),
                        VariantKind::Upscaled3x,
                        opts.refine,
                        prep_ns,
                        snap.as_deref_mut(),
                    );
                }
            );
            pool_decode!();
            // The fallback may have surfaced candidates: plan the units it
            // earned and serve them with cheap ROIs.
            let mut rois = plan_upscale_rois(&l.evidence, source);
            rois.extend(plan_pair_rois(&l.pool, &l.evidence, &l.out.codes, source));
            roi_passes!(rois);
        } else if l.evidence.any_uncovered() && !exhausted {
            // ROI misses → whole-frame 2× (different NN-substrate buffer).
            // Domain gold (419 obs-frames): Upscaled2x fired 52× for
            // **zero** first-decodes at 13 ms mean — pure p95 poison under
            // production ROBUST_FAST (deblur off). Keep the path only when
            // deblur is on (gate3c / FULL: the 2× is the feeder that
            // surfaces a coarse candidate for Van Cittert). When deblur is
            // off, ROI 1:1 / 2× / 3× already exhausted the resolution axis
            // at ~1 ms per unit.
            if cfg.enable_deblur {
                let sub_floor = l
                    .evidence
                    .points
                    .iter()
                    .any(|e| !e.covered && e.module < RESCAN_MIN_MODULE_PX);
                let ran_roi2 =
                    l.out.variants.iter().any(
                        |v| matches!(v.kind, VariantKind::UpscaledRoi { factor } if factor >= 2),
                    );
                if sub_floor || !ran_roi2 {
                    full_frame_2x!();
                }
            }
        }
    }

    // ---- Stage 6: ROI-scoped deblur, LAST — only for evidence units
    // still unexplained after everything above. Direction and extent are
    // measured ON each ROI (diagnosis D3: the ROI-local θ is the
    // physically correct smear direction; whole-frame firing paid 47% of
    // frame time for zero yield). The ROI tensor probe itself is µs-scale
    // and consumes no budget slot; every detect pass does.
    if cfg.enable_deblur && !exhausted {
        for u in plan_deblur_rois(&l, source) {
            if exhausted {
                break;
            }
            if !roi_still_uncovered(&l, &u) {
                continue;
            }
            let Some(roi_view) = source.sub_view(u.x0, u.y0, u.w, u.h) else {
                continue;
            };
            let tensor_clock = StageClock::start();
            let (theta, confidence) = structure_tensor_blur_direction(&roi_view);
            // Charged to the first deblur pass that actually runs.
            let tensor_ns = tensor_clock.elapsed_ns();
            if confidence < MIN_DIRECTIONAL_CONFIDENCE {
                // Near-isotropic ROI: no direction to sharpen along.
                continue;
            }
            // Smear extent measured up front (ROI-local 20-80% edge
            // rise): the Van Cittert gate for every unit, and the whole
            // ADMISSION gate for single-finder units — a lone finder is
            // only deblur-tier evidence if its window actually measures a
            // recoverable deconvolution-class smear, `l_hat ∈
            // [MIN_DEBLUR_LEN px, VC_MAX_SMEAR_MODULES · m]` (below: the
            // cheap unsharp rungs' territory; above: past the box-MTF
            // sign flip). Without this gate the tier would fire on every
            // frame holding one noise candidate — the exact whole-frame
            // waste diagnosis D3 measured.
            let est_clock = StageClock::start();
            let l_hat_raw = edge_rise_extent(&roi_view, theta);
            let mut tensor_ns = tensor_ns + est_clock.elapsed_ns();
            let l_hat = l_hat_raw.filter(|&x| x.round() as usize >= MIN_DEBLUR_LEN);
            // Single-finder admission window: the class premise is that
            // the smear DESTROYED the unit's partner finders, and finder
            // run-matching survives sub-module smears (~±50% run
            // tolerance) — so a window whose measured extent is smaller
            // than one module of the surviving finder cannot explain the
            // missing partners (they would have been detected; the lone
            // candidate is noise). Above, the extent must stay inside the
            // linear-recovery band (VC_MAX_SMEAR_MODULES). Hence
            // l_hat ∈ [max(MIN_DEBLUR_LEN px, 1.0·m), 2.4·m].
            if matches!(u.target, RoiTarget::Single(_))
                && l_hat.is_none_or(|x| x < u.module || x > VC_MAX_SMEAR_MODULES * u.module)
            {
                continue;
            }
            let map = VariantMap {
                sx: 1.0,
                sy: 1.0,
                ox: u.x0 as f64,
                oy: u.y0 as f64,
            };
            // Kernel support ~0.75 of the unit's own module keeps
            // cross-module ringing bounded (same rule as the pre-rewrite
            // whole-frame pass, now fed by per-unit pitch).
            let len = (0.75 * u.module).round().clamp(3.0, 15.0) as usize;
            // The directional-unsharp pass runs for triplet/pair units
            // only: a single-finder unit is admitted (above) purely on a
            // deconvolution-class smear (l_hat ≥ MIN_DEBLUR_LEN), which is
            // BY DEFINITION beyond the unsharp support — the pass could
            // only reproduce work the isotropic Sharpened rung already
            // did, at the price of the tier's largest windows.
            let ds_applicable = !matches!(u.target, RoiTarget::Single(_));
            rung!(ds_applicable, || {
                let prep_clock = StageClock::start();
                let (buf, dw, dh) = directional_unsharp(&roi_view, theta, len);
                let prep_ns = prep_clock.elapsed_ns() + std::mem::take(&mut tensor_ns);
                let dview = LumaView::new(&buf, dw, dh, dw).expect("deblurred ROI packed");
                run_variant(
                    &mut l,
                    &dview,
                    Some(SourceView {
                        view: &roi_view,
                        sx: 1.0,
                        sy: 1.0,
                    }),
                    map,
                    true,
                    BinarizeSpec::default(),
                    VariantKind::DirectionalSharpened {
                        theta_deg: snap_deg(theta),
                    },
                    opts.refine,
                    prep_ns,
                    snap.as_deref_mut(),
                );
            });
            // Van Cittert candidate sweep: the up-front extent, bracketed
            // per VAN_CITTERT_LEN_BRACKET; extents inside the unsharp
            // support are not deconvolution work (see MIN_DEBLUR_LEN).
            let mut est_ns = std::mem::take(&mut tensor_ns);
            if let Some(l_hat) = l_hat {
                let mut seen: [usize; VAN_CITTERT_LEN_BRACKET.len()] =
                    [0; VAN_CITTERT_LEN_BRACKET.len()];
                let mut n_seen = 0usize;
                for (num, den) in VAN_CITTERT_LEN_BRACKET {
                    let vlen = ((l_hat * num as f64 / den as f64).round() as usize)
                        .clamp(MIN_DEBLUR_LEN, MAX_DEBLUR_LEN)
                        | 1; // odd (matches van_cittert_directional)
                    if seen[..n_seen].contains(&vlen) {
                        continue; // clamp collision — same kernel
                    }
                    seen[n_seen] = vlen;
                    n_seen += 1;
                    rung!(true, || {
                        let prep_clock = StageClock::start();
                        let (buf, dw, dh) = van_cittert_directional(&roi_view, theta, vlen);
                        let prep_ns = prep_clock.elapsed_ns() + std::mem::take(&mut est_ns);
                        let dview =
                            LumaView::new(&buf, dw, dh, dw).expect("deconvolved ROI packed");
                        // Samples its OWN buffer (None source view) — see
                        // VariantKind::VanCittert.
                        run_variant(
                            &mut l,
                            &dview,
                            None,
                            map,
                            false,
                            BinarizeSpec::default(),
                            VariantKind::VanCittert {
                                theta_deg: snap_deg(theta),
                                len: vlen as u16,
                            },
                            opts.refine,
                            prep_ns,
                            snap.as_deref_mut(),
                        );
                    });
                }
            }
            pool_decode!();
        }
    }

    // The final rung's budget increment is intentionally unread — the
    // counter exists for the inter-rung checks above.
    let _ = variants_run;
    l.out.budget_exhausted = exhausted;
    l.out.triplet_evidence = l.evidence.points.iter().map(|e| e.p).collect();
    // The pool is the single source of truth behind the public unified
    // finder union (same dedup rule and semantics as ever) — materialize
    // it.
    l.out.finders = l
        .pool
        .iter()
        .map(|p| FinderCandidate {
            x: p.x,
            y: p.y,
            module: p.module,
            inverted: p.inverted,
            hits: p.hits,
        })
        .collect();
    l.out.total_ns = call_clock.elapsed_ns();
    l.out
}

/// Run one variant detection pass and fold its results into the pipeline
/// state: accept its codes, record its [`VariantRecord`], ADD its finder
/// candidates to the shared pool and its triplets to the public union +
/// evidence set. `map` is the variant view's coordinate map back to the
/// original source (per-axis scale plus, for ROI variants, the ROI's
/// source-px origin); `refined_in_source` says whether a passed
/// [`SourceView`] made the refiner emit that view's own source px directly
/// (else refined corners are variant px and get fully remapped here;
/// either way the ROI origin shift is applied — see
/// [`VariantMap::offset_only`]).
#[allow(clippy::too_many_arguments)]
fn run_variant(
    l: &mut Ladder,
    view: &LumaView,
    source_view: Option<SourceView>,
    map: VariantMap,
    refined_in_source: bool,
    spec: BinarizeSpec,
    kind: VariantKind,
    refine: bool,
    prep_ns: u64,
    snap: Option<&mut DebugCapture>,
) {
    if let Some(cap) = snap {
        cap.snapshots.push(snapshot_thumb(view, kind));
    }
    let clock = StageClock::start();
    let det = detect_with_source(view, source_view, None, refine, spec);
    let total_ns = clock.elapsed_ns() + prep_ns;
    let new_codes = accept(l, &det.codes, kind, &map, refined_in_source);
    record(&mut l.out, kind, &det, total_ns);
    pool_finders(&mut l.pool, &det.finders, &map, kind);
    union_triplets(&mut l.out.triplets, &det.triplets, &map);
    if let Some(last) = l.out.variants.last_mut() {
        last.new_codes = new_codes;
    }
    l.evidence.add_triplets(&det.triplets, &map, &l.out.codes);
}

/// The pipeline's namesake step: map the WHOLE cross-variant candidate
/// pool onto the box working view, run [`group_triplets`] once over the
/// union, and push any freshly formed triple straight through
/// [`decode_candidates`] (module sampling reads the pristine source).
/// Closes the pooling gap (module doc / diagnosis D1-D2: triples whose
/// members only co-appear across different variants could never group
/// per-variant). µs-scale on <32 candidates ⇒ consumes no budget slot.
/// Triples already attempted (order-normalized pool-index key — pool
/// indices are append-only stable) or already explained by an accepted
/// code are skipped; fresh but undecoded triples join the evidence set
/// and public union so they drive ROI recovery exactly like
/// variant-native triplets.
fn pool_group_and_decode(
    l: &mut Ladder,
    box_view: &LumaView,
    box_grid: &TileGrid,
    box_map: &VariantMap,
    source: &LumaView,
    refine: bool,
) {
    l.grouped_len = l.seed_pool.len() + l.pool.len();
    // Cross-frame seeds FIRST, then the current pool — seeds are fixed for
    // the frame while the pool only appends, so this ordering keeps every
    // `finder_indices` triple key stable across a frame's batches (the
    // append-only invariant the `attempted_triples` registry relies on).
    // Both are source px; `box_map` is full-frame (zero origin), so the
    // inverse of `to_source` is a pure per-axis scale.
    let mapped: Vec<FinderCandidate> = l
        .seed_pool
        .iter()
        .chain(l.pool.iter())
        .map(|p| FinderCandidate {
            x: p.x * box_map.sx,
            y: p.y * box_map.sy,
            module: p.module * box_map.sx,
            inverted: p.inverted,
            hits: p.hits,
        })
        .collect();
    let grouped = group_triplets(box_view, box_grid, &mapped);
    let mut fresh: Vec<TripletCandidate> = Vec::new();
    for t in grouped {
        let mut key = t.finder_indices;
        key.sort_unstable();
        if l.attempted_triples.contains(&key) {
            continue;
        }
        let c_box = [
            (t.tl[0] + t.tr[0] + t.bl[0]) / 3.0,
            (t.tl[1] + t.tr[1] + t.bl[1]) / 3.0,
        ];
        let c_src = box_map.to_source(c_box);
        if l.out.codes.iter().any(|cd| covers(cd, c_src)) {
            continue;
        }
        l.attempted_triples.push(key);
        fresh.push(t);
    }
    if fresh.is_empty() {
        return;
    }
    let (codes, _attempts, _timings, _trace) = decode_candidates(
        box_view,
        box_grid,
        &mapped,
        &fresh,
        false,
        Some(SourceView {
            view: source,
            sx: box_map.sx,
            sy: box_map.sy,
        }),
        refine,
    );
    accept(l, &codes, VariantKind::CrossVariant, box_map, true);
    l.evidence.add_triplets(&fresh, box_map, &l.out.codes);
    union_triplets(&mut l.out.triplets, &fresh, box_map);
}

/// Fold one pass's finder candidates into the shared source-px pool (the
/// source of truth behind [`RobustDetections::finders`] — see that field's
/// doc for the dedup rule and the one-pipeline rationale). Geometry maps
/// through the pass's [`VariantMap`]; lengths (`module`) divide by the
/// width-axis scale — the same width-pinned approximation as
/// [`Detections::source_scale`]. Dedup: same polarity within half a finder
/// span (3.5 modules) of an earlier entry — the earlier (cheaper-rung)
/// candidate wins, mirroring code dedup.
fn pool_finders(
    pool: &mut Vec<PooledFinder>,
    finders: &[FinderCandidate],
    map: &VariantMap,
    origin: VariantKind,
) {
    for f in finders {
        let [x, y] = map.to_source([f.x, f.y]);
        let module = f.module / map.sx;
        let dup = pool.iter().any(|e| {
            e.inverted == f.inverted && {
                let (dx, dy) = (e.x - x, e.y - y);
                (dx * dx + dy * dy).sqrt() < 3.5 * e.module
            }
        });
        if !dup {
            pool.push(PooledFinder {
                x,
                y,
                module,
                inverted: f.inverted,
                hits: f.hits,
                origin,
            });
        }
    }
}

/// Fold one pass's triplets into the public source-px union on
/// [`RobustDetections::triplets`] (see the field's doc for the dedup
/// rule). For pooled (cross-variant) triples the incoming
/// `finder_indices` genuinely index the unified pool == the public
/// `finders` list; for variant-native triples they index the ORIGIN
/// variant's own finder list (provenance only — the field's documented
/// caveat).
fn union_triplets(
    out: &mut Vec<TripletCandidate>,
    triplets: &[TripletCandidate],
    map: &VariantMap,
) {
    for t in triplets {
        let tl = map.to_source(t.tl);
        let tr = map.to_source(t.tr);
        let bl = map.to_source(t.bl);
        let module = t.module / map.sx;
        let c = [(tl[0] + tr[0] + bl[0]) / 3.0, (tl[1] + tr[1] + bl[1]) / 3.0];
        let dup = out.iter().any(|e| {
            let ec = [
                (e.tl[0] + e.tr[0] + e.bl[0]) / 3.0,
                (e.tl[1] + e.tr[1] + e.bl[1]) / 3.0,
            ];
            let (dx, dy) = (ec[0] - c[0], ec[1] - c[1]);
            (dx * dx + dy * dy).sqrt() < 7.0 * e.module
        });
        if !dup {
            out.push(TripletCandidate {
                tl,
                tr,
                bl,
                module,
                dimension: t.dimension,
                snap_error: t.snap_error,
                inverted: t.inverted,
                finder_indices: t.finder_indices,
            });
        }
    }
}

/// Fold a pass's decoded codes into the accepted set (dedup by payload +
/// source-space center distance under half the code's mean edge — multi-code
/// frames can legitimately repeat a payload at different positions). Returns
/// how many were new.
fn accept(
    l: &mut Ladder,
    codes: &[DecodedCode],
    kind: VariantKind,
    map: &VariantMap,
    refined_in_source: bool,
) -> usize {
    let mut new_count = 0;
    for code in codes {
        let corners_source = code.corners.map(|c| map.to_source(c));
        let dup = l.out.codes.iter().any(|existing| {
            if existing.code.payload_bytes != code.payload_bytes {
                return false;
            }
            let c0 = quad_center(&existing.corners_source);
            let c1 = quad_center(&corners_source);
            let e = quad_mean_edge(&existing.corners_source);
            let (dx, dy) = (c0[0] - c1[0], c0[1] - c1[1]);
            (dx * dx + dy * dy).sqrt() < 0.5 * e
        });
        if dup {
            continue;
        }
        let refined_corners_source = code.refined_corners.map(|rc| {
            if refined_in_source {
                rc.map(|c| map.offset_only(c))
            } else {
                rc.map(|c| map.to_source(c))
            }
        });
        let accepted = RobustCode {
            code: code.clone(),
            corners_source,
            refined_corners_source,
            variant: kind,
            stage: kind.stage(),
        };
        l.evidence.cover_with(&accepted);
        l.out.codes.push(accepted);
        new_count += 1;
    }
    new_count
}

fn record(out: &mut RobustDetections, kind: VariantKind, det: &Detections, total_ns: u64) {
    out.variants.push(VariantRecord {
        kind,
        stage: kind.stage(),
        timings: det.timings,
        total_ns,
        finders: det.finders.len(),
        triplets: det.triplets.len(),
        codes: det.codes.len(),
        new_codes: det.codes.len(), // patched by run_variant for non-baseline
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan;

    fn flat(w: usize, h: usize) -> Vec<u8> {
        vec![128u8; w * h]
    }

    #[test]
    fn baseline_config_matches_scan_on_a_flat_frame() {
        let d = flat(320, 240);
        let view = LumaView::new(&d, 320, 240, 320).unwrap();
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let robust = scan_robust(&view, &opts, &ScanConfig::default());
        let plain = scan(&view, &opts);
        assert_eq!(robust.codes.len(), plain.codes.len());
        assert_eq!(robust.variants.len(), 1);
        assert_eq!(robust.variants[0].kind, VariantKind::Baseline);
        assert!(!robust.budget_exhausted);
    }

    #[test]
    fn disabled_flags_run_no_variants_even_without_early_exit() {
        let d = flat(320, 240);
        let view = LumaView::new(&d, 320, 240, 320).unwrap();
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let cfg = ScanConfig {
            enable_early_exit: false,
            ..ScanConfig::default()
        };
        let robust = scan_robust(&view, &opts, &cfg);
        assert_eq!(robust.variants.len(), 1, "only the baseline may run");
    }

    #[test]
    fn flat_frame_skips_the_blind_upscale_without_a_deblur_consumer() {
        // A flat frame pools NO finder candidates at any binarization. The
        // whole-frame detection-starved upscale on a zero-finder frame has
        // zero direct recall (diagnosis D1: 1862 ms / 0 codes over 154 real
        // codeless frames) and earns its cost only by feeding the deblur
        // tier — so with deblur OFF (production ROBUST_FAST shape) it must
        // NOT fire. Only the cheap rungs run; shadow/sharpen (pool-gated)
        // and the evidence tiers stay quiet.
        let d = flat(640, 480);
        let view = LumaView::new(&d, 640, 480, 640).unwrap();
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let no_deblur = ScanConfig {
            enable_deblur: false,
            enable_early_exit: false,
            ..ScanConfig::ROBUST_FULL_BENCHMARK
        };
        let robust = scan_robust(&view, &opts, &no_deblur);
        // baseline + pyramid(1: 320x240; second level 160<200 stops) +
        // 3 threshold + sauvola = 6. No BoxWorking (no downscale ⇒ box view
        // IS the source), no shadow/sharpen (empty pool), NO whole-frame
        // 2×/3× (empty pool, deblur off), no deblur.
        assert_eq!(robust.variants.len(), 6, "{:#?}", robust.variants);
        assert!(
            robust.variants.iter().all(|v| !matches!(
                v.kind,
                VariantKind::Upscaled2x
                    | VariantKind::Upscaled3x
                    | VariantKind::ShadowNormalized
                    | VariantKind::Sharpened
            )),
            "a zero-finder deblur-off frame must skip the whole-frame upscale + normalization rungs: {:#?}",
            robust.variants
        );
        assert!(!robust.early_exited);
    }

    #[test]
    fn flat_frame_keeps_the_blind_upscale_when_deblur_can_consume_it() {
        // With deblur ON (the benchmark config), the zero-finder whole-frame
        // upscale still fires — it is the only path that surfaces a coarse
        // candidate for the deblur tier on a detection-starved heavily
        // degraded code (gate3c). The 2× then the 3× tail both run (the
        // frame stays blind after 2× and 640 ≤ the 3× size budget).
        let d = flat(640, 480);
        let view = LumaView::new(&d, 640, 480, 640).unwrap();
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let robust = scan_robust(&view, &opts, &ScanConfig::ROBUST_FULL_BENCHMARK);
        assert!(
            robust
                .variants
                .iter()
                .any(|v| v.kind == VariantKind::Upscaled2x),
            "deblur-on empty-pool frame must run the whole-frame 2× feeder: {:#?}",
            robust.variants
        );
        assert!(!robust.early_exited);
    }

    #[test]
    fn pyramid_depth_gates_on_octave_coverage() {
        let cfg = ScanConfig {
            enable_multi_scale: true,
            enable_early_exit: false,
            ..ScanConfig::default()
        };
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        // 1280-wide working frame: the 0.5x level (640) is under
        // DEEP_LEVEL_MIN_PARENT_DIM, so no code beyond its module ceiling
        // fits — 0.25x must not run.
        let d = flat(1280, 960);
        let view = LumaView::new(&d, 1280, 960, 1280).unwrap();
        let r = scan_robust(&view, &opts, &cfg);
        assert!(r
            .variants
            .iter()
            .any(|v| v.kind == VariantKind::Pyramid { level: 1 }));
        assert!(r
            .variants
            .iter()
            .all(|v| v.kind != VariantKind::Pyramid { level: 2 }));
        // 2880-wide working frame: the 0.5x level (1440) clears the gate —
        // a beyond-ceiling code could fit, so 0.25x runs.
        let d = flat(2880, 2160);
        let view = LumaView::new(&d, 2880, 2160, 2880).unwrap();
        let r = scan_robust(&view, &opts, &cfg);
        assert!(r
            .variants
            .iter()
            .any(|v| v.kind == VariantKind::Pyramid { level: 2 }));
    }

    #[test]
    fn plan_upscale_rois_pads_clamps_and_picks_factor() {
        let mut ev = Evidence::new();
        // Ordinary low-res point: pitch 2 px/module ⇒ factor 2, pad 18 px.
        ev.points.push(EvidencePoint {
            p: [120.0, 100.0],
            covered: false,
            module: 2.0,
            bbox: [100.0, 80.0, 140.0, 120.0],
        });
        // Covered points get no ROI.
        ev.points.push(EvidencePoint {
            p: [30.0, 30.0],
            covered: true,
            module: 2.0,
            bbox: [20.0, 20.0, 40.0, 40.0],
        });
        // Deeply sub-Nyquist point (1.4 < 1.5) ⇒ factor 4.
        ev.points.push(EvidencePoint {
            p: [300.0, 60.0],
            covered: false,
            module: 1.4,
            bbox: [280.0, 40.0, 320.0, 80.0],
        });
        // Sub-pixel-pitch point: the raw padded rect is tiny, but the
        // tile alignment (clamp_roi) floors every window at a 3-tile span,
        // so it still clears MIN_UPSCALE_ROI_DIM at factor 4 and gets an
        // ROI — the pipeline would rather scan 48 px of context than drop
        // an evidence unit.
        ev.points.push(EvidencePoint {
            p: [10.0, 200.0],
            covered: false,
            module: 0.2,
            bbox: [8.0, 198.0, 12.0, 202.0],
        });
        let data = vec![0u8; 400 * 300];
        let src = LumaView::new(&data, 400, 300, 400).unwrap();
        let rois = plan_upscale_rois(&ev, &src);
        assert_eq!(rois.len(), 3);
        assert!(matches!(rois[2].target, RoiTarget::Evidence(3)));
        assert_eq!(rois[2].factor, 4);
        // pad = 9 modules * 2 px = 18 px around the bbox ⇒ (82,62)-(158,138),
        // then snapped outward to the 16-px tile grid + 1 tile ring
        // (see clamp_roi): (82,62) → (64,32); (158,138) → (176,160).
        assert_eq!(
            (rois[0].x0, rois[0].y0, rois[0].w, rois[0].h),
            (64, 32, 112, 128)
        );
        assert_eq!(rois[0].factor, 2);
        assert!(matches!(rois[0].target, RoiTarget::Evidence(0)));
        // pad = 9 * 1.4 = 12.6 px ⇒ (267,27)-(333,93), tile-aligned to
        // (240,0)-(352,112).
        assert_eq!(
            (rois[1].x0, rois[1].y0, rois[1].w, rois[1].h),
            (240, 0, 112, 112)
        );
        assert_eq!(rois[1].factor, 4);
        assert!(matches!(rois[1].target, RoiTarget::Evidence(2)));
    }

    #[test]
    fn upscale_factor_is_pitch_derived_and_resolution_independent() {
        // ≥3.5 px/module: the source already carries decode-grade
        // resolution — pure rescan.
        assert_eq!(upscale_factor_for_pitch(3.5), 1);
        assert_eq!(upscale_factor_for_pitch(4.7), 1);
        // [1.75, 3.5): 2x lifts it over the ~3.5 px/module decode floor.
        assert_eq!(upscale_factor_for_pitch(3.49), 2);
        assert_eq!(upscale_factor_for_pitch(1.75), 2);
        // [1.5, 1.75): 3x; deeply sub-Nyquist tail: 4x.
        assert_eq!(upscale_factor_for_pitch(1.5), 3);
        assert_eq!(upscale_factor_for_pitch(1.4), 4);
    }

    #[test]
    fn plan_pair_rois_selects_coherent_pairs_smallest_separation_first() {
        let data = vec![0u8; 800 * 600];
        let src = LumaView::new(&data, 800, 600, 800).unwrap();
        let evidence = Evidence::new();
        let codes: Vec<RobustCode> = Vec::new();
        let pf = |x: f64, y: f64, module: f64, inverted: bool| PooledFinder {
            x,
            y,
            module,
            inverted,
            hits: 3,
            origin: VariantKind::Baseline,
        };
        let pool = vec![
            // A plausible v1-ish pair at 4 px/module: separation 72 px =
            // 18 modules ∈ [7, 170√2].
            pf(100.0, 100.0, 4.0, false),
            pf(172.0, 100.0, 4.0, false),
            // Same geometry but opposite polarity: must not pair with the
            // two above.
            pf(400.0, 100.0, 4.0, true),
            // Too close to its would-be partner (8 px = 2 modules < 7):
            // duplicate-class, not a pair.
            pf(400.0, 108.0, 4.0, true),
            // Module ratio 2.0 > 1.5 vs everything at 4.0: never pairs.
            pf(600.0, 300.0, 8.0, false),
        ];
        let rois = plan_pair_rois(&pool, &evidence, &codes, &src);
        assert_eq!(rois.len(), 1, "exactly the one coherent pair");
        let roi = &rois[0];
        // Midpoint (136, 100); side = 1.6*72 + 2*9*4 = 187.2 → half 93.6
        // ⇒ (42,6)-(230,194), tile-aligned to (16,0)-(256,224).
        assert!(matches!(roi.target, RoiTarget::Point(p) if p == [136.0, 100.0]));
        assert_eq!((roi.x0, roi.y0), (16, 0));
        // 4 px/module ≥ 3.5 ⇒ factor 1 (pure rescan).
        assert_eq!(roi.factor, 1);
    }

    #[test]
    fn plan_pair_rois_skips_finders_consumed_by_evidence_or_codes() {
        let data = vec![0u8; 800 * 600];
        let src = LumaView::new(&data, 800, 600, 800).unwrap();
        let pf = |x: f64, y: f64| PooledFinder {
            x,
            y,
            module: 4.0,
            inverted: false,
            hits: 3,
            origin: VariantKind::Baseline,
        };
        let pool = vec![pf(100.0, 100.0), pf(172.0, 100.0)];
        // An evidence triplet whose bbox contains the first finder: the
        // pair hypothesis is redundant with the (stronger) triplet unit.
        let mut evidence = Evidence::new();
        evidence.points.push(EvidencePoint {
            p: [110.0, 110.0],
            covered: false,
            module: 4.0,
            bbox: [90.0, 90.0, 130.0, 130.0],
        });
        let rois = plan_pair_rois(&pool, &evidence, &[], &src);
        assert!(rois.is_empty(), "member consumed by triplet evidence");
    }

    #[test]
    fn plan_upscale_rois_clamps_to_frame_and_caps_count() {
        let mut ev = Evidence::new();
        // Point hugging the frame origin: padded rect clamps to (0,0).
        ev.points.push(EvidencePoint {
            p: [10.0, 10.0],
            covered: false,
            module: 3.0,
            bbox: [4.0, 4.0, 40.0, 40.0],
        });
        // 5 more uncovered points; only MAX_UPSCALE_ROIS total survive.
        for i in 0..5 {
            let x = 120.0 + 100.0 * i as f64;
            ev.points.push(EvidencePoint {
                p: [x, 150.0],
                covered: false,
                module: 3.0,
                bbox: [x - 20.0, 130.0, x + 20.0, 170.0],
            });
        }
        let data = vec![0u8; 700 * 300];
        let src = LumaView::new(&data, 700, 300, 700).unwrap();
        let rois = plan_upscale_rois(&ev, &src);
        assert_eq!(rois.len(), MAX_UPSCALE_ROIS);
        assert_eq!((rois[0].x0, rois[0].y0), (0, 0));
        // ceil(40 + 27) = 67, tile-aligned outward to 96 (6 tiles).
        assert_eq!(rois[0].w, 96);
    }

    /// The delicate E5a seam: a detection made on an UPSCALED SOURCE ROI
    /// must land back on the true source-px geometry through
    /// [`VariantMap`]'s offset+scale (and its [`SourceView`] must sample
    /// the ROI sub-view correctly, or the decode itself fails).
    fn roi_mapping_case(factor: u8) {
        let payload = b"ROI-MAP-TEST";
        let code =
            qrcode::QrCode::with_version(payload, qrcode::Version::Normal(2), qrcode::EcLevel::M)
                .unwrap();
        let dim = code.width(); // 25
        let module_px = 4.0;
        let size = dim as f64 * module_px; // 100 px
        let (ox, oy) = (317.0, 203.0); // deliberately non-round vs ROI origin
        let quad = [
            [ox, oy],
            [ox + size, oy],
            [ox + size, oy + size],
            [ox, oy + size],
        ];
        let unit = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let t = crate::homography::PerspectiveTransform::quad_to_quad(unit, quad).unwrap();
        let (img_w, img_h) = (640usize, 480usize);
        let img = crate::testpaint::render_module_grid_transformed_antialiased(
            dim,
            |x, y| code[(x, y)] == qrcode::Color::Dark,
            20,
            235,
            &t,
            img_w,
            img_h,
            4,
        );
        let src = LumaView::new(&img, img_w, img_h, img_w).unwrap();

        // ROI covering the code plus margin, origin ≠ (0,0).
        let (rx, ry, rw, rh) = (300usize, 190usize, 140usize, 130usize);
        let roi_view = src.sub_view(rx, ry, rw, rh).unwrap();
        // Factor 1 = pure rescan: detection runs on the crop itself.
        let up_owned: Option<(Vec<u8>, usize, usize)> = match factor {
            1 => None,
            4 => Some(bilinear_upscale_4x(&roi_view)),
            3 => Some(bilinear_upscale_3x(&roi_view)),
            _ => Some(bilinear_upscale_2x(&roi_view)),
        };
        let uview: LumaView = match &up_owned {
            Some((buf, uw, uh)) => LumaView::new(buf, *uw, *uh, *uw).unwrap(),
            None => roi_view,
        };
        let f = factor as f64;

        let mut l = Ladder {
            out: RobustDetections {
                codes: Vec::new(),
                variants: Vec::new(),
                early_exited: false,
                budget_exhausted: false,
                total_ns: 0,
                triplet_evidence: Vec::new(),
                finders: Vec::new(),
                triplets: Vec::new(),
            },
            evidence: Evidence::new(),
            pool: Vec::new(),
            seed_pool: Vec::new(),
            attempted_triples: Vec::new(),
            grouped_len: 0,
        };
        run_variant(
            &mut l,
            &uview,
            Some(SourceView {
                view: &roi_view,
                sx: f,
                sy: f,
            }),
            VariantMap {
                sx: f,
                sy: f,
                ox: rx as f64,
                oy: ry as f64,
            },
            true,
            BinarizeSpec::default(),
            VariantKind::UpscaledRoi { factor },
            false,
            0,
            None,
        );

        let (out, evidence) = (&l.out, &l.evidence);
        assert_eq!(out.codes.len(), 1, "ROI recovery x{factor} must decode");
        let c = &out.codes[0];
        assert_eq!(c.code.payload.as_bytes(), payload);
        assert_eq!(c.variant, VariantKind::UpscaledRoi { factor });
        // Geometry mapped back to SOURCE px: compare center + mean edge
        // (corner ordering is the decoder's own convention).
        let center = quad_center(&c.corners_source);
        let truth_center = quad_center(&quad);
        let dx = center[0] - truth_center[0];
        let dy = center[1] - truth_center[1];
        assert!(
            (dx * dx + dy * dy).sqrt() < 2.0,
            "center off by ({dx:.2}, {dy:.2}) source px"
        );
        let edge = quad_mean_edge(&c.corners_source);
        assert!(
            (edge - size).abs() / size < 0.05,
            "mean edge {edge:.1} vs truth {size:.1}"
        );
        // Evidence bookkeeping is in source px too.
        assert!(evidence
            .points
            .iter()
            .all(|e| e.p[0] > ox && e.p[0] < ox + size && e.p[1] > oy && e.p[1] < oy + size));
    }

    #[test]
    fn roi_rescan_factor1_maps_detections_back_to_source_px() {
        roi_mapping_case(1);
    }

    #[test]
    fn roi_upscale_2x_maps_detections_back_to_source_px() {
        roi_mapping_case(2);
    }

    #[test]
    fn roi_upscale_3x_maps_detections_back_to_source_px() {
        roi_mapping_case(3);
    }

    #[test]
    fn roi_upscale_4x_maps_detections_back_to_source_px() {
        roi_mapping_case(4);
    }

    #[test]
    fn variant_budget_truncates_the_ladder() {
        let d = flat(640, 480);
        let view = LumaView::new(&d, 640, 480, 640).unwrap();
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let cfg = ScanConfig {
            max_variants_per_frame: 2,
            enable_early_exit: false,
            ..ScanConfig::ROBUST_FULL_BENCHMARK
        };
        let robust = scan_robust(&view, &opts, &cfg);
        assert_eq!(robust.variants.len(), 3, "baseline + 2 budgeted variants");
        assert!(robust.budget_exhausted);
    }

    #[test]
    fn rotation_set_period_one_is_all() {
        for f in 0..7 {
            let r = RotationSet::for_frame(1, f);
            assert!(
                r.substrate && r.threshold && r.sauvola,
                "period 1 = full ladder"
            );
        }
        // Period 0 is treated as no-rotation too (guard).
        let r = RotationSet::for_frame(0, 5);
        assert!(r.substrate && r.threshold && r.sauvola);
    }

    #[test]
    fn rotation_set_round_robin_covers_each_group_once_per_period() {
        // Over one period of 3, each group runs on exactly one frame.
        let (mut sub, mut thr, mut sau) = (0, 0, 0);
        for f in 0..3u64 {
            let r = RotationSet::for_frame(3, f);
            sub += r.substrate as u32;
            thr += r.threshold as u32;
            sau += r.sauvola as u32;
            // Exactly one group per frame at period 3.
            assert_eq!(
                r.substrate as u32 + r.threshold as u32 + r.sauvola as u32,
                1
            );
        }
        assert_eq!(
            (sub, thr, sau),
            (1, 1, 1),
            "each group runs once per period"
        );
        // The schedule is periodic in `period`.
        assert_eq!(
            RotationSet::for_frame(3, 0).substrate,
            RotationSet::for_frame(3, 3).substrate
        );
    }

    #[test]
    fn session_frame_zero_period_one_matches_scan_robust() {
        // A fresh session (empty recent) on frame 0 with rotation disabled
        // must run exactly the same rungs as the single-frame entry point:
        // FrameControl::full's contract, exercised end-to-end.
        let d = flat(640, 480);
        let view = LumaView::new(&d, 640, 480, 640).unwrap();
        let opts = ScanOptions {
            max_working_dim: 0,
            refine: false,
        };
        let cfg = ScanConfig {
            enable_early_exit: false,
            ..ScanConfig::ROBUST_FULL_BENCHMARK
        };
        let plain = scan_robust(&view, &opts, &cfg);
        let mut session = ScanSession::new(
            cfg,
            SessionConfig {
                rotation_period: 1,
                pool_ttl_frames: 4,
            },
        );
        let sess = session.scan_frame(&view, &opts);
        let kinds_plain: Vec<_> = plain.variants.iter().map(|v| v.kind).collect();
        let kinds_sess: Vec<_> = sess.variants.iter().map(|v| v.kind).collect();
        assert_eq!(
            kinds_sess, kinds_plain,
            "period-1 frame-0 session == scan_robust rungs"
        );
        assert_eq!(session.frame_index(), 1);
    }

    #[test]
    fn session_rotation_runs_fewer_rungs_per_frame() {
        // On a flat frame the pool stays empty, so no cross-frame seeds
        // accumulate; each rotated frame runs baseline + only its slot's
        // group. Concretely: frame 0 (substrate slot) with a flat 960px
        // frame runs baseline + box/pyramid; frame 1 (threshold slot) runs
        // baseline + the 3 threshold variants; frame 2 (sauvola slot) runs
        // baseline + Sauvola. None runs the whole cheap batch at once.
        let d = flat(960, 720);
        let view = LumaView::new(&d, 960, 720, 960).unwrap();
        let opts = ScanOptions {
            max_working_dim: 640,
            refine: false,
        };
        let cfg = ScanConfig {
            enable_early_exit: false,
            enable_deblur: false,
            ..ScanConfig::ROBUST_FULL_BENCHMARK
        };
        let mut session = ScanSession::new(cfg, SessionConfig::default());
        let full = {
            let mut s = ScanSession::new(
                ScanConfig { ..cfg },
                SessionConfig {
                    rotation_period: 1,
                    pool_ttl_frames: 4,
                },
            );
            s.scan_frame(&view, &opts).variants.len()
        };
        let f0 = session.scan_frame(&view, &opts).variants.len();
        let f1 = session.scan_frame(&view, &opts).variants.len();
        let f2 = session.scan_frame(&view, &opts).variants.len();
        assert!(
            f0 < full && f1 < full && f2 < full,
            "rotated frames ({f0},{f1},{f2}) must each run fewer rungs than the full ladder ({full})"
        );
    }
}
