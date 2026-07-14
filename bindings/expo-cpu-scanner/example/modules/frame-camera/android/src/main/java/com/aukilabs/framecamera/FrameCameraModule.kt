package com.aukilabs.framecamera

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class FrameCameraModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("FrameCamera")

    // Preview + analysis surface; emits `onFrame` with Y-plane luma.
    View(FrameCameraView::class) {
      Events("onFrame", "onError")

      Prop("targetFps") { view: FrameCameraView, fps: Double? ->
        view.setTargetFps(fps?.toInt() ?: 12)
      }

      Prop("active") { view: FrameCameraView, active: Boolean? ->
        view.setActive(active ?: true)
      }
    }
  }
}
