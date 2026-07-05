import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { DecodedCode } from "../../scanner/types";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { decodedLayer } from "./decoded";

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
    corner_refined: [false, false, false, false],
    ...overrides,
  };
}

describe("decodedLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    decodedLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws a payload + version/ecc/mirrored badge at each code's centroid", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [code({ payload: "Q:near_00:0", version: 1, ecc: "M", mirrored: false })],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: null,
    };
    decodedLayer.draw(ctx);

    const texts = fake.callsNamed("fillText");
    expect(texts).toHaveLength(2);
    expect(texts[0]!.args[0]).toBe("Q:near_00:0");
    expect(texts[1]!.args[0]).toBe("v1 M");
    // centroid of the 20x20 axis-aligned quad is (10,10); identity view.
    expect(texts[0]!.args[1]).toBeCloseTo(10, 9);
  });

  it("appends ' mirrored' to the badge when mirrored is true", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [code({ version: 7, ecc: "Q", mirrored: true })],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: null,
    };
    decodedLayer.draw(ctx);

    const texts = fake.callsNamed("fillText");
    expect(texts[1]!.args[0]).toBe("v7 Q mirrored");
  });

  it("draws no failed-attempt markers when trace is null", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS, source_scale: 1 },
      trace: null,
    };
    decodedLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws a red ✕ + outcome at the failed attempt's triplet TL, skipping decoded attempts", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [
          { tl: [5, 5], tr: [25, 5], bl: [5, 25], module: 4, dimension: 21, snap_error: 0, inverted: false, finder_indices: [0, 1, 2] },
        ],
        codes: [],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [
          {
            triplet_index: 0,
            dimension_est: 21,
            dimension_final: 21,
            timing_check: null,
            version_bits: null,
            alignment_found: 0,
            alignment_total: 0,
            oob_fraction: 0,
            refined_corner: false,
            outcome: "Content",
            rounds: ["parallelogram:failed_rs"],
          },
          {
            triplet_index: 0,
            dimension_est: 21,
            dimension_final: 21,
            timing_check: null,
            version_bits: null,
            alignment_found: 0,
            alignment_total: 0,
            oob_fraction: 0,
            refined_corner: false,
            outcome: "decoded",
            rounds: ["parallelogram:decoded"],
          },
        ],
        codes: [],
        alignment: [],
        sample_regions: [],
        bits: null,
        refine: null,
      },
    };
    decodedLayer.draw(ctx);

    // Only the FAILED attempt draws a marker; the "decoded" one is skipped.
    expect(fake.callsNamed("beginPath")).toHaveLength(1);
    expect(fake.callsNamed("moveTo")).toHaveLength(2);
    expect(fake.callsNamed("lineTo")).toHaveLength(2);
    expect(fake.callsNamed("stroke")).toHaveLength(1);
    const texts = fake.callsNamed("fillText");
    expect(texts).toHaveLength(1);
    expect(texts[0]!.args[0]).toBe("Content");
    // triplet.tl = [5,5] (identity view); label offset is
    // (tx + MARKER_HALF(5) + 3, ty + 4) = (13, 9).
    expect(texts[0]!.args[1]).toBeCloseTo(13, 9);
    expect(texts[0]!.args[2]).toBeCloseTo(9, 9);
  });

  it("skips an attempt whose triplet_index has no matching triplet", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [], timings: EMPTY_TIMINGS, source_scale: 1 },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [
          {
            triplet_index: 5,
            dimension_est: 21,
            dimension_final: 21,
            timing_check: null,
            version_bits: null,
            alignment_found: 0,
            alignment_total: 0,
            oob_fraction: 0,
            refined_corner: false,
            outcome: "Content",
            rounds: ["parallelogram:failed_rs"],
          },
        ],
        codes: [],
        alignment: [],
        sample_regions: [],
        bits: null,
        refine: null,
      },
    };
    decodedLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });
});
