import { describe, expect, it } from "vitest";
import {
  fitToView,
  identity,
  imageToScreen,
  pan,
  screenToImage,
  zoomAt,
  type ViewTransform,
} from "./transform";

/** Loose float equality for a [number, number] pair. */
function expectPointClose(
  actual: [number, number],
  expected: [number, number],
  digits = 6,
): void {
  expect(actual[0]).toBeCloseTo(expected[0], digits);
  expect(actual[1]).toBeCloseTo(expected[1], digits);
}

describe("identity", () => {
  it("is scale=1, no translation", () => {
    expect(identity).toEqual({ scale: 1, tx: 0, ty: 0 });
  });
});

describe("imageToScreen / screenToImage", () => {
  it("round-trips screenToImage(imageToScreen(p)) = p under an arbitrary transform", () => {
    const t: ViewTransform = { scale: 2.5, tx: 13, ty: -7 };
    const points: [number, number][] = [
      [0, 0],
      [100, 50],
      [-30, 200],
      [1.5, -2.5],
    ];
    for (const p of points) {
      expectPointClose(screenToImage(t, imageToScreen(t, p)), p);
    }
  });

  it("round-trips imageToScreen(screenToImage(p)) = p under an arbitrary transform", () => {
    const t: ViewTransform = { scale: 0.33, tx: -400, ty: 250 };
    const points: [number, number][] = [
      [0, 0],
      [800, 600],
      [-15, 42],
    ];
    for (const p of points) {
      expectPointClose(imageToScreen(t, screenToImage(t, p)), p);
    }
  });

  it("identity maps image coords straight through to screen coords", () => {
    expectPointClose(imageToScreen(identity, [42, 17]), [42, 17]);
    expectPointClose(screenToImage(identity, [42, 17]), [42, 17]);
  });
});

describe("zoomAt", () => {
  it("keeps the anchor screen point fixed across the zoom", () => {
    const t: ViewTransform = { scale: 1, tx: 10, ty: 20 };
    const screenPoint: [number, number] = [150, 80];
    const zoomed = zoomAt(t, screenPoint, 2);

    // The image point under the cursor before the zoom must still land on
    // the same screen point after the zoom.
    const anchorImagePoint = screenToImage(t, screenPoint);
    expectPointClose(imageToScreen(zoomed, anchorImagePoint), screenPoint);
  });

  it("scales by the given factor", () => {
    const t: ViewTransform = { scale: 2, tx: 0, ty: 0 };
    const zoomed = zoomAt(t, [50, 50], 1.5);
    expect(zoomed.scale).toBeCloseTo(3, 6);
  });

  it("factor=1 is a no-op", () => {
    const t: ViewTransform = { scale: 1.7, tx: 5, ty: -5 };
    const zoomed = zoomAt(t, [10, 10], 1);
    expect(zoomed.scale).toBeCloseTo(t.scale, 6);
    expect(zoomed.tx).toBeCloseTo(t.tx, 6);
    expect(zoomed.ty).toBeCloseTo(t.ty, 6);
  });

  it("clamps scale to the [0.05, 64] range", () => {
    const t: ViewTransform = { scale: 1, tx: 0, ty: 0 };

    const zoomedInHard = zoomAt(t, [0, 0], 1000);
    expect(zoomedInHard.scale).toBeCloseTo(64, 6);

    const zoomedOutHard = zoomAt(t, [0, 0], 0.0001);
    expect(zoomedOutHard.scale).toBeCloseTo(0.05, 6);
  });

  it("still anchors the screen point correctly when the result is clamped", () => {
    const t: ViewTransform = { scale: 60, tx: 0, ty: 0 };
    const screenPoint: [number, number] = [200, 120];
    const anchorImagePoint = screenToImage(t, screenPoint);

    const zoomed = zoomAt(t, screenPoint, 10); // would be 600, clamps to 64
    expect(zoomed.scale).toBeCloseTo(64, 6);
    expectPointClose(imageToScreen(zoomed, anchorImagePoint), screenPoint);
  });
});

describe("pan", () => {
  it("adds the delta to the translation and leaves scale untouched", () => {
    const t: ViewTransform = { scale: 1.5, tx: 10, ty: -4 };
    const panned = pan(t, 5, 3);
    expect(panned).toEqual({ scale: 1.5, tx: 15, ty: -1 });
  });

  it("composes additively: pan(pan(t, a), b) == pan(t, a+b)", () => {
    const t: ViewTransform = { scale: 2, tx: 0, ty: 0 };
    const sequential = pan(pan(t, 4, -6), 10, 2);
    const combined = pan(t, 14, -4);
    expect(sequential).toEqual(combined);
  });
});

describe("fitToView", () => {
  it("letterboxes a wide image (wider than the view) with top/bottom bars", () => {
    // image 400x100 (4:1) into a 200x200 view -> scale limited by width.
    const t = fitToView(400, 100, 200, 200);
    expect(t.scale).toBeCloseTo(0.5, 6);
    expect(t.tx).toBeCloseTo(0, 6); // full width used, no horizontal bars
    expect(t.ty).toBeCloseTo(75, 6); // (200 - 100*0.5) / 2

    // The full image should land centered and fully inside the view.
    expectPointClose(imageToScreen(t, [0, 0]), [0, 75]);
    expectPointClose(imageToScreen(t, [400, 100]), [200, 125]);
  });

  it("letterboxes a tall image (taller than the view) with left/right bars", () => {
    // image 100x400 (1:4) into a 200x200 view -> scale limited by height.
    const t = fitToView(100, 400, 200, 200);
    expect(t.scale).toBeCloseTo(0.5, 6);
    expect(t.ty).toBeCloseTo(0, 6); // full height used, no vertical bars
    expect(t.tx).toBeCloseTo(75, 6); // (200 - 100*0.5) / 2

    expectPointClose(imageToScreen(t, [0, 0]), [75, 0]);
    expectPointClose(imageToScreen(t, [100, 400]), [125, 200]);
  });

  it("exactly fills a view matching the image aspect ratio", () => {
    const t = fitToView(800, 600, 400, 300);
    expect(t.scale).toBeCloseTo(0.5, 6);
    expect(t.tx).toBeCloseTo(0, 6);
    expect(t.ty).toBeCloseTo(0, 6);
  });

  it("clamps the contain-scale to the zoom floor, accepting clipping at extreme aspect ratios", () => {
    // 100000x10 strip into a 100x100 view would need scale 0.001 to fit;
    // the shared zoom clamp floors it at 0.05, so the strip is centered
    // but clipped horizontally. Pins the documented trade-off in
    // fitToView: the zoom floor wins over the "whole image visible"
    // contract for pathological inputs.
    const t = fitToView(100000, 10, 100, 100);
    expect(t.scale).toBeCloseTo(0.05, 6);
    expect(t.tx).toBeCloseTo((100 - 100000 * 0.05) / 2, 6); // -2450: clipped
    expect(t.ty).toBeCloseTo((100 - 10 * 0.05) / 2, 6); // 49.75: centered
    // Left edge of the image lands far off-screen — clipping is accepted.
    expect(imageToScreen(t, [0, 0])[0]).toBeLessThan(0);
  });

  it("falls back to identity for degenerate (non-positive) dimensions", () => {
    expect(fitToView(0, 100, 200, 200)).toEqual(identity);
    expect(fitToView(100, 0, 200, 200)).toEqual(identity);
    expect(fitToView(100, 100, 0, 200)).toEqual(identity);
    expect(fitToView(100, 100, 200, -1)).toEqual(identity);
  });
});
