import { describe, expect, it } from "vitest";
import { identity } from "../../viewport/transform";
import type { DecodedCode } from "../../scanner/types";
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

const EMPTY_TIMINGS = {
  tiles_ns: 0,
  finders_ns: 0,
  triplets_ns: 0,
  version_ns: 0,
  alignment_ns: 0,
  sample_decode_ns: 0,
  refine_ns: 0,
};

/** Minimal `DecodedCode` stub — only `finder_indices` matters to this
 * layer's winner/candidate match. */
function decodedCode(finderIndices: [number, number, number]): DecodedCode {
  return {
    payload: "Q:test:0",
    payload_bytes: [],
    version: 1,
    ecc: "M",
    mirrored: false,
    dimension: 21,
    corners: [[0, 0], [0, 0], [0, 0], [0, 0]],
    inverted: false,
    finder_indices: finderIndices,
    refined_corners: null,
    corner_refined: [false, false, false, false],
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
    tripletsLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws leg lines, TL/TR/BL markers, and a dim label for a WINNER triplet", () => {
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
            finder_indices: [0, 1, 2],
          },
        ],
        // Matches the triplet's finder_indices (order-independent) -> WINNER.
        codes: [decodedCode([2, 0, 1])],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
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

    // Winner: solid (no dash), full alpha, save/restore bracketing.
    expect(fake.callsNamed("setLineDash")).toHaveLength(1);
    expect(fake.callsNamed("setLineDash")[0]!.args).toEqual([[]]);
    expect(fake.ctx.globalAlpha).toBe(1);
    expect(fake.callsNamed("save")).toHaveLength(1);
    expect(fake.callsNamed("restore")).toHaveLength(1);
  });

  it("draws a CANDIDATE/loser triplet dashed, at reduced alpha, with no dimension label", () => {
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
            finder_indices: [0, 1, 2],
          },
        ],
        // No decoded code shares this triplet's finder set -> candidate/loser.
        codes: [decodedCode([3, 4, 5])],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: null,
    };
    tripletsLayer.draw(ctx);

    // Same primitive shape as a winner (legs/markers still drawn)...
    expect(fake.callsNamed("beginPath")).toHaveLength(3);
    expect(fake.callsNamed("strokeRect")).toHaveLength(1);
    expect(fake.callsNamed("arc")).toHaveLength(1);
    expect(fake.callsNamed("stroke")).toHaveLength(3);
    // ...but no dimension label, dashed legs, and reduced alpha.
    expect(fake.callsNamed("fillText")).toHaveLength(0);
    expect(fake.callsNamed("setLineDash")).toHaveLength(1);
    expect(fake.callsNamed("setLineDash")[0]!.args).toEqual([[4, 3]]);
    expect(fake.ctx.globalAlpha).toBe(0.35);
  });

  it("one winner + one loser in the same frame: dash/alpha calls differ per triplet", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    const winnerTriplet = {
      tl: [0, 0] as [number, number],
      tr: [56, 0] as [number, number],
      bl: [0, 56] as [number, number],
      module: 4,
      dimension: 21,
      snap_error: 0.1,
      inverted: false,
      finder_indices: [0, 1, 2] as [number, number, number],
    };
    const loserTriplet = {
      ...winnerTriplet,
      tl: [200, 200] as [number, number],
      finder_indices: [3, 4, 5] as [number, number, number],
    };
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [winnerTriplet, loserTriplet],
        // Only the first triplet's finder set is decoded.
        codes: [decodedCode([0, 1, 2])],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: null,
    };
    tripletsLayer.draw(ctx);

    const dashCalls = fake.callsNamed("setLineDash");
    expect(dashCalls).toHaveLength(2);
    expect(dashCalls[0]!.args).toEqual([[]]); // winner: solid
    expect(dashCalls[1]!.args).toEqual([[4, 3]]); // loser: dashed
    // Only the winner gets a dimension label.
    expect(fake.callsNamed("fillText")).toHaveLength(1);
    expect(fake.callsNamed("save")).toHaveLength(2);
    expect(fake.callsNamed("restore")).toHaveLength(2);
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
      finder_indices: [0, 1, 2] as [number, number, number],
    };
    ctx.scan = {
      detections: {
        finders: [],
        triplets: [triplet, { ...triplet, tl: [200, 200] }],
        // Both triplets share finder_indices [0,1,2] -> both WINNERS, so
        // this test's "two triplets' worth of primitives" assertions
        // (including the per-triplet dimension label) are unaffected by
        // the winner/candidate distinction.
        codes: [decodedCode([0, 1, 2])],
        timings: EMPTY_TIMINGS,
        source_scale: 1,
      },
      trace: null,
    };
    tripletsLayer.draw(ctx);
    expect(fake.callsNamed("fillText")).toHaveLength(2);
    expect(fake.callsNamed("strokeRect")).toHaveLength(2);
  });
});
