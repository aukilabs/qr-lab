import { describe, expect, it } from "vitest";
import type { GroundTruthCode } from "../groundtruth-types";
import { expectedFinderCenters } from "./expected-finder-centers";

function code(overrides: Partial<GroundTruthCode> = {}): GroundTruthCode {
  return {
    corners_px: [
      [0, 0],
      [210, 0],
      [210, 210],
      [0, 210],
    ],
    version: 1,
    module_size_px: 10,
    inverted: false,
    opaque_plate: true,
    payload: "Q:test:0",
    ...overrides,
  };
}

describe("expectedFinderCenters", () => {
  it("computes TL/TR/BL at the 3.5/n finder-center fraction for an axis-aligned v1 code", () => {
    // v1: n = 4*1+17 = 21. f = 3.5/21 = 1/6, g = 17.5/21 = 5/6. Corners
    // form a 210x210 axis-aligned square (10px/module), so this is the
    // same affine (parallelogram) branch as homography.test.ts's
    // "scales linearly" case — hand-computable exactly.
    const centers = expectedFinderCenters(code());
    expect(centers).toHaveLength(3);
    const [tl, tr, bl] = centers;
    expect(tl![0]).toBeCloseTo(35, 9); // 210/6
    expect(tl![1]).toBeCloseTo(35, 9);
    expect(tr![0]).toBeCloseTo(175, 9); // 210*5/6
    expect(tr![1]).toBeCloseTo(35, 9);
    expect(bl![0]).toBeCloseTo(35, 9);
    expect(bl![1]).toBeCloseTo(175, 9);
  });

  it("scales n with version (v7: n = 4*7+17 = 45)", () => {
    // Keep the same 210x210 quad but bump version; f/g shrink toward the
    // corner since a bigger code packs more modules into the same px span
    // in this synthetic example.
    const centers = expectedFinderCenters(code({ version: 7 }));
    const n = 45;
    const f = 3.5 / n;
    const expectedTl = 210 * f;
    expect(centers[0]![0]).toBeCloseTo(expectedTl, 9);
    expect(centers[0]![1]).toBeCloseTo(expectedTl, 9);
  });

  it("returns an empty array for a degenerate (collinear) quad", () => {
    const centers = expectedFinderCenters(
      code({
        corners_px: [
          [0, 0],
          [1, 0],
          [2, 0],
          [3, 0],
        ],
      }),
    );
    expect(centers).toEqual([]);
  });
});
