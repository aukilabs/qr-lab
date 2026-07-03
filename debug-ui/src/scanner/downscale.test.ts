import { describe, expect, it } from "vitest";
import { downscaleRgba } from "./downscale";

/** Build a flat RGBA buffer where pixel `i` (row-major) has R=i so tests
 * can identify exactly which source pixel survived the resample. */
function markedRgba(width: number, height: number): Uint8ClampedArray {
  const out = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < width * height; i++) {
    out[i * 4 + 0] = i; // R = source pixel index
    out[i * 4 + 1] = 100 + i;
    out[i * 4 + 2] = 200 + i;
    out[i * 4 + 3] = 255;
  }
  return out;
}

function pixelAt(
  rgba: Uint8ClampedArray,
  width: number,
  x: number,
  y: number,
): [number, number, number, number] {
  const idx = (y * width + x) * 4;
  return [rgba[idx]!, rgba[idx + 1]!, rgba[idx + 2]!, rgba[idx + 3]!];
}

describe("downscaleRgba", () => {
  it("downscales 4x2 to 2x1 with exact nearest-neighbor source pixels", () => {
    const src = markedRgba(4, 2);
    const { rgba, width, height } = downscaleRgba(src, 4, 2, 2);

    expect(width).toBe(2);
    expect(height).toBe(1);
    expect(rgba.length).toBe(2 * 1 * 4);

    // dstW=2, dstH=1 → srcX = floor(x*4/2), srcY = floor(y*2/1)
    // x=0 -> srcX=0, x=1 -> srcX=2; y=0 -> srcY=0
    expect(pixelAt(rgba, 2, 0, 0)).toEqual(pixelAt(src, 4, 0, 0));
    expect(pixelAt(rgba, 2, 1, 0)).toEqual(pixelAt(src, 4, 2, 0));
  });

  it("is an identity passthrough (same buffer) when maxDim <= 0", () => {
    const src = markedRgba(5, 3);
    const result = downscaleRgba(src, 5, 3, 0);
    expect(result.rgba).toBe(src);
    expect(result.width).toBe(5);
    expect(result.height).toBe(3);

    const negative = downscaleRgba(src, 5, 3, -10);
    expect(negative.rgba).toBe(src);
  });

  it("is an identity passthrough (same buffer) when max(w,h) <= maxDim", () => {
    const src = markedRgba(4, 2);
    const exact = downscaleRgba(src, 4, 2, 4);
    expect(exact.rgba).toBe(src);
    expect(exact.width).toBe(4);
    expect(exact.height).toBe(2);

    const larger = downscaleRgba(src, 4, 2, 100);
    expect(larger.rgba).toBe(src);
  });

  it("matches the production rounding formula round(dim * maxDim / max(w,h))", () => {
    // w=7,h=5,maxDim=3 -> longest=7: newW=round(7*3/7)=3, newH=round(5*3/7)=round(2.142857)=2
    const a = downscaleRgba(markedRgba(7, 5), 7, 5, 3);
    expect(a.width).toBe(Math.round((7 * 3) / 7));
    expect(a.height).toBe(Math.round((5 * 3) / 7));
    expect(a.width).toBe(3);
    expect(a.height).toBe(2);

    // Half-integer boundary: w=2,h=1,maxDim=1 -> longest=2:
    // newW=round(2*1/2)=round(1)=1, newH=round(1*1/2)=round(0.5)=1 (round-half-up)
    const b = downscaleRgba(markedRgba(2, 1), 2, 1, 1);
    expect(b.width).toBe(1);
    expect(b.height).toBe(1);
  });
});
