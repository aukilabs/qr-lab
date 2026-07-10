import { describe, expect, it } from "vitest";
import type { RobustDetections } from "../../scanner/robust-types";
import { identity } from "../../viewport/transform";
import type { OverlayContext } from "../registry";
import { createFakeCanvas } from "../test-support/fake-canvas";
import { evidenceLayer } from "./evidence";

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

function robustDetections(evidence: [number, number][]): RobustDetections {
  return {
    codes: [],
    variants: [],
    early_exited: false,
    budget_exhausted: false,
    total_ns: 0,
    triplet_evidence: evidence,
  };
}

describe("evidenceLayer", () => {
  it("draws nothing when robust is null or evidence is empty", () => {
    const fake = createFakeCanvas();
    evidenceLayer.draw(baseContext(fake));
    expect(fake.calls).toHaveLength(0);

    const fake2 = createFakeCanvas();
    const ctx = baseContext(fake2);
    ctx.robust = robustDetections([]);
    evidenceLayer.draw(ctx);
    expect(fake2.calls).toHaveLength(0);
  });

  it("draws one 4-segment crosshair per evidence point", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.robust = robustDetections([
      [100, 100],
      [200, 150],
    ]);
    evidenceLayer.draw(ctx);

    // Per point: 1 beginPath, 4 moveTo/lineTo segment pairs, 1 stroke.
    expect(fake.callsNamed("beginPath")).toHaveLength(2);
    expect(fake.callsNamed("moveTo")).toHaveLength(8);
    expect(fake.callsNamed("lineTo")).toHaveLength(8);
    expect(fake.callsNamed("stroke")).toHaveLength(2);
  });

  it("maps SOURCE-px evidence through workingScale before projecting", () => {
    const fake = createFakeCanvas();
    const ctx = baseContext(fake);
    ctx.workingScale = 0.25;
    ctx.robust = robustDetections([[400, 200]]);
    evidenceLayer.draw(ctx);

    // [400, 200] source px -> [100, 50] working px under identity view;
    // the first segment starts at x - CROSSHAIR_HALF (7).
    const [firstMove] = fake.callsNamed("moveTo");
    expect(firstMove!.args).toEqual([93, 50]);
  });
});
