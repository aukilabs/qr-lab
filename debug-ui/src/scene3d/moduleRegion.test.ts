import { describe, expect, it } from "vitest";
import { moduleRegionLocalCorners, moduleRegionLocalCornersArray } from "./moduleRegion";

describe("moduleRegionLocalCorners", () => {
  it("insets each edge by quiet/(dim+2*quiet) of the physical size", () => {
    // dim=21 (v1), quiet=4, physicalSize=1: total=29, inset=4/29.
    const c = moduleRegionLocalCorners(21, 4, 1);
    const inset = 4 / 29;
    const inner = 0.5 - inset;
    expect(c.tl).toEqual([-inner, inner, 0]);
    expect(c.tr).toEqual([inner, inner, 0]);
    expect(c.br).toEqual([inner, -inner, 0]);
    expect(c.bl).toEqual([-inner, -inner, 0]);
  });

  it("scales linearly with physicalSize", () => {
    const c1 = moduleRegionLocalCorners(25, 4, 1);
    const c2 = moduleRegionLocalCorners(25, 4, 0.3);
    for (const key of ["tl", "tr", "br", "bl"] as const) {
      expect(c2[key][0]).toBeCloseTo(c1[key][0] * 0.3, 12);
      expect(c2[key][1]).toBeCloseTo(c1[key][1] * 0.3, 12);
      expect(c2[key][2]).toBe(0);
    }
  });

  it("shrinks toward zero as quiet grows relative to dim (larger margin)", () => {
    const small = moduleRegionLocalCorners(21, 4, 1);
    const large = moduleRegionLocalCorners(21, 20, 1); // huge margin
    expect(large.tr[0]).toBeLessThan(small.tr[0]);
    expect(large.tr[0]).toBeGreaterThan(0);
  });

  it("degenerates to the full plane extent when quiet=0", () => {
    const c = moduleRegionLocalCorners(21, 0, 2);
    expect(c.tl).toEqual([-1, 1, 0]);
    expect(c.tr).toEqual([1, 1, 0]);
    expect(c.br).toEqual([1, -1, 0]);
    expect(c.bl).toEqual([-1, -1, 0]);
  });

  it.each([
    ["dim", () => moduleRegionLocalCorners(0, 4, 1)],
    ["dim negative", () => moduleRegionLocalCorners(-5, 4, 1)],
    ["quiet", () => moduleRegionLocalCorners(21, -1, 1)],
    ["physicalSize", () => moduleRegionLocalCorners(21, 4, 0)],
  ])("rejects invalid %s", (_label, fn) => {
    expect(fn).toThrow(RangeError);
  });

  it("array form matches the object form in TL,TR,BR,BL order", () => {
    const obj = moduleRegionLocalCorners(29, 4, 0.15);
    const arr = moduleRegionLocalCornersArray(29, 4, 0.15);
    expect(arr).toEqual([obj.tl, obj.tr, obj.br, obj.bl]);
  });
});
