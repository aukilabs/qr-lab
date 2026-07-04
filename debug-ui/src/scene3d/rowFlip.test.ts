import { describe, expect, it } from "vitest";
import { flipRowsRgba } from "./rowFlip";

function makeRows(rows: number[][]): Uint8ClampedArray {
  // Each row is 4 R-values (one per pixel of a width-4-ish test, using G/B/A
  // filled predictably so the whole row is identifiable at a glance).
  const flat: number[] = [];
  for (const row of rows) {
    for (const v of row) flat.push(v, v + 1, v + 2, 255);
  }
  return new Uint8ClampedArray(flat);
}

describe("flipRowsRgba", () => {
  it("reverses row order for a 2x2 image", () => {
    const rgba = makeRows([
      [10, 20], // row 0 (top)
      [30, 40], // row 1 (bottom)
    ]);
    const flipped = flipRowsRgba(rgba, 2, 2);
    const expected = makeRows([
      [30, 40],
      [10, 20],
    ]);
    expect(Array.from(flipped)).toEqual(Array.from(expected));
  });

  it("is its own inverse", () => {
    const rgba = makeRows([
      [1, 2, 3],
      [4, 5, 6],
      [7, 8, 9],
    ]);
    const twice = flipRowsRgba(flipRowsRgba(rgba, 3, 3), 3, 3);
    expect(Array.from(twice)).toEqual(Array.from(rgba));
  });

  it("leaves a single-row image unchanged", () => {
    const rgba = makeRows([[1, 2, 3]]);
    const flipped = flipRowsRgba(rgba, 3, 1);
    expect(Array.from(flipped)).toEqual(Array.from(rgba));
  });

  it("does not mutate the input buffer", () => {
    const rgba = makeRows([
      [10, 20],
      [30, 40],
    ]);
    const before = Array.from(rgba);
    flipRowsRgba(rgba, 2, 2);
    expect(Array.from(rgba)).toEqual(before);
  });

  it("throws on a buffer size that doesn't match width*height*4", () => {
    const rgba = new Uint8ClampedArray(10);
    expect(() => flipRowsRgba(rgba, 2, 2)).toThrow(RangeError);
  });
});
