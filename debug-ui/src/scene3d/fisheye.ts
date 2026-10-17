// Synthetic OpenCV fisheye: theta_d = theta * (1 + k1 * theta^2).
// A cached inverse map resamples the rendered pinhole image. The source FOV
// stays fixed, so rays outside that render receive the scene background;
// this is a lens-warp test, not a 180-degree environment renderer.
import type { CameraCalibration, Intrinsics } from "../scanner/calibration";
import type { Point2 } from "../overlays/homography";

export type FisheyeCalibration = CameraCalibration & {
  distortion: { model: "fisheye"; coefficients: [number, number, number, number] };
};

export function fisheyeCalibration(intrinsics: Intrinsics, strength: number): FisheyeCalibration {
  if (!Number.isFinite(strength) || strength < 0 || strength > 1) {
    throw new RangeError("fisheye strength must be in [0, 1]");
  }
  // Even strength 0 is equidistant fisheye (D=0), not pinhole.
  return { intrinsics, distortion: { model: "fisheye", coefficients: [-0.5 * strength, 0, 0, 0] } };
}

export function distortPoint([px, py]: Point2, calibration: FisheyeCalibration): Point2 {
  const { fx, fy, cx, cy } = calibration.intrinsics;
  const x = (px - cx) / fx;
  const y = (py - cy) / fy;
  const r = Math.hypot(x, y);
  if (r === 0) return [cx, cy];
  const theta = Math.atan(r);
  const scale = theta * (1 + calibration.distortion.coefficients[0] * theta * theta) / r;
  return [fx * x * scale + cx, fy * y * scale + cy];
}

/** Inverse lens map plus reusable output. No frame allocations or iterative
 * solves during remapping. Input/output must be distinct (output is reused). */
export class FisheyeWarp {
  private readonly map: Float32Array;
  private readonly output: Uint8ClampedArray<ArrayBuffer>;

  constructor(
    readonly width: number,
    readonly height: number,
    readonly calibration: FisheyeCalibration,
    private readonly border: readonly [number, number, number] = [5, 7, 13],
  ) {
    const { fx, fy, cx, cy } = calibration.intrinsics;
    const [k1, k2, k3, k4] = calibration.distortion.coefficients;
    if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width <= 0 || height <= 0 ||
        ![fx, fy, cx, cy, k1, k2, k3, k4].every(Number.isFinite) || fx <= 0 || fy <= 0 ||
        k2 !== 0 || k3 !== 0 || k4 !== 0) {
      throw new RangeError("invalid synthetic fisheye camera");
    }
    const maxX = Math.max(Math.abs(cx), Math.abs(width - 1 - cx)) / fx;
    const maxY = Math.max(Math.abs(cy), Math.abs(height - 1 - cy)) / fy;
    const thetaMax = Math.atan(Math.hypot(maxX, maxY));
    if (1 + 3 * k1 * thetaMax * thetaMax <= 0) {
      throw new RangeError("fisheye mapping must be monotonic over the rendered view");
    }
    const radiusMax = thetaMax * (1 + k1 * thetaMax * thetaMax);
    this.map = new Float32Array(width * height * 2).fill(-1);
    this.output = new Uint8ClampedArray(width * height * 4);
    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const nx = (x - cx) / fx;
        const ny = (y - cy) / fy;
        const rd = Math.hypot(nx, ny);
        if (rd > radiusMax) continue;
        let lo = 0, hi = thetaMax;
        for (let i = 0; i < 28; i++) {
          const theta = (lo + hi) / 2;
          if (theta * (1 + k1 * theta * theta) < rd) lo = theta;
          else hi = theta;
        }
        const scale = rd === 0 ? 1 : Math.tan((lo + hi) / 2) / rd;
        const sx = cx + fx * nx * scale;
        const sy = cy + fy * ny * scale;
        if (sx < 0 || sy < 0 || sx > width - 1 || sy > height - 1) continue;
        const offset = 2 * (y * width + x);
        this.map[offset] = sx;
        this.map[offset + 1] = sy;
      }
    }
  }

  apply(rgba: Uint8ClampedArray): Uint8ClampedArray<ArrayBuffer> {
    const { width, height, output, map, border } = this;
    if (rgba.length !== output.length || rgba.buffer === output.buffer) {
      throw new RangeError("fisheye input must be a separate, correctly sized RGBA buffer");
    }
    for (let p = 0; p < width * height; p++) {
      const x = map[2 * p]!, y = map[2 * p + 1]!;
      const dst = 4 * p;
      if (x < 0) {
        output[dst] = border[0]; output[dst + 1] = border[1]; output[dst + 2] = border[2];
        output[dst + 3] = 255;
        continue;
      }
      const x0 = Math.floor(x), y0 = Math.floor(y);
      const x1 = Math.min(x0 + 1, width - 1), y1 = Math.min(y0 + 1, height - 1);
      const dx = x - x0, dy = y - y0;
      for (let c = 0; c < 4; c++) {
        const a = rgba[4 * (y0 * width + x0) + c]! * (1 - dx) + rgba[4 * (y0 * width + x1) + c]! * dx;
        const b = rgba[4 * (y1 * width + x0) + c]! * (1 - dx) + rgba[4 * (y1 * width + x1) + c]! * dx;
        output[dst + c] = a * (1 - dy) + b * dy;
      }
    }
    return output;
  }
}
