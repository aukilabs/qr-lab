import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { DecodedCode } from "../../scanner/types";
import type { GroundTruthCode } from "../groundtruth-types";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { refinedLayer } from "./refined";

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

const EMPTY_TIMINGS = {
  tiles_ns: 0,
  finders_ns: 0,
  triplets_ns: 0,
  version_ns: 0,
  alignment_ns: 0,
  sample_decode_ns: 0,
  refine_ns: 0,
};

function code(overrides: Partial<DecodedCode> = {}): DecodedCode {
  return {
    payload: "Q:test:0",
    payload_bytes: [],
    version: 1,
    ecc: "M",
    mirrored: false,
    dimension: 21,
    corners: [
      [0, 0],
      [20, 0],
      [20, 20],
      [0, 20],
    ],
    inverted: false,
    finder_indices: [0, 1, 2],
    refined_corners: null,
    ...overrides,
  };
}

function scanWith(codes: DecodedCode[]): NonNullable<OverlayContext["scan"]> {
  return {
    detections: { finders: [], triplets: [], codes, timings: EMPTY_TIMINGS, source_scale: 1 },
    trace: null,
  };
}

function truth(overrides: Partial<GroundTruthCode> = {}): GroundTruthCode {
  return {
    corners_px: [
      [0, 0],
      [20, 0],
      [20, 20],
      [0, 20],
    ],
    version: 1,
    module_size_px: 4,
    inverted: false,
    opaque_plate: true,
    payload: "Q:test:0",
    ...overrides,
  };
}

describe("refinedLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    refinedLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing for a code whose refined_corners is null", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = scanWith([code({ refined_corners: null })]);
    refinedLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws 4 whiskers, 4 hollow squares, and 4 crosshairs for one refined code", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = scanWith([
      code({
        refined_corners: [
          [0.5, 0.5],
          [20.5, 0.5],
          [20.5, 20.5],
          [0.5, 20.5],
        ],
      }),
    ]);
    refinedLayer.draw(ctx);

    // Whiskers: 4 x (beginPath + moveTo + lineTo + stroke).
    // Squares: 4 x strokeRect (no beginPath needed).
    // Crosshairs: 4 x (beginPath + moveTo + lineTo + moveTo + lineTo + stroke).
    expect(fake.callsNamed("strokeRect")).toHaveLength(4);
    expect(fake.callsNamed("moveTo")).toHaveLength(4 + 4 * 2);
    expect(fake.callsNamed("lineTo")).toHaveLength(4 + 4 * 2);
    expect(fake.callsNamed("stroke")).toHaveLength(4 + 4);
    expect(fake.callsNamed("fillText")).toHaveLength(0);
  });

  it("scales refined (source-px) corners by workingScale but not coarse corners", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.workingScale = 0.5; // e.g. source 1280 -> working 640
    ctx.scan = scanWith([
      code({
        corners: [
          [10, 10],
          [30, 10],
          [30, 30],
          [10, 30],
        ],
        refined_corners: [
          [20, 20],
          [60, 20],
          [60, 60],
          [20, 60],
        ],
      }),
    ]);
    refinedLayer.draw(ctx);

    // Whisker 0: moveTo(coarse[0]) -> lineTo(refined[0] * workingScale).
    const [firstMoveTo] = fake.callsNamed("moveTo");
    expect(firstMoveTo!.args).toEqual([10, 10]);
    const [firstLineTo] = fake.callsNamed("lineTo");
    expect(firstLineTo!.args).toEqual([10, 10]); // refined [20,20] * 0.5

    // Coarse hollow square is centered on the UNSCALED coarse corner.
    const [firstRect] = fake.callsNamed("strokeRect");
    expect(firstRect!.args[0]).toBeCloseTo(10 - 4, 9); // SQUARE_HALF = 4
    expect(firstRect!.args[1]).toBeCloseTo(10 - 4, 9);
  });

  it("draws a per-corner source-px error label when a matching-payload ground truth exists", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.groundTruth = [truth({ payload: "Q:test:0", corners_px: [[3, 4], [23, 4], [23, 24], [3, 24]] })];
    ctx.scan = scanWith([
      code({
        payload: "Q:test:0",
        refined_corners: [
          [0, 0],
          [20, 0],
          [20, 20],
          [0, 20],
        ],
      }),
    ]);
    refinedLayer.draw(ctx);

    const texts = fake.callsNamed("fillText");
    expect(texts).toHaveLength(4);
    // Corner 0: refined [0,0] vs truth [3,4] -> error = 5.
    expect(texts[0]!.args[0]).toBe("5.00px");
  });

  it("does not draw error labels when no ground-truth payload matches", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.groundTruth = [truth({ payload: "Q:other:0" })];
    ctx.scan = scanWith([
      code({
        payload: "Q:test:0",
        refined_corners: [
          [0, 0],
          [20, 0],
          [20, 20],
          [0, 20],
        ],
      }),
    ]);
    refinedLayer.draw(ctx);
    expect(fake.callsNamed("fillText")).toHaveLength(0);
  });

  it("draws primitives per code across multiple decoded codes", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = scanWith([
      code({
        payload: "Q:a:0",
        refined_corners: [
          [0, 0],
          [20, 0],
          [20, 20],
          [0, 20],
        ],
      }),
      code({
        payload: "Q:b:0",
        refined_corners: [
          [100, 100],
          [120, 100],
          [120, 120],
          [100, 120],
        ],
      }),
    ]);
    refinedLayer.draw(ctx);
    expect(fake.callsNamed("strokeRect")).toHaveLength(8);
  });
});
