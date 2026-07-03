import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { tripletsLayer } from "./triplets";

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

describe("tripletsLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    tripletsLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when there are no triplet candidates", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], timings: { tiles_ns: 0, finders_ns: 0, triplets_ns: 0 } },
      trace: null,
    };
    tripletsLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws leg lines, TL/TR/BL markers, and a dim label per triplet", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [
          {
            tl: [100, 100],
            tr: [156, 100],
            bl: [100, 156],
            module: 4,
            dimension: 21,
            snap_error: 0.4,
            inverted: false,
          },
        ],
        timings: { tiles_ns: 0, finders_ns: 0, triplets_ns: 0 },
      },
      trace: null,
    };
    tripletsLayer.draw(ctx);

    // legs (1 path, 2 segments) + TL square (strokeRect) + TR triangle (1
    // closed path) + BL circle (1 arc path) + text label.
    expect(fake.callsNamed("beginPath")).toHaveLength(3); // legs, TR triangle, BL circle
    expect(fake.callsNamed("moveTo")).toHaveLength(2 + 1); // 2 leg moves + 1 triangle move
    expect(fake.callsNamed("lineTo")).toHaveLength(2 + 2); // 2 legs + 2 triangle edges
    expect(fake.callsNamed("closePath")).toHaveLength(1); // triangle
    expect(fake.callsNamed("strokeRect")).toHaveLength(1); // TL square marker
    expect(fake.callsNamed("arc")).toHaveLength(1); // BL circle marker
    expect(fake.callsNamed("stroke")).toHaveLength(3); // legs, triangle, circle
    expect(fake.callsNamed("fillText")).toHaveLength(1);

    const [label] = fake.callsNamed("fillText");
    expect(label!.args[0]).toBe("dim=21 (±0.40)");
  });

  it("draws two triplets' worth of primitives for two candidates", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    const triplet = {
      tl: [0, 0] as [number, number],
      tr: [56, 0] as [number, number],
      bl: [0, 56] as [number, number],
      module: 4,
      dimension: 21,
      snap_error: 0.1,
      inverted: false,
    };
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [triplet, { ...triplet, tl: [200, 200] }],
        timings: { tiles_ns: 0, finders_ns: 0, triplets_ns: 0 },
      },
      trace: null,
    };
    tripletsLayer.draw(ctx);
    expect(fake.callsNamed("fillText")).toHaveLength(2);
    expect(fake.callsNamed("strokeRect")).toHaveLength(2);
  });
});
