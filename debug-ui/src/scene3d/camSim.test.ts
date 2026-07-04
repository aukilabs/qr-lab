import { describe, expect, it } from "vitest";
import { applyExposureOffset, applyGaussianNoise, mulberry32 } from "./camSim";

function flatRgba(n: number, value: number): Uint8ClampedArray {
  const out = new Uint8ClampedArray(n * 4);
  out.fill(value);
  for (let i = 3; i < out.length; i += 4) out[i] = 255; // alpha
  return out;
}

describe("mulberry32", () => {
  it("is deterministic for a given seed", () => {
    const a = mulberry32(42);
    const b = mulberry32(42);
    const seqA = Array.from({ length: 10 }, () => a());
    const seqB = Array.from({ length: 10 }, () => b());
    expect(seqA).toEqual(seqB);
  });

  it("returns values in [0, 1)", () => {
    const rand = mulberry32(1);
    for (let i = 0; i < 1000; i++) {
      const v = rand();
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(1);
    }
  });

  it("produces different sequences for different seeds", () => {
    const a = mulberry32(1);
    const b = mulberry32(2);
    expect(a()).not.toBe(b());
  });
});

describe("applyExposureOffset", () => {
  it("shifts every RGB channel by the offset, clamped, leaving alpha untouched", () => {
    const rgba = new Uint8ClampedArray([10, 20, 30, 128, 250, 5, 0, 64]);
    const out = applyExposureOffset(rgba, 20);
    expect(Array.from(out)).toEqual([30, 40, 50, 128, 255 /* 250+20 clamped */, 25, 20, 64]);
  });

  it("clamps negative offsets at 0", () => {
    const rgba = new Uint8ClampedArray([10, 5, 0, 255]);
    const out = applyExposureOffset(rgba, -20);
    expect(Array.from(out)).toEqual([0, 0, 0, 255]);
  });

  it("returns an unmodified copy for offset 0", () => {
    const rgba = new Uint8ClampedArray([1, 2, 3, 4]);
    const out = applyExposureOffset(rgba, 0);
    expect(Array.from(out)).toEqual([1, 2, 3, 4]);
    expect(out).not.toBe(rgba);
  });

  it("does not mutate the input", () => {
    const rgba = new Uint8ClampedArray([10, 20, 30, 255]);
    const before = Array.from(rgba);
    applyExposureOffset(rgba, 50);
    expect(Array.from(rgba)).toEqual(before);
  });
});

describe("applyGaussianNoise", () => {
  it("is a no-op copy for sigma <= 0", () => {
    const rgba = flatRgba(4, 100);
    const out = applyGaussianNoise(rgba, 0, 7);
    expect(Array.from(out)).toEqual(Array.from(rgba));
    expect(out).not.toBe(rgba);
  });

  it("is deterministic for a given seed", () => {
    const rgba = flatRgba(64, 128);
    const a = applyGaussianNoise(rgba, 5, 123);
    const b = applyGaussianNoise(rgba, 5, 123);
    expect(Array.from(a)).toEqual(Array.from(b));
  });

  it("produces a different result for a different seed", () => {
    const rgba = flatRgba(64, 128);
    const a = applyGaussianNoise(rgba, 5, 123);
    const b = applyGaussianNoise(rgba, 5, 456);
    expect(Array.from(a)).not.toEqual(Array.from(b));
  });

  it("leaves alpha untouched", () => {
    const rgba = flatRgba(16, 100);
    const out = applyGaussianNoise(rgba, 8, 1);
    for (let i = 3; i < out.length; i += 4) {
      expect(out[i]).toBe(255);
    }
  });

  it("keeps the empirical standard deviation close to sigma over a large sample", () => {
    // Statistical sanity check (not a byte-exact one): with enough
    // samples, the sample stddev of (noisy - original) should converge
    // close to the requested sigma. 20000 samples keeps the test's own
    // false-failure rate low without being slow.
    const n = 20000;
    const rgba = flatRgba(n, 128);
    const sigma = 6;
    const out = applyGaussianNoise(rgba, sigma, 999);
    const diffs: number[] = [];
    for (let i = 0; i < out.length; i += 4) {
      diffs.push(out[i]! - 128); // R channel only (channels are i.i.d.)
    }
    const mean = diffs.reduce((a, b) => a + b, 0) / diffs.length;
    const variance = diffs.reduce((a, b) => a + (b - mean) ** 2, 0) / diffs.length;
    const stddev = Math.sqrt(variance);
    expect(Math.abs(mean)).toBeLessThan(0.5);
    expect(stddev).toBeGreaterThan(sigma * 0.85);
    expect(stddev).toBeLessThan(sigma * 1.15);
  });

  it("does not mutate the input", () => {
    const rgba = flatRgba(8, 128);
    const before = Array.from(rgba);
    applyGaussianNoise(rgba, 4, 1);
    expect(Array.from(rgba)).toEqual(before);
  });
});
