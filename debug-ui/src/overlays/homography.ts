// TS port of `crates/qr-lab-core/src/homography.rs`'s `PerspectiveTransform`
// (`square_to_quad` + `map` only — `inverse` isn't needed by the overlay
// layers this exists for). Kept algebraically identical to the Rust source
// so the two never quietly drift; see `homography.test.ts` for the
// corners-exact + interior-point cross-check against the Rust
// implementation itself.
//
// Used by `layers/expected-finder-centers.ts` to compute the ground-truth
// overlay's expected finder-center dots: the golden-fixture JSON gives a
// code's four corners in source-image px, and this maps unit-square
// (module-fraction) coordinates through that quad's homography to locate
// arbitrary points inside the code (e.g. the center of the 7x7-module
// finder patterns) in the same px space as the corners.

export type Point2 = [number, number];

/** 3x3 homography stored row-major as coefficients a11..a33. Maps
 * (u,v) -> ((a11*u + a21*v + a31)/w, (a12*u + a22*v + a32)/w),
 * w = a13*u + a23*v + a33. */
export interface PerspectiveTransform {
  a11: number;
  a21: number;
  a31: number;
  a12: number;
  a22: number;
  a32: number;
  a13: number;
  a23: number;
  a33: number;
}

/**
 * Unit square (0,0),(1,0),(1,1),(0,1) -> quad [TL, TR, BR, BL].
 *
 * Returns `null` for a degenerate `q` that admits no valid homography: the
 * affine branch (`q` a parallelogram) is degenerate iff its two edge
 * vectors are parallel (zero cross product — zero-area parallelogram);
 * the general projective branch is degenerate iff its `den` denominator
 * is exactly zero. Mirrors the Rust `Option<Self>` return exactly,
 * including which branch is taken for which `q`.
 */
export function squareToQuad(
  q: [Point2, Point2, Point2, Point2],
): PerspectiveTransform | null {
  const [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = q;
  const dx3 = x0 - x1 + x2 - x3;
  const dy3 = y0 - y1 + y2 - y3;

  if (dx3 === 0 && dy3 === 0) {
    const cross = (x1 - x0) * (y3 - y0) - (y1 - y0) * (x3 - x0);
    if (cross === 0) return null;
    return {
      a11: x1 - x0,
      a21: x2 - x1,
      a31: x0,
      a12: y1 - y0,
      a22: y2 - y1,
      a32: y0,
      a13: 0,
      a23: 0,
      a33: 1,
    };
  }

  const dx1 = x1 - x2;
  const dx2 = x3 - x2;
  const dy1 = y1 - y2;
  const dy2 = y3 - y2;
  const den = dx1 * dy2 - dx2 * dy1;
  if (den === 0) return null;

  const a13 = (dx3 * dy2 - dx2 * dy3) / den;
  const a23 = (dx1 * dy3 - dx3 * dy1) / den;
  return {
    a11: x1 - x0 + a13 * x1,
    a21: x3 - x0 + a23 * x3,
    a31: x0,
    a12: y1 - y0 + a13 * y1,
    a22: y3 - y0 + a23 * y3,
    a32: y0,
    a13,
    a23,
    a33: 1,
  };
}

/** Maps a unit-square point `(u, v)` to its image under `h`. See
 * homography.rs's `map` docs for the (deliberately unguarded) behavior
 * near the transform's horizon line — not expected to matter here since
 * ground-truth corners always come from real, in-frame image geometry. */
export function mapPoint(h: PerspectiveTransform, u: number, v: number): Point2 {
  const w = h.a13 * u + h.a23 * v + h.a33;
  return [(h.a11 * u + h.a21 * v + h.a31) / w, (h.a12 * u + h.a22 * v + h.a32) / w];
}
