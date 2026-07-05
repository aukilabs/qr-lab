// Pure time-formatting/clamping helpers for the video scrubber (Task 5e).
// Split out of `useVideoSource`/the scrubber component so they're
// vitest-able without a DOM — this file has no browser dependency at all.

/**
 * Format seconds as `mm:ss.d` (minutes:seconds.tenths), e.g. `01:03.5`.
 * Minutes grow unboundedly rather than rolling into an `h:mm:ss` form —
 * simplest correct behavior for an hours-long source (there's no upper
 * bound requirement, just "doesn't wrap/break"). Non-finite or negative
 * input (duration before `loadedmetadata`, or a `NaN`/`Infinity` edge case)
 * renders as `--:--.-` rather than `NaN:NaN` or throwing.
 */
export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "--:--.-";

  // Round to the nearest tenth in integer space (avoids float noise like
  // `59.999999996` formatting as `01:00.0` one tick early or `00:59.10`).
  const totalTenths = Math.round(seconds * 10);
  const tenths = totalTenths % 10;
  const totalSeconds = Math.floor(totalTenths / 10);
  const secs = totalSeconds % 60;
  const mins = Math.floor(totalSeconds / 60);

  return `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}.${tenths}`;
}

/**
 * Clamp `time` into `[0, duration]`. Mirrors `useVideoSource`'s own
 * `stepFrame` clamp (`Number.isFinite(duration) ? duration : Infinity`) so
 * a `NaN`/`Infinity` duration (no metadata yet) doesn't clamp a valid seek
 * target down to 0 — callers are expected to separately gate seeking on
 * `duration` being a usable, positive, finite number (see the scrubber's
 * `disabled` check), this just avoids this specific helper misbehaving if
 * called anyway.
 */
export function clampTime(time: number, duration: number): number {
  // Only `NaN` needs a special case — `Math.max`/`Math.min` already handle
  // `±Infinity` correctly (an infinite `time` clamps to whichever bound it
  // overshoots; `NaN` propagates through both and would otherwise "clamp"
  // to `NaN`).
  if (Number.isNaN(time)) return 0;
  const max = Number.isFinite(duration) ? duration : Infinity;
  return Math.min(Math.max(time, 0), max);
}
