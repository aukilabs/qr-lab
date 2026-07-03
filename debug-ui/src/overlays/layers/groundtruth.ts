// Ground-truth overlay: a green quad through each golden-fixture code's
// `corners_px`, plus dots at the expected finder-pattern centers (computed
// via `expectedFinderCenters`). `corners_px` (and everything derived from
// it) is in SOURCE-image px — `workingScale` converts to working-res px
// (the space `view`/`imageToScreen` operate in) before projecting, per
// `OverlayContext`'s documented contract. No trace/scan data is needed;
// this layer draws purely from `groundTruth`.
import { imageToScreen } from "../../viewport/transform";
import type { Point2 } from "../homography";
import type { OverlayLayer } from "../registry";
import { expectedFinderCenters } from "./expected-finder-centers";

const COLOR = "#00c853"; // green
const DOT_RADIUS = 3; // screen px

export const groundtruthLayer: OverlayLayer = {
  id: "groundtruth",
  label: "Ground truth",
  defaultEnabled: true,
  draw({ ctx, view, groundTruth, workingScale }) {
    if (!groundTruth || groundTruth.length === 0) return;

    ctx.strokeStyle = COLOR;
    ctx.fillStyle = COLOR;
    ctx.lineWidth = 2;

    const toScreen = (p: Point2): Point2 =>
      imageToScreen(view, [p[0] * workingScale, p[1] * workingScale]);

    for (const code of groundTruth) {
      const quad = code.corners_px.map(toScreen);

      ctx.beginPath();
      ctx.moveTo(quad[0]![0], quad[0]![1]);
      for (let i = 1; i < quad.length; i++) {
        ctx.lineTo(quad[i]![0], quad[i]![1]);
      }
      ctx.closePath();
      ctx.stroke();

      for (const center of expectedFinderCenters(code)) {
        const [sx, sy] = toScreen(center);
        ctx.beginPath();
        ctx.arc(sx, sy, DOT_RADIUS, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  },
};
