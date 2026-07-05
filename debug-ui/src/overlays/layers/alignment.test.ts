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
          refine_ns: 0,
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
          refine_ns: 0,
        },
        source_scale: 1,
      },
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
          refine_ns: 0,
        },
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [],
        alignment: [{ predicted: [10, 20], found: null }],
        sample_regions: [],
        bits: null,
        refine: null,
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
          refine_ns: 0,
        },
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [],
        alignment: [
          { predicted: [10, 20], found: [10.5, 20.5] },
          { predicted: [30, 40], found: null },
        ],
        sample_regions: [],
        bits: null,
        refine: null,
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

  // Plan 5C: multi-code trace — a DECODED frame's alignment search lives on
  // `trace.codes[*].alignment` now; the legacy singular `trace.alignment`
  // is empty on every successful decode (failure-diagnosis only). Guards
  // the regression where this layer kept reading only the singular field
  // and silently drew nothing for every successful decode.
  it("draws from trace.codes[*].alignment on a decoded frame (singular alignment empty)", () => {
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
          refine_ns: 0,
        },
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [
          {
            code_index: 0,
            sample_regions: [],
            bits: { dim: 25, words: [] },
            alignment: [{ predicted: [10, 20], found: [10.5, 20.5] }],
          },
          {
            code_index: 1,
            sample_regions: [],
            bits: { dim: 25, words: [] },
            alignment: [{ predicted: [110, 120], found: null }],
          },
        ],
        // The decoded case: singular field is empty — must NOT cause an
        // early return.
        alignment: [],
        sample_regions: [],
        bits: null,
        refine: null,
      },
    };
    alignmentLayer.draw(ctx);

    // Code 0: 1 × (beginPath+stroke) + 1 found ● (beginPath+arc+fill);
    // code 1: 1 × only — BOTH codes' entries draw, not just one.
    expect(fake.callsNamed("beginPath")).toHaveLength(3);
    expect(fake.callsNamed("stroke")).toHaveLength(2);
    expect(fake.callsNamed("arc")).toHaveLength(1);
    expect(fake.callsNamed("fill")).toHaveLength(1);

    // Code 1's × is centered on its own predicted [110, 120].
    const moveTos = fake.callsNamed("moveTo");
    expect(moveTos[2]!.args[0]).toBeCloseTo(110 - 4, 9); // px - MARKER_HALF
    expect(moveTos[2]!.args[1]).toBeCloseTo(120 - 4, 9);
  });

  it("falls back to the singular trace.alignment only when trace.codes is empty (nothing decoded)", () => {
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
          refine_ns: 0,
        },
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        // Nothing decoded: `codes` empty, singular field carries the first
        // attempt's search (the Plan 4B Fix A failure-diagnosis shape).
        codes: [],
        alignment: [{ predicted: [50, 60], found: null }],
        sample_regions: [],
        bits: null,
        refine: null,
      },
    };
    alignmentLayer.draw(ctx);

    // The fallback entry's × draws (1 path, 2 segments, no ●).
    expect(fake.callsNamed("beginPath")).toHaveLength(1);
    expect(fake.callsNamed("stroke")).toHaveLength(1);
    expect(fake.callsNamed("arc")).toHaveLength(0);
    const [moveTo] = fake.callsNamed("moveTo");
    expect(moveTo!.args[0]).toBeCloseTo(50 - 4, 9);
    expect(moveTo!.args[1]).toBeCloseTo(60 - 4, 9);
  });
});
