// TS mirror of the golden-fixture JSON's per-code shape (see
// `fixtures/*.json`'s `codes[]`, and `crates/qrk-core/tests/common/mod.rs`'s
// `CodeTruth` for the Rust-side deserialization contract) — trimmed to only
// the fields the overlay layers need (the ground-truth quad + expected
// finder centers). Extra fixture fields (`ecc`, `mirrored`, `distance_m`,
// `physical_size_m`, `tilt_deg`, ...) are intentionally not modeled here;
// Task 6's source panel is the eventual owner of loading/assembling the
// full `GroundTruthCode[]` array from a fixture file.
//
// `parseGroundTruth` follows `scanner/types.ts`'s `parseScanResult`
// defensive-parsing style (structural validation with a field path on
// failure) but is a self-contained, smaller copy of the same handful of
// helpers rather than a shared import — `types.ts` doesn't export them
// either, and duplicating ~10 lines here avoids coupling two independently
// evolving contracts (the wasm envelope vs. the fixture JSON) through a
// shared internal module.
import type { Point2 } from "./homography";

export interface GroundTruthCode {
  /** [TL, TR, BR, BL], source-image px (see homography.rs's
   * `square_to_quad` convention) — NOT working-res px; the `groundtruth`
   * layer scales by `OverlayContext.workingScale` before drawing. */
  corners_px: [Point2, Point2, Point2, Point2];
  version: number;
  module_size_px: number;
  inverted: boolean;
  opaque_plate: boolean;
  payload: string;
}

/** Thrown by {@link parseGroundTruth} when the input doesn't structurally
 * match {@link GroundTruthCode}. `path` is a JS-ish accessor path (e.g.
 * `"corners_px[1]"`) pointing at the first field that failed validation. */
export class GroundTruthParseError extends Error {
  readonly path: string;

  constructor(path: string, message: string) {
    super(`GroundTruthCode: ${path || "<root>"}: ${message}`);
    this.name = "GroundTruthParseError";
    this.path = path;
  }
}

function fail(path: string, message: string): never {
  throw new GroundTruthParseError(path, message);
}

function joinPath(path: string, key: string): string {
  return path ? `${path}.${key}` : key;
}

function indexPath(path: string, index: number): string {
  return `${path}[${index}]`;
}

function typeOf(v: unknown): string {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  return typeof v;
}

function expectObject(v: unknown, path: string): Record<string, unknown> {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    fail(path, `expected an object, got ${typeOf(v)}`);
  }
  return v as Record<string, unknown>;
}

function expectNumber(v: unknown, path: string): number {
  if (typeof v !== "number" || Number.isNaN(v)) {
    fail(path, `expected a number, got ${typeOf(v)}`);
  }
  return v;
}

function expectBoolean(v: unknown, path: string): boolean {
  if (typeof v !== "boolean") {
    fail(path, `expected a boolean, got ${typeOf(v)}`);
  }
  return v;
}

function expectString(v: unknown, path: string): string {
  if (typeof v !== "string") {
    fail(path, `expected a string, got ${typeOf(v)}`);
  }
  return v;
}

function expectArray(v: unknown, path: string): unknown[] {
  if (!Array.isArray(v)) {
    fail(path, `expected an array, got ${typeOf(v)}`);
  }
  return v;
}

function expectField(obj: Record<string, unknown>, key: string, path: string): unknown {
  if (!(key in obj)) {
    fail(joinPath(path, key), "required field is missing");
  }
  return obj[key];
}

function parsePoint2(v: unknown, path: string): Point2 {
  const arr = expectArray(v, path);
  if (arr.length !== 2) {
    fail(path, `expected a 2-element tuple, got ${arr.length} elements`);
  }
  return [expectNumber(arr[0], indexPath(path, 0)), expectNumber(arr[1], indexPath(path, 1))];
}

function parseCorners(v: unknown, path: string): [Point2, Point2, Point2, Point2] {
  const arr = expectArray(v, path);
  if (arr.length !== 4) {
    fail(path, `expected a 4-element tuple, got ${arr.length} elements`);
  }
  const pts = arr.map((item, i) => parsePoint2(item, indexPath(path, i)));
  return [pts[0]!, pts[1]!, pts[2]!, pts[3]!];
}

/**
 * Structurally validate an arbitrary JSON value (one entry of a
 * golden-fixture's `codes[]` array) against {@link GroundTruthCode},
 * returning a type-narrowed copy with only the modeled fields on success.
 * Throws {@link GroundTruthParseError} — with a field path — on the first
 * mismatch.
 */
export function parseGroundTruth(v: unknown): GroundTruthCode {
  const obj = expectObject(v, "");
  return {
    corners_px: parseCorners(expectField(obj, "corners_px", ""), "corners_px"),
    version: expectNumber(expectField(obj, "version", ""), "version"),
    module_size_px: expectNumber(expectField(obj, "module_size_px", ""), "module_size_px"),
    inverted: expectBoolean(expectField(obj, "inverted", ""), "inverted"),
    opaque_plate: expectBoolean(expectField(obj, "opaque_plate", ""), "opaque_plate"),
    payload: expectString(expectField(obj, "payload", ""), "payload"),
  };
}
