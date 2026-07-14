import ExpoModulesCore

public class ExpoCpuScannerModule: Module {
  public func definition() -> ModuleDefinition {
    Name("ExpoCpuScanner")

    Function("scanLuma") {
      (
        luma: Data,
        width: Int,
        height: Int,
        stride: Int,
        maxDim: Int,
        refine: Bool
      ) -> String in
      return try QrkBridge.scanLuma(
        luma: luma,
        width: width,
        height: height,
        stride: stride,
        maxDim: maxDim,
        refine: refine
      )
    }

    Function("destroyScanner") {
      // Stateless v1 API — reserved for a future ScanSession.
    }
  }
}
