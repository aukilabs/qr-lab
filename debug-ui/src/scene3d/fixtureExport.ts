// "Save as fixture" (Plan 5d): build the golden-fixture-schema JSON meta
// object for the scene's current frame, plus the impure glue to encode the
// captured rgba into a `.png`/`.luma` pair and trigger three browser
// downloads. Schema reference: `tools/fixtures/generate.py`'s `meta` dict
// (`render_fixture`) and its Rust consumer, `crates/qrk-core/tests/
// common/mod.rs`'s `Meta`/`CodeTruth` (a subset — extra fields are
// ignored, so this file's harmless extra `exposure_offset` field is safe).
//
// The pure JSON-builder half ({@link buildFixtureMeta} and its small
// helpers) is unit-tested (round-tripped through `parseGroundTruth`); the
// impure half (`pngBlobFromRgba`/`downloadBlob`/`saveSceneFixture`) needs a
// real `HTMLCanvasElement`/DOM and is manual/headless-browser-QA'd only —
// same split this codebase already uses throughout `scene3d/` (see
// `camSim.ts`'s module doc on the same pure/impure canvas split).
import type { Intrinsics } from "./intrinsics";
import { FIXTURE_EXPORT_SEED } from "./consts";
import { lumaBufferFromRgba } from "../media/luma";

export type Point2 = [number, number];
export type EccLetter = "l" | "m" | "q" | "h";

/** `generate_qr`'s `0..=3` ecc index -> the lowercase letter the fixture
 * schema (and `segno`/`tools/fixtures/render.py`) expects — see
 * `crates/qrk-core/tests/common/mod.rs`'s `CodeTruth.ecc: String`. */
export function eccLetterFromIndex(ecc: number): EccLetter {
  const letters: readonly EccLetter[] = ["l", "m", "q", "h"];
  const letter = letters[ecc];
  if (!letter) throw new RangeError(`eccLetterFromIndex: ecc must be 0..=3, got ${ecc}`);
  return letter;
}

/** Inverse of a QR bit matrix's own `dim = 4*version + 17` relationship
 * (ISO/IEC 18004) — recovers the actual generated version from a
 * `GeneratedQr.dim` (`scanner/qrgen.ts`), since the scene's own `version`
 * state can be `0` ("auto"), which isn't a valid fixture-schema value. */
export function versionFromDim(dim: number): number {
  if (!Number.isFinite(dim) || dim < 21 || (dim - 17) % 4 !== 0) {
    throw new RangeError(`versionFromDim: not a valid QR module-count dim, got ${dim}`);
  }
  return (dim - 17) / 4;
}

/** Lowercase, filesystem/URL-safe slug: non-alphanumeric runs collapse to
 * a single underscore, leading/trailing underscores trimmed. Falls back
 * to `"payload"` for an input with no alphanumeric characters at all
 * (never returns an empty string, which would produce a bare `scene_`
 * default fixture name). */
export function slugifyPayload(payload: string): string {
  const slug = payload
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return slug || "payload";
}

/** The fixture-name text field's default value: `scene_<payload-slug>`. */
export function defaultFixtureName(payload: string): string {
  return `scene_${slugifyPayload(payload)}`;
}

/** `opaque_plate` is emergent from the QR-background alpha slider: fully
 * opaque (`alpha >= 1`) means the quiet zone + light modules are an
 * opaque painted plate (matching `render.py`'s `opaque_plate=True`);
 * anything less than fully opaque means the scene background shows
 * through them (`opaque_plate=False`, the `trans_` fixture scenario). */
export function opaquePlateFromAlpha(alpha: number): boolean {
  return alpha >= 1;
}

export interface FixtureCodeInput {
  payload: string;
  /** Actual generated version (never `0`/"auto" — see {@link versionFromDim}). */
  version: number;
  eccLetter: EccLetter;
  /** MODULE-REGION-ONLY physical size, meters — see `moduleRegion.ts`'s
   * `moduleRegionPhysicalSize` doc for the full derivation/justification
   * of why this differs from the scene's own (full-plane) `physicalSize`. */
  physicalSizeM: number;
  distanceM: number;
  /** = the camera-stats incidence angle (degrees). */
  tiltDeg: number;
  moduleSizePx: number;
  /** [TL, TR, BR, BL], readback px — `projectAllToPixels`' output. */
  cornersPx: [Point2, Point2, Point2, Point2];
  inverted: boolean;
  opaquePlate: boolean;
}

export interface FixtureMetaInput {
  name: string;
  /** Readback resolution (square). */
  width: number;
  height: number;
  camera: Intrinsics;
  blurSigma: number;
  noiseSigma: number;
  /** No schema field for this (a fixture's plate rendering has no
   * exposure knob) — recorded as an EXTRA top-level field, harmless to
   * every schema consumer (`tools/fixtures/generate.py` never reads it
   * back; the Rust loader's `Meta` struct simply doesn't declare it, and
   * `serde_json` ignores unknown fields by default). */
  exposureOffset: number;
  code: FixtureCodeInput;
}

export interface FixtureMeta {
  name: string;
  width: number;
  height: number;
  camera: Intrinsics;
  blur_sigma: number;
  noise_sigma: number;
  exposure_offset: number;
  seed: number;
  codes: Record<string, unknown>[];
}

/**
 * Build the full generator-schema JSON meta object for one captured
 * scene frame (always exactly one code entry — the scene renders a single
 * QR plane). See this module's doc for the schema reference and the
 * `exposure_offset` extra-field note.
 */
export function buildFixtureMeta(input: FixtureMetaInput): FixtureMeta {
  const { code } = input;
  return {
    name: input.name,
    width: input.width,
    height: input.height,
    camera: { fx: input.camera.fx, fy: input.camera.fy, cx: input.camera.cx, cy: input.camera.cy },
    blur_sigma: input.blurSigma,
    noise_sigma: input.noiseSigma,
    exposure_offset: input.exposureOffset,
    seed: FIXTURE_EXPORT_SEED,
    codes: [
      {
        payload: code.payload,
        version: code.version,
        ecc: code.eccLetter,
        mirrored: false,
        physical_size_m: code.physicalSizeM,
        distance_m: code.distanceM,
        tilt_deg: code.tiltDeg,
        // Not recoverable losslessly from a single incidence angle + a
        // single in-plane-roll reading (see `cameraStats.ts`'s doc on
        // `inPlaneRollDeg` being an approximation, and the fact that
        // `tilt_azimuth_deg` — WHICH in-plane axis the tilt happened
        // about — has no analog in this scene's live camera-stats at
        // all) — recorded as 0 with this comment as the documented
        // caveat (see also the README's fixture-export section).
        tilt_azimuth_deg: 0,
        inplane_deg: 0,
        module_size_px: code.moduleSizePx,
        corners_px: code.cornersPx,
        inverted: code.inverted,
        opaque_plate: code.opaquePlate,
      },
    ],
  };
}

/** Minimal 2D-canvas-producing surface {@link pngBlobFromRgba} needs —
 * injectable for testability, matching `camSim.ts`'s `makeCanvas`
 * pattern (untested here regardless, since `HTMLCanvasElement.toBlob`
 * needs a real browser). */
export type MakeCanvas = (w: number, h: number) => HTMLCanvasElement;

/**
 * Encode a captured rgba frame as a PNG `Blob` via a temporary canvas.
 * Untested (needs a real `HTMLCanvasElement.toBlob`, unavailable in
 * vitest's `node` environment) — manual/headless-browser QA only.
 */
export function pngBlobFromRgba(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  makeCanvas: MakeCanvas = (w, h) => {
    const c = document.createElement("canvas");
    c.width = w;
    c.height = h;
    return c;
  },
): Promise<Blob> {
  const canvas = makeCanvas(width, height);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("pngBlobFromRgba: 2d context unavailable");
  ctx.putImageData(new ImageData(rgba.slice(), width, height), 0, 0);
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error("pngBlobFromRgba: canvas.toBlob returned null"));
    }, "image/png");
  });
}

/**
 * Trigger a browser download of `blob` named `filename` via a throwaway
 * `<a download>` click — the standard no-server-round-trip download
 * pattern. `URL.revokeObjectURL` is deferred (not immediate) so the
 * browser's own download-start handling has time to read the blob URL
 * first; a handful of the three downloads happening back-to-back is the
 * exact scenario this delay exists for.
 */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export interface SaveSceneFixtureInput {
  name: string;
  /** The clean post-camSim readback rgba — exactly what the scanner last
   * saw (no HUD/overlays baked in). */
  rgba: Uint8ClampedArray;
  meta: FixtureMeta;
}

/**
 * Produce the three fixture-triplet downloads (`.json`, `.png`, `.luma`)
 * for one captured scene frame. Pure data flow (meta -> JSON text,
 * rgba -> luma bytes) delegates to already-tested pure functions; only
 * the PNG encode and the download trigger touch the DOM.
 */
export async function saveSceneFixture(input: SaveSceneFixtureInput, width: number, height: number): Promise<void> {
  const jsonBlob = new Blob([JSON.stringify(input.meta, null, 1) + "\n"], { type: "application/json" });
  const lumaBytes = lumaBufferFromRgba(input.rgba, width, height);
  const lumaBlob = new Blob([lumaBytes], { type: "application/octet-stream" });
  const pngBlob = await pngBlobFromRgba(input.rgba, width, height);

  downloadBlob(jsonBlob, `${input.name}.json`);
  downloadBlob(pngBlob, `${input.name}.png`);
  downloadBlob(lumaBlob, `${input.name}.luma`);
}
