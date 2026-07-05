// Sampled-bits overlay (Plan 4 Task 6): a semi-transparent fill over every
// DARK module of a decoded candidate's sampled bit matrix (packed row-major
// `u32` words — see the Rust `trace::BitsTrace` doc).
//
// WHICH CODE(S) (Plan 5C: multi-code trace): `scan.trace.codes` carries one
// entry PER DECODED code this frame, each pairing a `bits` matrix with the
// `code_index` into `scan.detections.codes` its homography corners come
// from (see `qrk_core::trace::Trace::codes`'s doc) — this layer fills EVERY
// entry's modules, so a multi-code scene (e.g. `multi_07`'s 4 codes) shows
// all of them, not just one. Only when NOTHING decoded this frame
// (`trace.codes` empty) does it fall back to the legacy singular
// `scan.trace.bits` field paired with `scan.detections.codes`' own LAST
// entry — that pairing is failure-diagnosis-only in practice (`bits` is
// `null` whenever nothing decoded — see `Trace`'s doc), so this fallback
// only ever draws in the pre-Plan-5C "there is exactly one decoded code"
// case, kept for zero behavior change there. Pre-Plan-5C this "last code"
// pairing was the layer's ONLY option — a multi-code scene's `bits` always
// corresponded to just `scan.detections.codes`' LAST entry, a known
// limitation this task's `trace.codes` addition fixes.
//
// Module fill quads come from mapping each unit-square module cell through
// the TS homography port (`squareToQuad`) built from the code's own
// `corners` — the same "reconstruct the grid from a homography, not
// per-module trace points" approach the plan's trace-compactness
// constraint calls for.
//
// Zoom-gated PER CODE: a code is skipped entirely when its module would
// render smaller than `MIN_MODULE_SCREEN_PX` screen px (`view.scale *
// module_px < 4`) — v40's ~31k modules would otherwise flood the canvas
// with sub-pixel fills for no visible benefit. "module_px" here is the
// code's own top-edge (TL-TR) pixel length divided by its `bits.dim`, i.e.
// one module's on-screen width in working-res image px before the view's
// own zoom scale is applied — a multi-code scene can have codes at very
// different apparent sizes, so each one is gated independently rather than
// a single frame-wide check.
import { imageToScreen } from "../../viewport/transform";
import type { ViewTransform } from "../../viewport/transform";
import { mapPoint, squareToQuad, type Point2 } from "../homography";
import type { BitsTrace, DecodedCode } from "../../scanner/types";
import type { OverlayLayer } from "../registry";

const FILL_COLOR = "rgba(0, 0, 0, 0.55)";
const MIN_MODULE_SCREEN_PX = 4;

function wordsPerRow(dim: number): number {
  return Math.max(1, Math.ceil(dim / 32));
}

/** `true` (dark) iff module `(x, y)` is set in `words` — mirrors Rust
 * `BitMatrix::get`'s packing exactly (row-major, `ceil(dim/32)` `u32` words
 * per row, bit `x % 32` of word `x / 32`). */
function bitAt(words: number[], dim: number, x: number, y: number): boolean {
  const word = words[y * wordsPerRow(dim) + Math.floor(x / 32)] ?? 0;
  return ((word >>> (x % 32)) & 1) !== 0;
}

/** Fill one code's dark modules, applying this code's own zoom gate. */
function drawBits(
  ctx: CanvasRenderingContext2D,
  view: ViewTransform,
  bits: BitsTrace,
  code: DecodedCode,
): void {
  const [tl, tr] = code.corners;
  const quadWidthPx = Math.hypot(tr[0] - tl[0], tr[1] - tl[1]);
  const modulePx = quadWidthPx / bits.dim;
  if (view.scale * modulePx < MIN_MODULE_SCREEN_PX) return;

  const h = squareToQuad(code.corners);
  if (!h) return;

  ctx.fillStyle = FILL_COLOR;
  const dimf = bits.dim;
  for (let y = 0; y < bits.dim; y++) {
    for (let x = 0; x < bits.dim; x++) {
      if (!bitAt(bits.words, bits.dim, x, y)) continue;
      const corners: Point2[] = [
        mapPoint(h, x / dimf, y / dimf),
        mapPoint(h, (x + 1) / dimf, y / dimf),
        mapPoint(h, (x + 1) / dimf, (y + 1) / dimf),
        mapPoint(h, x / dimf, (y + 1) / dimf),
      ].map((p) => imageToScreen(view, p));
      ctx.beginPath();
      ctx.moveTo(corners[0]![0], corners[0]![1]);
      for (let i = 1; i < corners.length; i++) {
        ctx.lineTo(corners[i]![0], corners[i]![1]);
      }
      ctx.closePath();
      ctx.fill();
    }
  }
}

export const bitsLayer: OverlayLayer = {
  id: "bits",
  label: "Sampled bits",
  defaultEnabled: false,
  draw({ ctx, view, scan }) {
    const codes = scan?.detections.codes;
    if (!codes || codes.length === 0) return;

    const traceCodes = scan?.trace?.codes;
    if (traceCodes && traceCodes.length > 0) {
      for (const codeTrace of traceCodes) {
        const code = codes[codeTrace.code_index];
        if (!code) continue;
        drawBits(ctx, view, codeTrace.bits, code);
      }
      return;
    }

    const bits = scan?.trace?.bits;
    if (!bits) return;
    drawBits(ctx, view, bits, codes[codes.length - 1]!);
  },
};
