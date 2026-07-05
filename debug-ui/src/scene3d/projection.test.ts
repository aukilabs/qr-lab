import { describe, expect, it } from "vitest";
import { PerspectiveCamera, Vector3 } from "three";
import { projectAllToPixels, projectToPixel } from "./projection";

/** Build + finalize a camera the way a real render loop would: set
 * position/orientation, then flush `matrixWorld` (three.js only
 * recomputes it during a render traversal or on explicit request — a
 * bare `.position.set(...)` doesn't update it). */
function cameraAt(
  position: [number, number, number],
  opts: { fov?: number; aspect?: number; near?: number; far?: number } = {},
): PerspectiveCamera {
  const { fov = 90, aspect = 1, near = 0.1, far = 100 } = opts;
  const cam = new PerspectiveCamera(fov, aspect, near, far);
  cam.position.set(...position);
  cam.updateMatrixWorld(true);
  return cam;
}

describe("projectToPixel", () => {
  it("maps a point on the optical axis to the principal point (w-1)/2 (pixel-centers-at-integers)", () => {
    // Camera at (0,0,5), unrotated -> looks down -Z, so the origin sits
    // directly on the optical axis at view-space depth 5. Under the
    // centers-at-integers convention (matching tools/fixtures/camera.py's
    // cx=(w-1)/2 and intrinsics.ts's intrinsicsFromFov) the optical axis
    // lands on (200-1)/2 = 99.5 — NOT 100, the corner-based width/2 the
    // pre-fix version produced (Plan 5d review fix: that mismatch put a
    // 0.5px principal-point inconsistency inside one exported JSON).
    const cam = cameraAt([0, 0, 5]);
    const [x, y] = projectToPixel(new Vector3(0, 0, 0), cam, 200, 200);
    expect(x).toBeCloseTo(99.5, 9);
    expect(y).toBeCloseTo(99.5, 9);
  });

  it("matches hand-derived pixel coordinates for an off-axis point (90deg vertical fov, square aspect)", () => {
    // fov=90 (vertical), aspect=1 -> at view-space depth d, the visible
    // half-extent on BOTH axes is d*tan(45deg) = d (tan(45deg)=1).
    // Camera at (0,0,5) unrotated: view-space of world point (2.5, 1.25, 0)
    // is (2.5, 1.25, -5) (translate-only, no rotation) -> depth 5, so
    // half-extent there is 5. NDC x = 2.5/5 = 0.5, NDC y = 1.25/5 = 0.25.
    // pixel x = (0.5+1)/2*200 - 0.5 = 149.5; pixel y = (1-0.25)/2*200 -
    // 0.5 = 74.5 (y flips: NDC +y is "up", pixel +y is "down"; the -0.5
    // is the centers-at-integers conversion).
    const cam = cameraAt([0, 0, 5]);
    const [x, y] = projectToPixel(new Vector3(2.5, 1.25, 0), cam, 200, 200);
    expect(x).toBeCloseTo(149.5, 6);
    expect(y).toBeCloseTo(74.5, 6);
  });

  it("keeps a point on the optical axis at the principal point regardless of camera position/orientation", () => {
    // A non-axis-aligned camera (arbitrary position, looking along +X via
    // lookAt) exercises the rotation component projectToPixel must handle
    // correctly, not just translation. Any point further along the same
    // viewing ray must still land exactly at the principal point
    // ((w-1)/2, (h-1)/2) — that invariant holds independent of fov/
    // aspect/resolution.
    const cam = new PerspectiveCamera(60, 1.5, 0.1, 50);
    cam.position.set(3, 4, 5);
    cam.lookAt(13, 4, 5); // looking along +X
    cam.updateMatrixWorld(true);

    const onAxis = new Vector3(20, 4, 5); // further down the same ray
    const [x, y] = projectToPixel(onAxis, cam, 640, 480);
    expect(x).toBeCloseTo(319.5, 6);
    expect(y).toBeCloseTo(239.5, 6);
  });

  it("maps world 'up' (relative to a right-side-up camera) to a smaller pixel y", () => {
    const cam = cameraAt([0, 0, 5]);
    const [, yUp] = projectToPixel(new Vector3(0, 1, 0), cam, 200, 200);
    const [, yCenter] = projectToPixel(new Vector3(0, 0, 0), cam, 200, 200);
    const [, yDown] = projectToPixel(new Vector3(0, -1, 0), cam, 200, 200);
    expect(yUp).toBeLessThan(yCenter);
    expect(yDown).toBeGreaterThan(yCenter);
  });

  it("maps world 'right' to a larger pixel x", () => {
    const cam = cameraAt([0, 0, 5]);
    const [xRight] = projectToPixel(new Vector3(1, 0, 0), cam, 200, 200);
    const [xCenter] = projectToPixel(new Vector3(0, 0, 0), cam, 200, 200);
    expect(xRight).toBeGreaterThan(xCenter);
  });

  it("does not mutate the input point", () => {
    const cam = cameraAt([0, 0, 5]);
    const p = new Vector3(1, 2, 0);
    const before = p.clone();
    projectToPixel(p, cam, 200, 200);
    expect(p.equals(before)).toBe(true);
  });
});

describe("projectAllToPixels", () => {
  it("projects every point in order", () => {
    const cam = cameraAt([0, 0, 5]);
    const points = [new Vector3(0, 0, 0), new Vector3(1, 0, 0), new Vector3(0, 1, 0)];
    const results = projectAllToPixels(points, cam, 200, 200);
    expect(results).toHaveLength(3);
    expect(results[0]).toEqual(projectToPixel(points[0]!, cam, 200, 200));
    expect(results[1]).toEqual(projectToPixel(points[1]!, cam, 200, 200));
    expect(results[2]).toEqual(projectToPixel(points[2]!, cam, 200, 200));
  });
});
