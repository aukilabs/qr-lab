// Pure(-ish) camera/plane geometry derivation for the 3D scene's HUD
// (Plan 5d) — distance, incidence angle, and in-plane roll between a
// three.js camera and a plane mesh's world transform. Real `three` math
// (no mocking), same precedent as `projection.ts`: camera/matrix math has
// no DOM/WebGL dependency, so this runs identically in vitest's `node`
// environment and in a real browser — see that module's doc.
import { Matrix3, Matrix4, Vector3, type Camera } from "three";

export interface CameraStats {
  /** Distance from the camera to the plane's local origin (its center),
   * in the SAME units as `physicalSize` — meters, per this scene's
   * convention (`consts.ts`'s `DEFAULT_PHYSICAL_SIZE_M`). */
  distanceM: number;
  /** Angle (degrees, 0-90) between the camera-to-plane-center ray and the
   * plane's world-space normal. 0deg = head-on (looking straight along the
   * normal), 90deg = grazing/edge-on. Takes the ray/normal dot product's
   * absolute value, so it doesn't matter which of the plane's two faces
   * the normal happens to point toward. */
  incidenceDeg: number;
  /** Approximate in-plane roll (degrees): the angle, as seen in the
   * camera's own local XY plane, of the plane mesh's local +X axis
   * projected into camera space. 0deg when the plane's local +X aligns
   * with the camera's local +X (the common "upright, unrolled" case).
   * Cheap/approximate by design (task brief: "in-plane roll if cheap") —
   * it's a direction-only projection, not a full decomposition of the
   * plane's orientation relative to the camera, so a combination of large
   * out-of-plane tilt AND in-plane rotation can shift this value away from
   * the "pure" in-plane-rotation angle a full decomposition would report. */
  inPlaneRollDeg: number;
}

const RAD_TO_DEG = 180 / Math.PI;

/** World-space direction of `matrixWorld`'s local +axis (X when
 * `axis="x"`, Z when `axis="z"`) — i.e. just the rotation/scale part of
 * the transform applied to a unit axis vector, translation ignored (a
 * direction, not a point). Normalized. */
function worldDirection(matrixWorld: Matrix4, axis: "x" | "z"): Vector3 {
  const m = new Matrix3().setFromMatrix4(matrixWorld);
  const v = axis === "x" ? new Vector3(1, 0, 0) : new Vector3(0, 0, 1);
  return v.applyMatrix3(m).normalize();
}

/**
 * Derive {@link CameraStats} for `camera` viewing a plane mesh whose world
 * transform is `planeMatrixWorld` (i.e. `mesh.matrixWorld` after
 * `mesh.updateMatrixWorld(true)` — same precondition `Scene3D.tsx`'s
 * ground-truth projection already relies on). The plane's local +Z is
 * its normal and its local origin is its center, matching
 * `moduleRegion.ts`'s local-frame convention (`PlaneGeometry`'s default:
 * flat in local XY, normal along local +Z).
 */
export function computeCameraStats(camera: Camera, planeMatrixWorld: Matrix4): CameraStats {
  const cameraPos = new Vector3().setFromMatrixPosition(camera.matrixWorld);
  const planeCenter = new Vector3().setFromMatrixPosition(planeMatrixWorld);

  const toCam = new Vector3().subVectors(cameraPos, planeCenter);
  const distanceM = toCam.length();

  const normal = worldDirection(planeMatrixWorld, "z");
  const rayToCam = distanceM > 0 ? toCam.clone().divideScalar(distanceM) : new Vector3(0, 0, 1);
  const cosIncidence = Math.min(1, Math.max(-1, Math.abs(rayToCam.dot(normal))));
  const incidenceDeg = Math.acos(cosIncidence) * RAD_TO_DEG;

  // Project the plane's local +X world direction into the camera's own
  // local frame (rotation-only — matrixWorldInverse's upper 3x3), then
  // read its angle in the camera's XY plane.
  const planeXWorld = worldDirection(planeMatrixWorld, "x");
  const camRot = new Matrix3().setFromMatrix4(camera.matrixWorldInverse);
  const planeXInCam = planeXWorld.clone().applyMatrix3(camRot);
  const inPlaneRollDeg = Math.atan2(planeXInCam.y, planeXInCam.x) * RAD_TO_DEG;

  return { distanceM, incidenceDeg, inPlaneRollDeg };
}
