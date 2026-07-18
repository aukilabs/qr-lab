// Ground-truth pixel centers of a code's three finder patterns (TL, TR,
// BL), computed the same way as `crates/qr-lab-core/tests/common/mod.rs`'s
// `expected_finder_centers`: map the finder-center points in unit-square
// (module-fraction) coordinates — 3.5 modules in from each relevant edge,
// the center of a 7x7-module finder pattern — through the code's
// square-to-quad homography built from `corners_px`.
import { mapPoint, squareToQuad, type Point2 } from "../homography";
import type { GroundTruthCode } from "../groundtruth-types";

/**
 * Returns `[TL, TR, BL]` in the same px space as `code.corners_px` (source
 * px — the caller, `groundtruth.ts`, is responsible for scaling by
 * `workingScale` before projecting through `imageToScreen`). Returns an
 * empty array for a degenerate quad rather than throwing — real
 * golden-fixture corners are never degenerate, but a layer should stay
 * robust to malformed ground-truth data rather than crash the whole
 * overlay pass (the registry also guards against this, but a layer
 * shouldn't rely solely on that backstop).
 */
export function expectedFinderCenters(code: GroundTruthCode): Point2[] {
  const h = squareToQuad(code.corners_px);
  if (!h) return [];

  const n = 4 * code.version + 17;
  const f = 3.5 / n;
  const g = (n - 3.5) / n;
  return [mapPoint(h, f, f), mapPoint(h, g, f), mapPoint(h, f, g)];
}
