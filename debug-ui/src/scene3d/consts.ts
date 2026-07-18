// Shared constants for the 3D orbit debug scene (Plan 5 Task 5) — the
// debug UI's headline feature: an orbitable plane textured with a real,
// decodable QR code, whose module-region corners are known analytically
// (via the plane's own transform + the three.js camera) and compared each
// frame against `qr_lab_core::scan`'s live `refined_corners`.

/** Quiet-zone width in modules, matching the Rust pipeline's own
 * convention throughout `qr-lab-core` (`decode.rs`/`sample.rs`/`alignment.rs`/
 * `version.rs`'s `axis_aligned_quad`/`axis_aligned_transform` test
 * helpers, and `tools/fixtures/render.py`'s `QUIET_MODULES`) — the ISO/IEC
 * 18004 standard quiet zone is 4 modules on every side. */
export const QUIET_MODULES = 4;

/** Default plane physical size (meters) — per the task brief, "a plane
 * (configurable physical size, default 0.15m)". A 15cm code is a
 * plausible printed-QR size (roughly a business-card-to-A5 range) that
 * keeps default orbit distances in the 0.2-1m range human-legible. */
export const DEFAULT_PHYSICAL_SIZE_M = 0.15;

/** Default per-module texture resolution (px) — sharp edges (the caller
 * sets `magFilter = NearestFilter`), high enough that even a v40 (177
 * modules/side) code renders with a few px/module at reasonable orbit
 * distances. */
export const DEFAULT_TEXTURE_PX_PER_MODULE = 8;

/** Render-resolution knob options (readback buffer size, px, square) —
 * "camera resolution" per the task brief. */
export const RENDER_RESOLUTIONS = [640, 960, 1280] as const;
export type RenderResolution = (typeof RENDER_RESOLUTIONS)[number];
export const DEFAULT_RENDER_RESOLUTION: RenderResolution = 960;

/** Scan-loop throttle: at most one scan kicked off every this many ms,
 * absorbing bursts of orbit-drag frames into "latest-wins" (the
 * `ScannerClient` already does this at the request level; this throttle
 * additionally avoids doing a full GPU readback + postprocess on every
 * single animation frame when nothing warrants it). */
export const SCAN_THROTTLE_MS = 100;

/** Camera-sim knob ranges (all documented approximations — see
 * `camSim.ts`'s module doc). */
export const BLUR_SIGMA_RANGE = { min: 0, max: 3, step: 0.1, default: 0 } as const;
export const NOISE_SIGMA_RANGE = { min: 0, max: 8, step: 0.5, default: 0 } as const;
export const EXPOSURE_OFFSET_RANGE = { min: -60, max: 60, step: 1, default: 0 } as const;

export const DEFAULT_PAYLOAD = "HTTPS://AUKILABS.COM/CPUSCANNER2/SCENE3D";
/** `0` = auto-select the smallest version that fits the payload — see
 * `qr-lab-wasm`'s `generate_qr` doc. */
export const DEFAULT_VERSION = 0;
/** `1` = ECC level M — matches `generate_qr`'s `0..=3` -> L/M/Q/H mapping. */
export const DEFAULT_ECC = 1;

/** Plan 5d: QR appearance knobs. `inkColor` paints dark modules;
 * `bgColor`/`bgAlpha` paint the quiet zone + light modules (the "paper").
 * Defaults reproduce the pre-5d look exactly: opaque black-on-white. */
export const DEFAULT_QR_INK_COLOR = "#000000";
export const DEFAULT_QR_BG_COLOR = "#ffffff";
export const DEFAULT_QR_BG_ALPHA = 1;
export const QR_BG_ALPHA_RANGE = { min: 0, max: 1, step: 0.05 } as const;

/** `expectedInverted`'s contrast warning fires below this |Δluma| (0-255
 * scale). Set with headroom above `qr_lab_core::consts::CONTRAST_FLOOR` (12,
 * a per-TILE contrast floor the Rust detector actually enforces) — this
 * warning is a coarse whole-color heads-up for a human picking colors in
 * the UI, not a re-derivation of the tile-level floor, so it fires well
 * before real detection risk to leave headroom for blur/noise/exposure to
 * further erode contrast on top of the base color choice. */
export const CONTRAST_WARN_THRESHOLD = 30;

/** The scene's own background color (the `<color attach="background">`
 * value in `Scene3D.tsx`) — single source of truth shared between the
 * Canvas and the "reads as" indicator's alpha compositing
 * (`colorUtils.ts`'s `expectedInvertedComposited` needs to know what a
 * translucent QR paper composites over when no background image is set). */
export const SCENE_BACKGROUND_COLOR = "#05070d";

/** Plan 5d: scene-background plane. Sized as a multiple of the QR plane's
 * own `physicalSize` (a "floor" quad behind/coplanar-under the QR) so it
 * reads as an environment the code floats in front of rather than a tight
 * frame around it. */
export const BACKGROUND_PLANE_SCALE = 4;
/** Slightly behind the QR plane (same local Z axis) to avoid z-fighting
 * between the two coplanar quads — small relative to every plausible
 * `physicalSize`/orbit-distance combination. */
export const BACKGROUND_PLANE_Z_OFFSET = -0.001;

/** Default fixture-export seed — the scene has no camera-sim noise RNG
 * seed of its own to report (see `camSim.ts`'s per-tick seed advance,
 * which isn't a single fixed value); `0` matches `tools/fixtures/
 * generate.py`'s CLI default and is honest about "this wasn't captured
 * from a specific noise draw" for a manually-triggered save. */
export const FIXTURE_EXPORT_SEED = 0;
