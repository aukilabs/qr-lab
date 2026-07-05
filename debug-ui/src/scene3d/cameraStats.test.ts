import { describe, expect, it } from "vitest";
import { Mesh, PerspectiveCamera, PlaneGeometry } from "three";
import { computeCameraStats } from "./cameraStats";

/** Build + finalize a camera/plane pair the way a real render loop would
 * (see `projection.test.ts`'s `cameraAt` for the same precedent): set
 * position/orientation, then flush `matrixWorld` (three.js only recomputes
 * it during a render traversal or on explicit request). */
function cameraAt(position: [number, number, number], lookAt?: [number, number, number]): PerspectiveCamera {
  const cam = new PerspectiveCamera(50, 1, 0.01, 100);
  cam.position.set(...position);
  if (lookAt) cam.lookAt(...lookAt);
  cam.updateMatrixWorld(true);
  return cam;
}

function planeAt(
  position: [number, number, number] = [0, 0, 0],
  rotationZ = 0,
  rotationY = 0,
): Mesh {
  const mesh = new Mesh(new PlaneGeometry(0.15, 0.15));
  mesh.position.set(...position);
  mesh.rotation.z = rotationZ;
  mesh.rotation.y = rotationY;
  mesh.updateMatrixWorld(true);
  return mesh;
}

describe("computeCameraStats", () => {
  it("reports distance == the straight-line distance to the plane center", () => {
    const cam = cameraAt([0, 0, 5], [0, 0, 0]);
    const plane = planeAt([0, 0, 0]);
    const stats = computeCameraStats(cam, plane.matrixWorld);
    expect(stats.distanceM).toBeCloseTo(5, 9);
  });

  it("scales distance with camera position (not physicalSize-dependent)", () => {
    const near = computeCameraStats(cameraAt([0, 0, 2], [0, 0, 0]), planeAt().matrixWorld);
    const far = computeCameraStats(cameraAt([0, 0, 8], [0, 0, 0]), planeAt().matrixWorld);
    expect(far.distanceM).toBeCloseTo(near.distanceM * 4, 9);
  });

  it("reports ~0deg incidence for a camera looking straight at the plane head-on", () => {
    // Camera on the plane's own +Z axis (its normal), looking straight
    // back at it: view ray is exactly anti-parallel to the normal.
    const cam = cameraAt([0, 0, 5], [0, 0, 0]);
    const plane = planeAt([0, 0, 0]);
    const stats = computeCameraStats(cam, plane.matrixWorld);
    expect(stats.incidenceDeg).toBeCloseTo(0, 6);
  });

  it("reports ~90deg incidence for a camera viewing the plane edge-on", () => {
    // Camera on the plane's local X axis, looking at its center: the view
    // ray lies IN the plane, perpendicular to its normal.
    const cam = cameraAt([5, 0, 0], [0, 0, 0]);
    const plane = planeAt([0, 0, 0]);
    const stats = computeCameraStats(cam, plane.matrixWorld);
    expect(stats.incidenceDeg).toBeCloseTo(90, 6);
  });

  it("reports an intermediate incidence angle for a 45deg-offset camera", () => {
    // Camera at (5, 0, 5) looking at the origin: the view ray sits at
    // 45deg from the plane's +Z normal (equal x/z offset).
    const cam = cameraAt([5, 0, 5], [0, 0, 0]);
    const plane = planeAt([0, 0, 0]);
    const stats = computeCameraStats(cam, plane.matrixWorld);
    expect(stats.incidenceDeg).toBeCloseTo(45, 3);
  });

  it("is insensitive to which side of the plane the camera is on (abs dot product)", () => {
    const front = computeCameraStats(cameraAt([0, 0, 5], [0, 0, 0]), planeAt().matrixWorld);
    const back = computeCameraStats(cameraAt([0, 0, -5], [0, 0, 0]), planeAt().matrixWorld);
    expect(back.incidenceDeg).toBeCloseTo(front.incidenceDeg, 6);
  });

  it("reports ~0deg in-plane roll when the plane's local +X aligns with the camera's local +X", () => {
    const cam = cameraAt([0, 0, 5], [0, 0, 0]);
    const plane = planeAt([0, 0, 0], 0);
    const stats = computeCameraStats(cam, plane.matrixWorld);
    expect(stats.inPlaneRollDeg).toBeCloseTo(0, 3);
  });

  it("reports ~30deg in-plane roll after rotating the plane 30deg about its own normal", () => {
    const cam = cameraAt([0, 0, 5], [0, 0, 0]);
    const plane = planeAt([0, 0, 0], (30 * Math.PI) / 180);
    const stats = computeCameraStats(cam, plane.matrixWorld);
    expect(Math.abs(stats.inPlaneRollDeg)).toBeCloseTo(30, 1);
  });

  it("treats a camera coincident with the plane center as distance 0 without throwing", () => {
    const cam = cameraAt([0, 0, 0], [0, 0, -1]);
    const plane = planeAt([0, 0, 0]);
    expect(() => computeCameraStats(cam, plane.matrixWorld)).not.toThrow();
    const stats = computeCameraStats(cam, plane.matrixWorld);
    expect(stats.distanceM).toBe(0);
  });
});
