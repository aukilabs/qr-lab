// Pure logic behind `TimingsPanel`'s table + sparklines: a fixed-capacity
// rolling sample buffer (one per timed row) and the nanosecond/millisecond
// formatting rules. Kept separate from the component (which only owns
// canvas drawing + React wiring) so both can be unit-tested without a DOM.

/** Sparkline history length: how many past samples each row's rolling
 * buffer keeps, and therefore how many points its canvas sparkline draws. */
export const SPARKLINE_CAPACITY = 60;

/**
 * Fixed-capacity ring buffer of the last `capacity` samples pushed. Once
 * full, each `push` overwrites the oldest sample in place (an `O(1)`
 * write, no array shifting) — `values()` still returns them in
 * chronological order (oldest first) regardless of where the internal
 * write cursor currently sits, so callers never see wraparound as
 * anything but "the oldest point fell off, a new one appended".
 */
export class RollingBuffer {
  private readonly capacity: number;
  private readonly buf: number[];
  private writeIndex = 0;
  private count = 0;

  constructor(capacity: number = SPARKLINE_CAPACITY) {
    if (capacity < 1) {
      throw new RangeError(`RollingBuffer capacity must be >= 1, got ${capacity}`);
    }
    this.capacity = capacity;
    this.buf = new Array<number>(capacity).fill(0);
  }

  /** Append one sample, evicting the oldest once the buffer is full. */
  push(value: number): void {
    this.buf[this.writeIndex] = value;
    this.writeIndex = (this.writeIndex + 1) % this.capacity;
    this.count = Math.min(this.count + 1, this.capacity);
  }

  /** Samples in chronological order (oldest first), length `<= capacity` —
   * shorter than `capacity` only until the buffer has been filled once. */
  values(): number[] {
    if (this.count < this.capacity) {
      // Not wrapped yet: everything written so far starts at index 0.
      return this.buf.slice(0, this.count);
    }
    // Wrapped: the oldest sample is the one `writeIndex` is about to
    // overwrite next.
    return [...this.buf.slice(this.writeIndex), ...this.buf.slice(0, this.writeIndex)];
  }
}

const NS_PER_US = 1_000;
const US_PER_MS = 1_000;

/**
 * Format a stage duration in nanoseconds as a human-scale string:
 * `"n/a"` for `<= 0` (the wasm `StageClock` reports exactly 0ns for a
 * stage that ran faster than its ms-resolution `Date.now()` clock could
 * observe — see `qr_lab_core::StageClock`'s doc comment — so 0 reads as "not
 * measurable" rather than "instant"), microseconds with one decimal below
 * 1ms, milliseconds with one decimal at or above it.
 *
 * Branches on the *rounded* µs value, not the raw one: a raw value like
 * 999.95µs rounds to "1000.0" under `toFixed(1)`, which would otherwise
 * take the `us < US_PER_MS` branch (999.95 < 1000) and print the
 * self-contradictory "1000.0 µs" instead of "1.0 ms". Rounding first
 * (half-up, matching `toFixed`'s own rounding) makes the unit boundary and
 * the displayed digits agree.
 */
export function formatNs(ns: number): string {
  if (ns <= 0) return "n/a";
  const us = ns / NS_PER_US;
  const usRounded = Math.round(us * 10) / 10;
  if (usRounded < US_PER_MS) {
    return `${usRounded.toFixed(1)} µs`;
  }
  return `${(usRounded / US_PER_MS).toFixed(1)} ms`;
}

/**
 * Format a millisecond duration (already-ms values, e.g. worker wall time
 * or main-thread round-trip) with two decimals, or `"n/a"` when no sample
 * has arrived yet (`null`/`undefined`) — unlike {@link formatNs}, `0` here
 * is a plausible real measurement, not evidence of clock underflow, so
 * only "no value at all" reads as n/a.
 */
export function formatMs(ms: number | null | undefined): string {
  if (ms == null) return "n/a";
  return `${ms.toFixed(2)} ms`;
}
