import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { samplegridLayer } from "./samplegrid";

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
};

describe("samplegridLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    samplegridLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when trace.sample_regions is empty", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        alignment: [],
        sample_regions: [],
        bits: null,
      },
    };
    samplegridLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws one closed 4-corner path per region, no per-module lines", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        alignment: [],
        sample_regions: [
          {
            module_rect: [0, 0, 21, 21],
            quad: [
              [0, 0],
              [84, 0],
              [84, 84],
              [0, 84],
            ],
          },
          {
            module_rect: [0, 0, 10, 10],
            quad: [
              [0, 0],
              [40, 0],
              [40, 40],
              [0, 40],
            ],
          },
        ],
        bits: null,
      },
    };
    samplegridLayer.draw(ctx);

    expect(fake.callsNamed("beginPath")).toHaveLength(2);
    expect(fake.callsNamed("moveTo")).toHaveLength(2);
    expect(fake.callsNamed("lineTo")).toHaveLength(3 * 2); // 3 lineTo per 4-corner quad
    expect(fake.callsNamed("closePath")).toHaveLength(2);
    expect(fake.callsNamed("stroke")).toHaveLength(2);
  });

  it("projects region quads through imageToScreen (scaled by view)", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.view = { scale: 2, tx: 10, ty: 5 };
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        alignment: [],
        sample_regions: [
          {
            module_rect: [0, 0, 21, 21],
            quad: [
              [0, 0],
              [84, 0],
              [84, 84],
              [0, 84],
            ],
          },
        ],
        bits: null,
      },
    };
    samplegridLayer.draw(ctx);

    const [moveTo] = fake.callsNamed("moveTo");
    // imageToScreen({scale:2,tx:10,ty:5}, [0,0]) = [10, 5]
    expect(moveTo!.args).toEqual([10, 5]);
    const lineTos = fake.callsNamed("lineTo");
    // imageToScreen(..., [84,0]) = [178, 5]
    expect(lineTos[0]!.args).toEqual([178, 5]);
  });
});
