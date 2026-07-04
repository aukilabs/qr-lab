// Live accuracy meter for the 3D-scene mode (Plan 5 Task 5) — the debug
// UI's headline feature: per-corner |refined - truth| in px this frame,
// plus a rolling 60-sample mean/p95 of the per-frame mean error. Mirrors
// `TimingsPanel.tsx`'s table+sparkline shape closely (same rolling-buffer
// + redraw-on-sampleId-change pattern) since it's the same kind of panel;
// see that component's doc for the full rationale of that pattern.
import { useEffect, useRef } from "react";
import type { CornerErrors } from "./errorStats";
import { newErrorRollingBuffer, summarizeRolling } from "./errorStats";
import type { RollingBuffer } from "../panels/timings-model";

export interface ErrorSample {
  /** This frame's per-corner + mean error (SOURCE px == readback px in
   * the 3D scene, since it always scans at `maxDim: 0` — see
   * `Scene3D.tsx`'s doc on why no scale conversion is needed here). */
  corners: CornerErrors;
}

export interface ErrorPanelProps {
  /** Latest frame's error sample, or `null` before any code has decoded
   * yet (nothing to compare refined corners against). */
  sample: ErrorSample | null;
  /** Increments once per frame a sample was computed — same contract as
   * `TimingsPanel`'s `sampleId` (a new sample is pushed when this
   * changes, not when the sample's values change, so two identical
   * consecutive frames still count as two rolling-buffer samples). */
  sampleId: number;
}

const SPARKLINE_WIDTH = 160;
const SPARKLINE_HEIGHT = 32;
const SPARKLINE_LINE_COLOR = "#f472b6";
const SPARKLINE_FILL_COLOR = "rgba(244, 114, 182, 0.15)";

// Duplicated from `TimingsPanel.tsx` rather than shared: sparkline drawing
// needs a real `CanvasRenderingContext2D`, so (per this codebase's
// existing split — see `panels/timings-model.ts`'s module doc) it lives
// in the component, not a shared pure-logic module; ~20 lines of drawing
// code isn't worth introducing a shared non-pure "sparkline widget"
// module for two call sites.
function drawSparkline(ctx: CanvasRenderingContext2D, values: readonly number[]): void {
  ctx.clearRect(0, 0, SPARKLINE_WIDTH, SPARKLINE_HEIGHT);
  if (values.length === 0) return;

  const max = Math.max(...values, 0);
  const points: [number, number][] = values.map((v, i) => {
    const x = values.length === 1 ? SPARKLINE_WIDTH : (i / (values.length - 1)) * SPARKLINE_WIDTH;
    const y = max > 0 ? SPARKLINE_HEIGHT - (v / max) * SPARKLINE_HEIGHT : SPARKLINE_HEIGHT;
    return [x, y];
  });

  ctx.beginPath();
  ctx.moveTo(points[0]![0], SPARKLINE_HEIGHT);
  for (const [x, y] of points) ctx.lineTo(x, y);
  ctx.lineTo(points[points.length - 1]![0], SPARKLINE_HEIGHT);
  ctx.closePath();
  ctx.fillStyle = SPARKLINE_FILL_COLOR;
  ctx.fill();

  ctx.beginPath();
  ctx.moveTo(points[0]![0], points[0]![1]);
  for (const [x, y] of points.slice(1)) ctx.lineTo(x, y);
  ctx.strokeStyle = SPARKLINE_LINE_COLOR;
  ctx.lineWidth = 1.5;
  ctx.stroke();
}

function fmtPx(v: number): string {
  return `${v.toFixed(2)}px`;
}

export function ErrorPanel({ sample, sampleId }: ErrorPanelProps) {
  const bufferRef = useRef<RollingBuffer>(newErrorRollingBuffer());
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const lastPushedSampleId = useRef<number | null>(null);

  useEffect(() => {
    if (sample && lastPushedSampleId.current !== sampleId) {
      lastPushedSampleId.current = sampleId;
      bufferRef.current.push(sample.corners.mean);
    }
    const ctx = canvasRef.current?.getContext("2d");
    if (ctx) drawSparkline(ctx, bufferRef.current.values());
  }, [sample, sampleId]);

  const rolling = summarizeRolling(bufferRef.current);
  const rows: [string, number | null][] = sample
    ? [
        ["TL", sample.corners.tl],
        ["TR", sample.corners.tr],
        ["BR", sample.corners.br],
        ["BL", sample.corners.bl],
        ["mean", sample.corners.mean],
      ]
    : [
        ["TL", null],
        ["TR", null],
        ["BR", null],
        ["BL", null],
        ["mean", null],
      ];

  return (
    <div style={{ fontSize: 12, fontFamily: "monospace" }}>
      <table style={{ borderCollapse: "collapse", width: "100%" }}>
        <thead>
          <tr>
            <th style={{ textAlign: "left", padding: "2px 8px 2px 0" }}>corner</th>
            <th style={{ textAlign: "left", padding: "2px 0" }}>error</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([label, value]) => (
            <tr key={label}>
              <td style={{ padding: "2px 8px 2px 0", color: "#9ca3af" }}>{label}</td>
              <td style={{ padding: "2px 0", fontVariantNumeric: "tabular-nums" }}>
                {value == null ? "n/a" : fmtPx(value)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div style={{ marginTop: 8, color: "#d1d5db" }}>
        rolling mean: <span style={{ fontVariantNumeric: "tabular-nums" }}>{fmtPx(rolling.mean)}</span>
        {" · "}
        p95: <span style={{ fontVariantNumeric: "tabular-nums" }}>{fmtPx(rolling.p95)}</span>
      </div>
      <canvas
        ref={canvasRef}
        width={SPARKLINE_WIDTH}
        height={SPARKLINE_HEIGHT}
        style={{ display: "block", marginTop: 4 }}
      />
    </div>
  );
}
