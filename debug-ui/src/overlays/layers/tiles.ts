// Tile threshold heatmap overlay: an alpha-blended gray rectangle per
// 16px tile (working res), colored by the tile's binarization threshold,
// plus a red hatch mark over tiles the tile stage decided to skip (see
// `crates/qr-lab-core/src/tiles.rs`'s `TILE` const — 16px, mirrored here so
// this layer's rects line up with the trace data's tile indexing).
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";

const TILE_PX = 16;
const HEATMAP_ALPHA = 0.35;
const SKIP_HATCH_COLOR = "rgba(255, 0, 0, 0.7)";

export const tilesLayer: OverlayLayer = {
  id: "tiles",
  label: "Tile thresholds",
  defaultEnabled: false,
  draw({ ctx, view, scan }) {
    const tiles = scan?.trace?.tiles;
    if (!tiles) return;

    const { tiles_x, tiles_y, thresholds, skip } = tiles;
    for (let ty = 0; ty < tiles_y; ty++) {
      for (let tx = 0; tx < tiles_x; tx++) {
        const idx = ty * tiles_x + tx;
        const threshold = thresholds[idx] ?? 0;
        const isSkip = skip[idx] ?? false;

        const [sx0, sy0] = imageToScreen(view, [tx * TILE_PX, ty * TILE_PX]);
        const [sx1, sy1] = imageToScreen(view, [(tx + 1) * TILE_PX, (ty + 1) * TILE_PX]);

        const gray = Math.max(0, Math.min(255, Math.round(threshold)));
        ctx.fillStyle = `rgba(${gray}, ${gray}, ${gray}, ${HEATMAP_ALPHA})`;
        ctx.fillRect(sx0, sy0, sx1 - sx0, sy1 - sy0);

        if (isSkip) {
          ctx.strokeStyle = SKIP_HATCH_COLOR;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(sx0, sy0);
          ctx.lineTo(sx1, sy1);
          ctx.moveTo(sx1, sy0);
          ctx.lineTo(sx0, sy1);
          ctx.stroke();
        }
      }
    }
  },
};
