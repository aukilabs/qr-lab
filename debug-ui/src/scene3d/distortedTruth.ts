import { mapPoint, squareToQuad, type Point2 } from "../overlays/homography";
import type { GroundTruthCode } from "../overlays/groundtruth-types";
import { distortPoint, type FisheyeCalibration } from "./fisheye";

/** Curves and finder centers must be mapped BEFORE discarding the pinhole
 * geometry: a homography through distorted corners cannot represent a lens. */
export function distortTruth(code: GroundTruthCode, calibration: FisheyeCalibration): GroundTruthCode {
  const project = (p: Point2) => distortPoint(p, calibration);
  const h = squareToQuad(code.corners_px);
  const outline: Point2[] = [];
  for (let edge = 0; edge < 4; edge++) {
    const a = code.corners_px[edge]!, b = code.corners_px[(edge + 1) % 4]!;
    for (let step = 0; step < 32; step++) {
      const t = step / 32;
      outline.push(project([a[0] * (1 - t) + b[0] * t, a[1] * (1 - t) + b[1] * t]));
    }
  }
  const n = 4 * code.version + 17;
  const f = 3.5 / n, g = 1 - f;
  const corners = code.corners_px.map(project) as GroundTruthCode["corners_px"];
  return {
    ...code,
    corners_px: corners,
    module_size_px: Math.hypot(corners[1][0] - corners[0][0], corners[1][1] - corners[0][1]) / n,
    outline_px: outline,
    finder_centers_px: h ? [[f, f], [g, f], [f, g]].map(([u, v]) => project(mapPoint(h, u!, v!))) : [],
  };
}
