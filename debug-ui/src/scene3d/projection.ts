// Pure projection math: a world-space point -> the pixel coordinate it
// lands on in a `width x height` render of a given three.js camera. Real
// `three` math (no mocking) — `Camera.project` composes the camera's view
// (`matrixWorldInverse`) and projection matrices into normalized device
// coordinates (NDC, both axes in [-1, 1] when in view); this module only
// adds the NDC -> pixel conversion, which is the one step `three` doesn't
// do for you. `three` has no DOM/WebGL dependency for this — camera
// matrices are pure linear algebra — so this runs identically in vitest's
// `node` environment and in a real browser.
import type { Camera, Vector3 } from "three";

/**
 * Project `point` (world space) through `camera` into pixel coordinates
 * for a `width x height` render target — the SAME convention
 * `qr_lab_core::scan`'s SOURCE-px geometry (and this debug UI's `refined_
 * corners`) uses: pixel CENTERS at integer coordinates, `(0, 0)` at the
 * top-left pixel's center, x right, y DOWN. This matches
 * `tools/fixtures/camera.py` ("pixel centers at integer coordinates";
 * `cx = (w-1)/2`) and `intrinsics.ts`'s `intrinsicsFromFov` — the two
 * ends of the fixture-export contract this projection feeds.
 *
 * NDC -> pixel: three.js's NDC has y UP (`+1` at the top of the view,
 * `-1` at the bottom) and both axes spanning `[-1, 1]`; image-pixel space
 * has y DOWN — hence the `(1 - ndc.y)` flip on the y axis (no flip on
 * x). The trailing `- 0.5` converts from the continuous [0, width]
 * corner-based span to the centers-at-integers convention: NDC 0 (the
 * optical axis) lands on `(width-1)/2` — exactly `cx` — not `width/2`.
 * (The pre-fix version omitted the `- 0.5`, leaving a 0.5px principal-
 * point inconsistency between an exported fixture's `corners_px` and its
 * own `camera.cx/cy`, and a half-pixel bias in the live error panel's
 * ground truth — Plan 5d review fix.)
 *
 * Does NOT mutate `point`; `Camera.project` would mutate its receiver, so
 * this clones first.
 */
export function projectToPixel(
  point: Vector3,
  camera: Camera,
  width: number,
  height: number,
): [number, number] {
  const ndc = point.clone().project(camera);
  const x = ((ndc.x + 1) / 2) * width - 0.5;
  const y = ((1 - ndc.y) / 2) * height - 0.5;
  return [x, y];
}

/** Project every point in `points` (see {@link projectToPixel}). */
export function projectAllToPixels(
  points: Vector3[],
  camera: Camera,
  width: number,
  height: number,
): [number, number][] {
  return points.map((p) => projectToPixel(p, camera, width, height));
}
