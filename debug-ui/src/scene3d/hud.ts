// On-canvas HUD (Plan 5d): a small monospace status block — camera-sim
// knobs (blur/noise/exposure) plus live camera/plane geometry stats
// (distance/incidence/roll) — drawn on the 2D OVERLAY canvas ONLY.
//
// This MUST NOT touch the offscreen `WebGLRenderTarget` readback that
// feeds `ScannerClient.scan` (and, via Plan 5d, the fixture-export PNG):
// `Scene3D.tsx`'s `handleResult` draws the HUD on the same
// `overlayCanvasRef` canvas `overlayRegistry.drawAll` already uses for
// every other overlay layer, strictly AFTER the scan/capture pipeline has
// already run for that tick — the readback buffer is a completely
// separate `WebGLRenderTarget`, never a canvas 2D context, so there is no
// code path by which drawing here could contaminate it.
import type { CameraStats } from "./cameraStats";

export interface HudCamSimValues {
  blurSigma: number;
  noiseSigma: number;
  exposureOffset: number;
}

/**
 * Pure text-formatting half of the HUD — one line per stat, always camSim
 * first (always available) then camera stats (omitted, not blank, when
 * `stats` is `null` — e.g. before the first scan tick has run). When
 * `sensorView` is true a `[sensor view]` tag line leads the block — the
 * truthfulness marker telling users the canvas under the HUD is showing
 * the scanner's own processed input frame, not the live WebGL render
 * (see `sensorView.ts`).
 */
export function formatHudLines(
  camSim: HudCamSimValues,
  stats: CameraStats | null,
  sensorView = false,
): string[] {
  const lines = [
    `blur    sigma=${camSim.blurSigma.toFixed(1)}px`,
    `noise   sigma=${camSim.noiseSigma.toFixed(1)}`,
    `exposure ${camSim.exposureOffset > 0 ? "+" : ""}${camSim.exposureOffset}`,
  ];
  if (sensorView) lines.unshift("[sensor view]");
  if (stats) {
    lines.push(
      `dist    ${stats.distanceM.toFixed(3)}m`,
      `incid   ${stats.incidenceDeg.toFixed(1)}deg`,
      `roll    ${stats.inPlaneRollDeg.toFixed(1)}deg`,
    );
  }
  return lines;
}

const HUD_PADDING = 8;
const HUD_LINE_HEIGHT = 14;
const HUD_FONT = "12px monospace";
const HUD_TEXT_COLOR = "#e5e7eb";
const HUD_BG_COLOR = "rgba(0, 0, 0, 0.55)";

/**
 * Draw `lines` as a small monospace block, top-left, on `ctx`. Untested
 * (needs a real `CanvasRenderingContext2D`, unavailable in vitest's
 * `node` environment) — same manual/headless-browser-QA-only class as
 * `ErrorPanel.tsx`'s `drawSparkline`.
 */
export function drawHud(ctx: CanvasRenderingContext2D, lines: string[]): void {
  if (lines.length === 0) return;
  ctx.save();
  ctx.font = HUD_FONT;
  ctx.textBaseline = "top";
  const blockWidth = Math.max(...lines.map((l) => ctx.measureText(l).width)) + HUD_PADDING * 2;
  const blockHeight = lines.length * HUD_LINE_HEIGHT + HUD_PADDING * 2;
  ctx.fillStyle = HUD_BG_COLOR;
  ctx.fillRect(0, 0, blockWidth, blockHeight);
  ctx.fillStyle = HUD_TEXT_COLOR;
  lines.forEach((line, i) => {
    ctx.fillText(line, HUD_PADDING, HUD_PADDING + i * HUD_LINE_HEIGHT);
  });
  ctx.restore();
}
