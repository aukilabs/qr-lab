import { describe, expect, it } from "vitest";
import { formatMs, formatNs, RollingBuffer, SPARKLINE_CAPACITY } from "./timings-model";

describe("RollingBuffer", () => {
  it("returns pushed values in chronological order before it fills", () => {
    const buf = new RollingBuffer(4);
    buf.push(1);
    buf.push(2);
    buf.push(3);
    expect(buf.values()).toEqual([1, 2, 3]);
  });

  it("starts empty", () => {
    const buf = new RollingBuffer(4);
    expect(buf.values()).toEqual([]);
  });

  it("wraps around once capacity is exceeded, evicting the oldest sample first", () => {
    const buf = new RollingBuffer(4);
    for (let i = 1; i <= 4; i++) buf.push(i);
    expect(buf.values()).toEqual([1, 2, 3, 4]);

    // A 5th push overwrites the oldest (1); order stays chronological.
    buf.push(5);
    expect(buf.values()).toEqual([2, 3, 4, 5]);

    // Push well past capacity (multiple full wraps) — still just the last
    // `capacity` values, oldest first.
    for (let i = 6; i <= 11; i++) buf.push(i);
    expect(buf.values()).toEqual([8, 9, 10, 11]);
  });

  it("never returns more than capacity samples", () => {
    const buf = new RollingBuffer(3);
    for (let i = 0; i < 100; i++) buf.push(i);
    expect(buf.values()).toHaveLength(3);
    expect(buf.values()).toEqual([97, 98, 99]);
  });

  it("defaults to SPARKLINE_CAPACITY when no capacity is given", () => {
    const buf = new RollingBuffer();
    for (let i = 0; i < SPARKLINE_CAPACITY + 10; i++) buf.push(i);
    expect(buf.values()).toHaveLength(SPARKLINE_CAPACITY);
  });

  it("rejects a non-positive capacity", () => {
    expect(() => new RollingBuffer(0)).toThrow(RangeError);
    expect(() => new RollingBuffer(-1)).toThrow(RangeError);
  });
});

describe("formatNs", () => {
  it("reports n/a for zero (clock too coarse to observe the stage)", () => {
    expect(formatNs(0)).toBe("n/a");
  });

  it("reports n/a for a negative value (defensive, should not occur)", () => {
    expect(formatNs(-5)).toBe("n/a");
  });

  it("formats sub-millisecond durations in microseconds with one decimal", () => {
    expect(formatNs(1234)).toBe("1.2 µs");
  });

  it("formats millisecond-scale durations in milliseconds with one decimal", () => {
    expect(formatNs(5_600_000)).toBe("5.6 ms");
  });

  it("switches units exactly at the 1ms boundary", () => {
    expect(formatNs(999_000)).toBe("999.0 µs");
    expect(formatNs(1_000_000)).toBe("1.0 ms");
  });
});

describe("formatMs", () => {
  it("reports n/a for null or undefined", () => {
    expect(formatMs(null)).toBe("n/a");
    expect(formatMs(undefined)).toBe("n/a");
  });

  it("formats zero as a real measurement, not n/a", () => {
    expect(formatMs(0)).toBe("0.00 ms");
  });

  it("formats with two decimals", () => {
    expect(formatMs(12.3456)).toBe("12.35 ms");
  });
});
