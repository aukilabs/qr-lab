// Error-aggregation model behind the 3D-scene mode's error panel (Plan 5
// Task 5) — the debug UI's live accuracy meter: per-corner
// |refined - truth| in px each frame, and a rolling mean/p95 of the
// per-frame mean error over the last `SPARKLINE_CAPACITY` samples.
// Reuses `panels/timings-model.ts`'s `RollingBuffer` (already exactly
// this: a fixed-capacity ring buffer feeding a sparkline) rather than a
// second implementation of the same fixed-capacity-ring-buffer concept.
import { RollingBuffer, SPARKLINE_CAPACITY } from "../panels/timings-model";

export type Corner4<T> = [T, T, T, T]; // [TL, TR, BR, BL]

/** Per-corner Euclidean distance (px) between two same-space 4-corner
 * quads, plus their mean — the metric `tests/refine_gate.rs` and the
 * `refined` overlay layer both already use for "corner error", applied
 * here to the 3D scene's live refined-vs-projected-truth comparison. */
export interface CornerErrors {
  tl: number;
  tr: number;
  br: number;
  bl: number;
  mean: number;
}

/** Euclidean distance between two points. */
export function pointError(a: [number, number], b: [number, number]): number {
  const dx = a[0] - b[0];
  const dy = a[1] - b[1];
  return Math.sqrt(dx * dx + dy * dy);
}

/** Per-corner error between `refined` and `truth` (same order, same
 * space — see `refined.ts`'s doc on why the two must already share a
 * pixel space before calling this). */
export function cornerErrors(
  refined: Corner4<[number, number]>,
  truth: Corner4<[number, number]>,
): CornerErrors {
  const tl = pointError(refined[0], truth[0]);
  const tr = pointError(refined[1], truth[1]);
  const br = pointError(refined[2], truth[2]);
  const bl = pointError(refined[3], truth[3]);
  return { tl, tr, br, bl, mean: (tl + tr + br + bl) / 4 };
}

/**
 * The `p`-th percentile (`0 <= p <= 100`) of `values`, via
 * nearest-rank on a sorted copy (does not mutate `values`). Returns `0`
 * for an empty input (matching the sparkline's "nothing sampled yet"
 * convention elsewhere in this debug UI, e.g. `RollingBuffer` starting
 * all-zero). `p=95` (used by the error panel) with `n` samples picks the
 * `ceil(0.95*n)`-th smallest value (1-indexed) — the conventional
 * nearest-rank definition, which unlike interpolated methods always
 * returns an actual observed sample.
 */
export function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  if (p <= 0) return Math.min(...values);
  if (p >= 100) return Math.max(...values);
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.ceil((p / 100) * sorted.length);
  const index = Math.min(sorted.length, Math.max(1, rank)) - 1;
  return sorted[index]!;
}

export interface ErrorSummary {
  mean: number;
  p95: number;
}

/** Mean and p95 of a rolling buffer's current samples — `{mean: 0, p95:
 * 0}` before any sample has been pushed. */
export function summarizeRolling(buffer: RollingBuffer): ErrorSummary {
  const values = buffer.values();
  if (values.length === 0) return { mean: 0, p95: 0 };
  const mean = values.reduce((a, b) => a + b, 0) / values.length;
  return { mean, p95: percentile(values, 95) };
}

/** A fresh rolling buffer sized for the error panel's sparkline —
 * exported so `Scene3D.tsx` doesn't need to import `SPARKLINE_CAPACITY`
 * from `timings-model` directly just to construct one. */
export function newErrorRollingBuffer(): RollingBuffer {
  return new RollingBuffer(SPARKLINE_CAPACITY);
}
