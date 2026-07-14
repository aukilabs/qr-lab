import { NativeModule, requireNativeModule } from "expo";

declare class ExpoCpuScannerModule extends NativeModule {
  /**
   * Scan a tightly packed or strided 8-bit luma plane.
   * Returns a JSON string matching `QrScanResult`.
   */
  scanLuma(
    luma: Uint8Array,
    width: number,
    height: number,
    stride: number,
    maxDim: number,
    refine: boolean,
  ): string;

  /** Reserved for a future session API; currently a no-op. */
  destroyScanner(): void;
}

export default requireNativeModule<ExpoCpuScannerModule>("ExpoCpuScanner");
