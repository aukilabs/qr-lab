import type { QrScanResult, ScanLumaOptions } from "./ExpoCpuScanner.types";
import ExpoCpuScannerModule from "./ExpoCpuScannerModule";

export type {
  QrCodeDetection,
  QrCorner,
  QrScanResult,
  QrScanTimings,
  ScanLumaOptions,
} from "./ExpoCpuScanner.types";

/**
 * Scan an 8-bit grayscale frame.
 *
 * `luma` is row-major Y; `stride` defaults to `width` (tight packing).
 * `maxDim` caps the working resolution (0 = full source). `refine` enables
 * subpixel corner refinement (`refinedCorners` in source pixels).
 */
export function scanLuma(
  luma: Uint8Array,
  width: number,
  height: number,
  options: ScanLumaOptions & { stride?: number } = {},
): QrScanResult {
  const stride = options.stride ?? width;
  const maxDim = options.maxDim ?? 0;
  const refine = options.refine ?? false;
  const raw = ExpoCpuScannerModule.scanLuma(luma, width, height, stride, maxDim, refine);
  if (!raw) {
    throw new Error("expo-cpu-scanner: scanLuma returned empty result");
  }
  return JSON.parse(raw) as QrScanResult;
}

/** Reserved for a future session API; currently a no-op on native. */
export function destroyScanner(): void {
  ExpoCpuScannerModule.destroyScanner();
}
