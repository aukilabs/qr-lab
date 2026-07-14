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
    guard width > 0, height > 0, stride >= width else {
      throw ScanError.invalidInput
    }
    let needed = stride * (height - 1) + width
    guard luma.count >= needed else {
      throw ScanError.invalidInput
    }

    let jsonPtr: UnsafeMutablePointer<CChar>? = luma.withUnsafeBytes { raw in
      guard let base = raw.bindMemory(to: UInt8.self).baseAddress else {
        return nil
      }
      return qrk_scan_luma(
        base,
        UInt32(width),
        UInt32(height),
        UInt32(stride),
        UInt32(max(0, maxDim)),
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
