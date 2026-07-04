// Canvas-render a generated QR bit matrix (Plan 5 Task 5) into a texture
// for the 3D scene's plane: dark/light modules plus the quiet-zone
// border, sharp (unantialiased) edges — the plane's material sets
// `magFilter = NearestFilter` (and, here, `minFilter` too — see
// `makeQrTexture`'s doc) so those crisp module edges survive both
// magnification (orbiting close) and minification (orbiting far) without
// blurring or mipmap moiré.
//
// `bitAt`/`wordsPerRow` mirror `overlays/layers/bits.ts`'s helpers of the
// same name exactly (same packed format, `qrk_core::trace::BitsTrace`) —
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
import { DEFAULT_TEXTURE_PX_PER_MODULE, QUIET_MODULES } from "./consts";

function wordsPerRow(dim: number): number {
  return Math.max(1, Math.ceil(dim / 32));
}

function bitAt(words: number[], dim: number, x: number, y: number): boolean {
  const word = words[y * wordsPerRow(dim) + Math.floor(x / 32)] ?? 0;
  return ((word >>> (x % 32)) & 1) !== 0;
}

/**
 * Render `qr` into a fresh `HTMLCanvasElement`: a white background (quiet
 * zone + light modules) with black squares for every dark module, inset
 * by `quiet` modules of margin on every side. `pxPerModule` sets the
 * texture's resolution (texture side = `(qr.dim + 2*quiet) *
 * pxPerModule` px).
 */
export function renderQrCanvas(
  qr: GeneratedQr,
  quiet: number = QUIET_MODULES,
  pxPerModule: number = DEFAULT_TEXTURE_PX_PER_MODULE,
): HTMLCanvasElement {
  const total = qr.dim + 2 * quiet;
  const side = total * pxPerModule;
  const canvas = document.createElement("canvas");
  canvas.width = side;
  canvas.height = side;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("renderQrCanvas: 2d context unavailable");

  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, side, side);
  ctx.fillStyle = "#000000";
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
): CanvasTexture {
  const canvas = renderQrCanvas(qr, quiet, pxPerModule);
  const texture = new CanvasTexture(canvas);
  texture.magFilter = NearestFilter;
  texture.minFilter = LinearMipMapLinearFilter;
  texture.generateMipmaps = true;
  texture.needsUpdate = true;
  return texture;
}
