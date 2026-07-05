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
  refine_ns: 0,
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
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS, source_scale: 1 },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [],
        alignment: [],
        sample_regions: [],
        bits: null,
        refine: null,
      },
    };
    samplegridLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws one closed 4-corner path per region, no per-module lines", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS, source_scale: 1 },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [],
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
        refine: null,
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
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS, source_scale: 1 },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [],
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
        refine: null,
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

  // Plan 5C: multi-code trace — every DECODED code's own regions must draw,
  // not just one. `trace.sample_regions` (the legacy singular,
  // failure-diagnosis-only field) is deliberately left populated here too,
  // to prove `trace.codes` wins outright rather than being merged/ignored.
  it("draws every decoded code's own regions from trace.codes, not just one", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    const region = (n: number) => ({
      module_rect: [0, 0, n, n] as [number, number, number, number],
      quad: [
        [0, 0],
        [n * 4, 0],
        [n * 4, n * 4],
        [0, n * 4],
      ] as [[number, number], [number, number], [number, number], [number, number]],
    });
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS, source_scale: 1 },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [
          { code_index: 0, sample_regions: [region(21)], bits: { dim: 21, words: [] }, alignment: [] },
          {
            code_index: 1,
            sample_regions: [region(25), region(10)],
            bits: { dim: 25, words: [] },
            alignment: [],
          },
        ],
        // Legacy failure-diagnosis field — must be ignored since `codes`
        // is non-empty.
        alignment: [],
        sample_regions: [region(99)],
        bits: null,
        refine: null,
      },
    };
    samplegridLayer.draw(ctx);

    // 1 region (code 0) + 2 regions (code 1) = 3 total, NOT the legacy
    // field's single (different-sized) region.
    expect(fake.callsNamed("beginPath")).toHaveLength(3);
    expect(fake.callsNamed("closePath")).toHaveLength(3);
    expect(fake.callsNamed("stroke")).toHaveLength(3);
  });
});
