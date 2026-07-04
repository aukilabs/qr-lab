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
  /** The three `FinderCandidate` indices this triplet groups (Plan 4 Task
   * 5's `triplet.rs` addition — not new to Task 6, but never mirrored here
   * until now: `decode.rs`'s `DecodeAttemptTrace.triplet_index` and the
   * `decoded` overlay layer both need to resolve a triplet back to its
   * finders). */
  finder_indices: [number, number, number];
}

/** A fully decoded QR payload (Plan 4 Task 6) — mirrors Rust's
 * `qrk_core::decode::DecodedCode`. `corners` is `[TL, TR, BR, BL]` in
 * working-res image px, the same space `scan`'s other geometry lives in. */
export interface DecodedCode {
  payload: string;
  payload_bytes: number[];
  version: number;
  ecc: string;
  mirrored: boolean;
  dimension: number;
  corners: [[number, number], [number, number], [number, number], [number, number]];
  inverted: boolean;
  finder_indices: [number, number, number];
}

/** One decode attempt's trace (mirrors `qrk_core::decode::DecodeAttemptTrace`).
 * `outcome === "decoded"` (or starts with `"decoded"` — see the dimension-
 * mismatch discrepancy note on the Rust struct) marks a successful decode;
 * anything else is a short failure reason. `rounds` is per-round visibility
 * into the sample+decode sub-pipeline (`"parallelogram:failed_rs"`,
 * `"anchor_line:decoded"`, ...) — see the Rust doc for the full tag
 * vocabulary. */
export interface DecodeAttemptTrace {
  triplet_index: number;
  dimension_est: number;
  dimension_final: number;
  timing_check: number | null;
  version_bits: number | null;
  alignment_found: number;
  alignment_total: number;
  oob_fraction: number;
  refined_corner: boolean;
  outcome: string;
  rounds: string[];
}

/** One alignment-pattern lattice slot's predicted-vs-found position (mirrors
 * `qrk_core::trace::AlignmentTraceEntry`); finder-corner slots are excluded
 * (never searched — see the Rust doc). */
export interface AlignmentTraceEntry {
  predicted: [number, number];
  found: [number, number] | null;
}

/** One sample region's module rectangle and the image-pixel quad its four
 * corners map to (mirrors `qrk_core::trace::SampleRegionTrace`).
 * `module_rect` is `[x0, y0, x1, y1)` in raw module units; `quad` is
 * `[TL, TR, BR, BL]` in working-res image px. */
export interface SampleRegionTrace {
  module_rect: [number, number, number, number];
  quad: [[number, number], [number, number], [number, number], [number, number]];
}

/** The last successfully decoded candidate's sampled bit matrix (mirrors
 * `qrk_core::trace::BitsTrace`) — packed row-major `u32` words, `dim` wide.
 * `words.length === ceil(dim / 32) * dim`. */
export interface BitsTrace {
  dim: number;
  words: number[];
}

export interface StageTimings {
  tiles_ns: number;
  finders_ns: number;
  triplets_ns: number;
  version_ns: number;
  alignment_ns: number;
  sample_decode_ns: number;
}

export interface Detections {
  finders: FinderCandidate[];
  triplets: TripletCandidate[];
  codes: DecodedCode[];
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
  attempts: DecodeAttemptTrace[];
  alignment: AlignmentTraceEntry[];
  sample_regions: SampleRegionTrace[];
  bits: BitsTrace | null;
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

/** `[TL, TR, BR, BL]`-style 4-corner quad, each corner a 2-element pair. */
function parseQuad(
  v: unknown,
  path: string,
): [[number, number], [number, number], [number, number], [number, number]] {
  const arr = expectArray(v, path);
  if (arr.length !== 4) {
    fail(path, `expected a 4-element tuple, got ${arr.length} elements`);
  }
  return [
    parsePair(arr[0], indexPath(path, 0)),
    parsePair(arr[1], indexPath(path, 1)),
    parsePair(arr[2], indexPath(path, 2)),
    parsePair(arr[3], indexPath(path, 3)),
  ];
}

/** A flat 4-number tuple (e.g. `module_rect: [x0, y0, x1, y1]`), as opposed
 * to {@link parseQuad}'s 4 nested pairs. */
function parseQuadruple(v: unknown, path: string): [number, number, number, number] {
  const arr = expectArray(v, path);
  if (arr.length !== 4) {
    fail(path, `expected a 4-element tuple, got ${arr.length} elements`);
  }
  return [
    expectNumber(arr[0], indexPath(path, 0)),
    expectNumber(arr[1], indexPath(path, 1)),
    expectNumber(arr[2], indexPath(path, 2)),
    expectNumber(arr[3], indexPath(path, 3)),
  ];
}

/** A flat 3-number tuple (`finder_indices: [a, b, c]`). */
function parseTriple(v: unknown, path: string): [number, number, number] {
  const arr = expectArray(v, path);
  if (arr.length !== 3) {
    fail(path, `expected a 3-element tuple, got ${arr.length} elements`);
  }
  return [
    expectNumber(arr[0], indexPath(path, 0)),
    expectNumber(arr[1], indexPath(path, 1)),
    expectNumber(arr[2], indexPath(path, 2)),
  ];
}

function expectString(v: unknown, path: string): string {
  if (typeof v !== "string") {
    fail(path, `expected a string, got ${typeOf(v)}`);
  }
  return v;
}

function parseStringArray(v: unknown, path: string): string[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => expectString(item, indexPath(path, i)));
}

function parseNumberOrNull(v: unknown, path: string): number | null {
  if (v === null) return null;
  return expectNumber(v, path);
}

function parsePairOrNull(v: unknown, path: string): [number, number] | null {
  if (v === null) return null;
  return parsePair(v, path);
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
    finder_indices: parseTriple(
      expectField(obj, "finder_indices", path),
      joinPath(path, "finder_indices"),
    ),
  };
}

function parseTripletCandidateArray(v: unknown, path: string): TripletCandidate[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parseTripletCandidate(item, indexPath(path, i)));
}

function parseDecodedCode(v: unknown, path: string): DecodedCode {
  const obj = expectObject(v, path);
  return {
    payload: expectString(expectField(obj, "payload", path), joinPath(path, "payload")),
    payload_bytes: parseNumberArray(
      expectField(obj, "payload_bytes", path),
      joinPath(path, "payload_bytes"),
    ),
    version: expectNumber(expectField(obj, "version", path), joinPath(path, "version")),
    ecc: expectString(expectField(obj, "ecc", path), joinPath(path, "ecc")),
    mirrored: expectBoolean(expectField(obj, "mirrored", path), joinPath(path, "mirrored")),
    dimension: expectNumber(expectField(obj, "dimension", path), joinPath(path, "dimension")),
    corners: parseQuad(expectField(obj, "corners", path), joinPath(path, "corners")),
    inverted: expectBoolean(expectField(obj, "inverted", path), joinPath(path, "inverted")),
    finder_indices: parseTriple(
      expectField(obj, "finder_indices", path),
      joinPath(path, "finder_indices"),
    ),
  };
}

function parseDecodedCodeArray(v: unknown, path: string): DecodedCode[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parseDecodedCode(item, indexPath(path, i)));
}

function parseDecodeAttemptTrace(v: unknown, path: string): DecodeAttemptTrace {
  const obj = expectObject(v, path);
  return {
    triplet_index: expectNumber(
      expectField(obj, "triplet_index", path),
      joinPath(path, "triplet_index"),
    ),
    dimension_est: expectNumber(
      expectField(obj, "dimension_est", path),
      joinPath(path, "dimension_est"),
    ),
    dimension_final: expectNumber(
      expectField(obj, "dimension_final", path),
      joinPath(path, "dimension_final"),
    ),
    timing_check: parseNumberOrNull(
      expectField(obj, "timing_check", path),
      joinPath(path, "timing_check"),
    ),
    version_bits: parseNumberOrNull(
      expectField(obj, "version_bits", path),
      joinPath(path, "version_bits"),
    ),
    alignment_found: expectNumber(
      expectField(obj, "alignment_found", path),
      joinPath(path, "alignment_found"),
    ),
    alignment_total: expectNumber(
      expectField(obj, "alignment_total", path),
      joinPath(path, "alignment_total"),
    ),
    oob_fraction: expectNumber(
      expectField(obj, "oob_fraction", path),
      joinPath(path, "oob_fraction"),
    ),
    refined_corner: expectBoolean(
      expectField(obj, "refined_corner", path),
      joinPath(path, "refined_corner"),
    ),
    outcome: expectString(expectField(obj, "outcome", path), joinPath(path, "outcome")),
    rounds: parseStringArray(expectField(obj, "rounds", path), joinPath(path, "rounds")),
  };
}

function parseDecodeAttemptTraceArray(v: unknown, path: string): DecodeAttemptTrace[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parseDecodeAttemptTrace(item, indexPath(path, i)));
}

function parseAlignmentTraceEntry(v: unknown, path: string): AlignmentTraceEntry {
  const obj = expectObject(v, path);
  return {
    predicted: parsePair(expectField(obj, "predicted", path), joinPath(path, "predicted")),
    found: parsePairOrNull(expectField(obj, "found", path), joinPath(path, "found")),
  };
}

function parseAlignmentTraceEntryArray(v: unknown, path: string): AlignmentTraceEntry[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parseAlignmentTraceEntry(item, indexPath(path, i)));
}

function parseSampleRegionTrace(v: unknown, path: string): SampleRegionTrace {
  const obj = expectObject(v, path);
  return {
    module_rect: parseQuadruple(
      expectField(obj, "module_rect", path),
      joinPath(path, "module_rect"),
    ),
    quad: parseQuad(expectField(obj, "quad", path), joinPath(path, "quad")),
  };
}

function parseSampleRegionTraceArray(v: unknown, path: string): SampleRegionTrace[] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parseSampleRegionTrace(item, indexPath(path, i)));
}

function parseBitsTrace(v: unknown, path: string): BitsTrace {
  const obj = expectObject(v, path);
  return {
    dim: expectNumber(expectField(obj, "dim", path), joinPath(path, "dim")),
    words: parseNumberArray(expectField(obj, "words", path), joinPath(path, "words")),
  };
}

function parseBitsTraceOrNull(v: unknown, path: string): BitsTrace | null {
  if (v === null) return null;
  return parseBitsTrace(v, path);
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
    version_ns: expectNumber(
      expectField(obj, "version_ns", path),
      joinPath(path, "version_ns"),
    ),
    alignment_ns: expectNumber(
      expectField(obj, "alignment_ns", path),
      joinPath(path, "alignment_ns"),
    ),
    sample_decode_ns: expectNumber(
      expectField(obj, "sample_decode_ns", path),
      joinPath(path, "sample_decode_ns"),
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
    codes: parseDecodedCodeArray(
      expectField(obj, "codes", path),
      joinPath(path, "codes"),
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
    attempts: parseDecodeAttemptTraceArray(
      expectField(obj, "attempts", path),
      joinPath(path, "attempts"),
    ),
    alignment: parseAlignmentTraceEntryArray(
      expectField(obj, "alignment", path),
      joinPath(path, "alignment"),
    ),
    sample_regions: parseSampleRegionTraceArray(
      expectField(obj, "sample_regions", path),
      joinPath(path, "sample_regions"),
    ),
    bits: parseBitsTraceOrNull(expectField(obj, "bits", path), joinPath(path, "bits")),
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
