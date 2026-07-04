// Sampled-bits overlay (Plan 4 Task 6): a semi-transparent fill over every
// DARK module of the last successfully decoded candidate's sampled bit
// matrix (`scan.trace.bits`, packed row-major `u32` words — see the Rust
// `trace::BitsTrace` doc). There is only ever one `bits` per frame (the
// LAST successfully decoded candidate, not one per `DecodedCode`), so a
// multi-code scene's `bits` corresponds to `scan.detections.codes`' own
// LAST entry — codes are appended in decode order (see
// `decode::decode_candidates`), so this is exactly the same candidate
// `bits` was recorded for. Ambiguous per-code attribution in a multi-code
// scene is a known v1 limitation, documented here and in the Rust trace
// doc; a v7+ (multi-attempt) QA screenshot is deferred to Task 7.
//
// Module fill quads come from mapping each unit-square module cell through
// the TS homography port (`squareToQuad`) built from the code's own
// `corners` — the same "reconstruct the grid from a homography, not
// per-module trace points" approach the plan's trace-compactness
// constraint calls for.
//
// Zoom-gated: skipped entirely when a module would render smaller than
// `MIN_MODULE_SCREEN_PX` screen px (`view.scale * module_px < 4`) — v40's
// ~31k modules would otherwise flood the canvas with sub-pixel fills for no
// visible benefit. "module_px" here is the code's own top-edge (TL-TR)
// pixel length divided by `bits.dim`, i.e. one module's on-screen width in
// working-res image px before the view's own zoom scale is applied.
import { imageToScreen } from "../../viewport/transform";
import { mapPoint, squareToQuad, type Point2 } from "../homography";
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

export const bitsLayer: OverlayLayer = {
  id: "bits",
  label: "Sampled bits",
  defaultEnabled: false,
  draw({ ctx, view, scan }) {
    const bits = scan?.trace?.bits;
    const codes = scan?.detections.codes;
    if (!bits || !codes || codes.length === 0) return;
    const code = codes[codes.length - 1]!;

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
  },
};
