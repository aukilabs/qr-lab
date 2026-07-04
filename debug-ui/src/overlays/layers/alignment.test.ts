import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { alignmentLayer } from "./alignment";

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

describe("alignmentLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    alignmentLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when scan.trace is null", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [],
        timings: {
          tiles_ns: 0,
          finders_ns: 0,
          triplets_ns: 0,
          version_ns: 0,
          alignment_ns: 0,
          sample_decode_ns: 0,
        },
        source_scale: 1,
      },
      trace: null,
    };
    alignmentLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when trace.alignment is empty (v1 has no alignment patterns)", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [],
        timings: {
          tiles_ns: 0,
          finders_ns: 0,
          triplets_ns: 0,
          version_ns: 0,
          alignment_ns: 0,
          sample_decode_ns: 0,
        },
        source_scale: 1,
      },
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
    alignmentLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws a predicted × only when found is null", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [],
        timings: {
          tiles_ns: 0,
          finders_ns: 0,
          triplets_ns: 0,
          version_ns: 0,
          alignment_ns: 0,
          sample_decode_ns: 0,
        },
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        alignment: [{ predicted: [10, 20], found: null }],
        sample_regions: [],
        bits: null,
      },
    };
    alignmentLayer.draw(ctx);

    expect(fake.callsNamed("beginPath")).toHaveLength(1); // the × only
    expect(fake.callsNamed("moveTo")).toHaveLength(2);
    expect(fake.callsNamed("lineTo")).toHaveLength(2);
    expect(fake.callsNamed("stroke")).toHaveLength(1);
    expect(fake.callsNamed("arc")).toHaveLength(0);
    expect(fake.callsNamed("fill")).toHaveLength(0);
  });

  it("draws both the predicted × and a found ● when found is present", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [],
        timings: {
          tiles_ns: 0,
          finders_ns: 0,
          triplets_ns: 0,
          version_ns: 0,
          alignment_ns: 0,
          sample_decode_ns: 0,
        },
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        alignment: [
          { predicted: [10, 20], found: [10.5, 20.5] },
          { predicted: [30, 40], found: null },
        ],
        sample_regions: [],
        bits: null,
      },
    };
    alignmentLayer.draw(ctx);

    // 2 entries: 2 ×'s (beginPath+stroke each) + 1 found ● (beginPath+arc+fill).
    expect(fake.callsNamed("beginPath")).toHaveLength(3);
    expect(fake.callsNamed("stroke")).toHaveLength(2);
    expect(fake.callsNamed("arc")).toHaveLength(1);
    expect(fake.callsNamed("fill")).toHaveLength(1);

    const [arcCall] = fake.callsNamed("arc");
    expect(arcCall!.args[0]).toBeCloseTo(10.5, 9);
    expect(arcCall!.args[1]).toBeCloseTo(20.5, 9);
  });
});
