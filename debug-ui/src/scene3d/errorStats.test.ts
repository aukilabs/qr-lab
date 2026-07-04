import { describe, expect, it } from "vitest";
import { RollingBuffer } from "../panels/timings-model";
import { cornerErrors, percentile, pointError, summarizeRolling } from "./errorStats";

describe("pointError", () => {
  it("computes Euclidean distance", () => {
    expect(pointError([0, 0], [3, 4])).toBeCloseTo(5, 10);
  });

  it("is 0 for identical points", () => {
    expect(pointError([1.5, -2.5], [1.5, -2.5])).toBe(0);
  });
});

describe("cornerErrors", () => {
  it("computes per-corner error and the mean of the four", () => {
    const refined: [[number, number], [number, number], [number, number], [number, number]] = [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ];
    const truth: [[number, number], [number, number], [number, number], [number, number]] = [
      [0, 3], // error 3
      [10, 4], // error 4
      [10, 10], // error 0
      [4, 10], // error 4
    ];
    const errs = cornerErrors(refined, truth);
    expect(errs.tl).toBeCloseTo(3, 10);
    expect(errs.tr).toBeCloseTo(4, 10);
    expect(errs.br).toBeCloseTo(0, 10);
    expect(errs.bl).toBeCloseTo(4, 10);
    expect(errs.mean).toBeCloseTo((3 + 4 + 0 + 4) / 4, 10);
  });
});

describe("percentile", () => {
  it("returns 0 for an empty array", () => {
    expect(percentile([], 95)).toBe(0);
  });

  it("returns the max at p=100 and min at p=0", () => {
    const values = [5, 1, 9, 3, 7];
    expect(percentile(values, 100)).toBe(9);
    expect(percentile(values, 0)).toBe(1);
  });

  it("picks the nearest-rank value for p95 on a known set", () => {
    // 1..20: ceil(0.95*20)=19th smallest (1-indexed) = 19.
    const values = Array.from({ length: 20 }, (_, i) => i + 1);
    expect(percentile(values, 95)).toBe(19);
  });

  it("does not mutate the input array", () => {
    const values = [5, 1, 9, 3, 7];
    const before = [...values];
    percentile(values, 50);
    expect(values).toEqual(before);
  });
});

describe("summarizeRolling", () => {
  it("returns zeros for an empty buffer", () => {
    const buf = new RollingBuffer(10);
    expect(summarizeRolling(buf)).toEqual({ mean: 0, p95: 0 });
  });

  it("computes mean and p95 over pushed samples", () => {
    const buf = new RollingBuffer(10);
    for (const v of [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]) buf.push(v);
    const summary = summarizeRolling(buf);
    expect(summary.mean).toBeCloseTo(5.5, 10);
    expect(summary.p95).toBe(10);
  });

  it("reflects eviction once the buffer wraps", () => {
    const buf = new RollingBuffer(3);
    for (const v of [100, 100, 100, 1, 2, 3]) buf.push(v); // first 3 evicted
    const summary = summarizeRolling(buf);
    expect(summary.mean).toBeCloseTo(2, 10);
  });
});
