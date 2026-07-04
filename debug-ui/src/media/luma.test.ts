import { describe, expect, it } from "vitest";
import { lumaAt } from "./luma";

/** Build a tightly packed 2x2 RGBA buffer, opaque alpha, from 4 pixels
 * given in row-major (TL, TR, BL, BR) order. */
function makeRgba2x2(px: [number, number, number][]): Uint8ClampedArray {
  const out = new Uint8ClampedArray(2 * 2 * 4);
  px.forEach(([r, g, b], i) => {
    out[i * 4] = r;
    out[i * 4 + 1] = g;
    out[i * 4 + 2] = b;
    out[i * 4 + 3] = 255;
  });
  return out;
}

describe("lumaAt", () => {
  it("returns 255 for pure white and 0 for pure black", () => {
    const rgba = makeRgba2x2([
      [255, 255, 255],
      [0, 0, 0],
      [0, 0, 0],
      [0, 0, 0],
    ]);
    expect(lumaAt(rgba, 2, 2, 0, 0)).toBe(255);
    expect(lumaAt(rgba, 2, 2, 1, 0)).toBe(0);
  });

  it("matches the scanner's fixed-point formula exactly, not a rounded approximation", () => {
    // 77*200 + 150*100 + 29*50 + 128 = 15400 + 15000 + 1450 + 128 = 31978;
    // 31978 >> 8 = 124 (floor of 31978/256, i.e. truncating right-shift).
    const rgba = makeRgba2x2([
      [200, 100, 50],
      [0, 0, 0],
      [0, 0, 0],
      [0, 0, 0],
    ]);
    expect(lumaAt(rgba, 2, 2, 0, 0)).toBe(124);
  });

  it("floors fractional coordinates onto the pixel they fall inside", () => {
    const rgba = makeRgba2x2([
      [10, 10, 10],
      [200, 200, 200],
      [10, 10, 10],
      [10, 10, 10],
    ]);
    expect(lumaAt(rgba, 2, 2, 1.9, 0.999)).toBe(lumaAt(rgba, 2, 2, 1, 0));
  });

  it("returns null outside the buffer's bounds in every direction", () => {
    const rgba = makeRgba2x2([
      [1, 1, 1],
      [1, 1, 1],
      [1, 1, 1],
      [1, 1, 1],
    ]);
    expect(lumaAt(rgba, 2, 2, -1, 0)).toBeNull();
    expect(lumaAt(rgba, 2, 2, 0, -1)).toBeNull();
    expect(lumaAt(rgba, 2, 2, 2, 0)).toBeNull();
    expect(lumaAt(rgba, 2, 2, 0, 2)).toBeNull();
  });

  it("returns null for any coordinate when width/height are 0 (nothing loaded)", () => {
    expect(lumaAt(new Uint8ClampedArray(0), 0, 0, 0, 0)).toBeNull();
  });
});
