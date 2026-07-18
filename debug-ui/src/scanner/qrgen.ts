// Main-thread wrapper around the wasm `generate_qr` binding (Plan 5 Task
// 5, feature `qr-gen` — see `crates/qr-lab-wasm/src/qrgen.rs`'s doc). Unlike
// `scan_rgba` (which only ever runs on the scanner Worker, see
// `scanner/worker.ts`), the 3D-scene mode needs QR generation on the MAIN
// thread — it textures a three.js plane with the result, and three.js
// scenes live on the main thread. This imports the SAME wasm package
// `worker.ts` does, but instantiates its own independent wasm instance
// (wasm modules aren't shared across threads without the shared-memory
// threads proposal, which this project doesn't use) — a second `init()`
// call here is expected, not a bug.
import init, { generate_qr } from "../wasm/qr_lab_wasm.js";

/** Same packed shape as `qr_lab_core::trace::BitsTrace` (see `overlays/
 * layers/bits.ts`'s `bitAt` for the unpacking convention this mirrors):
 * row-major `u32` words, `ceil(dim/32)` words per row, `dim` rows. */
export interface GeneratedQr {
  dim: number;
  words: number[];
}

/** Thrown by {@link generateQr} when the wasm call errors (payload
 * doesn't fit the requested version/ecc, invalid ecc, ...) or its result
 * doesn't structurally match {@link GeneratedQr}. */
export class GenerateQrError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "GenerateQrError";
  }
}

function parseGeneratedQr(v: unknown): GeneratedQr {
  if (typeof v !== "object" || v === null || Array.isArray(v)) {
    throw new GenerateQrError(`generate_qr: expected an object result, got ${typeOf(v)}`);
  }
  const obj = v as Record<string, unknown>;
  const dim = obj.dim;
  if (typeof dim !== "number" || !Number.isFinite(dim)) {
    throw new GenerateQrError(`generate_qr: expected numeric "dim", got ${typeOf(dim)}`);
  }
  const words = obj.words;
  if (!Array.isArray(words) || !words.every((w) => typeof w === "number")) {
    throw new GenerateQrError('generate_qr: expected a numeric array "words"');
  }
  return { dim, words: words as number[] };
}

function typeOf(v: unknown): string {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  return typeof v;
}

let readyPromise: Promise<void> | null = null;

function ready(): Promise<void> {
  if (!readyPromise) {
    readyPromise = init().then(() => undefined);
  }
  return readyPromise;
}

/**
 * Generate a QR bit matrix for `payload` on the main thread. `version`:
 * `0` = auto-select the smallest version that fits; `1..=40` requests
 * that exact version. `ecc`: `0..=3` for L/M/Q/H. See `generate_qr`'s
 * Rust doc (`crates/qr-lab-wasm/src/qrgen.rs`) for the full contract.
 *
 * Rejects with {@link GenerateQrError} if the wasm call errors (e.g.
 * payload too large for the requested version) or its result doesn't
 * structurally match {@link GeneratedQr}.
 */
export async function generateQr(payload: string, version: number, ecc: number): Promise<GeneratedQr> {
  await ready();
  let result: unknown;
  try {
    result = generate_qr(payload, version, ecc);
  } catch (err) {
    throw new GenerateQrError(err instanceof Error ? err.message : String(err));
  }
  return parseGeneratedQr(result);
}
