import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { GroundTruthCode } from "../groundtruth-types";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { groundtruthLayer } from "./groundtruth";

function baseContext(fake: ReturnType<typeof createFakeCanvas>): OverlayContext {
  return {
    ctx: fake.ctx,
    view: identity,
    scan: null,
    groundTruth: null,
    imageSize: [64, 32],
    workingScale: 1,
  };
}

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

describe("groundtruthLayer", () => {
  it("draws nothing when groundTruth is null", () => {
    const fake = createFakeCanvas();
    groundtruthLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when groundTruth is an empty array", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.groundTruth = [];
    groundtruthLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws a closed quad + 3 finder-center dots per code", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.groundTruth = [code()];
    groundtruthLayer.draw(ctx);

    // Quad: 1 path (moveTo + 3 lineTo + closePath) + 1 stroke.
    // Dots: 3 x (beginPath + arc + fill).
    expect(fake.callsNamed("beginPath")).toHaveLength(1 + 3);
    expect(fake.callsNamed("moveTo")).toHaveLength(1);
    expect(fake.callsNamed("lineTo")).toHaveLength(3);
    expect(fake.callsNamed("closePath")).toHaveLength(1);
    expect(fake.callsNamed("stroke")).toHaveLength(1);
    expect(fake.callsNamed("arc")).toHaveLength(3);
    expect(fake.callsNamed("fill")).toHaveLength(3);
  });

  it("scales corners_px and finder centers by workingScale before projecting", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.workingScale = 0.5; // e.g. source 1280 -> working 640
    ctx.groundTruth = [code()];
    groundtruthLayer.draw(ctx);

    const [moveTo] = fake.callsNamed("moveTo");
    // corners_px[0] = [0,0] -> scaled [0,0] -> screen [0,0] (identity view).
    expect(moveTo!.args).toEqual([0, 0]);

    const [firstArc] = fake.callsNamed("arc");
    // TL finder center at source px [35,35] (see
    // expected-finder-centers.test.ts) -> working px [17.5, 17.5].
    expect(firstArc!.args[0]).toBeCloseTo(17.5, 9);
    expect(firstArc!.args[1]).toBeCloseTo(17.5, 9);
  });

  it("draws primitives for each code when groundTruth has multiple entries", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.groundTruth = [code(), code({ corners_px: [[300, 300], [510, 300], [510, 510], [300, 510]] })];
    groundtruthLayer.draw(ctx);
    expect(fake.callsNamed("stroke")).toHaveLength(2);
    expect(fake.callsNamed("fill")).toHaveLength(6);
  });
});
