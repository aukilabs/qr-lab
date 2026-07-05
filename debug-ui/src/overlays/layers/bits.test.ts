import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { DecodedCode } from "../../scanner/types";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { bitsLayer } from "./bits";

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

function code(corners: DecodedCode["corners"]): DecodedCode {
  return {
    payload: "Q:test:0",
    payload_bytes: [],
    version: 1,
    ecc: "M",
    mirrored: false,
    dimension: 21,
    corners,
    inverted: false,
    finder_indices: [0, 1, 2],
    refined_corners: null,
    corner_refined: [false, false, false, false],
  };
}

// A 2x2 dim matrix, axis-aligned 20x20 image-px quad (module = 10px):
// dark at (0,0) and (1,1), light at (1,0) and (0,1).
// Row 0 (y=0): bit0 set (x=0 dark), bit1 clear -> word 0b01 = 1.
// Row 1 (y=1): bit0 clear, bit1 set (x=1 dark) -> word 0b10 = 2.
const AXIS_ALIGNED_QUAD: DecodedCode["corners"] = [
  [0, 0],
  [20, 0],
  [20, 20],
  [0, 20],
];
const BITS_2X2 = { dim: 2, words: [1, 2] };

describe("bitsLayer", () => {
  it("draws nothing when scan is null", () => {
    const fake = createFakeCanvas();
    bitsLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when trace.bits is null", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [code(AXIS_ALIGNED_QUAD)], timings: EMPTY_TIMINGS, source_scale: 1 },
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
    bitsLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws nothing when detections.codes is empty (no code to map bits through)", () => {
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
        bits: BITS_2X2,
        refine: null,
      },
    };
    bitsLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("zoom gate: draws nothing when the module would render smaller than 4 screen px", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    // module_px = 20/2 = 10; view.scale=0.1 -> 1 screen px/module < 4.
    ctx.view = { scale: 0.1, tx: 0, ty: 0 };
    ctx.scan = {
      detections: { finders: [], triplets: [], codes: [code(AXIS_ALIGNED_QUAD)], timings: EMPTY_TIMINGS, source_scale: 1 },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [],
        alignment: [],
        sample_regions: [],
        bits: BITS_2X2,
        refine: null,
      },
    };
    bitsLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("fills exactly the dark modules, using the LAST code when multiple decoded (legacy fallback, trace.codes empty)", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    // module_px = 10, view.scale = 1 -> 10 screen px/module >= 4: passes.
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        // An earlier decoded code with a degenerate quad the layer must
        // NOT use — only the last entry matters (see the layer's doc).
        codes: [code([[0, 0], [0, 0], [0, 0], [0, 0]]), code(AXIS_ALIGNED_QUAD)],
        timings: EMPTY_TIMINGS,
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
        bits: BITS_2X2,
        refine: null,
      },
    };
    bitsLayer.draw(ctx);

    // 2 dark modules -> 2 filled quads (beginPath+moveTo+3 lineTo+closePath+fill each).
    expect(fake.callsNamed("beginPath")).toHaveLength(2);
    expect(fake.callsNamed("fill")).toHaveLength(2);
    expect(fake.callsNamed("moveTo")).toHaveLength(2);
    expect(fake.callsNamed("lineTo")).toHaveLength(2 * 3);

    // Module (0,0)'s quad: unit square [0,0.5]x[0,0.5] mapped through the
    // axis-aligned 20px quad -> image px (0,0)-(10,0)-(10,10)-(0,10);
    // identity view leaves it unchanged.
    const [moveTo] = fake.callsNamed("moveTo");
    expect(moveTo!.args[0]).toBeCloseTo(0, 9);
    expect(moveTo!.args[1]).toBeCloseTo(0, 9);
    const lineTos = fake.callsNamed("lineTo");
    expect(lineTos[0]!.args[0]).toBeCloseTo(10, 9);
    expect(lineTos[0]!.args[1]).toBeCloseTo(0, 9);
    expect(lineTos[1]!.args[0]).toBeCloseTo(10, 9);
    expect(lineTos[1]!.args[1]).toBeCloseTo(10, 9);
  });

  // Plan 5C: multi-code trace — every DECODED code's own bit matrix must
  // fill through ITS OWN quad, not just the last one.
  it("fills every decoded code's own bits from trace.codes, not just the last one", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    // A second code, offset 100px to the right, same axis-aligned shape/size.
    const secondQuad: DecodedCode["corners"] = [
      [100, 0],
      [120, 0],
      [120, 20],
      [100, 20],
    ];
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [code(AXIS_ALIGNED_QUAD), code(secondQuad)],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [
          { code_index: 0, sample_regions: [], bits: BITS_2X2, alignment: [] },
          { code_index: 1, sample_regions: [], bits: BITS_2X2, alignment: [] },
        ],
        // Legacy failure-diagnosis field, deliberately left `null` — must
        // be irrelevant since `trace.codes` is non-empty.
        alignment: [],
        sample_regions: [],
        bits: null,
        refine: null,
      },
    };
    bitsLayer.draw(ctx);

    // 2 dark modules per code x 2 codes = 4 filled quads.
    expect(fake.callsNamed("beginPath")).toHaveLength(4);
    expect(fake.callsNamed("fill")).toHaveLength(4);

    // The second code's first dark module (0,0) is offset +100px in x
    // (secondQuad's TL is [100,0], same 20px axis-aligned size).
    const moveTos = fake.callsNamed("moveTo");
    expect(moveTos[0]!.args[0]).toBeCloseTo(0, 9); // code 0's module (0,0)
    expect(moveTos[2]!.args[0]).toBeCloseTo(100, 9); // code 1's module (0,0)
  });

  it("zoom-gates each trace.codes entry independently", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    // A second code far smaller on screen (module_px = 1/2 = 0.5, view.scale
    // 1 -> 0.5 screen px/module < 4: this one must be gated out while the
    // first (module_px = 10) still draws.
    const tinyQuad: DecodedCode["corners"] = [
      [200, 0],
      [201, 0],
      [201, 1],
      [200, 1],
    ];
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [],
        codes: [code(AXIS_ALIGNED_QUAD), code(tinyQuad)],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: {
        tiles: null,
        finders: [],
        triplets: [],
        attempts: [],
        codes: [
          { code_index: 0, sample_regions: [], bits: BITS_2X2, alignment: [] },
          { code_index: 1, sample_regions: [], bits: BITS_2X2, alignment: [] },
        ],
        alignment: [],
        sample_regions: [],
        bits: null,
        refine: null,
      },
    };
    bitsLayer.draw(ctx);

    // Only code 0 passes the zoom gate: 2 dark modules, not 4.
    expect(fake.callsNamed("beginPath")).toHaveLength(2);
    expect(fake.callsNamed("fill")).toHaveLength(2);
  });
});
