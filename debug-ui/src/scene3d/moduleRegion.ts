// Pure geometry: where a QR's module-region corners (the dark-square
// boundary, quiet zone excluded — same "corner" definition Plan 5's Rust
// side uses throughout, see qrk-core's Global Constraints doc) sit in the
// LOCAL (object-space, z=0) coordinate frame of a plane mesh that has the
// FULL code + quiet zone painted across its entire physical extent.
//
// No three.js dependency here (plain `[x, y, z]` tuples) — this is pure
// arithmetic, independent of any particular 3D library; `Scene3D.tsx`
// wraps the result in `THREE.Vector3` and applies the mesh's
// `matrixWorld` before projecting through the camera (`projection.ts`).
//
// Local-frame convention: the plane spans `[-physicalSize/2,
// +physicalSize/2]` on both local X and Y, with local +Y "up" (matching a
// `THREE.PlaneGeometry`'s default UV layout: local +Y is where
// `CanvasTexture`'s default `flipY = true` puts row 0 of the source
// canvas — i.e. local +Y is the TOP of the rendered QR image, same as
// image row 0 / smaller image-Y). TL/TR/BR/BL below follows that: "top"
// means larger local Y.
export type Point3 = [number, number, number];

export interface ModuleRegionCorners {
  tl: Point3;
  tr: Point3;
  br: Point3;
  bl: Point3;
}

/**
 * The four module-region corners in local (object-space, z=0) coordinates
 * for a `dim x dim`-module QR (plus `quiet` modules of margin on every
 * side) painted across a `physicalSize`-meter-square plane.
 *
 * Derivation: the full painted canvas is `dim + 2*quiet` modules per
 * side, spanning the whole `physicalSize` extent; the module region is
 * inset from each edge by `quiet / (dim + 2*quiet) * physicalSize` — the
 * fraction of the canvas the quiet zone occupies on that side, converted
 * to local-space units.
 */
export function moduleRegionLocalCorners(
  dim: number,
  quiet: number,
  physicalSize: number,
): ModuleRegionCorners {
  if (!Number.isFinite(dim) || dim <= 0) {
    throw new RangeError(`moduleRegionLocalCorners: dim must be > 0, got ${dim}`);
  }
  if (!Number.isFinite(quiet) || quiet < 0) {
    throw new RangeError(`moduleRegionLocalCorners: quiet must be >= 0, got ${quiet}`);
  }
  if (!Number.isFinite(physicalSize) || physicalSize <= 0) {
    throw new RangeError(`moduleRegionLocalCorners: physicalSize must be > 0, got ${physicalSize}`);
  }

  const total = dim + 2 * quiet;
  const inset = (quiet / total) * physicalSize;
  const half = physicalSize / 2;
  const inner = half - inset; // module-region edge, both "positive" edges
  const outer = -inner; // ... and both "negative" edges (symmetric inset)

  return {
    tl: [outer, inner, 0],
    tr: [inner, inner, 0],
    br: [inner, outer, 0],
    bl: [outer, outer, 0],
  };
}

/** `[TL, TR, BR, BL]` array form of {@link moduleRegionLocalCorners} —
 * convenient for iteration (projecting all four corners in a loop). */
export function moduleRegionLocalCornersArray(
  dim: number,
  quiet: number,
  physicalSize: number,
): [Point3, Point3, Point3, Point3] {
  const c = moduleRegionLocalCorners(dim, quiet, physicalSize);
  return [c.tl, c.tr, c.br, c.bl];
}
