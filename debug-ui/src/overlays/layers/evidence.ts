// Triplet-evidence overlay (Plan 6): a crosshair marker at every
// `robust.triplet_evidence` point — the ladder's deduplicated
// coherent-triplet centroids, i.e. "detection saw something QR-shaped
// here" even on frames where nothing decoded. SOURCE px, so multiplied by
// `workingScale` before projecting (same convention as `groundtruth.ts` /
// `robust-codes.ts`). Cyan, distinct from every `stageColor` entry and
// from the classic layers' palette.
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";

const COLOR = "#18ffff"; // cyan
const CROSSHAIR_HALF = 7; // screen px
const GAP_HALF = 2; // screen px — open center so the point itself stays visible

export const evidenceLayer: OverlayLayer = {
  id: "evidence",
  label: "Triplet evidence",
  defaultEnabled: true,
  draw({ ctx, view, robust, workingScale }) {
    if (!robust || robust.triplet_evidence.length === 0) return;

    ctx.strokeStyle = COLOR;
    ctx.lineWidth = 1.5;
    for (const p of robust.triplet_evidence) {
      const [sx, sy] = imageToScreen(view, [p[0] * workingScale, p[1] * workingScale]);
      ctx.beginPath();
      ctx.moveTo(sx - CROSSHAIR_HALF, sy);
      ctx.lineTo(sx - GAP_HALF, sy);
      ctx.moveTo(sx + GAP_HALF, sy);
      ctx.lineTo(sx + CROSSHAIR_HALF, sy);
      ctx.moveTo(sx, sy - CROSSHAIR_HALF);
      ctx.lineTo(sx, sy - GAP_HALF);
      ctx.moveTo(sx, sy + GAP_HALF);
      ctx.lineTo(sx, sy + CROSSHAIR_HALF);
      ctx.stroke();
    }
  },
};
