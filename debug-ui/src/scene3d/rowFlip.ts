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
 * twice returns the original row order). Returns a new buffer; does not
 * mutate `rgba`.
 */
export function flipRowsRgba(rgba: Uint8ClampedArray, width: number, height: number): Uint8ClampedArray {
  const rowBytes = width * 4;
  const expected = rowBytes * height;
  if (rgba.length !== expected) {
    throw new RangeError(
      `flipRowsRgba: buffer is ${rgba.length} bytes, expected width*height*4 = ${expected} (${width}x${height})`,
    );
  }
  const out = new Uint8ClampedArray(rgba.length);
  for (let y = 0; y < height; y++) {
    const srcStart = y * rowBytes;
    const dstStart = (height - 1 - y) * rowBytes;
    out.set(rgba.subarray(srcStart, srcStart + rowBytes), dstStart);
  }
  return out;
}
