// TypeScript mirror of the JSON envelope `qrk-wasm`'s `scan_rgba` returns
// (via `serde-wasm-bindgen`) and, byte-for-byte, of
// `debug-ui/src/scanner/__snapshots__/envelope.near_00.json` — the
// Rust-generated snapshot from `crates/qrk-wasm/tests/envelope_snapshot.rs`.
// That snapshot is the source of truth: these field names were read off it,
// not guessed. If `WasmResult`'s shape changes on the Rust side, regenerate
// the snapshot (`UPDATE_SNAPSHOT=1 cargo test -p qrk-wasm --test
// envelope_snapshot`) and update the types/parser here to match — the
// envelope.test.ts suite fails loudly if the two drift apart.

export interface FinderCandidate {
  x: number;
  y: number;
  module: number;
  inverted: boolean;
  hits: number;
}

export interface TripletCandidate {
  tl: [number, number];
  tr: [number, number];
  bl: [number, number];
  module: number;
  dimension: number;
  snap_error: number;
  inverted: boolean;
}

export interface StageTimings {
  tiles_ns: number;
  finders_ns: number;
  triplets_ns: number;
}

export interface Detections {
  finders: FinderCandidate[];
  triplets: TripletCandidate[];
  timings: StageTimings;
}

export interface TileTrace {
  tiles_x: number;
  tiles_y: number;
  thresholds: number[];
  skip: boolean[];
}

export interface Trace {
  tiles: TileTrace | null;
  finders: FinderCandidate[];
  triplets: TripletCandidate[];
}

export interface ScanResult {
  detections: Detections;
  trace: Trace | null;
}

/**
 * Thrown by {@link parseScanResult} when the input doesn't structurally
 * match {@link ScanResult}. `path` is a JS-ish accessor path (e.g.
 * `"detections.finders[2].module"`) pointing at the first field that
 * failed validation, so callers can report *where* the wasm contract
 * drifted instead of just "invalid data".
 */
export class ScanResultParseError extends Error {
  readonly path: string;

  constructor(path: string, message: string) {
    super(`ScanResult: ${path || "<root>"}: ${message}`);
    this.name = "ScanResultParseError";
    this.path = path;
  }
}

function fail(path: string, message: string): never {
  throw new ScanResultParseError(path, message);
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

function parsePair(v: unknown, path: string): [number, number] {
  const arr = expectArray(v, path);
  if (arr.length !== 2) {
    fail(path, `expected a 2-element tuple, got ${arr.length} elements`);
  }
  return [
    expectNumber(arr[0], indexPath(path, 0)),
    expectNumber(arr[1], indexPath(path, 1)),
  ];
}

function parseFinderCandidate(v: unknown, path: string): FinderCandidate {
  const obj = expectObject(v, path);
  return {
    x: expectNumber(expectField(obj, "x", path), joinPath(path, "x")),
    y: expectNumber(expectField(obj, "y", path), joinPath(path, "y")),
    module: expectNumber(expectField(obj, "module", path), joinPath(path, "module")),
    inverted: expectBoolean(
      expectField(obj, "inverted", path),
      joinPath(path, "inverted"),
    ),
    hits: expectNumber(expectField(obj, "hits", path), joinPath(path, "hits")),
  };
}

function parseFinderCandidateArray(v: unknown, path: string): FinderCandidate[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parseFinderCandidate(item, indexPath(path, i)));
}

function parseTripletCandidate(v: unknown, path: string): TripletCandidate {
  const obj = expectObject(v, path);
  return {
    tl: parsePair(expectField(obj, "tl", path), joinPath(path, "tl")),
    tr: parsePair(expectField(obj, "tr", path), joinPath(path, "tr")),
    bl: parsePair(expectField(obj, "bl", path), joinPath(path, "bl")),
    module: expectNumber(expectField(obj, "module", path), joinPath(path, "module")),
    dimension: expectNumber(
      expectField(obj, "dimension", path),
      joinPath(path, "dimension"),
    ),
    snap_error: expectNumber(
      expectField(obj, "snap_error", path),
      joinPath(path, "snap_error"),
    ),
    inverted: expectBoolean(
      expectField(obj, "inverted", path),
      joinPath(path, "inverted"),
    ),
  };
}

function parseTripletCandidateArray(v: unknown, path: string): TripletCandidate[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parseTripletCandidate(item, indexPath(path, i)));
}

function parseStageTimings(v: unknown, path: string): StageTimings {
  const obj = expectObject(v, path);
  return {
    tiles_ns: expectNumber(
      expectField(obj, "tiles_ns", path),
      joinPath(path, "tiles_ns"),
    ),
    finders_ns: expectNumber(
      expectField(obj, "finders_ns", path),
      joinPath(path, "finders_ns"),
    ),
    triplets_ns: expectNumber(
      expectField(obj, "triplets_ns", path),
      joinPath(path, "triplets_ns"),
    ),
  };
}

function parseDetections(v: unknown, path: string): Detections {
  const obj = expectObject(v, path);
  return {
    finders: parseFinderCandidateArray(
      expectField(obj, "finders", path),
      joinPath(path, "finders"),
    ),
    triplets: parseTripletCandidateArray(
      expectField(obj, "triplets", path),
      joinPath(path, "triplets"),
    ),
    timings: parseStageTimings(
      expectField(obj, "timings", path),
      joinPath(path, "timings"),
    ),
  };
}

function parseNumberArray(v: unknown, path: string): number[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => expectNumber(item, indexPath(path, i)));
}

function parseBooleanArray(v: unknown, path: string): boolean[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => expectBoolean(item, indexPath(path, i)));
}

function parseTileTrace(v: unknown, path: string): TileTrace {
  const obj = expectObject(v, path);
  return {
    tiles_x: expectNumber(
      expectField(obj, "tiles_x", path),
      joinPath(path, "tiles_x"),
    ),
    tiles_y: expectNumber(
      expectField(obj, "tiles_y", path),
      joinPath(path, "tiles_y"),
    ),
    thresholds: parseNumberArray(
      expectField(obj, "thresholds", path),
      joinPath(path, "thresholds"),
    ),
    skip: parseBooleanArray(expectField(obj, "skip", path), joinPath(path, "skip")),
  };
}

function parseTileTraceOrNull(v: unknown, path: string): TileTrace | null {
  if (v === null) return null;
  return parseTileTrace(v, path);
}

function parseTrace(v: unknown, path: string): Trace {
  const obj = expectObject(v, path);
  return {
    tiles: parseTileTraceOrNull(
      expectField(obj, "tiles", path),
      joinPath(path, "tiles"),
    ),
    finders: parseFinderCandidateArray(
      expectField(obj, "finders", path),
      joinPath(path, "finders"),
    ),
    triplets: parseTripletCandidateArray(
      expectField(obj, "triplets", path),
      joinPath(path, "triplets"),
    ),
  };
}

function parseTraceOrNull(v: unknown, path: string): Trace | null {
  if (v === null) return null;
  return parseTrace(v, path);
}

/**
 * Structurally validate an arbitrary JSON value (typically
 * `JSON.parse(...)` of a `scan_rgba` result, or the deserialized wasm
 * `JsValue`) against {@link ScanResult}, returning a type-narrowed copy on
 * success. Throws {@link ScanResultParseError} — with a field path — on
 * the first mismatch, so a shape drift between the Rust envelope and this
 * file surfaces immediately instead of as a `undefined` deep in a
 * component.
 */
export function parseScanResult(v: unknown): ScanResult {
  const obj = expectObject(v, "");
  return {
    detections: parseDetections(
      expectField(obj, "detections", ""),
      "detections",
    ),
    trace: parseTraceOrNull(expectField(obj, "trace", ""), "trace"),
  };
}
