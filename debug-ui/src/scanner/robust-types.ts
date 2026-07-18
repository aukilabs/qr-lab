// TypeScript mirror of the Plan 6 robust envelope `qr-lab-wasm`'s
// `scan_rgba_robust` returns (via `serde-wasm-bindgen`) and, byte-for-byte,
// of `debug-ui/src/scanner/__snapshots__/envelope.robust.shadow_04.json` —
// the Rust-generated snapshot from `crates/qr-lab-wasm/tests/
// envelope_snapshot.rs`. That snapshot is the source of truth: field names
// were read off it, not guessed. If `WasmRobustResult`'s Rust shape changes,
// regenerate the snapshot and update the types/parser here to match —
// `robust-types.test.ts` fails loudly on drift.
//
// Same serde-wasm-bindgen gotcha as `types.ts` (see `parseNumberOrNull`'s
// doc there): the committed snapshot is JSON text, so a Rust `Option::None`
// renders as `null` — but the LIVE binding renders it as `undefined`. Every
// optional-field parser in this file treats both identically, and the test
// suite covers BOTH shapes.

import {
  expectArray,
  expectBoolean,
  expectField,
  expectNumber,
  expectObject,
  fail,
  indexPath,
  joinPath,
  parseDecodedCode,
  parseDetections,
  parsePair,
  parseQuad,
  parseQuadOrNull,
  parseStageTimings,
  type DecodedCode,
  type Detections,
  type StageTimings,
} from "./types";

/** `[TL, TR, BR, BL]` quad, matching `parseQuad`'s return shape. */
type Quad = [[number, number], [number, number], [number, number], [number, number]];

/** The unit (no-payload) `VariantKind` values, exactly as Rust's externally
 * tagged serde renders them: a bare string. */
export const BARE_VARIANT_KINDS = [
  "Baseline",
  // Plan 6 §9 cumulative pipeline: the shared area-averaged working
  // substrate every enhancement pass detects on (stage 1), and the
  // decode-only provenance of a code grouped from finders POOLED across
  // several variants that no single variant found alone (stage 7).
  "BoxWorking",
  "CrossVariant",
  "LowContrastFloor",
  "SauvolaThreshold",
  "ShadowNormalized",
  "Sharpened",
  "Upscaled2x",
  "Upscaled3x",
] as const;

export type BareVariantKind = (typeof BARE_VARIANT_KINDS)[number];

/** Mirrors `qr_lab_core::ladder::VariantKind` — an externally tagged serde
 * enum: unit variants cross as bare strings, payload variants as a
 * single-key object (`{"Pyramid":{"level":1}}`). */
export type VariantKind =
  | BareVariantKind
  | { Pyramid: { level: number } }
  | { ThresholdOffset: { offset: number } }
  | { UpscaledRoi: { factor: number } }
  | { DirectionalSharpened: { theta_deg: number } }
  | { VanCittert: { theta_deg: number; len: number } };

/** One decoded code plus its ladder provenance — mirrors
 * `qr_lab_core::ladder::RobustCode`. Coordinate spaces (load-bearing, from the
 * Rust doc): `corners_source`/`refined_corners_source` are SOURCE px;
 * `code.corners` stays in that code's own VARIANT working px — never draw
 * those directly. */
export interface RobustCode {
  code: DecodedCode;
  /** Coarse module-region corners in SOURCE px. */
  corners_source: Quad;
  /** Refined corners in SOURCE px when refinement ran, else `null`. */
  refined_corners_source: Quad | null;
  variant: VariantKind;
  stage: number;
}

/** One ladder rung's execution record — mirrors
 * `qr_lab_core::ladder::VariantRecord`. `variants[0]` is always the baseline
 * pass. */
export interface VariantRecord {
  kind: VariantKind;
  stage: number;
  timings: StageTimings;
  /** Whole-variant wall time (buffer preparation + detection), ns. */
  total_ns: number;
  finders: number;
  triplets: number;
  codes: number;
  /** Codes this variant contributed that no earlier variant had found. */
  new_codes: number;
}

/** One ladder variant's grayscale buffer thumbnail (capture mode only) —
 * mirrors `qr_lab_wasm::WasmSnapshot`; ≤320px longest side, one entry per
 * `variants` record, same order. */
export interface RobustSnapshot {
  kind: VariantKind;
  stage: number;
  width: number;
  height: number;
  /** Tightly packed `width * height` grayscale bytes. Over the real wasm
   * boundary this arrives as a `Uint8Array` (serde_bytes); in a JSON
   * snapshot it's a plain `number[]` — the parser accepts both and always
   * normalizes to `Uint8Array`. */
  luma: Uint8Array;
}

/** One frame's ladder result — mirrors `qr_lab_core::ladder::RobustDetections`. */
export interface RobustDetections {
  /** Accepted (deduplicated) codes, cheapest rung first. */
  codes: RobustCode[];
  /** Every variant that ran, in execution order (index 0 = baseline). */
  variants: VariantRecord[];
  early_exited: boolean;
  budget_exhausted: boolean;
  total_ns: number;
  /** Deduplicated coherent-triplet centroids in SOURCE px. */
  triplet_evidence: [number, number][];
}

/** Mirrors `qr_lab_wasm::WasmRobustResult` — the one-pipeline contract:
 * `detections` has EXACTLY the classic envelope's shape and conventions
 * (finders/triplets/codes in working px, `source_scale`, baseline stage
 * timings), assembled in Rust from the ladder's cross-variant union, so
 * every classic consumer (overlays, timings) works identically in robust
 * mode — enabling flags simply yields MORE entries. `robust` carries the
 * ladder metadata (per-variant records, per-code provenance in SOURCE px,
 * evidence, exit flags); `snapshots` (the filmstrip) is the only
 * capture-gated payload. */
export interface RobustScanResult {
  detections: Detections;
  robust: RobustDetections;
  snapshots: RobustSnapshot[] | null;
  scan_width: number;
  scan_height: number;
}

/** camelCase mirror of `qr_lab_wasm::RobustConfig` (itself mirroring
 * `qr_lab_core::ScanConfig`) — the plain object `scan_rgba_robust` takes and
 * `robust_presets()` returns. */
export interface RobustConfig {
  enableMultiScale: boolean;
  enableContrastNormalization: boolean;
  enableShadowNormalization: boolean;
  enableAdaptiveThresholding: boolean;
  enableSharpening: boolean;
  enableDeblur: boolean;
  enableLowResUpscaling: boolean;
  /** `0` = unlimited. */
  maxVariantsPerFrame: number;
  enableEarlyExit: boolean;
}

/** Mirrors `qr_lab_wasm::RobustPresets` — the authoritative Rust preset
 * values (`ScanConfig::{BASELINE, ROBUST_FAST, ROBUST_FULL_BENCHMARK}`),
 * fetched at runtime so the UI never hardcodes copies that could drift. */
export interface RobustPresets {
  baseline: RobustConfig;
  robustFast: RobustConfig;
  robustFullBenchmark: RobustConfig;
}

function parseVariantPayloadNumber(
  obj: Record<string, unknown>,
  key: string,
  path: string,
): number {
  return expectNumber(expectField(obj, key, path), joinPath(path, key));
}

export function parseVariantKind(v: unknown, path: string): VariantKind {
  if (typeof v === "string") {
    const bare = BARE_VARIANT_KINDS.find((k) => k === v);
    if (!bare) fail(path, `unknown VariantKind "${v}"`);
    return bare;
  }
  const obj = expectObject(v, path);
  const keys = Object.keys(obj);
  if (keys.length !== 1) {
    fail(path, `expected a single-key tagged VariantKind object, got ${keys.length} keys`);
  }
  const tag = keys[0]!;
  const payloadPath = joinPath(path, tag);
  const payload = expectObject(obj[tag], payloadPath);
  switch (tag) {
    case "Pyramid":
      return { Pyramid: { level: parseVariantPayloadNumber(payload, "level", payloadPath) } };
    case "ThresholdOffset":
      return {
        ThresholdOffset: { offset: parseVariantPayloadNumber(payload, "offset", payloadPath) },
      };
    case "UpscaledRoi":
      return { UpscaledRoi: { factor: parseVariantPayloadNumber(payload, "factor", payloadPath) } };
    case "DirectionalSharpened":
      return {
        DirectionalSharpened: {
          theta_deg: parseVariantPayloadNumber(payload, "theta_deg", payloadPath),
        },
      };
    case "VanCittert":
      return {
        VanCittert: {
          theta_deg: parseVariantPayloadNumber(payload, "theta_deg", payloadPath),
          len: parseVariantPayloadNumber(payload, "len", payloadPath),
        },
      };
    default:
      fail(path, `unknown VariantKind tag "${tag}"`);
  }
}

/** Short human label for a ladder rung, used by the ladder panel, the
 * filmstrip captions, and the robust-codes overlay chip. */
export function variantKindLabel(kind: VariantKind): string {
  if (typeof kind === "string") {
    switch (kind) {
      case "Baseline":
        return "baseline";
      case "BoxWorking":
        return "box substrate";
      case "CrossVariant":
        return "cross-variant pool";
      case "LowContrastFloor":
        return "low contrast floor";
      case "SauvolaThreshold":
        return "Sauvola threshold";
      case "ShadowNormalized":
        return "background divide";
      case "Sharpened":
        return "unsharp";
      case "Upscaled2x":
        return "2× upscale";
      case "Upscaled3x":
        return "3× upscale";
    }
  }
  if ("Pyramid" in kind) {
    // level halvings from the working view: 1 = 0.5×, 2 = 0.25×.
    return `pyramid ${1 / 2 ** kind.Pyramid.level}×`;
  }
  if ("ThresholdOffset" in kind) {
    const offset = kind.ThresholdOffset.offset;
    return `threshold ${offset < 0 ? "−" : "+"}${Math.abs(offset)}`;
  }
  if ("UpscaledRoi" in kind) {
    // Factor 1 is a 1:1 pristine-source rescan of the evidence ROI (no
    // upscale) — the pitch-derived recovery for codes already at/above the
    // decode floor; factors 2/3 upscale genuinely sub-resolution ROIs.
    return kind.UpscaledRoi.factor === 1 ? "ROI rescan" : `ROI ${kind.UpscaledRoi.factor}×`;
  }
  if ("DirectionalSharpened" in kind) {
    return `directional ${kind.DirectionalSharpened.theta_deg}°`;
  }
  return `Van Cittert ${kind.VanCittert.theta_deg}°·${kind.VanCittert.len}px`;
}

/** One color per ladder stage (0 = baseline … 6 = deblur, 7 = cross-variant
 * pool — the Plan 6 §9 stage numbering, see `VariantKind::stage()` in
 * `qr-lab-core`). Stage 0 reuses the decoded layer's green family
 * (`decoded.ts`'s `DECODED_COLOR`) so a baseline-found code reads as "the
 * normal case"; recovery stages get visually distinct hues. */
const STAGE_COLORS = [
  "#00e676", // 0 baseline — same green family as decodedLayer
  "#40c4ff", // 1 multi-scale / box substrate — sky blue
  "#ffd740", // 2 contrast — amber
  "#ff6e40", // 3 shadow/adaptive threshold — deep orange
  "#e040fb", // 4 sharpening — magenta
  "#ff4081", // 5 upscaling / ROI recovery — pink
  "#8c9eff", // 6 deblur — indigo
  "#1de9b6", // 7 cross-variant pool — teal (a code no single variant found)
] as const;

export function stageColor(stage: number): string {
  const i = Math.min(Math.max(Math.trunc(stage), 0), STAGE_COLORS.length - 1);
  return STAGE_COLORS[i]!;
}

function parseRobustCode(v: unknown, path: string): RobustCode {
  const obj = expectObject(v, path);
  return {
    code: parseDecodedCode(expectField(obj, "code", path), joinPath(path, "code")),
    corners_source: parseQuad(
      expectField(obj, "corners_source", path),
      joinPath(path, "corners_source"),
    ),
    // null in the JSON snapshot, undefined over the live wasm boundary —
    // both mean "refinement didn't run/converge" (see the module doc).
    refined_corners_source: parseQuadOrNull(
      expectField(obj, "refined_corners_source", path),
      joinPath(path, "refined_corners_source"),
    ),
    variant: parseVariantKind(expectField(obj, "variant", path), joinPath(path, "variant")),
    stage: expectNumber(expectField(obj, "stage", path), joinPath(path, "stage")),
  };
}

function parseVariantRecord(v: unknown, path: string): VariantRecord {
  const obj = expectObject(v, path);
  return {
    kind: parseVariantKind(expectField(obj, "kind", path), joinPath(path, "kind")),
    stage: expectNumber(expectField(obj, "stage", path), joinPath(path, "stage")),
    timings: parseStageTimings(expectField(obj, "timings", path), joinPath(path, "timings")),
    total_ns: expectNumber(expectField(obj, "total_ns", path), joinPath(path, "total_ns")),
    finders: expectNumber(expectField(obj, "finders", path), joinPath(path, "finders")),
    triplets: expectNumber(expectField(obj, "triplets", path), joinPath(path, "triplets")),
    codes: expectNumber(expectField(obj, "codes", path), joinPath(path, "codes")),
    new_codes: expectNumber(expectField(obj, "new_codes", path), joinPath(path, "new_codes")),
  };
}

/** `luma` is a `Uint8Array` over the real wasm boundary (serde_bytes) but a
 * plain `number[]` in the JSON snapshot — accept both, normalize to
 * `Uint8Array` (see the module doc's null/undefined note for the sibling
 * gotcha this parallels). */
function parseLuma(v: unknown, path: string, expectedLen: number): Uint8Array {
  let out: Uint8Array;
  if (v instanceof Uint8Array) {
    out = v;
  } else {
    const arr = expectArray(v, path);
    out = new Uint8Array(arr.length);
    for (let i = 0; i < arr.length; i++) {
      out[i] = expectNumber(arr[i], indexPath(path, i));
    }
  }
  if (out.length !== expectedLen) {
    fail(path, `expected width * height = ${expectedLen} luma bytes, got ${out.length}`);
  }
  return out;
}

function parseRobustSnapshot(v: unknown, path: string): RobustSnapshot {
  const obj = expectObject(v, path);
  const width = expectNumber(expectField(obj, "width", path), joinPath(path, "width"));
  const height = expectNumber(expectField(obj, "height", path), joinPath(path, "height"));
  return {
    kind: parseVariantKind(expectField(obj, "kind", path), joinPath(path, "kind")),
    stage: expectNumber(expectField(obj, "stage", path), joinPath(path, "stage")),
    width,
    height,
    luma: parseLuma(expectField(obj, "luma", path), joinPath(path, "luma"), width * height),
  };
}

function parsePairArray(v: unknown, path: string): [number, number][] {
  const arr = expectArray(v, path);
  return arr.map((item, i) => parsePair(item, indexPath(path, i)));
}

function parseRobustDetections(v: unknown, path: string): RobustDetections {
  const obj = expectObject(v, path);
  const codes = expectArray(expectField(obj, "codes", path), joinPath(path, "codes"));
  const variants = expectArray(expectField(obj, "variants", path), joinPath(path, "variants"));
  return {
    codes: codes.map((item, i) => parseRobustCode(item, indexPath(joinPath(path, "codes"), i))),
    variants: variants.map((item, i) =>
      parseVariantRecord(item, indexPath(joinPath(path, "variants"), i)),
    ),
    early_exited: expectBoolean(
      expectField(obj, "early_exited", path),
      joinPath(path, "early_exited"),
    ),
    budget_exhausted: expectBoolean(
      expectField(obj, "budget_exhausted", path),
      joinPath(path, "budget_exhausted"),
    ),
    total_ns: expectNumber(expectField(obj, "total_ns", path), joinPath(path, "total_ns")),
    triplet_evidence: parsePairArray(
      expectField(obj, "triplet_evidence", path),
      joinPath(path, "triplet_evidence"),
    ),
  };
}

/**
 * Structurally validate an arbitrary value (the deserialized
 * `scan_rgba_robust` JsValue, or JSON text of it) against
 * {@link RobustScanResult}, returning a type-narrowed copy. Throws
 * `ScanResultParseError` with a field path on the first mismatch — same
 * throw-on-drift contract as `parseScanResult`.
 */
export function parseRobustScanResult(v: unknown): RobustScanResult {
  const obj = expectObject(v, "");
  const snapshotsRaw = expectField(obj, "snapshots", "");
  return {
    // The unified pipeline result — same parser as the classic envelope's
    // `detections`, because it IS the same contract.
    detections: parseDetections(expectField(obj, "detections", ""), "detections"),
    robust: parseRobustDetections(expectField(obj, "robust", ""), "robust"),
    // null (JSON snapshot) and undefined (live serde-wasm-bindgen binding)
    // are the SAME "capture off" state — see the module doc.
    snapshots:
      snapshotsRaw === null || snapshotsRaw === undefined
        ? null
        : expectArray(snapshotsRaw, "snapshots").map((item, i) =>
            parseRobustSnapshot(item, indexPath("snapshots", i)),
          ),
    scan_width: expectNumber(expectField(obj, "scan_width", ""), "scan_width"),
    scan_height: expectNumber(expectField(obj, "scan_height", ""), "scan_height"),
  };
}

function parseRobustConfig(v: unknown, path: string): RobustConfig {
  const obj = expectObject(v, path);
  const flag = (key: string) => expectBoolean(expectField(obj, key, path), joinPath(path, key));
  return {
    enableMultiScale: flag("enableMultiScale"),
    enableContrastNormalization: flag("enableContrastNormalization"),
    enableShadowNormalization: flag("enableShadowNormalization"),
    enableAdaptiveThresholding: flag("enableAdaptiveThresholding"),
    enableSharpening: flag("enableSharpening"),
    enableDeblur: flag("enableDeblur"),
    enableLowResUpscaling: flag("enableLowResUpscaling"),
    maxVariantsPerFrame: expectNumber(
      expectField(obj, "maxVariantsPerFrame", path),
      joinPath(path, "maxVariantsPerFrame"),
    ),
    enableEarlyExit: flag("enableEarlyExit"),
  };
}

/** Validate `robust_presets()`'s return value — same strictness as the
 * envelope parsers, so a preset-shape drift surfaces as a loud error
 * instead of a checkbox silently reading `undefined` as unchecked. */
export function parseRobustPresets(v: unknown): RobustPresets {
  const obj = expectObject(v, "");
  return {
    baseline: parseRobustConfig(expectField(obj, "baseline", ""), "baseline"),
    robustFast: parseRobustConfig(expectField(obj, "robustFast", ""), "robustFast"),
    robustFullBenchmark: parseRobustConfig(
      expectField(obj, "robustFullBenchmark", ""),
      "robustFullBenchmark",
    ),
  };
}
