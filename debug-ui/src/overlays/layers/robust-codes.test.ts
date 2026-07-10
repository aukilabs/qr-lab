import { describe, expect, it } from "vitest";
import { stageColor, type RobustCode, type RobustDetections } from "../../scanner/robust-types";
import type { DecodedCode } from "../../scanner/types";
import { identity } from "../../viewport/transform";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { robustCodesLayer } from "./robust-codes";

function baseContext(fake: ReturnType<typeof createFakeCanvas>): OverlayContext {
  return {
    ctx: fake.ctx,
    view: identity,
    scan: null,
    groundTruth: null,
    imageSize: [64, 32],
    workingScale: 1,
    robust: null,
  };
}

function decodedCode(): DecodedCode {
  return {
    payload: "Q:test:0",
    payload_bytes: [],
    version: 1,
    ecc: "M",
    mirrored: false,
    dimension: 21,
    corners: [[0, 0], [0, 0], [0, 0], [0, 0]],
    inverted: false,
    finder_indices: [0, 1, 2],
    refined_corners: null,
    corner_refined: [false, false, false, false],
  };
}

function robustCode(overrides: Partial<RobustCode> = {}): RobustCode {
  return {
    code: decodedCode(),
    corners_source: [[10, 10], [50, 10], [50, 50], [10, 50]],
    refined_corners_source: null,
    variant: "SauvolaThreshold",
    stage: 3,
    ...overrides,
  };
}

function robustDetections(codes: RobustCode[]): RobustDetections {
  return {
    codes,
    variants: [],
    early_exited: false,
    budget_exhausted: false,
    total_ns: 0,
    triplet_evidence: [],
  };
}

describe("robustCodesLayer", () => {
  it("draws nothing when robust is null/absent", () => {
    const fake = createFakeCanvas();
    robustCodesLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);

    const fake2 = createFakeCanvas();
    const noRobust = baseContext(fake2);
    delete noRobust.robust; // absent (pre-Plan-6 context constructors)
    robustCodesLayer.draw(noRobust);
    expect(fake2.calls).toHaveLength(0);
  });

  it("draws nothing when robust.codes is empty", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.robust = robustDetections([]);
    robustCodesLayer.draw(ctx);
    expect(fake.calls).toHaveLength(0);
  });

  it("draws a stroked quad + label chip for a code without refined corners", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.robust = robustDetections([robustCode()]);
    robustCodesLayer.draw(ctx);

    // Quad: 1 path (moveTo + 3 lineTo + closePath + stroke).
    expect(fake.callsNamed("moveTo")).toHaveLength(1);
    expect(fake.callsNamed("lineTo")).toHaveLength(3);
    expect(fake.callsNamed("closePath")).toHaveLength(1);
    expect(fake.callsNamed("stroke")).toHaveLength(1);
    // Chip: filled rect + variant label text.
    expect(fake.callsNamed("fillRect")).toHaveLength(1);
    const [label] = fake.callsNamed("fillText");
    expect(label!.args[0]).toBe("Sauvola threshold");
    // No refined corners -> no dots.
    expect(fake.callsNamed("arc")).toHaveLength(0);
  });

  it("draws 4 refined-corner dots when refined_corners_source is present", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.robust = robustDetections([
      robustCode({
        refined_corners_source: [[11, 11], [49, 11], [49, 49], [11, 49]],
      }),
    ]);
    robustCodesLayer.draw(ctx);

    expect(fake.callsNamed("arc")).toHaveLength(4);
    expect(fake.callsNamed("fill")).toHaveLength(4);
  });

  it("maps SOURCE-px corners through workingScale before projecting", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.workingScale = 0.5;
    ctx.robust = robustDetections([robustCode()]);
    robustCodesLayer.draw(ctx);

    // corners_source[0] is [10, 10] -> working px [5, 5] under identity view.
    const [move] = fake.callsNamed("moveTo");
    expect(move!.args).toEqual([5, 5]);
  });

  it("strokes each code in its own stage color", () => {
    const fake = createFakeCanvas();
    const strokeStyles: unknown[] = [];
    // Capture strokeStyle at each stroke() call — the fake records methods
    // only, so snapshot the style field alongside.
    const origStroke = (fake.ctx as unknown as Record<string, unknown>).stroke as () => void;
    (fake.ctx as unknown as Record<string, unknown>).stroke = () => {
      strokeStyles.push(fake.ctx.strokeStyle);
      origStroke();
    };

    const ctx = baseContext(fake);
    ctx.robust = robustDetections([
      robustCode({ variant: "Baseline", stage: 0 }),
      robustCode({ variant: "Upscaled2x", stage: 5 }),
    ]);
    robustCodesLayer.draw(ctx);

    expect(strokeStyles).toEqual([stageColor(0), stageColor(5)]);
  });
});
