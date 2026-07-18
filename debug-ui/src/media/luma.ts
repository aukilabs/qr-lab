// Point-sample luma (grayscale) at an integer-rounded pixel coordinate in a
// tightly packed RGBA buffer, using the scanner's own integer BT.601-ish
// formula (see `crates/qr-lab-core/src/luma.rs`'s `luma_from_rgba`) rather than
// a from-scratch grayscale approximation — so the debug UI's status-bar
// readout shows the same value the scanner itself would derive from this
// pixel, not merely "a" luma.

/**
 * `rgba[y*width+x]`'s luma, 0..255, or `null` when `(x, y)` (rounded down)
 * falls outside `[0, width) x [0, height)` — e.g. the cursor has left the
 * displayed bitmap, or nothing is loaded yet (`width`/`height` are 0).
 * `x`/`y` are floored rather than rounded to match how a screen pixel maps
 * onto the single source pixel it's inside, consistent with `downscaleRgba`
 * and `Viewport`'s own `Math.floor` use at the same image/screen boundary.
 */
export function lumaAt(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  x: number,
  y: number,
): number | null {
  const ix = Math.floor(x);
  const iy = Math.floor(y);
  if (ix < 0 || iy < 0 || ix >= width || iy >= height) return null;

  const idx = (iy * width + ix) * 4;
  const r = rgba[idx] ?? 0;
  const g = rgba[idx + 1] ?? 0;
  const b = rgba[idx + 2] ?? 0;
  // Same fixed-point coefficients (77/150/29 over 256, rounded) as
  // `luma_from_rgba` — kept numerically identical, not just "close", so a
  // side-by-side comparison against the scanner's own luma plane is
  // meaningful.
  return (77 * r + 150 * g + 29 * b + 128) >> 8;
}

/**
 * Whole-buffer sibling of {@link lumaAt}: converts a tightly-packed
 * `width x height` rgba buffer into a `width * height`-byte luma plane,
 * pixel-for-pixel, via the exact same fixed-point formula as
 * `qr_lab_core::luma_from_rgba` (`crates/qr-lab-core/src/luma.rs`) — used by the
 * 3D scene's "save as fixture" `.luma` export (Plan 5d), which must
 * byte-match what the Rust pipeline would derive from the same rgba frame,
 * not merely approximate it.
 */
export function lumaBufferFromRgba(
  rgba: Uint8ClampedArray | Uint8Array,
  width: number,
  height: number,
): Uint8Array<ArrayBuffer> {
  const expected = width * height * 4;
  if (rgba.length !== expected) {
    throw new RangeError(
      `lumaBufferFromRgba: buffer is ${rgba.length} bytes, expected width*height*4 = ${expected} (${width}x${height})`,
    );
  }
  const out = new Uint8Array(width * height);
  for (let i = 0, p = 0; i < rgba.length; i += 4, p++) {
    const r = rgba[i]!;
    const g = rgba[i + 1]!;
    const b = rgba[i + 2]!;
    out[p] = (77 * r + 150 * g + 29 * b + 128) >> 8;
  }
  return out;
}
