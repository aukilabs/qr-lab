import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { tilesLayer } from "./tiles";

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

describe("tilesLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    tilesLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when scan.trace is null", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
      }, source_scale: 1 },
      trace: null,
    };
    tilesLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when trace.tiles is null", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
      }, source_scale: 1 },
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
    tilesLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws one heatmap fillRect per tile, plus hatching only for skip tiles", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    // 2x1 tile grid: tile 0 kept, tile 1 skipped.
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
      }, source_scale: 1 },
      trace: {
        tiles: { tiles_x: 2, tiles_y: 1, thresholds: [100, 150], skip: [false, true] },
        finders: [],
        triplets: [],
        attempts: [],
        alignment: [],
        sample_regions: [],
        bits: null,
      },
    };
    tilesLayer.draw(ctx);

    expect(fake.callsNamed("fillRect")).toHaveLength(2);
    // Hatching for the one skip tile: a beginPath'd 2-line X, one stroke.
    expect(fake.callsNamed("beginPath")).toHaveLength(1);
    expect(fake.callsNamed("moveTo")).toHaveLength(2);
    expect(fake.callsNamed("lineTo")).toHaveLength(2);
    expect(fake.callsNamed("stroke")).toHaveLength(1);
  });

  it("projects tile rects through imageToScreen (scaled by view)", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.view = { scale: 2, tx: 10, ty: 5 };
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
      }, source_scale: 1 },
      trace: {
        tiles: { tiles_x: 1, tiles_y: 1, thresholds: [80], skip: [false] },
        finders: [],
        triplets: [],
        attempts: [],
        alignment: [],
        sample_regions: [],
        bits: null,
      },
    };
    tilesLayer.draw(ctx);

    const [fillRect] = fake.callsNamed("fillRect");
    // Tile (0,0) spans image px [0,16]x[0,16] -> screen [10,42]x[5,37] at
    // scale=2, tx=10, ty=5.
    expect(fillRect!.args).toEqual([10, 5, 32, 32]);
  });
});
