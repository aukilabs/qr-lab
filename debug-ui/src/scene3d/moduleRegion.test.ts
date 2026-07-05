import { describe, expect, it } from "vitest";
import {
  moduleRegionLocalCorners,
  moduleRegionLocalCornersArray,
  moduleRegionPhysicalSize,
} from "./moduleRegion";

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

describe("moduleRegionPhysicalSize", () => {
  it("matches 2x the local-corner half-extent derived by moduleRegionLocalCorners", () => {
    // moduleRegionLocalCorners(21, 4, 1).tr[0] is the module-region's
    // positive-X half-extent in local units; the full module-region side
    // length must be exactly double that (a square region centered at 0).
    const c = moduleRegionLocalCorners(21, 4, 1);
    expect(moduleRegionPhysicalSize(21, 4, 1)).toBeCloseTo(2 * c.tr[0], 12);
  });

  it("scales linearly with physicalSize", () => {
    const a = moduleRegionPhysicalSize(29, 4, 0.15);
    const b = moduleRegionPhysicalSize(29, 4, 0.3);
    expect(b).toBeCloseTo(a * 2, 12);
  });

  it("degenerates to physicalSize itself when quiet=0 (no margin to subtract)", () => {
    expect(moduleRegionPhysicalSize(21, 0, 0.2)).toBeCloseTo(0.2, 12);
  });

  it("shrinks as quiet grows relative to dim", () => {
    const small = moduleRegionPhysicalSize(21, 4, 1);
    const large = moduleRegionPhysicalSize(21, 20, 1);
    expect(large).toBeLessThan(small);
    expect(large).toBeGreaterThan(0);
  });

  it.each([
    ["dim", () => moduleRegionPhysicalSize(0, 4, 1)],
    ["quiet", () => moduleRegionPhysicalSize(21, -1, 1)],
    ["physicalSize", () => moduleRegionPhysicalSize(21, 4, 0)],
  ])("rejects invalid %s", (_label, fn) => {
    expect(fn).toThrow(RangeError);
  });
});
