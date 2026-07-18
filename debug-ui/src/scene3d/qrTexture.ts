// Canvas-render a generated QR bit matrix (Plan 5 Task 5) into a texture
// for the 3D scene's plane: dark/light modules plus the quiet-zone
// border, sharp (unantialiased) edges — the plane's material sets
// `magFilter = NearestFilter` (and, here, `minFilter` too — see
// `makeQrTexture`'s doc) so those crisp module edges survive both
// magnification (orbiting close) and minification (orbiting far) without
// blurring or mipmap moiré.
//
// `bitAt`/`wordsPerRow` mirror `overlays/layers/bits.ts`'s helpers of the
// same name exactly (same packed format, `qr_lab_core::trace::BitsTrace`) —
// duplicated rather than imported, matching this codebase's established
// precedent for small, self-contained copies across independently
// evolving contracts (see `groundtruth-types.ts`'s doc comment on exactly
// this tradeoff).
//
// Untested: canvas rendering needs a real `CanvasRenderingContext2D`,
// unavailable in vitest's `node` environment (same reasoning as
// `camSim.ts`'s `applyGaussianBlurCanvas`) — visual correctness is a
// manual-QA concern (Task 7).
import { CanvasTexture, LinearMipMapLinearFilter, NearestFilter } from "three";
import type { GeneratedQr } from "../scanner/qrgen";
import {
  DEFAULT_QR_BG_ALPHA,
  DEFAULT_QR_BG_COLOR,
  DEFAULT_QR_INK_COLOR,
  DEFAULT_TEXTURE_PX_PER_MODULE,
  QUIET_MODULES,
} from "./consts";
import { hexToRgb } from "./colorUtils";

function wordsPerRow(dim: number): number {
  return Math.max(1, Math.ceil(dim / 32));
}

function bitAt(words: number[], dim: number, x: number, y: number): boolean {
  const word = words[y * wordsPerRow(dim) + Math.floor(x / 32)] ?? 0;
  return ((word >>> (x % 32)) & 1) !== 0;
}

/** {@link renderQrCanvas}'s color/alpha knobs (Plan 5d) — `bgAlpha` only
 * affects the "paper" fill (quiet zone + light modules); ink is always
 * painted fully opaque regardless, matching `tools/fixtures/render.py`'s
 * `opaque_plate=False` mode ("constant ink color; the alpha mask alone
 * carries the module edges") — the point of the alpha slider is to let
 * the SCENE BACKGROUND show through the paper, not to fade the code's own
 * ink. */
export interface QrColorOptions {
  inkColor: string;
  bgColor: string;
  bgAlpha: number;
}

export const DEFAULT_QR_COLOR_OPTIONS: QrColorOptions = {
  inkColor: DEFAULT_QR_INK_COLOR,
  bgColor: DEFAULT_QR_BG_COLOR,
  bgAlpha: DEFAULT_QR_BG_ALPHA,
};

/**
 * Render `qr` into a fresh `HTMLCanvasElement`: a `colors.bgColor` "paper"
 * fill (quiet zone + light modules, at `colors.bgAlpha` opacity — `<1`
 * makes it partially/fully transparent, letting the scene background
 * plane show through) with `colors.inkColor` squares (always fully
 * opaque) for every dark module, inset by `quiet` modules of margin on
 * every side. `pxPerModule` sets the texture's resolution (texture side =
 * `(qr.dim + 2*quiet) * pxPerModule` px).
 */
export function renderQrCanvas(
  qr: GeneratedQr,
  quiet: number = QUIET_MODULES,
  pxPerModule: number = DEFAULT_TEXTURE_PX_PER_MODULE,
  colors: QrColorOptions = DEFAULT_QR_COLOR_OPTIONS,
): HTMLCanvasElement {
  const total = qr.dim + 2 * quiet;
  const side = total * pxPerModule;
  const canvas = document.createElement("canvas");
  canvas.width = side;
  canvas.height = side;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("renderQrCanvas: 2d context unavailable");

  const [bgR, bgG, bgB] = hexToRgb(colors.bgColor);
  ctx.clearRect(0, 0, side, side);
  ctx.fillStyle = `rgba(${bgR}, ${bgG}, ${bgB}, ${colors.bgAlpha})`;
  ctx.fillRect(0, 0, side, side);
  ctx.fillStyle = colors.inkColor;
  for (let y = 0; y < qr.dim; y++) {
    for (let x = 0; x < qr.dim; x++) {
      if (!bitAt(qr.words, qr.dim, x, y)) continue;
      ctx.fillRect((x + quiet) * pxPerModule, (y + quiet) * pxPerModule, pxPerModule, pxPerModule);
    }
  }
  return canvas;
}

/**
 * {@link renderQrCanvas} wrapped in a `THREE.CanvasTexture`: `NearestFilter`
 * for magnification (per the task brief — orbiting close keeps hard,
 * unblurred module edges rather than bilinear-smoothed ones) but a
 * MIPMAPPED linear filter (`LinearMipMapLinearFilter`, three's own
 * default minFilter, restored explicitly here for clarity) for
 * minification. A first attempt used `NearestFilter` + no mipmaps for
 * BOTH directions (maximally "sharp" in principle), but that broke real
 * decoding: under the scene's default keystone, the far half of the
 * plane minifies the texture — nearest-neighbor sampling with no mipmap
 * then aliases (each screen pixel samples exactly one texel from a whole
 * texel neighborhood, ignoring the rest), which corrupts enough modules
 * to fail ECC. Mipmapped linear minification correctly averages that
 * neighborhood instead, fixing decode; needs `generateMipmaps = true`
 * (the default, set explicitly here since we DO override other texture
 * properties) and a power-of-two-friendly canvas size isn't required for
 * WebGL2 (this project targets modern browsers only).
 */
export function makeQrTexture(
  qr: GeneratedQr,
  quiet: number = QUIET_MODULES,
  pxPerModule: number = DEFAULT_TEXTURE_PX_PER_MODULE,
  colors: QrColorOptions = DEFAULT_QR_COLOR_OPTIONS,
): CanvasTexture {
  const canvas = renderQrCanvas(qr, quiet, pxPerModule, colors);
  const texture = new CanvasTexture(canvas);
  texture.magFilter = NearestFilter;
  texture.minFilter = LinearMipMapLinearFilter;
  texture.generateMipmaps = true;
  texture.needsUpdate = true;
  return texture;
}
