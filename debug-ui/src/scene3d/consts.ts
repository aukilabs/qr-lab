// Shared constants for the 3D orbit debug scene (Plan 5 Task 5) — the
// debug UI's headline feature: an orbitable plane textured with a real,
// decodable QR code, whose module-region corners are known analytically
// (via the plane's own transform + the three.js camera) and compared each
// frame against `qrk_core::scan`'s live `refined_corners`.

/** Quiet-zone width in modules, matching the Rust pipeline's own
 * convention throughout `qrk-core` (`decode.rs`/`sample.rs`/`alignment.rs`/
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
 * `qrk-wasm`'s `generate_qr` doc. */
export const DEFAULT_VERSION = 0;
/** `1` = ECC level M — matches `generate_qr`'s `0..=3` -> L/M/Q/H mapping. */
export const DEFAULT_ECC = 1;
