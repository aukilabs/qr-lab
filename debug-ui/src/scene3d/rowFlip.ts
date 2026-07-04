// `WebGLRenderingContext.readPixels` (and `readRenderTargetPixels`, which
// wraps it) always returns rows BOTTOM-UP — row 0 of the returned buffer
// is the bottom scanline of the rendered image, the opposite of every
// other rgba buffer this debug UI handles (`useImageSource`/
// `useVideoSource`'s `getImageData`, `downscaleRgba`'s output, ...), which
// are all top-down (row 0 = top). `qrk_core::scan` (and every image the
// Rust pipeline was designed against) assumes top-down rows, so a
// readback buffer MUST be flipped before it's handed to
// `ScannerClient.scan` — otherwise the scan (and the ground-truth
// comparison) runs against an upside-down image.

/**
 * Flip an rgba buffer's rows top<->bottom (its own inverse — flipping
 * twice returns the original row order). Writes into `out` when provided
 * (added for the 3D scene's per-tick readback path, Plan 5 Task 5 review
 * — one persistent scratch buffer instead of a fresh allocation per scan
 * tick), else into a fresh buffer; `rgba` is never mutated. Unlike the
 * per-pixel `camSim.ts` transforms, `out` must NOT alias `rgba`'s storage
 * (row `y` is written into slot `height-1-y` before that source row has
 * been read — aliasing would corrupt half the image); rejected up front,
 * including a same-`ArrayBuffer` different-view alias.
 */
export function flipRowsRgba(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  out?: Uint8ClampedArray,
): Uint8ClampedArray {
  const rowBytes = width * 4;
  const expected = rowBytes * height;
  if (rgba.length !== expected) {
    throw new RangeError(
      `flipRowsRgba: buffer is ${rgba.length} bytes, expected width*height*4 = ${expected} (${width}x${height})`,
    );
  }
  let dst: Uint8ClampedArray;
  if (out === undefined) {
    dst = new Uint8ClampedArray(rgba.length);
  } else {
    if (out.length !== expected) {
      throw new RangeError(`flipRowsRgba: out buffer is ${out.length} bytes, expected ${expected}`);
    }
    if (out.buffer === rgba.buffer) {
      throw new RangeError("flipRowsRgba: out must not share storage with rgba (see doc)");
    }
    dst = out;
  }
  for (let y = 0; y < height; y++) {
    const srcStart = y * rowBytes;
    const dstStart = (height - 1 - y) * rowBytes;
    dst.set(rgba.subarray(srcStart, srcStart + rowBytes), dstStart);
  }
  return dst;
}
