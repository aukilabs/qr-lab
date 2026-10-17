import { describe, expect, it } from "vitest";
import { FisheyeWarp, distortPoint, fisheyeCalibration } from "./fisheye";
import { intrinsicsFromFov } from "./intrinsics";
import { distortTruth } from "./distortedTruth";
import { buildFixtureMeta } from "./fixtureExport";
import { parseGroundTruth, type GroundTruthCode } from "../overlays/groundtruth-types";
import { expectedFinderCenters } from "../overlays/layers/expected-finder-centers";
import { sensorViewActive } from "./sensorView";

const k = intrinsicsFromFov(50, 101, 101);

describe("synthetic fisheye", () => {
  it("matches the equidistant analytic projection, including D=0 and optical center", () => {
    const camera = fisheyeCalibration({ fx: 100, fy: 110, cx: 50, cy: 60 }, 0);
    expect(distortPoint([50, 60], camera)).toEqual([50, 60]);
    const q = distortPoint([150, 60], camera);
    expect(q[0]).toBeCloseTo(50 + 100 * Math.PI / 4, 12);
    expect(q[1]).toBe(60);
    expect(distortPoint([150, 60], fisheyeCalibration(camera.intrinsics, 1))[0])
      .toBeCloseTo(50 + 100 * (Math.PI / 4) * (1 - 0.5 * (Math.PI / 4) ** 2), 12);
  });

  it.each([0, 0.5, 1])("inverse resampling agrees with forward geometry (strength %s)", (strength) => {
    // Encode source x/y in channels, so sampling recovers the inverse map.
    const rgba = new Uint8ClampedArray(101 * 101 * 4);
    for (let y = 0; y < 101; y++) for (let x = 0; x < 101; x++) {
      const p = 4 * (y * 101 + x);
      rgba[p] = x * 2; rgba[p + 1] = y * 2; rgba[p + 3] = 255;
    }
    const original = rgba.slice();
    const camera = fisheyeCalibration(k, strength);
    const warp = new FisheyeWarp(101, 101, camera);
    const result = warp.apply(rgba);
    for (const [x, y] of [[50, 50], [20, 35], [75, 80], [50, 15]]) {
      const p = 4 * (y! * 101 + x!);
      const recovered = distortPoint([result[p]! / 2, result[p + 1]! / 2], camera);
      expect(Math.hypot(recovered[0] - x!, recovered[1] - y!)).toBeLessThan(0.36);
    }
    expect(Array.from(result.slice(0, 4))).toEqual([5, 7, 13, 255]);
    expect(rgba).toEqual(original);
    expect(warp.apply(rgba)).toBe(result); // cache reused
    expect(() => warp.apply(result)).toThrow(RangeError);
    expect(() => warp.apply(new Uint8ClampedArray(4))).toThrow(RangeError);
  });

  it("rejects unsupported or folded models", () => {
    expect(() => fisheyeCalibration(k, NaN)).toThrow(RangeError);
    expect(() => fisheyeCalibration(k, 1.1)).toThrow(RangeError);
    expect(() => new FisheyeWarp(101, 101, fisheyeCalibration({ ...k, fx: 1, fy: 1 }, 1))).toThrow(/monotonic/);
  });

  it("exports distorted corners, curved edges, finder centers, and matching calibration", () => {
    const camera = fisheyeCalibration(k, 1);
    const original: GroundTruthCode = {
      corners_px: [[10, 10], [85, 15], [80, 90], [15, 80]], version: 2,
      module_size_px: 3, payload: "FISHEYE", inverted: false, opaque_plate: true,
    };
    const truth = distortTruth(original, camera);
    expect(truth.corners_px).toEqual(original.corners_px.map(p => distortPoint(p, camera)));
    expect(truth.outline_px).toHaveLength(128);
    expect(truth.outline_px![32]).toEqual(truth.corners_px[1]);
    expect(expectedFinderCenters(truth)).toEqual(expectedFinderCenters(original).map(p => distortPoint(p, camera)));
    const meta = buildFixtureMeta({
      name: "fisheye", width: 101, height: 101, camera: k, distortion: camera.distortion,
      blurSigma: 0, noiseSigma: 0, exposureOffset: 0,
      code: { payload: truth.payload, version: 2, eccLetter: "m", physicalSizeM: 0.15,
        distanceM: 0.3, tiltDeg: 0, moduleSizePx: truth.module_size_px, cornersPx: truth.corners_px,
        outlinePx: truth.outline_px!, finderCentersPx: truth.finder_centers_px!, inverted: false, opaquePlate: true },
    });
    const saved = JSON.parse(JSON.stringify(meta));
    expect(saved.camera).toEqual({ ...k, distortion: camera.distortion });
    expect(parseGroundTruth(saved.codes[0])).toEqual(truth);
    expect(sensorViewActive("off", { fisheye: true, blurSigma: 0, noiseSigma: 0, exposureOffset: 0 })).toBe(true);
  });
});
