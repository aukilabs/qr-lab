package com.aukilabs.cpuscanner

import expo.modules.kotlin.exception.CodedException

/**
 * JNI bridge to `libqrk_ffi.so` (built by `just expo-android`).
 * Library name must match the cdylib: `libqrk_ffi.so` → `System.loadLibrary("qrk_ffi")`.
 */
object QrkNative {
  val isAvailable: Boolean =
    runCatching { System.loadLibrary("qrk_ffi") }.isSuccess

  @JvmStatic
  external fun nativeScanLuma(
    luma: ByteArray,
    width: Int,
    height: Int,
    stride: Int,
    maxDim: Int,
    refine: Boolean,
  ): String

  fun scanLuma(
    luma: ByteArray,
    width: Int,
    height: Int,
    stride: Int,
    maxDim: Int,
    refine: Boolean,
  ): String {
    if (!isAvailable) {
      throw CodedException(
        "expo-cpu-scanner native library missing — run `just expo-android` to build libqrk_ffi.so",
      )
    }
    return nativeScanLuma(luma, width, height, stride, maxDim, refine)
  }
}
