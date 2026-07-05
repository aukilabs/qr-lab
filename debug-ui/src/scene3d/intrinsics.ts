// Pure pinhole-intrinsics derivation from a three.js `PerspectiveCamera`'s
// vertical FOV — used by the 3D scene's "save as fixture" export (Plan
// 5d) to populate the fixture JSON's `camera: {fx, fy, cx, cy}` block.
// MUST match `tools/fixtures/camera.py`'s `Intrinsics.default()` derivation
// exactly (same formula, same pixel-centers-at-integers convention) so a
// scene-exported fixture's `camera` block is directly comparable to a
// Python-generated one, not merely "close": `fy = (h/2) / tan(fovY/2)`,
// `fx = fy` (the readback target is always square — see `Scene3D.tsx`'s
// module doc on why `camera.aspect` is forced to 1 — so square pixels are
// a safe assumption here, unlike a general aspect-ratio camera), `cx =
// (w-1)/2`, `cy = (h-1)/2`.
export interface Intrinsics {
  fx: number;
  fy: number;
  cx: number;
  cy: number;
}

/**
 * Derive pinhole intrinsics for a `width x height` (square, per this
 * scene's readback contract) render of a camera with vertical field of
 * view `fovYDeg` degrees.
 */
export function intrinsicsFromFov(fovYDeg: number, width: number, height: number): Intrinsics {
  const fovYRad = (fovYDeg * Math.PI) / 180;
  const fy = height / 2 / Math.tan(fovYRad / 2);
  return {
    fx: fy,
    fy,
    cx: (width - 1) / 2,
    cy: (height - 1) / 2,
  };
}
