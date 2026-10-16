/// <reference types="node" />
// Requires `npm run build:wasm`; run with `npm run test:calibrated`.
// Exercises the actual simulator warp -> compiled Rust scanner -> UI parser.
import { readFileSync } from "node:fs";
import { beforeAll, expect, it } from "vitest";
import { initSync, generate_qr, scan_rgba, WasmCalibratedScanner } from "../wasm/qr_lab_wasm.js";
import { parseCalibratedScanResult, parseScanResult } from "../scanner/types";
import { FisheyeWarp, distortPoint, fisheyeCalibration } from "./fisheye";
import { intrinsicsFromFov } from "./intrinsics";
import type { Point2 } from "../overlays/homography";

beforeAll(() => {
  initSync({ module: readFileSync(new URL("../wasm/qr_lab_wasm_bg.wasm", import.meta.url)) });
});

it.each([
  { resolution: 640, strength: 0, maxDim: 0, inverted: false },
  { resolution: 960, strength: 0.5, maxDim: 0, inverted: false },
  { resolution: 1280, strength: 1, maxDim: 0, inverted: false },
  { resolution: 641, strength: 1, maxDim: 400, inverted: true },
])("decodes distorted scene pixels and returns sensor corners: %j", ({ resolution, strength, maxDim, inverted }) => {
  const payload = "QR-LAB FISHEYE SIMULATION";
  const qr = generate_qr(payload, 3, 1) as { dim: number; words: number[] };
  const module = Math.floor(resolution / 90);
  const x0 = 4 * module, y0 = 5 * module; // deliberately off-axis
  const rgba = new Uint8ClampedArray(resolution * resolution * 4).fill(inverted ? 0 : 255);
  for (let y = 0; y < resolution; y++) for (let x = 0; x < resolution; x++) {
    const mx = Math.floor((x - x0) / module), my = Math.floor((y - y0) / module);
    const dark = mx >= 0 && my >= 0 && mx < qr.dim && my < qr.dim &&
      ((qr.words[my * Math.ceil(qr.dim / 32) + Math.floor(mx / 32)]! >>> (mx % 32)) & 1) !== 0;
    const i = 4 * (y * resolution + x);
    const v = dark !== inverted ? 0 : 255;
    rgba[i] = v; rgba[i + 1] = v; rgba[i + 2] = v; rgba[i + 3] = 255;
  }
  const calibration = fisheyeCalibration(intrinsicsFromFov(50, resolution, resolution), strength);
  const distorted = new FisheyeWarp(resolution, resolution, calibration).apply(rgba);
  const scanner = new WasmCalibratedScanner(resolution, resolution, calibration, maxDim, true);
  try {
    const bytes = new Uint8Array(distorted.buffer);
    const a = x0 - 0.5, b = y0 - 0.5, side = qr.dim * module;
    const truth = ([[a, b], [a + side, b], [a + side, b + side], [a, b + side]] as Point2[])
      .map(p => distortPoint(p, calibration));
    for (let frame = 0; frame < 2; frame++) {
      const raw = scanner.scan_rgba(bytes);
      expect([raw.scan_width, raw.scan_height]).toEqual([resolution, resolution]);
      const result = parseCalibratedScanResult(raw);
      expect(result.trace).toBeNull();
      expect(result.detections.finders).toEqual([]);
      expect(result.detections.codes).toHaveLength(1);
      const code = result.detections.codes[0]!;
      expect(code.payload).toBe(payload);
      expect(code.inverted).toBe(inverted);
      expect(code.finder_indices).toBeNull();
      expect(code.refined_corners).not.toBeNull();
      const error = Math.max(...code.refined_corners!.map((p, i) => Math.hypot(p[0] - truth[i]![0], p[1] - truth[i]![1])));
      // Integer raster edges + bilinear remapping: require subpixel accuracy.
      expect(error).toBeLessThan(1);
      if (strength === 1 && !inverted && frame === 0) {
        const rawCode = parseScanResult(scan_rgba(bytes, resolution, resolution, 0, false, true)).detections.codes[0];
        if (rawCode?.refined_corners) {
          const rawError = Math.max(...rawCode.refined_corners.map((p, i) => Math.hypot(p[0] - truth[i]![0], p[1] - truth[i]![1])));
          expect(rawError).toBeGreaterThan(error + 1);
        }
      }
    }
    expect(() => scanner.scan_rgba(new Uint8Array(4))).toThrow();
    // Invalid input does not poison the reusable WASM scanner.
    expect(parseCalibratedScanResult(scanner.scan_rgba(bytes)).detections.codes).toHaveLength(1);
  } finally { scanner.free(); }
});

it("rejects malformed calibration at the WASM boundary", () => {
  const calibration = fisheyeCalibration(intrinsicsFromFov(50, 640, 640), 0.5);
  expect(() => new WasmCalibratedScanner(640, 640, { ...calibration, intrinsics: { ...calibration.intrinsics, fx: 0 } }, 0, true)).toThrow();
  expect(() => new WasmCalibratedScanner(640, 640, { ...calibration, distortion: { model: "fisheye", coefficients: [0, 0] } }, 0, true)).toThrow();
});
