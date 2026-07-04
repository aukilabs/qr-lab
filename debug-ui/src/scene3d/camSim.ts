// Camera-simulation knobs (Plan 5 Task 5): post-process the readback rgba
// buffer to approximate a real camera's imperfections before handing it
// to `ScannerClient.scan`, so the 3D-scene mode can demonstrate how
// refinement accuracy degrades under blur/noise/exposure rather than
// only ever scanning a pristine synthetic render.
//
// All three are documented approximations, not physically accurate camera
// models:
//  - `applyExposureOffset` is a flat per-channel additive shift, not a
//    true photographic exposure/gamma curve.
//  - `applyGaussianNoise` is i.i.d. per-channel Gaussian noise, not sensor
//    read/shot noise's actual (signal-dependent, spatially-correlated)
//    statistics.
//  - `applyGaussianBlurCanvas` delegates to `CanvasRenderingContext2D`'s
//    `filter: blur(...)` — a real (if unspecified-exact-kernel) Gaussian
//    blur the browser implements, but "blur in screen-space after
//    rendering" is itself an approximation of optical defocus (no depth-
//    of-field, no lens PSF shape); untested here (canvas-dependent, no
//    DOM in vitest's `node` environment) — manual QA only (Task 7).
//
// Noise and exposure are deterministic/seeded so they're unit-testable:
// same seed -> byte-identical output, letting a test assert reproducibility
// and approximate statistics without flaking.

/** Small, fast, deterministic 32-bit PRNG (mulberry32) — good enough for
 * a visual noise approximation, not cryptographic use. Returns floats in
 * `[0, 1)`, same contract as `Math.random()`. */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Standard-normal sample via the Box-Muller transform, driven by a
 * `[0,1)`-uniform generator (e.g. {@link mulberry32}'s return value). */
function nextGaussian(rand: () => number): number {
  // Box-Muller needs u1 in (0, 1] (never exactly 0 — log(0) is -Infinity)
  // — mulberry32 can return 0 in principle, so nudge it into range.
  const u1 = Math.max(rand(), Number.EPSILON);
  const u2 = rand();
  return Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
}

function clamp255(v: number): number {
  return Math.min(255, Math.max(0, v));
}

/** Resolve the shared optional-out-buffer contract of
 * `applyExposureOffset`/`applyGaussianNoise`: `out` omitted -> fresh
 * allocation (the original pure behavior, unchanged); `out` provided ->
 * written into and returned — and it MAY alias `rgba` itself
 * (`out === rgba`, true in-place), which is safe here because both
 * transforms are strictly per-pixel (each output byte depends only on the
 * same input byte, so no read-after-overwrite hazard exists). Added for
 * the 3D scene's per-tick readback path (Plan 5 Task 5 review), which
 * reuses one persistent scratch buffer across scan ticks instead of
 * allocating several fresh buffers per ~100ms tick. */
function resolveOut(
  rgba: Uint8ClampedArray,
  out: Uint8ClampedArray | undefined,
): Uint8ClampedArray {
  if (out === undefined) return new Uint8ClampedArray(rgba.length);
  if (out.length !== rgba.length) {
    throw new RangeError(`out buffer is ${out.length} bytes, expected ${rgba.length}`);
  }
  return out;
}

/**
 * Add a flat offset to every R/G/B channel (alpha untouched), clamped to
 * `[0, 255]`. `offset` in `[-60, 60]` per the task brief's slider range,
 * though this function itself doesn't enforce that bound. Writes into
 * `out` when provided (which may be `rgba` itself — in-place is safe; see
 * `resolveOut`'s doc), else into a fresh buffer; `rgba` is never mutated
 * unless it IS `out`.
 */
export function applyExposureOffset(
  rgba: Uint8ClampedArray,
  offset: number,
  out?: Uint8ClampedArray,
): Uint8ClampedArray {
  const dst = resolveOut(rgba, out);
  if (offset === 0) {
    if (dst !== rgba) dst.set(rgba);
    return dst;
  }
  for (let i = 0; i < rgba.length; i += 4) {
    dst[i] = clamp255(rgba[i]! + offset);
    dst[i + 1] = clamp255(rgba[i + 1]! + offset);
    dst[i + 2] = clamp255(rgba[i + 2]! + offset);
    dst[i + 3] = rgba[i + 3]!;
  }
  return dst;
}

/**
 * Add i.i.d. Gaussian noise (mean 0, standard deviation `sigma`) to every
 * R/G/B channel independently (alpha untouched), clamped to `[0, 255]`.
 * `sigma <= 0` is a no-op (copy or passthrough). Deterministic given
 * `seed` — the same `(rgba, sigma, seed)` always produces byte-identical
 * output, which is what makes this unit-testable (a real per-call
 * `Math.random()` would make output assertions flaky). Writes into `out`
 * when provided (which may be `rgba` itself — in-place is safe; see
 * `resolveOut`'s doc), else into a fresh buffer; `rgba` is never mutated
 * unless it IS `out`.
 */
export function applyGaussianNoise(
  rgba: Uint8ClampedArray,
  sigma: number,
  seed: number,
  out?: Uint8ClampedArray,
): Uint8ClampedArray {
  const dst = resolveOut(rgba, out);
  if (sigma <= 0) {
    if (dst !== rgba) dst.set(rgba);
    return dst;
  }
  const rand = mulberry32(seed);
  for (let i = 0; i < rgba.length; i += 4) {
    dst[i] = clamp255(rgba[i]! + nextGaussian(rand) * sigma);
    dst[i + 1] = clamp255(rgba[i + 1]! + nextGaussian(rand) * sigma);
    dst[i + 2] = clamp255(rgba[i + 2]! + nextGaussian(rand) * sigma);
    dst[i + 3] = rgba[i + 3]!;
  }
  return dst;
}

/** Minimal 2D-canvas surface {@link applyGaussianBlurCanvas} needs —
 * matches both `HTMLCanvasElement`/`CanvasRenderingContext2D` and
 * `OffscreenCanvas`/`OffscreenCanvasRenderingContext2D`, so callers can
 * pass whichever their environment prefers (a worker has no
 * `HTMLCanvasElement`). */
export interface Canvas2DLike {
  width: number;
  height: number;
  getContext(id: "2d"): {
    filter: string;
    putImageData(imageData: ImageData, dx: number, dy: number): void;
    drawImage(image: unknown, dx: number, dy: number): void;
    getImageData(sx: number, sy: number, sw: number, sh: number): ImageData;
    clearRect(x: number, y: number, w: number, h: number): void;
  } | null;
}

/**
 * Approximate a Gaussian blur of `sigmaPx` by round-tripping the buffer
 * through a 2D canvas with `ctx.filter = "blur(...)"` — the browser's own
 * (real, if kernel-unspecified) Gaussian blur implementation. `sigmaPx <=
 * 0` is a no-op copy (skips touching the canvas entirely).
 *
 * Two-canvas round trip: `ctx.filter` only affects `drawImage` calls, not
 * `putImageData` (which writes pixels directly, bypassing the filter
 * pipeline) — so the source pixels are painted onto `srcCanvas`
 * un-filtered via `putImageData`, then drawn (filtered) onto `dstCanvas`
 * via `drawImage`, and finally read back from `dstCanvas`.
 *
 * Untested (needs a real `CanvasRenderingContext2D`, unavailable in
 * vitest's `node` environment) — manual QA only (Task 7's browser pass:
 * "knobs sweep — blur up -> error up smoothly, no crash").
 */
export function applyGaussianBlurCanvas(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  sigmaPx: number,
  makeCanvas: (w: number, h: number) => Canvas2DLike,
): Uint8ClampedArray {
  if (sigmaPx <= 0) return rgba.slice();

  const srcCanvas = makeCanvas(width, height);
  const srcCtx = srcCanvas.getContext("2d");
  if (!srcCtx) throw new Error("applyGaussianBlurCanvas: 2d context unavailable (source canvas)");
  srcCtx.putImageData(new ImageData(rgba.slice(), width, height), 0, 0);

  const dstCanvas = makeCanvas(width, height);
  const dstCtx = dstCanvas.getContext("2d");
  if (!dstCtx) throw new Error("applyGaussianBlurCanvas: 2d context unavailable (dest canvas)");
  dstCtx.filter = `blur(${sigmaPx}px)`;
  dstCtx.drawImage(srcCanvas, 0, 0);

  return dstCtx.getImageData(0, 0, width, height).data as Uint8ClampedArray;
}
