import { describe, expect, it } from "vitest";
import { workingScaleFor } from "./scaling";

describe("workingScaleFor", () => {
  it("returns scanWidth / sourceWidth", () => {
    expect(workingScaleFor(1920, 1280)).toBeCloseTo(1280 / 1920, 10);
  });

  it("returns 1 when the source is already at working resolution (no downscale)", () => {
    expect(workingScaleFor(1280, 1280)).toBe(1);
  });

  it("returns a factor > 1 when scanWidth exceeds sourceWidth (defensive; shouldn't happen in practice)", () => {
    expect(workingScaleFor(100, 200)).toBe(2);
  });

  it("returns 1 (identity) for a non-positive sourceWidth instead of dividing by zero", () => {
    expect(workingScaleFor(0, 1280)).toBe(1);
    expect(workingScaleFor(-10, 1280)).toBe(1);
  });
});
