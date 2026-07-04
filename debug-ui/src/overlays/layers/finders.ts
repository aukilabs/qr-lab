// Finder-candidate overlay: a circle around each candidate (radius scales
// with zoom since it's meant to trace the actual finder-pattern extent —
// 3.5 modules from center to edge — in image-proportional terms), colored
// by polarity, labeled with its `hits` count. Draws from
// `scan.detections.finders`, which is always populated regardless of
// whether a trace was captured (unlike `tilesLayer`, which needs
// `trace.tiles`) — see `crates/qrk-core/src/scanner.rs`'s `detect_with`,
// where `trace.finders` is just a recorded copy of the same `finders` that
// flow into `Detections`.
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";

const NORMAL_COLOR = "#00e5ff"; // cyan: dark-ink (non-inverted) finder
const INVERTED_COLOR = "#ff9100"; // orange: inverted-polarity finder
const LABEL_FONT = "10px monospace";

export const findersLayer: OverlayLayer = {
  id: "finders",
  label: "Finder candidates",
  defaultEnabled: true,
  draw({ ctx, view, scan }) {
    const finders = scan?.detections.finders;
    if (!finders || finders.length === 0) return;

    ctx.lineWidth = 1.5;
    ctx.font = LABEL_FONT;

    for (const f of finders) {
      const [sx, sy] = imageToScreen(view, [f.x, f.y]);
      const radius = 3.5 * f.module * view.scale;
      const color = f.inverted ? INVERTED_COLOR : NORMAL_COLOR;

      ctx.strokeStyle = color;
      ctx.beginPath();
      ctx.arc(sx, sy, radius, 0, Math.PI * 2);
      ctx.stroke();

      ctx.fillStyle = color;
      ctx.fillText(String(f.hits), sx + radius + 2, sy);
    }
  },
};
