// Decoded-payload overlay (Plan 4 Task 6): a green payload badge (payload
// text + a `v{n} {ecc}[ mirrored]` line) centered on each successfully
// decoded code's centroid, drawn from `scan.detections.codes` (always
// populated regardless of whether a trace was captured — same precedent
// `finders.ts` already established for `scan.detections.finders`); plus a
// small red ✕ + failure reason for every FAILED decode attempt, drawn at
// its triplet's TL corner (needs `scan.trace.attempts`, since only the
// trace records per-attempt outcomes — this half of the layer draws
// nothing without a captured trace, same precedent `tiles.ts` established
// for `trace.tiles`). "Failed" is defined as any attempt whose `outcome`
// does NOT start with `"decoded"` (a successful decode with the
// dimension-mismatch discrepancy note still starts with `"decoded"` — see
// the Rust `DecodeAttemptTrace.outcome` doc). A triplet with multiple
// failed corner-role-rotation attempts (see `decode_candidates`'s own doc)
// draws one ✕ per failed rotation, all at the same triplet's TL — a known,
// documented overlap; each still carries its own distinct `outcome` text.
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";
import type { Point2 } from "../homography";

const DECODED_COLOR = "#00e676"; // green
const FAILED_COLOR = "#ff1744"; // red
const LABEL_FONT = "11px monospace";
const MARKER_HALF = 5; // screen px

function centroid(corners: readonly [Point2, Point2, Point2, Point2]): Point2 {
  const sx = corners.reduce((sum, p) => sum + p[0], 0) / corners.length;
  const sy = corners.reduce((sum, p) => sum + p[1], 0) / corners.length;
  return [sx, sy];
}

export const decodedLayer: OverlayLayer = {
  id: "decoded",
  label: "Decoded payloads",
  defaultEnabled: true,
  draw({ ctx, view, scan }) {
    if (!scan) return;

    ctx.font = LABEL_FONT;
    ctx.textAlign = "center";
    ctx.fillStyle = DECODED_COLOR;
    for (const code of scan.detections.codes) {
      const [cx, cy] = imageToScreen(view, centroid(code.corners));
      const badge = `v${code.version} ${code.ecc}${code.mirrored ? " mirrored" : ""}`;
      ctx.fillText(code.payload, cx, cy - 6);
      ctx.fillText(badge, cx, cy + 8);
    }
    ctx.textAlign = "start";

    const triplets = scan.detections.triplets;
    for (const attempt of scan.trace?.attempts ?? []) {
      if (attempt.outcome.startsWith("decoded")) continue;
      const triplet = triplets[attempt.triplet_index];
      if (!triplet) continue;

      const [tx, ty] = imageToScreen(view, triplet.tl);
      ctx.strokeStyle = FAILED_COLOR;
      ctx.beginPath();
      ctx.moveTo(tx - MARKER_HALF, ty - MARKER_HALF);
      ctx.lineTo(tx + MARKER_HALF, ty + MARKER_HALF);
      ctx.moveTo(tx + MARKER_HALF, ty - MARKER_HALF);
      ctx.lineTo(tx - MARKER_HALF, ty + MARKER_HALF);
      ctx.stroke();

      ctx.fillStyle = FAILED_COLOR;
      ctx.fillText(attempt.outcome, tx + MARKER_HALF + 3, ty + 4);
    }
  },
};
