// Nearest-neighbor RGBA downscale that replicates the production capture
// path's working-resolution cap (see docs/superpowers/plans/2026-07-03-
// plan3-debug-ui-image-mode.md): scale so the longest side is at most
// `maxDim`, using `round(dim * maxDim / max(w,h))` for both output
// dimensions — the same rounding the native scanner uses — so the debug UI
// predicts on-device detection behavior instead of drifting from it.

/**
 * Downscale `rgba` (tightly packed, row stride == `w`) to at most `maxDim`
 * on its longest side, nearest-neighbor. Returns the *same* `rgba`
 * reference (no copy) when no resize is needed — `maxDim <= 0` (cap
 * disabled) or `max(w, h) <= maxDim` (already within budget) — so callers
 * can cheaply check for a no-op via `result.rgba === input`.
 */
export function downscaleRgba(
  rgba: Uint8ClampedArray,
  w: number,
  h: number,
  maxDim: number,
): { rgba: Uint8ClampedArray; width: number; height: number } {
  const longest = Math.max(w, h);
  // longest <= maxDim (identity) also covers longest === 0, which would
  // otherwise divide by zero below and propagate NaN into the output
  // dimensions — degenerate w/h has nothing sensible to downscale anyway.
  if (maxDim <= 0 || longest <= maxDim) {
    return { rgba, width: w, height: h };
  }

  // Production rounding: round(dim * maxDim / max(w,h)), never below 1px.
  const dstW = Math.max(1, Math.round((w * maxDim) / longest));
  const dstH = Math.max(1, Math.round((h * maxDim) / longest));

  const out = new Uint8ClampedArray(dstW * dstH * 4);
  for (let y = 0; y < dstH; y++) {
    const srcY = Math.min(h - 1, Math.floor((y * h) / dstH));
    const srcRowStart = srcY * w;
    const dstRowStart = y * dstW;
    for (let x = 0; x < dstW; x++) {
      const srcX = Math.min(w - 1, Math.floor((x * w) / dstW));
      const srcIdx = (srcRowStart + srcX) * 4;
      const dstIdx = (dstRowStart + x) * 4;
      out[dstIdx] = rgba[srcIdx]!;
      out[dstIdx + 1] = rgba[srcIdx + 1]!;
      out[dstIdx + 2] = rgba[srcIdx + 2]!;
      out[dstIdx + 3] = rgba[srcIdx + 3]!;
    }
  }

  return { rgba: out, width: dstW, height: dstH };
}
