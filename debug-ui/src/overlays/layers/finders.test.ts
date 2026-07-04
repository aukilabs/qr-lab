import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { findersLayer } from "./finders";

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

describe("findersLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    findersLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when there are no finder candidates", () => {
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
        refine_ns: 0,
      }, source_scale: 1 },
      trace: null,
    };
    findersLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws a circle + hits label per finder candidate", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [
          { x: 10, y: 10, module: 4, inverted: false, hits: 3 },
          { x: 20, y: 30, module: 5, inverted: true, hits: 5 },
        ],
        triplets: [],
        codes: [],
        timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
        refine_ns: 0,
        },
        source_scale: 1,
      },
      trace: null,
    };
    findersLayer.draw(ctx);

    expect(fake.callsNamed("beginPath")).toHaveLength(2);
    expect(fake.callsNamed("arc")).toHaveLength(2);
    expect(fake.callsNamed("stroke")).toHaveLength(2);
    expect(fake.callsNamed("fillText")).toHaveLength(2);

    const [firstFillText] = fake.callsNamed("fillText");
    expect(firstFillText!.args[0]).toBe("3");
    const [secondFillText] = fake.callsNamed("fillText").slice(1);
    expect(secondFillText!.args[0]).toBe("5");
  });

  it("radius is 3.5*module scaled by view.scale; center via imageToScreen", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.view = { scale: 2, tx: 1, ty: -3 };
    ctx.scan = {
      detections: {
        finders: [{ x: 10, y: 20, module: 4, inverted: false, hits: 1 }],
        triplets: [],
        codes: [],
        timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
        refine_ns: 0,
        },
        source_scale: 1,
      },
      trace: null,
    };
    findersLayer.draw(ctx);

    const [arcCall] = fake.callsNamed("arc");
    // center: imageToScreen({scale:2,tx:1,ty:-3}, [10,20]) = [21, 37]
    // radius: 3.5*4*2 = 28
    expect(arcCall!.args[0]).toBeCloseTo(21, 9);
    expect(arcCall!.args[1]).toBeCloseTo(37, 9);
    expect(arcCall!.args[2]).toBeCloseTo(28, 9);
  });
});
