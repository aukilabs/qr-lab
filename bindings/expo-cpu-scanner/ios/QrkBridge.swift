import Foundation
import Qrk

/// Thin Swift wrapper around the C ABI in `qrk.h` (vendored via Qrk.xcframework).
enum QrkBridge {
  enum ScanError: Error, LocalizedError {
    case invalidInput
    case emptyResult

    var errorDescription: String? {
      switch self {
      case .invalidInput:
        return "expo-cpu-scanner: invalid luma input (check width/height/stride)"
      case .emptyResult:
        return "expo-cpu-scanner: qrk_scan_luma returned null"
      }
    }
  }

  static func scanLuma(
    luma: Data,
    width: Int,
    height: Int,
    stride: Int,
    maxDim: Int,
    refine: Bool
  ) throws -> String {
    guard width > 0, height > 0, stride >= width,
          let width32 = UInt32(exactly: width),
          let height32 = UInt32(exactly: height),
          let stride32 = UInt32(exactly: stride),
          let maxDim32 = UInt32(exactly: max(0, maxDim)) else {
      throw ScanError.invalidInput
    }
    // Validate before arithmetic and C conversion: malformed JS dimensions
    // must throw a recoverable error instead of trapping the app process.
    let (rows, multiplyOverflow) = stride.multipliedReportingOverflow(by: height - 1)
    let (needed, addOverflow) = rows.addingReportingOverflow(width)
    guard !multiplyOverflow, !addOverflow, luma.count >= needed else {
      throw ScanError.invalidInput
    }

    let jsonPtr: UnsafeMutablePointer<CChar>? = luma.withUnsafeBytes { raw in
      guard let base = raw.bindMemory(to: UInt8.self).baseAddress else {
        return nil
      }
      return qrk_scan_luma(
        base,
        width32,
        height32,
        stride32,
        maxDim32,
        refine ? 1 : 0
      )
    }

    guard let jsonPtr else {
      throw ScanError.emptyResult
    }
    defer { qrk_free_string(jsonPtr) }

    return String(cString: jsonPtr)
  }
}
