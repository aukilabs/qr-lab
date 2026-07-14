import ExpoModulesCore

public class FrameCameraModule: Module {
  public func definition() -> ModuleDefinition {
    Name("FrameCamera")

    View(FrameCameraView.self) {
      Events("onFrame", "onError")

      Prop("targetFps") { (view: FrameCameraView, fps: Double?) in
        view.setTargetFps(Int(fps ?? 12))
      }

      Prop("active") { (view: FrameCameraView, active: Bool?) in
        view.setActive(active ?? true)
      }
    }
  }
}
