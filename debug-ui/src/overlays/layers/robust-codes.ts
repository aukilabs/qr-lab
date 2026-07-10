// Robust-mode decoded-codes overlay (Plan 6): one quad per
// `robust.codes` entry — the ladder's accepted codes, each stroked in its
// finding stage's color (`stageColor`) with a small filled label chip near
// the TL corner naming the variant that found it (`variantKindLabel`),
// plus a small filled dot at each refined corner when
// `refined_corners_source` is present.
//
// Coordinate space: `corners_source`/`refined_corners_source` are SOURCE
// px (see `RobustCode`'s doc — `code.corners` is VARIANT working px and
// must never be drawn), so both are multiplied by `workingScale` before
// projecting through `imageToScreen`, the same convention
// `groundtruth.ts` established for source-px data.
import { stageColor, variantKindLabel } from "../../scanner/robust-types";
import { imageToScreen } from "../../viewport/transform";
import type { Point2 } from "../homography";
import type { OverlayLayer } from "../registry";

const LABEL_FONT = "10px monospace";
const CHIP_TEXT_COLOR = "#0b0f17"; // dark text on the stage-colored chip
const CHIP_HEIGHT = 13; // screen px
const CHIP_PAD_X = 4; // screen px
// Approximate monospace advance at 10px — avoids `measureText` (which the
// test fake canvas doesn't model) for a debug chip whose exact width
// doesn't matter.
const CHIP_CHAR_WIDTH = 6;
const DOT_RADIUS = 2.5; // screen px

export const robustCodesLayer: OverlayLayer = {
  id: "robust-codes",
  label: "Robust codes",
  defaultEnabled: true,
  draw({ ctx, view, robust, workingScale }) {
    if (!robust || robust.codes.length === 0) return;

    const toScreen = (p: Point2): Point2 =>
      imageToScreen(view, [p[0] * workingScale, p[1] * workingScale]);

    for (const rc of robust.codes) {
      const color = stageColor(rc.stage);
      const quad = rc.corners_source.map(toScreen);

      ctx.strokeStyle = color;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(quad[0]![0], quad[0]![1]);
      for (let i = 1; i < quad.length; i++) {
        ctx.lineTo(quad[i]![0], quad[i]![1]);
      }
      ctx.closePath();
      ctx.stroke();

      // Label chip anchored just above the TL corner (corners are
      // [TL, TR, BR, BL], so quad[0] is TL).
      const label = variantKindLabel(rc.variant);
      const chipWidth = label.length * CHIP_CHAR_WIDTH + CHIP_PAD_X * 2;
      const [tlx, tly] = quad[0]!;
      ctx.fillStyle = color;
      ctx.fillRect(tlx, tly - CHIP_HEIGHT - 2, chipWidth, CHIP_HEIGHT);
      ctx.font = LABEL_FONT;
      ctx.fillStyle = CHIP_TEXT_COLOR;
      ctx.fillText(label, tlx + CHIP_PAD_X, tly - 5);

      if (rc.refined_corners_source) {
        ctx.fillStyle = color;
        for (const corner of rc.refined_corners_source) {
          const [sx, sy] = toScreen(corner);
          ctx.beginPath();
          ctx.arc(sx, sy, DOT_RADIUS, 0, Math.PI * 2);
          ctx.fill();
        }
      }
    }
  },
};
