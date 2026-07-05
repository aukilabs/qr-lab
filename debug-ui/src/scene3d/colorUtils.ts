// Pure color math for the 3D scene's QR-appearance controls (Plan 5d):
// hex parsing + the scanner's own luma formula (same 77/150/29 fixed-point
// coefficients as `media/luma.ts`'s `lumaAt`/`lumaBufferFromRgba`, in turn
// mirroring `qrk_core::luma_from_rgba`) applied to the two flat colors a
// user picks (ink, background) rather than a sampled pixel — so
// `expectedInverted` reasons about the SAME luma the scanner's binarizer
// would derive from those colors once rendered.
import { CONTRAST_WARN_THRESHOLD } from "./consts";

/** Parse a `#rrggbb` (or `#rgb` shorthand) hex color string into 0-255
 * integer channels. Throws on anything else — callers are `<input
 * type="color">` values (always `#rrggbb`) or this module's own
 * `rgbToHex` output, never arbitrary user text. */
export function hexToRgb(hex: string): [number, number, number] {
  const m = /^#?([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) throw new RangeError(`hexToRgb: not a #rgb/#rrggbb color: ${JSON.stringify(hex)}`);
  const body = m[1]!;
  const full = body.length === 3 ? body.split("").map((c) => c + c).join("") : body;
  const r = parseInt(full.slice(0, 2), 16);
  const g = parseInt(full.slice(2, 4), 16);
  const b = parseInt(full.slice(4, 6), 16);
  return [r, g, b];
}

function toHex2(v: number): string {
  return Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0");
}

/** Inverse of {@link hexToRgb}: 0-255 channels -> `#rrggbb`. */
export function rgbToHex(r: number, g: number, b: number): string {
  return `#${toHex2(r)}${toHex2(g)}${toHex2(b)}`;
}

/**
 * `qrk_core::luma_from_rgba`'s exact per-pixel formula (BT.601-ish
 * fixed-point coefficients, 77/150/29 over 256, rounded), applied to a
 * single flat color rather than a sampled buffer pixel — the same
 * arithmetic as `media/luma.ts`'s `lumaAt`/`lumaBufferFromRgba`, kept as an
 * independent small copy here (one integer expression) rather than an
 * import across `media/` <-> `scene3d/`, matching this codebase's existing
 * precedent for tiny duplicated formula copies over cross-module coupling
 * (see `qrTexture.ts`'s doc on `bitAt`/`wordsPerRow`).
 */
export function lumaFromRgb(r: number, g: number, b: number): number {
  return (77 * r + 150 * g + 29 * b + 128) >> 8;
}

export interface ExpectedInverted {
  /** `true` when the ink color is BRIGHTER (higher luma) than the
   * background color — i.e. what the scanner's luma-based binarizer would
   * read as "dark modules" is actually the light-colored background, the
   * same visual polarity `tools/fixtures/render.py`'s `inverted=True`
   * (light modules on a dark plate) describes. */
  inverted: boolean;
  /** `|luma(ink) - luma(bg)|`, 0-255. */
  deltaLuma: number;
  /** `true` when `deltaLuma` is below {@link CONTRAST_WARN_THRESHOLD} —
   * contrast this low risks failing the Rust detector's per-tile
   * `CONTRAST_FLOOR` (12) once blur/noise/exposure further erode it. */
  lowContrast: boolean;
}

/**
 * Derive whether a chosen `(inkColor, bgColor)` pair reads as a normal
 * (dark-ink-on-light-background) or inverted (light-ink-on-dark-background)
 * QR code, purely from the two colors' luma — the "inverted" status is
 * EMERGENT from color choice, not a separate flag, since the scanner
 * binarizes by local contrast rather than an absolute polarity. Also flags
 * low-contrast pairs (see {@link ExpectedInverted.lowContrast}).
 */
export function expectedInverted(inkColor: string, bgColor: string): ExpectedInverted {
  const [ir, ig, ib] = hexToRgb(inkColor);
  const [br, bg, bb] = hexToRgb(bgColor);
  const lumaInk = lumaFromRgb(ir, ig, ib);
  const lumaBg = lumaFromRgb(br, bg, bb);
  const deltaLuma = Math.abs(lumaInk - lumaBg);
  return {
    inverted: lumaInk > lumaBg,
    deltaLuma,
    lowContrast: deltaLuma < CONTRAST_WARN_THRESHOLD,
  };
}
