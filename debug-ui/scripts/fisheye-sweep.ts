// Deterministic geometric benchmark. The reference pinhole renderer is an
// analytic plane homography with supersampled pixels; the lens warp and both
// scanners are the actual production simulator/WASM implementations.
import { readFileSync, writeFileSync, mkdirSync, createWriteStream } from "node:fs";
import { resolve } from "node:path";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { deflateSync, crc32 } from "node:zlib";
import { initSync, generate_qr, scan_rgba, WasmCalibratedScanner } from "../src/wasm/qr_lab_wasm.js";
import { FisheyeWarp, fisheyeCalibration, distortPoint } from "../src/scene3d/fisheye";
import { intrinsicsFromFov } from "../src/scene3d/intrinsics";
import type { CameraCalibration } from "../src/scanner/calibration";
import type { Point2 } from "../src/overlays/homography";

import { sceneGeometry, renderScene, POSITIONS, SQUARE, type Pose } from "./fisheye-scene";
type Quad = [Point2, Point2, Point2, Point2];

const PAYLOAD = "HTTPS://AUKILABS.COM/CPUSCANNER2/SCENE3D";

function boundary(project: (x: number, y: number) => Point2, quiet = 0): Point2[] {
  return SQUARE.flatMap((a, i) => {
    const b = SQUARE[(i + 1) % 4]!;
    return Array.from({ length: 65 }, (_, j) => {
      const t = j / 64;
      return project((a[0] * (1 - t) + b[0] * t) * (1 + 2 * quiet) - quiet,
        (a[1] * (1 - t) + b[1] * t) * (1 + 2 * quiet) - quiet);
    });
  });
}
function margin(points: Point2[], n: number) {
  return Math.min(...points.flatMap(([x, y]) => [x + 0.5, y + 0.5, n - 0.5 - x, n - 0.5 - y]));
}
function pitch(project: (x: number, y: number) => Point2, dim: number) {
  const sizes: number[] = [];
  for (const u of [0.5 / dim, 0.5, 1 - 1.5 / dim]) for (const v of [0.5 / dim, 0.5, 1 - 1.5 / dim]) {
    const p = project(u, v);
    for (const q of [project(u + 1 / dim, v), project(u, v + 1 / dim)]) sizes.push(Math.hypot(q[0] - p[0], q[1] - p[1]));
  }
  return Math.min(...sizes);
}
function errors(quad: Quad | null | undefined, truth: Quad) {
  if (!quad) return null;
  const per_corner = quad.map((p, i) => Math.hypot(p[0] - truth[i]![0], p[1] - truth[i]![1]));
  return { mean: per_corner.reduce((a, b) => a + b, 0) / 4, max: Math.max(...per_corner), per_corner };
}
function measure(result: any, truth: Quad, calibrated: boolean, wallMs: number) {
  const codes = result.detections.codes;
  const code = codes.find((c: any) => c.payload === PAYLOAD);
  const coarse = code ? (calibrated ? code.corners_source : code.corners) : null;
  const refined = code ? (calibrated ? code.refined_corners_source : code.refined_corners) : null;
  const refinedError = errors(refined, truth);
  const bestError = errors(refined ?? coarse, truth);
  return { decoded: !!code, unexpected_payloads: codes.filter((c: any) => c.payload !== PAYLOAD).map((c: any) => c.payload),
    coarse_error: errors(coarse, truth), refined_error: refinedError, returned_error: bestError,
    fitted_corners: code?.corner_refined.filter(Boolean).length ?? 0,
    corners: refined ?? coarse ?? null,
    within_1px: !!bestError && bestError.max <= 1, within_2px: !!bestError && bestError.max <= 2,
    wall_ms: wallMs };
}
function png(rgba: Uint8ClampedArray, n: number): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data]), out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0); body.copy(out, 4); out.writeUInt32BE(crc32(body), body.length + 4); return out;
  };
  const header = Buffer.alloc(13); header.writeUInt32BE(n, 0); header.writeUInt32BE(n, 4); header[8] = 8; header[9] = 6;
  const rows = Buffer.alloc(n * (1 + n * 4));
  for (let y = 0; y < n; y++) rows.set(rgba.subarray(y * n * 4, (y + 1) * n * 4), y * (1 + n * 4) + 1);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(rows)), chunk("IEND", Buffer.alloc(0))]);
}
function quantile(values: number[], q: number) {
  return values.length ? values.toSorted((a, b) => a - b)[Math.max(0, Math.ceil(values.length * q) - 1)]! : null;
}
export function summarize(rows: any[], mode: string) {
  const cases = rows.map(r => r[mode]);
  const errors = cases.filter(r => r.decoded).map(r => r.returned_error.max);
  return { cases: cases.length, decoded: cases.filter(r => r.decoded).length,
    within_1px: cases.filter(r => r.within_1px).length, within_2px: cases.filter(r => r.within_2px).length,
    no_refinement: cases.filter(r => r.decoded && !r.refined_error).length,
    partial_refinement: cases.filter(r => r.decoded && r.fitted_corners < 4).length,
    p50_worst_corner_px: quantile(errors, 0.5), p95_worst_corner_px: quantile(errors, 0.95),
    max_corner_px: errors.length ? Math.max(...errors) : null,
    unexpected_payloads: cases.reduce((n, r) => n + r.unexpected_payloads.length, 0) };
}

export async function run(args: string[]) {
  const flags: Record<string, string> = {};
  for (let i = 0; i < args.length; i += 2) {
    if (!args[i]?.startsWith("--") || !args[i + 1]) throw new Error("Use --name value pairs");
    flags[args[i]!.slice(2)] = args[i + 1]!;
  }
  const resolution = Number(flags.resolution ?? 640), side = Number(flags.side ?? resolution * 0.35);
  const phase = Number(flags.phase ?? 0), samples = Number(flags.samples ?? 4), version = Number(flags.version ?? 3);
  const tilts = (flags.tilts ?? "0,20,40,55,65,75,82").split(",").map(Number);
  const azimuths = (flags.azimuths ?? "0,90,180,270").split(",").map(Number);
  const margins = (flags.margins ?? "32,4,-8,-32").split(",").map(Number);
  const positions = (flags.positions ?? Object.keys(POSITIONS).join(",")).split(",");
  const strengths = (flags.strengths ?? "pinhole,0,0.25,0.5,0.75,1").split(",").map(s => s === "pinhole" ? s : String(Number(s)));
  const knownFlags = ["resolution", "side", "phase", "samples", "version", "tilts", "azimuths", "margins", "positions", "strengths", "out"];
  if (Object.keys(flags).some(key => !knownFlags.includes(key))) throw new Error("Unknown sweep option");
  if (![resolution, samples, version].every(n => Number.isInteger(n) && n > 0) || !Number.isFinite(side) || side <= 0 ||
      ![...tilts, ...azimuths, ...margins, phase].every(Number.isFinite) || positions.some(p => !POSITIONS[p]) ||
      strengths.some(s => s !== "pinhole" && (!Number.isFinite(Number(s)) || Number(s) < 0 || Number(s) > 1))) throw new Error("Invalid sweep settings");
  const out = resolve(flags.out ?? "../target/fisheye-sweep"); mkdirSync(out, { recursive: true });
  const wasmPath = resolve("src/wasm/qr_lab_wasm_bg.wasm"), wasm = readFileSync(wasmPath);
  initSync({ module: wasm });
  const qr = generate_qr(PAYLOAD, version, 1) as { dim: number; words: number[] };
  const k = intrinsicsFromFov(50, resolution, resolution);
  const lenses = strengths.map(strength => {
    const calibration: CameraCalibration = strength === "pinhole" ? { intrinsics: k, distortion: { model: "none" } } : fisheyeCalibration(k, Number(strength));
    return { strength, calibration,
      warp: strength === "pinhole" ? null : new FisheyeWarp(resolution, resolution, calibration as any),
      scanner: new WasmCalibratedScanner(resolution, resolution, calibration, 0, true) };
  });
  const metadata = { created_utc: new Date().toISOString(), node: process.version, resolution, side, phase, samples, version,
    payload: PAYLOAD, ecc: "M", max_working_dim: 0, refine: true, fov_deg: 50, tilts, azimuths, margins, positions, strengths,
    camera: k, wasm_sha256: createHash("sha256").update(wasm).digest("hex"),
    renderer_sha256: createHash("sha256").update(readFileSync(resolve("scripts/fisheye-scene.ts"))).digest("hex"),
    lens_warp_sha256: createHash("sha256").update(readFileSync(resolve("src/scene3d/fisheye.ts"))).digest("hex"),
    git_head: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    methodology: "Supersampled analytic pinhole plane (samples x samples per pixel), actual simulator bilinear fisheye warp, cached compiled WASM scanners. No noise/blur. Tilt relative to image plane; incidence measured separately. Corner errors in original sensor pixels. Missing decodes have null error, never zero. No statistical independence implied." };
  writeFileSync(resolve(out, "metadata.json"), JSON.stringify(metadata, null, 2));
  const rows: any[] = [], poses: Pose[] = [];
  for (const tilt of tilts) for (const azimuth of tilt === 0 ? [azimuths[0]!] : azimuths) for (const position of positions) {
    for (const margin of position === "center" ? [null] : margins) poses.push({ tilt, azimuth, position, margin, resolution, side, phase, dim: qr.dim });
  }
  const stream = createWriteStream(resolve(out, "cases.jsonl"));
  const start = performance.now();
  const blank = new Uint8Array(resolution * resolution * 4).fill(255);
  for (const lens of lenses) lens.scanner.scan_rgba(blank);
  scan_rgba(blank, resolution, resolution, 0, false, true);
  try {
    for (let pi = 0; pi < poses.length; pi++) {
      const pose = poses[pi]!, g = sceneGeometry(pose), source = renderScene(pose, qr.words, samples);
      for (const lens of lenses) {
        const project = (u: number, v: number): Point2 => lens.warp ? distortPoint(g.project(u, v), lens.calibration as any) : g.project(u, v);
        const truth = SQUARE.map(([u, v]) => project(u, v)) as Quad;
        const rgba = lens.warp ? lens.warp.apply(source) : source;
        const bytes = new Uint8Array(rgba.buffer, rgba.byteOffset, rgba.byteLength);
        let t = performance.now(); const raw = scan_rgba(bytes, resolution, resolution, 0, false, true);
        const uncorrected = measure(raw, truth, false, performance.now() - t);
        t = performance.now(); const corrected = lens.scanner.scan_rgba(bytes);
        const calibrated = measure(corrected, truth, true, performance.now() - t);
        const sourceModuleMargin = margin(g.corners, resolution), sourceQuietMargin = margin(g.quietCorners, resolution);
        const row = { id: `case_${String(rows.length).padStart(5, "0")}`, ...pose,
          lens: lens.strength === "pinhole" ? "pinhole" : "fisheye", strength: lens.strength === "pinhole" ? null : Number(lens.strength),
          k1: lens.strength === "pinhole" ? null : -0.5 * Number(lens.strength), incidence_deg: g.incidence,
          front_facing: g.frontFacing, source_module_margin_px: sourceModuleMargin,
          source_quiet_margin_px: sourceQuietMargin, module_clipped: sourceModuleMargin < -1e-6,
          quiet_clipped: sourceQuietMargin < -1e-6,
          sensor_module_margin_px: margin(boundary(project), resolution), sensor_quiet_margin_px: margin(boundary(project, g.quiet), resolution),
          min_module_pitch_px: pitch(project, pose.dim), truth, uncorrected, calibrated };
        rows.push(row); stream.write(JSON.stringify(row) + "\n");
      }
      if ((pi + 1) % 25 === 0 || pi + 1 === poses.length) {
        console.log(`${pi + 1}/${poses.length} poses; ${rows.length} images; ${((performance.now() - start) / 1000).toFixed(1)}s`);
        await new Promise<void>(r => setImmediate(r));
      }
    }
  } finally { for (const lens of lenses) lens.scanner.free(); await new Promise<void>(r => stream.end(r)); }
  const clean = rows.filter(r => r.front_facing && !r.quiet_clipped);
  const summary: any = { elapsed_seconds: (performance.now() - start) / 1000, images: rows.length,
    clean: { uncorrected: summarize(clean, "uncorrected"), calibrated: summarize(clean, "calibrated") },
    by_tilt: tilts.map(tilt => ({ tilt, uncorrected: summarize(clean.filter(r => r.tilt === tilt), "uncorrected"), calibrated: summarize(clean.filter(r => r.tilt === tilt), "calibrated") })),
    by_lens: strengths.map(s => ({ lens: s, uncorrected: summarize(clean.filter(r => (r.strength === null ? "pinhole" : String(r.strength)) === s), "uncorrected"), calibrated: summarize(clean.filter(r => (r.strength === null ? "pinhole" : String(r.strength)) === s), "calibrated") })) };
  writeFileSync(resolve(out, "summary.json"), JSON.stringify(summary, null, 2));
  const fields = ["id", "lens", "strength", "tilt", "azimuth", "position", "margin", "incidence_deg", "front_facing", "module_clipped", "quiet_clipped", "sensor_module_margin_px", "sensor_quiet_margin_px", "min_module_pitch_px"];
  const metricFields = ["decoded", "within_1px", "within_2px", "fitted_corners", "wall_ms"];
  const csv = [fields.concat(["uncorrected", "calibrated"].flatMap(m => metricFields.map(k => `${m}_${k}`).concat(`${m}_mean_error_px`, `${m}_max_error_px`))).join(",")];
  for (const row of rows) csv.push(fields.map(k => row[k] ?? "").concat(["uncorrected", "calibrated"].flatMap(m => metricFields.map(k => row[m][k]).concat(row[m].returned_error?.mean ?? "", row[m].returned_error?.max ?? ""))).join(","));
  writeFileSync(resolve(out, "cases.csv"), csv.join("\n") + "\n");
  // Preserve diagnostic fixtures, not every large image. Select before looking
  // at individual images: low-tilt failures, worst corner errors, calibration rescues.
  const selected = new Map<string, any>();
  const groups = {
    low_tilt_decode_failure: clean.filter(r => !r.calibrated.decoded).sort((a,b) => a.tilt - b.tilt || b.min_module_pitch_px - a.min_module_pitch_px).slice(0, 4),
    worst_corner_error: clean.filter(r => r.calibrated.decoded).sort((a,b) => b.calibrated.returned_error.max - a.calibrated.returned_error.max).slice(0, 4),
    calibrated_rescue: clean.filter(r => r.calibrated.within_1px && !r.uncorrected.decoded).slice(0, 4),
  };
  for (const [reason, cases] of Object.entries(groups)) for (const row of cases as any[]) {
    selected.set(row.id, { ...row, reason });
  }
  for (const row of selected.values()) {
    const source = renderScene(row, qr.words, samples);
    const lens = lenses.find(l => l.strength === (row.strength === null ? "pinhole" : String(row.strength)))!;
    const rgba = lens.warp ? lens.warp.apply(source) : source;
    writeFileSync(resolve(out, `${row.id}.png`), png(rgba, resolution));
    writeFileSync(resolve(out, `${row.id}.json`), JSON.stringify({ ...row, camera: lens.calibration }, null, 2));
  }
  writeFileSync(resolve(out, "examples.json"), JSON.stringify([...selected.values()], null, 2));
  console.log(JSON.stringify(summary, null, 2)); console.log(`Results: ${out}`);
}
