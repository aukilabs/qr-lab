import { Matrix3 } from "three";
import { intrinsicsFromFov } from "../src/scene3d/intrinsics";
import type { Point2 } from "../src/overlays/homography";

type Vec3 = [number, number, number];
type Quad = [Point2, Point2, Point2, Point2];
export interface Pose {
  tilt: number; azimuth: number; position: string; margin: number | null;
  resolution: number; side: number; phase: number; dim: number;
}
export const POSITIONS: Record<string, [number, number]> = {
  center: [0, 0], left: [-1, 0], right: [1, 0], top: [0, -1], bottom: [0, 1],
  tl: [-1, -1], tr: [1, -1], br: [1, 1], bl: [-1, 1],
};
export const SQUARE: Quad = [[0, 0], [1, 0], [1, 1], [0, 1]];

/** Rotation about an image-plane axis, parameterized by tilt and its azimuth.
 * Coordinates: camera x right, y down, z forward; normal at tilt 0 faces camera.
 * Tilt is relative to the IMAGE PLANE. Actual incidence also depends on position. */
export function sceneGeometry(pose: Pose) {
  const k = intrinsicsFromFov(50, pose.resolution, pose.resolution);
  const a = pose.azimuth * Math.PI / 180, t = pose.tilt * Math.PI / 180;
  const ca = Math.cos(a), sa = Math.sin(a), ct = Math.cos(t), st = Math.sin(t);
  const u: Vec3 = [ct * ca * ca + sa * sa, (ct - 1) * ca * sa, -st * ca];
  const v: Vec3 = [(ct - 1) * ca * sa, ct * sa * sa + ca * ca, -st * sa];
  const z = k.fx / pose.side;
  const quiet = 4 / pose.dim;
  const local = (x: number, y: number): Vec3 => [
    u[0] * (x - 0.5) + v[0] * (y - 0.5),
    u[1] * (x - 0.5) + v[1] * (y - 0.5),
    z + u[2] * (x - 0.5) + v[2] * (y - 0.5),
  ];
  const outer = SQUARE.map(([x, y]) => local(x * (1 + 2 * quiet) - quiet, y * (1 + 2 * quiet) - quiet));
  const [sx, sy] = POSITIONS[pose.position]!;
  const placement = (sign: number, axis: number, f: number, c: number) => {
    if (sign === 0) return pose.phase * z / f;
    // Image boundaries are at -0.5 and N-0.5, not pixel centers 0 and N-1.
    const edge = (sign < 0 ? -0.5 + pose.margin! : pose.resolution - 0.5 - pose.margin!) + pose.phase;
    const candidates = outer.map(p => (edge - c) * p[2]! / f - p[axis]!);
    return sign < 0 ? Math.max(...candidates) : Math.min(...candidates);
  };
  const tx = placement(sx, 0, k.fx, k.cx), ty = placement(sy, 1, k.fy, k.cy);
  const origin = local(0, 0); origin[0] += tx; origin[1] += ty;
  const h = new Matrix3().set(
    k.fx * u[0] + k.cx * u[2], k.fx * v[0] + k.cx * v[2], k.fx * origin[0] + k.cx * origin[2],
    k.fy * u[1] + k.cy * u[2], k.fy * v[1] + k.cy * v[2], k.fy * origin[1] + k.cy * origin[2],
    u[2], v[2], origin[2],
  );
  const project = (x: number, y: number): Point2 => {
    const p = local(x, y);
    return [k.fx * (p[0] + tx) / p[2] + k.cx, k.fy * (p[1] + ty) / p[2] + k.cy];
  };
  const normal: Vec3 = [st * ca, st * sa, ct];
  const dot = (normal[0] * tx + normal[1] * ty + normal[2] * z) / Math.hypot(tx, ty, z);
  const incidence = Math.acos(Math.max(-1, Math.min(1, dot))) * 180 / Math.PI;
  return { k, quiet, inverse: h.clone().invert().elements, project, frontFacing: dot > 0,
    incidence, corners: SQUARE.map(([x, y]) => project(x, y)) as Quad,
    quietCorners: SQUARE.map(([x, y]) => project(x * (1 + 2 * quiet) - quiet, y * (1 + 2 * quiet) - quiet)) as Quad };
}

/** Supersampling at subpixel centers, with an opaque four-module quiet zone.
 * The exact QR boundary remains u/v=0/1; no fitted quad supplies ground truth. */
export function renderScene(pose: Pose, words: number[], samples = 2): Uint8ClampedArray {
  const g = sceneGeometry(pose), n = pose.resolution;
  const rgba = new Uint8ClampedArray(n * n * 4);
  for (let p = 0; p < n * n; p++) rgba.set([5, 7, 13, 255], 4 * p);
  if (!g.frontFacing) return rgba;
  const m = g.inverse;
  const xs = g.quietCorners.map(p => p[0]), ys = g.quietCorners.map(p => p[1]);
  const left = Math.max(0, Math.floor(Math.min(...xs) - 1));
  const right = Math.min(n - 1, Math.ceil(Math.max(...xs) + 1));
  const top = Math.max(0, Math.floor(Math.min(...ys) - 1));
  const bottom = Math.min(n - 1, Math.ceil(Math.max(...ys) + 1));
  const samples2 = samples * samples, stride = Math.ceil(pose.dim / 32);
  for (let y = top; y <= bottom; y++) for (let x = left; x <= right; x++) {
    let inside = 0, white = 0;
    for (let j = 0; j < samples; j++) for (let i = 0; i < samples; i++) {
      const px = x + (i + 0.5) / samples - 0.5, py = y + (j + 0.5) / samples - 0.5;
      const w = m[2]! * px + m[5]! * py + m[8]!;
      const u = (m[0]! * px + m[3]! * py + m[6]!) / w;
      const v = (m[1]! * px + m[4]! * py + m[7]!) / w;
      if (u < -g.quiet || v < -g.quiet || u >= 1 + g.quiet || v >= 1 + g.quiet) continue;
      inside++;
      const mx = Math.floor(u * pose.dim), my = Math.floor(v * pose.dim);
      const dark = mx >= 0 && my >= 0 && mx < pose.dim && my < pose.dim &&
        ((words[my * stride + (mx >>> 5)]! >>> (mx % 32)) & 1) !== 0;
      if (!dark) white++;
    }
    const offset = 4 * (y * n + x);
    for (let c = 0; c < 3; c++) rgba[offset + c] = (white * 255 + (samples2 - inside) * [5, 7, 13][c]!) / samples2;
  }
  return rgba;
}
