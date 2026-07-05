import { describe, expect, it } from "vitest";
import { intrinsicsFromFov } from "./intrinsics";

describe("intrinsicsFromFov", () => {
  it("derives fy=100 for a 90deg vertical fov at height 200 (tan(45deg)=1)", () => {
    // Same identity projection.test.ts's cameraAt(fov=90) relies on: at
    // any depth d, the visible half-extent is d*tan(fovY/2) = d*tan(45deg)
    // = d. fy = (h/2)/tan(fovY/2) = 100/1 = 100.
    const k = intrinsicsFromFov(90, 200, 200);
    expect(k.fy).toBeCloseTo(100, 9);
    expect(k.fx).toBe(k.fy);
  });

  it("sets cx/cy to (w-1)/2 and (h-1)/2 (pixel-centers-at-integers convention)", () => {
    const k = intrinsicsFromFov(60, 961, 601);
    expect(k.cx).toBeCloseTo(480, 9);
    expect(k.cy).toBeCloseTo(300, 9);
  });

  it("scales fy inversely with tan(fovY/2) for a fixed height", () => {
    const narrow = intrinsicsFromFov(30, 1000, 1000);
    const wide = intrinsicsFromFov(90, 1000, 1000);
    // A narrower fov means a longer effective focal length for the same
    // resolution (more px per degree of view).
    expect(narrow.fy).toBeGreaterThan(wide.fy);
  });

  it("scales fy linearly with height for a fixed fov", () => {
    const small = intrinsicsFromFov(50, 640, 640);
    const large = intrinsicsFromFov(50, 640, 1280);
    expect(large.fy).toBeCloseTo(small.fy * 2, 6);
  });

  it("fx always equals fy (square-pixel readback assumption)", () => {
    const k = intrinsicsFromFov(72.5, 1280, 1280);
    expect(k.fx).toBe(k.fy);
  });
});
