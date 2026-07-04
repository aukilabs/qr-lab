// Subpixel-refined-corner overlay (Plan 5 Task 4): for every decoded code
// whose `refined_corners` is populated (`ScanOptions.refine === true` and
// refinement produced at least 2 valid edge lines — see
// `qrk_core::refine::refine_corners`'s doc), draws:
//   - a crosshair at each corner in `refined_corners` (SOURCE px, scaled to
//     working px via `workingScale` — the same convention `groundtruth.ts`
//     already establishes for source-px data): magenta where
//     `code.corner_refined[i]` is `true`, gray where it's `false` (that
//     corner is actually still the coarse fallback position, not a real
//     refinement — see `DecodedCode.corner_refined`'s doc);
//   - a hollow square at each COARSE corner (`code.corners`, already
//     working px — no scaling);
//   - a whisker line connecting each coarse corner to its refined
//     counterpart, so the correction is visible at a glance;
//   - when a golden-fixture ground-truth code with the same `payload` is
//     present in `groundTruth`, a small per-corner error label (in SOURCE
//     px, matching how `tests/refine_gate.rs` measures it) next to each
//     refined corner.
//
// A code with `refined_corners === null` draws nothing (there is no
// refined position to anchor a whisker/crosshair on).
//
// Ground-truth corner correspondence: IDENTITY (`corners_px[i]` <->
// `refined_corners[i]`), regardless of the code's `mirrored` flag — see
// `crates/qrk-core/tests/refine_gate.rs`'s module doc for the full
// derivation (QR finder patterns are content-independent, so the
// pipeline's purely-geometric TL/TR/BR/BL corner labeling never depends on
// which corner's data happens to be mirrored) and its empirical
// falsification of the naive "mirrored swaps TR/BL" hypothesis.
import { imageToScreen } from "../../viewport/transform";
import type { Point2 } from "../homography";
import type { OverlayLayer } from "../registry";

const REFINED_COLOR = "#ff00ff"; // magenta crosshairs
const FALLBACK_COLOR = "#9e9e9e"; // gray crosshair: corner_refined[i] === false (coarse fallback)
const COARSE_COLOR = "#ffd600"; // amber hollow squares
const WHISKER_COLOR = "#9e9e9e"; // neutral gray connecting line
const ERROR_LABEL_COLOR = "#ff00ff";
const LABEL_FONT = "10px monospace";
const CROSSHAIR_HALF = 5; // screen px
const SQUARE_HALF = 4; // screen px

function drawCrosshair(ctx: CanvasRenderingContext2D, p: Point2): void {
  ctx.beginPath();
  ctx.moveTo(p[0] - CROSSHAIR_HALF, p[1]);
  ctx.lineTo(p[0] + CROSSHAIR_HALF, p[1]);
  ctx.moveTo(p[0], p[1] - CROSSHAIR_HALF);
  ctx.lineTo(p[0], p[1] + CROSSHAIR_HALF);
  ctx.stroke();
}

function drawHollowSquare(ctx: CanvasRenderingContext2D, p: Point2): void {
  ctx.strokeRect(p[0] - SQUARE_HALF, p[1] - SQUARE_HALF, SQUARE_HALF * 2, SQUARE_HALF * 2);
}

function cornerError(a: Point2, b: Point2): number {
  const dx = a[0] - b[0];
  const dy = a[1] - b[1];
  return Math.sqrt(dx * dx + dy * dy);
}

export const refinedLayer: OverlayLayer = {
  id: "refined",
  label: "Refined corners",
  defaultEnabled: false,
  draw({ ctx, view, scan, groundTruth, workingScale }) {
    if (!scan) return;

    for (const code of scan.detections.codes) {
      const refined = code.refined_corners;
      if (!refined) continue;

      const truth = groundTruth?.find((g) => g.payload === code.payload) ?? null;

      const coarseScreen = code.corners.map((p) => imageToScreen(view, p));
      const refinedScreen = refined.map((p) =>
        imageToScreen(view, [p[0] * workingScale, p[1] * workingScale]),
      );

      // Whiskers first, so the crosshairs/squares draw on top of them.
      ctx.strokeStyle = WHISKER_COLOR;
      ctx.lineWidth = 1;
      for (let i = 0; i < 4; i++) {
        ctx.beginPath();
        ctx.moveTo(coarseScreen[i]![0], coarseScreen[i]![1]);
        ctx.lineTo(refinedScreen[i]![0], refinedScreen[i]![1]);
        ctx.stroke();
      }

      ctx.strokeStyle = COARSE_COLOR;
      ctx.lineWidth = 1.5;
      for (const p of coarseScreen) {
        drawHollowSquare(ctx, p);
      }

      // Per-corner color: gray marks a `corner_refined[i] === false` fallback
      // (this crosshair sits at the coarse position, not a real refinement —
      // see `DecodedCode.corner_refined`'s doc) instead of the usual magenta.
      ctx.lineWidth = 1.5;
      for (let i = 0; i < 4; i++) {
        ctx.strokeStyle = code.corner_refined[i] ? REFINED_COLOR : FALLBACK_COLOR;
        drawCrosshair(ctx, refinedScreen[i]!);
      }

      if (truth) {
        ctx.font = LABEL_FONT;
        ctx.fillStyle = ERROR_LABEL_COLOR;
        for (let i = 0; i < 4; i++) {
          // Error measured in SOURCE px (matches `refine_gate.rs`'s own
          // metric) — `refined`/`truth.corners_px` are both already
          // source px, so no `workingScale` conversion belongs here even
          // though the label is drawn at the working-px screen position.
          const err = cornerError(refined[i]!, truth.corners_px[i]!);
          const [sx, sy] = refinedScreen[i]!;
          ctx.fillText(`${err.toFixed(2)}px`, sx + 6, sy - 6);
        }
      }
    }
  },
};
