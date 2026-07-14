import AVFoundation
import ExpoModulesCore
import UIKit

// MARK: - Process-lifetime capture session
//
// Crash root cause (serve-sim / SimCamInjector):
//   EXC_BAD_ACCESS in -[AVCaptureConnection dealloc] when React unmounts the
//   native view and we remove outputs / nil the session. The inject dylib's
//   fake I/O objects do not survive AVFoundation teardown.
//
// Fix: one AVCaptureSession for the whole process. Views only attach a
// preview layer + frame callback. We never removeInput/removeOutput/stop
// on unmount.

private protocol FrameSink: AnyObject {
  func onLuma(_ luma: Data, width: Int, height: Int, stride: Int, frameId: Int, timestampMs: Double)
  func onCameraError(_ message: String)
}

private final class SharedCapture: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
  static let shared = SharedCapture()

  let session = AVCaptureSession()
  private let queue = DispatchQueue(label: "com.aukilabs.framecamera.session")
  private let sampleQ = DispatchQueue(label: "com.aukilabs.framecamera.sample", qos: .userInitiated)

  private var configured = false
  private var intervalNs: UInt64 = 1_000_000_000 / 8
  private var lastNs: UInt64 = 0
  private var frameId = 0
  private var active = true
  private weak var sink: FrameSink?

  func setFps(_ fps: Int) {
    intervalNs = 1_000_000_000 / UInt64(max(1, min(30, fps)))
  }

  func setActive(_ on: Bool) { active = on }

  func attach(_ sink: FrameSink) {
    self.sink = sink
    queue.async { [weak self] in
      guard let self else { return }
      self.ensureConfigured()
      if self.configured && !self.session.isRunning {
        self.session.startRunning()
      }
    }
  }

  func detach(_ sink: FrameSink) {
    if self.sink === sink { self.sink = nil }
    // Intentionally leave session running with I/O attached.
  }

  private func ensureConfigured() {
    if configured { return }

    let status = AVCaptureDevice.authorizationStatus(for: .video)
    if status == .notDetermined {
      let sem = DispatchSemaphore(value: 0)
      var ok = false
      AVCaptureDevice.requestAccess(for: .video) { granted in
        ok = granted
        sem.signal()
      }
      _ = sem.wait(timeout: .now() + 30)
      if !ok {
        DispatchQueue.main.async { [weak self] in
          self?.sink?.onCameraError("CAMERA permission denied")
        }
        return
      }
    } else if status != .authorized {
      DispatchQueue.main.async { [weak self] in
        self?.sink?.onCameraError("CAMERA permission not granted")
      }
      return
    }

    session.beginConfiguration()
    // Prefer 1080p when the device supports it (falls back automatically).
    if session.canSetSessionPreset(.hd1920x1080) {
      session.sessionPreset = .hd1920x1080
    } else {
      session.sessionPreset = .hd1280x720
    }

    if session.inputs.isEmpty {
      guard
        let device =
          AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
          ?? AVCaptureDevice.default(for: .video),
        let input = try? AVCaptureDeviceInput(device: device),
        session.canAddInput(input)
      else {
        session.commitConfiguration()
        DispatchQueue.main.async { [weak self] in
          self?.sink?.onCameraError("No camera available")
        }
        return
      }
      session.addInput(input)
    }

    if session.outputs.isEmpty {
      let output = AVCaptureVideoDataOutput()
      output.alwaysDiscardsLateVideoFrames = true
      // BGRA: serve-sim injects kCVPixelFormatType_32BGRA; real devices accept it too.
      // We convert BGRA → 8-bit luma (qrk_core formula) before scanning.
      output.videoSettings = [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA
      ]
      output.setSampleBufferDelegate(self, queue: sampleQ)
      guard session.canAddOutput(output) else {
        session.commitConfiguration()
        DispatchQueue.main.async { [weak self] in
          self?.sink?.onCameraError("Cannot add video data output")
        }
        return
      }
      session.addOutput(output)
      if let conn = output.connection(with: .video) {
        Self.rotatePortrait(conn)
      }
    }

    session.commitConfiguration()
    configured = true
  }

  static func rotatePortrait(_ connection: AVCaptureConnection) {
    if #available(iOS 17.0, *) {
      if connection.isVideoRotationAngleSupported(90) {
        connection.videoRotationAngle = 90
      }
    } else if connection.isVideoOrientationSupported {
      connection.videoOrientation = .portrait
    }
  }

  func captureOutput(
    _ output: AVCaptureOutput,
    didOutput sampleBuffer: CMSampleBuffer,
    from connection: AVCaptureConnection
  ) {
    guard active, sink != nil else { return }
    let now = DispatchTime.now().uptimeNanoseconds
    if now &- lastNs < intervalNs { return }
    lastNs = now

    guard let pb = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
    guard let packed = Self.extractLuma(from: pb) else { return }

    frameId += 1
    let id = frameId
    let ts = Double(now) / 1e6

    DispatchQueue.main.async { [weak self] in
      self?.sink?.onLuma(
        packed.luma,
        width: packed.width,
        height: packed.height,
        stride: packed.stride,
        frameId: id,
        timestampMs: ts
      )
    }
  }

  /// Extract tight-packed 8-bit luma from BGRA (serve-sim) or Y-plane (device).
  private static func extractLuma(from pb: CVPixelBuffer) -> (
    luma: Data, width: Int, height: Int, stride: Int
  )? {
    CVPixelBufferLockBaseAddress(pb, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }

    let w = CVPixelBufferGetWidth(pb)
    let h = CVPixelBufferGetHeight(pb)
    guard w > 0, h > 0 else { return nil }

    let fmt = CVPixelBufferGetPixelFormatType(pb)
    if fmt == kCVPixelFormatType_32BGRA || fmt == kCVPixelFormatType_32RGBA {
      return bgraToLuma(pb: pb, width: w, height: h, isRGBA: fmt == kCVPixelFormatType_32RGBA)
    }
    return yPlaneToLuma(pb: pb)
  }

  /// qrk_core::luma_from_rgba formula on BGRA/RGBA pixels → tight gray plane.
  private static func bgraToLuma(
    pb: CVPixelBuffer, width w: Int, height h: Int, isRGBA: Bool
  ) -> (luma: Data, width: Int, height: Int, stride: Int)? {
    guard let base = CVPixelBufferGetBaseAddress(pb) else { return nil }
    let srcStride = CVPixelBufferGetBytesPerRow(pb)
    let src = base.assumingMemoryBound(to: UInt8.self)
    var out = [UInt8](repeating: 0, count: w * h)
    for y in 0..<h {
      let rowOff = y * srcStride
      let dstOff = y * w
      for x in 0..<w {
        let i = rowOff + x * 4
        let c0 = UInt32(src[i])
        let c1 = UInt32(src[i + 1])
        let c2 = UInt32(src[i + 2])
        // BGRA: B,G,R,A  —  RGBA: R,G,B,A
        let r = isRGBA ? c0 : c2
        let g = c1
        let b = isRGBA ? c2 : c0
        out[dstOff + x] = UInt8((77 * r + 150 * g + 29 * b + 128) >> 8)
      }
    }
    return (Data(out), w, h, w)
  }

  private static func yPlaneToLuma(pb: CVPixelBuffer) -> (
    luma: Data, width: Int, height: Int, stride: Int
  )? {
    let yW = CVPixelBufferGetWidthOfPlane(pb, 0)
    let yH = CVPixelBufferGetHeightOfPlane(pb, 0)
    let yStride = CVPixelBufferGetBytesPerRowOfPlane(pb, 0)
    guard let base = CVPixelBufferGetBaseAddressOfPlane(pb, 0), yW > 0, yH > 0 else {
      return nil
    }
    let src = base.assumingMemoryBound(to: UInt8.self)
    if yStride == yW {
      return (Data(bytes: base, count: yW * yH), yW, yH, yW)
    }
    var out = [UInt8](repeating: 0, count: yW * yH)
    for y in 0..<yH {
      let srcRow = src.advanced(by: y * yStride)
      for x in 0..<yW {
        out[y * yW + x] = srcRow[x]
      }
    }
    return (Data(out), yW, yH, yW)
  }
}

// MARK: - Expo view (preview + event bridge only)

class FrameCameraView: ExpoView, FrameSink {
  let onFrame = EventDispatcher()
  let onError = EventDispatcher()

  private let previewLayer = AVCaptureVideoPreviewLayer()
  private var attached = false

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
    clipsToBounds = true
    backgroundColor = .black
    previewLayer.videoGravity = .resizeAspectFill
    previewLayer.session = SharedCapture.shared.session
    layer.addSublayer(previewLayer)
  }

  // No session teardown in deinit — SharedCapture owns the session forever.
  deinit {
    if attached {
      SharedCapture.shared.detach(self)
    }
  }

  func setTargetFps(_ fps: Int) { SharedCapture.shared.setFps(fps) }
  func setActive(_ active: Bool) { SharedCapture.shared.setActive(active) }

  override func layoutSubviews() {
    super.layoutSubviews()
    previewLayer.frame = bounds
    if let c = previewLayer.connection {
      SharedCapture.rotatePortrait(c)
    }
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window != nil {
      if previewLayer.session == nil {
        previewLayer.session = SharedCapture.shared.session
      }
      if !attached {
        SharedCapture.shared.attach(self)
        attached = true
      }
    } else if attached {
      SharedCapture.shared.detach(self)
      attached = false
      // Keep previewLayer.session set — nilling it can trigger connection dealloc
      // paths that crash under the serve-sim inject dylib.
    }
  }

  func onLuma(
    _ luma: Data, width: Int, height: Int, stride: Int, frameId: Int, timestampMs: Double
  ) {
    onFrame([
      "width": width,
      "height": height,
      "stride": stride,
      "luma": luma,
      "frameId": frameId,
      "timestampMs": timestampMs,
    ])
  }

  func onCameraError(_ message: String) {
    onError(["message": message])
  }
}
