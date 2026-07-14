package com.aukilabs.cpuscanner

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class ExpoCpuScannerModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("ExpoCpuScanner")

    Function("scanLuma") {
        luma: ByteArray,
        width: Int,
        height: Int,
        stride: Int,
        maxDim: Int,
        refine: Boolean,
      ->
      QrkNative.scanLuma(luma, width, height, stride, maxDim, refine)
    }

    Function("destroyScanner") {
      // Stateless v1 API — reserved for a future ScanSession.
    }
  }
}
