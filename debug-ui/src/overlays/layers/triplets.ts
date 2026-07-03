// Triplet-candidate overlay: distinguishable markers at each corner (TL =
// square, TR = triangle, BL = circle — matches the `tl`/`tr`/`bl` naming so
// a reader can tell which finder played which role without a legend), leg
// lines connecting them, and a `dim=N (±snap_error)` label. Draws from
// `scan.detections.triplets` (always populated; see `finders.ts`'s comment
// on why trace isn't required). Marker sizes are constant screen px (not
// image-proportional) so they stay legible at any zoom level.
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";

const COLOR = "#76ff03"; // lime
const TEXT_COLOR = "#ffffff";
const MARKER_HALF = 6; // screen px
const LABEL_FONT = "10px monospace";

export const tripletsLayer: OverlayLayer = {
  id: "triplets",
  label: "Triplets",
  defaultEnabled: true,
  draw({ ctx, view, scan }) {
    const triplets = scan?.detections.triplets;
    if (!triplets || triplets.length === 0) return;

    ctx.lineWidth = 1.5;
    ctx.font = LABEL_FONT;

    for (const t of triplets) {
      const [tlx, tly] = imageToScreen(view, t.tl);
      const [trx, try_] = imageToScreen(view, t.tr);
      const [blx, bly] = imageToScreen(view, t.bl);

      ctx.strokeStyle = COLOR;

      // Leg lines: TL-TR and TL-BL.
      ctx.beginPath();
      ctx.moveTo(tlx, tly);
      ctx.lineTo(trx, try_);
      ctx.moveTo(tlx, tly);
      ctx.lineTo(blx, bly);
      ctx.stroke();

      // TL marker: square.
      ctx.strokeRect(tlx - MARKER_HALF, tly - MARKER_HALF, MARKER_HALF * 2, MARKER_HALF * 2);

      // TR marker: triangle (point up).
      ctx.beginPath();
      ctx.moveTo(trx, try_ - MARKER_HALF);
      ctx.lineTo(trx + MARKER_HALF, try_ + MARKER_HALF);
      ctx.lineTo(trx - MARKER_HALF, try_ + MARKER_HALF);
      ctx.closePath();
      ctx.stroke();

      // BL marker: circle.
      ctx.beginPath();
      ctx.arc(blx, bly, MARKER_HALF, 0, Math.PI * 2);
      ctx.stroke();

      ctx.fillStyle = TEXT_COLOR;
      ctx.fillText(`dim=${t.dimension} (±${t.snap_error.toFixed(2)})`, tlx + 4, tly - 8);
    }
  },
};
