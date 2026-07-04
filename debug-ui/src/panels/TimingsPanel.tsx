import { useEffect, useMemo, useRef } from "react";
import type { StageTimings } from "../scanner/types";
import { formatMs, formatNs, RollingBuffer } from "./timings-model";

/** One completed scan's timing data — the subset of `ScanOutcome` (see
 * `scanner/client.ts`) this panel needs, plus a main-thread round-trip
 * measurement the client itself doesn't compute (it only knows the
 * worker's internal `wallMs`; the caller is the one who knows when
 * `client.scan(...)` was invoked, so it's the caller's job to time the
 * round trip and pass it in). */
export interface TimingsSample {
  timings: StageTimings;
  /** Worker-side `scan_rgba` wall time in ms (`performance.now()` delta
   * measured inside the worker — see `scanner/worker.ts`). */
  wallMs: number;
  /** Main-thread time from calling `ScannerClient.scan(...)` to its
   * promise resolving, in ms — covers worker wall time plus postMessage
   * marshalling/queueing overhead. */
  roundTripMs: number;
}

export interface TimingsPanelProps {
  /** Latest completed scan's timing data, or `null` before any scan has
   * finished. */
  sample: TimingsSample | null;
  /** Increments once per completed scan. A new sample is pushed into the
   * rolling buffers when this changes, not when `sample`'s field values
   * change — two consecutive scans can legitimately report identical
   * numbers (e.g. both stages clamp to 0ns at ms resolution), and each is
   * still a real sample the sparkline should show. */
  sampleId: number;
}

interface Row {
  label: string;
  buffer: RollingBuffer;
  format: (v: number) => string;
  /** Pulls this row's value out of a `TimingsSample`. */
  pick: (s: TimingsSample) => number;
}

const SPARKLINE_WIDTH = 120;
const SPARKLINE_HEIGHT = 24;
const SPARKLINE_LINE_COLOR = "#4ade80";
const SPARKLINE_FILL_COLOR = "rgba(74, 222, 128, 0.15)";

/**
 * Draw a filled line sparkline of `values` (chronological, oldest first)
 * scaled to fill the canvas. A flat (zero-height) line when every value is
 * `<= 0` reads as "flat", not "one huge spike" or a divide-by-zero NaN
 * path.
 */
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

/**
 * Table of per-stage detection timings (tiles/finders/triplets plus the
 * Plan 4 decode-stage totals version/alignment/sample+decode, from
 * `detections.timings` — real wasm `StageClock` measurements as of Plan 3
 * Task 5, ms-resolution via `js_sys::Date::now()`) plus worker wall time
 * and main-thread round-trip time, each with a 60-sample rolling
 * sparkline. Values format via `formatNs`/`formatMs` (timings-model.ts) —
 * a stage reporting exactly 0ns reads as "n/a" (clock too coarse to
 * observe it), not "took no time".
 *
 * Rolling buffers and the sparkline canvases live outside React state
 * (refs) so 60 samples' worth of history don't trigger per-sample
 * re-renders; pushing a new sample into every row's buffer and redrawing
 * every sparkline both happen together in one effect keyed on `sampleId`
 * — combining them avoids a parent/child effect-ordering hazard (a
 * separate per-row child effect would run its draw *before* a parent
 * effect that pushes the new sample, since child effects fire first,
 * leaving the sparkline permanently one sample behind).
 */
export function TimingsPanel({ sample, sampleId }: TimingsPanelProps) {
  const rows = useMemo<Row[]>(
    () => [
      { label: "tiles", buffer: new RollingBuffer(), format: formatNs, pick: (s) => s.timings.tiles_ns },
      { label: "finders", buffer: new RollingBuffer(), format: formatNs, pick: (s) => s.timings.finders_ns },
      { label: "triplets", buffer: new RollingBuffer(), format: formatNs, pick: (s) => s.timings.triplets_ns },
      // Plan 4 Task 6: the three decode-stage totals `decode::DecodeTimings`
      // accumulates across every attempt in the frame (see that struct's
      // doc for why they're summed rather than per-stage-phase like the
      // three above).
      { label: "version", buffer: new RollingBuffer(), format: formatNs, pick: (s) => s.timings.version_ns },
      { label: "alignment", buffer: new RollingBuffer(), format: formatNs, pick: (s) => s.timings.alignment_ns },
      {
        label: "sample+decode",
        buffer: new RollingBuffer(),
        format: formatNs,
        pick: (s) => s.timings.sample_decode_ns,
      },
      { label: "worker wall", buffer: new RollingBuffer(), format: formatMs, pick: (s) => s.wallMs },
      { label: "round trip", buffer: new RollingBuffer(), format: formatMs, pick: (s) => s.roundTripMs },
    ],
    [],
  );

  const canvasRefs = useRef<(HTMLCanvasElement | null)[]>([]);
  // Guards against pushing the same sample twice — React (StrictMode in
  // dev, in particular) may invoke an effect more than once for what is
  // logically a single commit.
  const lastPushedSampleId = useRef<number | null>(null);

  useEffect(() => {
    if (sample && lastPushedSampleId.current !== sampleId) {
      lastPushedSampleId.current = sampleId;
      for (const row of rows) row.buffer.push(row.pick(sample));
    }
    for (let i = 0; i < rows.length; i++) {
      const ctx = canvasRefs.current[i]?.getContext("2d");
      if (ctx) drawSparkline(ctx, rows[i]!.buffer.values());
    }
  }, [sample, sampleId, rows]);

  return (
    <table style={{ borderCollapse: "collapse", fontSize: 12, fontFamily: "monospace" }}>
      <thead>
        <tr>
          <th style={{ textAlign: "left", padding: "2px 8px 2px 0" }}>stage</th>
          <th style={{ textAlign: "left", padding: "2px 8px" }}>latest</th>
          <th style={{ textAlign: "left", padding: "2px 0" }}>last 60</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row, i) => (
          <tr key={row.label}>
            <td style={{ padding: "2px 8px 2px 0", color: "#9ca3af" }}>{row.label}</td>
            <td style={{ padding: "2px 8px", fontVariantNumeric: "tabular-nums" }}>
              {sample ? row.format(row.pick(sample)) : "n/a"}
            </td>
            <td style={{ padding: "2px 0" }}>
              <canvas
                ref={(el) => {
                  canvasRefs.current[i] = el;
                }}
                width={SPARKLINE_WIDTH}
                height={SPARKLINE_HEIGHT}
                style={{ display: "block" }}
              />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
