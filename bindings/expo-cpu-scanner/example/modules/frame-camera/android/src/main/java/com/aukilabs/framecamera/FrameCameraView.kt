package com.aukilabs.framecamera

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import android.graphics.Matrix
import android.graphics.RectF
import android.graphics.SurfaceTexture
import android.util.Base64
import android.util.Log
import android.util.Size
import android.view.Surface
import android.view.TextureView
import android.view.ViewGroup
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.SurfaceRequest
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.nio.ByteBuffer
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference

/**
 * Color hardware preview (TextureView + CameraX Preview) + high-res analysis.
 *
 * Orientation follows CameraX [PreviewTransformation] for TextureView:
 *  - When the Surface has a camera transform (normal device path), TextureView
 *    already applies sensor orientation — remaining matrix rotation is only
 *    related to target/display rotation (0 when target is ROTATION_0).
 *  - Do **not** apply TransformationInfo.rotationDegrees again; that double-rotates
 *    and is what made the feed swing 90° CW / 90° CCW.
 *  - FILL_CENTER is done via view scale/translation (same as PreviewView).
 *
 * PreviewView itself times out under RN Fabric on this device; TextureView with
 * an explicit Surface provider does not.
 */
class FrameCameraView(context: Context, appContext: AppContext) :
  ExpoView(context, appContext), TextureView.SurfaceTextureListener {

  private val onFrame by EventDispatcher()
  private val onError by EventDispatcher()

  private val textureView =
    TextureView(context).apply {
      layoutParams =
        LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
      surfaceTextureListener = this@FrameCameraView
    }

  private val analysisExecutor = Executors.newSingleThreadExecutor()
  private val mainExecutor by lazy { ContextCompat.getMainExecutor(context) }
  private var cameraProvider: ProcessCameraProvider? = null

  private var rawY: ByteArray? = null
  private var uprightY: ByteArray? = null
  private var emitY: ByteArray? = null
  private var bufW = 0
  private var bufH = 0
  private var uprightW = 0
  private var uprightH = 0
  private var emitW = 0
  private var emitH = 0

  private val emitIntervalNs = AtomicLong(1_000_000_000L / 6L)
  private val lastEmitNs = AtomicLong(0L)
  private val frameCounter = AtomicInteger(0)
  private val analysisActive = AtomicBoolean(true)
  private val bindingInFlight = AtomicBoolean(false)
  private val isBound = AtomicBoolean(false)
  private val jsInFlight = AtomicBoolean(false)
  private val surfaceReady = AtomicBoolean(false)
  private val pendingSurfaceRequest = AtomicReference<SurfaceRequest?>(null)
  private var bindAttempts = 0
  private var previewResolution: Size? = null
  private var lastTransformInfo: SurfaceRequest.TransformationInfo? = null

  init {
    layoutParams =
      LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
    // Clip children so FILL_CENTER overflow (TextureView larger than parent) is cropped.
    clipChildren = true
    clipToPadding = true
    setBackgroundColor(0xFF000000.toInt())
    addView(textureView)
    Log.i(TAG, "init TextureView — CameraX PreviewTransformation (no double-rotate)")
    post { scheduleBind("init") }
  }

  fun setTargetFps(fps: Int) {
    emitIntervalNs.set(1_000_000_000L / fps.coerceIn(1, 15))
  }

  fun setActive(active: Boolean) {
    analysisActive.set(active)
  }

  override fun onSurfaceTextureAvailable(surface: SurfaceTexture, width: Int, height: Int) {
    Log.i(TAG, "SurfaceTexture available ${width}x$height")
    surfaceReady.set(true)
    pendingSurfaceRequest.getAndSet(null)?.let { fulfilSurfaceRequest(it) }
    scheduleBind("surfaceAvailable")
  }

  override fun onSurfaceTextureSizeChanged(surface: SurfaceTexture, width: Int, height: Int) {
    previewResolution?.let { surface.setDefaultBufferSize(it.width, it.height) }
  }

  override fun onSurfaceTextureDestroyed(surface: SurfaceTexture): Boolean {
    Log.i(TAG, "SurfaceTexture destroyed")
    surfaceReady.set(false)
    isBound.set(false)
    return true
  }

  override fun onSurfaceTextureUpdated(surface: SurfaceTexture) {}

  override fun onAttachedToWindow() {
    super.onAttachedToWindow()
    scheduleBind("onAttachedToWindow")
  }

  override fun onDetachedFromWindow() {
    analysisActive.set(false)
    super.onDetachedFromWindow()
  }

  override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
    super.onMeasure(widthMeasureSpec, heightMeasureSpec)
    textureView.measure(
      MeasureSpec.makeMeasureSpec(measuredWidth, MeasureSpec.EXACTLY),
      MeasureSpec.makeMeasureSpec(measuredHeight, MeasureSpec.EXACTLY),
    )
  }

  override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
    super.onLayout(changed, left, top, right, bottom)
    val w = right - left
    val h = bottom - top
    if (w > 0 && h > 0) {
      layoutTextureFillCenter(w, h)
      if (changed) scheduleBind("onLayout")
    }
  }

  /**
   * Size TextureView to the upright content aspect and center it so the parent
   * clips overflow (true FILL_CENTER). Avoids setTransform stretch hacks that
   * fight the camera Surface transform.
   *
   * Buffer 1600×1200 with rotationDegrees=90 → upright 1200×1600 (3:4).
   */
  private fun layoutTextureFillCenter(parentW: Int, parentH: Int) {
    textureView.scaleX = 1f
    textureView.scaleY = 1f
    textureView.translationX = 0f
    textureView.translationY = 0f
    textureView.setTransform(Matrix())

    val res = previewResolution
    if (res == null) {
      textureView.layout(0, 0, parentW, parentH)
      return
    }
    val rot = ((lastTransformInfo?.rotationDegrees ?: 90) % 360 + 360) % 360
    val contentW = if (rot % 180 == 0) res.width.toFloat() else res.height.toFloat()
    val contentH = if (rot % 180 == 0) res.height.toFloat() else res.width.toFloat()
    // Scale so content covers parent (FILL), then center.
    val scale = maxOf(parentW / contentW, parentH / contentH)
    val childW = (contentW * scale).toInt().coerceAtLeast(1)
    val childH = (contentH * scale).toInt().coerceAtLeast(1)
    val x = (parentW - childW) / 2
    val y = (parentH - childH) / 2
    textureView.layout(x, y, x + childW, y + childH)
    Log.i(
      TAG,
      "layout fill-center content=${contentW.toInt()}x${contentH.toInt()} " +
        "child=${childW}x$childH @ ($x,$y) parent=${parentW}x$parentH",
    )
  }

  private fun scheduleBind(reason: String) {
    post {
      if (!isAttachedToWindow) return@post
      if (width <= 0 || height <= 0) {
        if (bindAttempts < 60) {
          bindAttempts++
          postDelayed({ scheduleBind("retry-size") }, 40)
        }
        return@post
      }
      if (!surfaceReady.get() && textureView.isAvailable) surfaceReady.set(true)
      if (!surfaceReady.get()) {
        if (bindAttempts < 60) {
          bindAttempts++
          postDelayed({ scheduleBind("retry-surface") }, 40)
        }
        return@post
      }
      analysisActive.set(true)
      tryBindCamera(reason)
    }
  }

  private fun tryBindCamera(reason: String) {
    if (isBound.get() || bindingInFlight.get()) return
    if (
      ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) !=
        PackageManager.PERMISSION_GRANTED
    ) {
      onError(mapOf("message" to "CAMERA permission not granted"))
      return
    }
    val owner = findLifecycleOwner()
    if (owner == null) {
      if (bindAttempts < 60) {
        bindAttempts++
        postDelayed({ tryBindCamera("retry-owner") }, 80)
      } else {
        onError(mapOf("message" to "No LifecycleOwner for CameraX"))
      }
      return
    }
    if (!bindingInFlight.compareAndSet(false, true)) return
    Log.i(TAG, "tryBindCamera ($reason)")

    val future = ProcessCameraProvider.getInstance(context)
    future.addListener(
      {
        try {
          val provider = future.get()
          cameraProvider = provider
          bindUseCases(provider, owner)
          isBound.set(true)
          Log.i(TAG, "Preview(TextureView)+ImageAnalysis bound")
        } catch (e: Exception) {
          Log.e(TAG, "bind failed", e)
          isBound.set(false)
          onError(mapOf("message" to (e.message ?: "camera bind failed")))
        } finally {
          bindingInFlight.set(false)
        }
      },
      mainExecutor,
    )
  }

  @Suppress("DEPRECATION")
  private fun displayRotation(): Int {
    val act = appContext.currentActivity
    if (act != null) return act.windowManager.defaultDisplay.rotation
    val wm = context.getSystemService(Context.WINDOW_SERVICE) as? android.view.WindowManager
    return wm?.defaultDisplay?.rotation ?: Surface.ROTATION_0
  }

  private fun bindUseCases(provider: ProcessCameraProvider, owner: LifecycleOwner) {
    provider.unbindAll()
    val targetRot = displayRotation()
    val highRes = highestResSelector(Size(1920, 1080))

    val preview =
      Preview.Builder()
        .setResolutionSelector(highRes)
        .setTargetRotation(targetRot)
        .build()
        .also { p ->
          p.setSurfaceProvider { request ->
            if (surfaceReady.get() && textureView.isAvailable) {
              fulfilSurfaceRequest(request)
            } else {
              Log.w(TAG, "Preview surface parked until TextureView ready")
              pendingSurfaceRequest.set(request)
            }
          }
        }

    val analysis =
      ImageAnalysis.Builder()
        .setResolutionSelector(highRes)
        .setTargetRotation(targetRot)
        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
        .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)
        .build()
        .also { it.setAnalyzer(analysisExecutor, ::analyzeFrame) }

    val selector =
      try {
        CameraSelector.DEFAULT_BACK_CAMERA
      } catch (_: Exception) {
        CameraSelector.DEFAULT_FRONT_CAMERA
      }
    provider.bindToLifecycle(owner, selector, preview, analysis)
  }

  private fun fulfilSurfaceRequest(request: SurfaceRequest) {
    val st = textureView.surfaceTexture
    if (st == null || !textureView.isAvailable) {
      request.willNotProvideSurface()
      return
    }
    val res = request.resolution
    previewResolution = res
    st.setDefaultBufferSize(res.width, res.height)
    Log.i(TAG, "Providing preview Surface ${res.width}x${res.height}")

    request.setTransformationInfoListener(mainExecutor) { info ->
      lastTransformInfo = info
      Log.i(
        TAG,
        "transform info rot=${info.rotationDegrees} target=${info.targetRotation} " +
          "hasCam=${info.hasCameraTransform()}",
      )
      // Re-layout TextureView with correct aspect once we know buffer + rotation.
      if (width > 0 && height > 0) layoutTextureFillCenter(width, height)
    }

    val surface = Surface(st)
    request.provideSurface(surface, mainExecutor) { surface.release() }
  }

  private fun highestResSelector(preferred: Size): ResolutionSelector =
    ResolutionSelector.Builder()
      .setResolutionStrategy(
        ResolutionStrategy(
          preferred,
          ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER,
        ),
      )
      .build()

  private fun analyzeFrame(image: ImageProxy) {
    try {
      if (!analysisActive.get()) return
      val now = System.nanoTime()
      val last = lastEmitNs.get()
      if (now - last < emitIntervalNs.get()) return
      if (jsInFlight.get()) return
      if (!lastEmitNs.compareAndSet(last, now)) return

      val srcW = image.width
      val srcH = image.height
      val rotation = image.imageInfo.rotationDegrees
      val plane = image.planes[0] ?: return

      ensureSrcBuffers(srcW, srcH)
      extractTightY(plane.buffer, srcW, srcH, plane.rowStride, plane.pixelStride, rawY!!)

      val (dstW, dstH) = uprightSize(srcW, srcH, rotation)
      ensureUprightBuffers(dstW, dstH)
      rotateY(rawY!!, srcW, srcH, rotation, uprightY!!)

      val id = frameCounter.incrementAndGet()
      if (id == 1 || id % 30 == 0) {
        Log.i(TAG, "scan #$id ${srcW}x$srcH rot=$rotation → ${dstW}x$dstH")
      }

      val (ew, eh, emit) = maybeDownscaleForEmit(uprightY!!, dstW, dstH)
      val b64 = Base64.encodeToString(emit, 0, ew * eh, Base64.NO_WRAP)
      if (!jsInFlight.compareAndSet(false, true)) return
      try {
        onFrame(
          mapOf(
            "width" to ew,
            "height" to eh,
            "stride" to ew,
            "luma" to b64,
            "encoding" to "base64",
            "frameId" to id,
            "timestampMs" to (now / 1_000_000L),
          ),
        )
      } finally {
        postDelayed({ jsInFlight.set(false) }, 20)
      }
    } catch (e: OutOfMemoryError) {
      Log.e(TAG, "OOM — lower emit rate", e)
      System.gc()
      emitIntervalNs.set(1_000_000_000L / 3L)
      jsInFlight.set(false)
      onError(mapOf("message" to "Out of memory — reduced scan rate"))
    } catch (e: Exception) {
      Log.e(TAG, "analyzeFrame failed", e)
      jsInFlight.set(false)
      onError(mapOf("message" to (e.message ?: "analyzeFrame failed")))
    } finally {
      image.close()
    }
  }

  private fun ensureSrcBuffers(w: Int, h: Int) {
    val n = w * h
    if (rawY == null || rawY!!.size < n || bufW != w || bufH != h) {
      rawY = ByteArray(n)
      bufW = w
      bufH = h
    }
  }

  private fun ensureUprightBuffers(w: Int, h: Int) {
    val n = w * h
    if (uprightY == null || uprightY!!.size < n || uprightW != w || uprightH != h) {
      uprightY = ByteArray(n)
      uprightW = w
      uprightH = h
    }
  }

  private fun maybeDownscaleForEmit(src: ByteArray, w: Int, h: Int): Triple<Int, Int, ByteArray> {
    val longSide = maxOf(w, h)
    if (longSide <= EMIT_MAX) {
      ensureEmitBuffers(w, h)
      System.arraycopy(src, 0, emitY!!, 0, w * h)
      return Triple(w, h, emitY!!)
    }
    val scale = EMIT_MAX.toFloat() / longSide
    val nw = (w * scale).toInt().coerceAtLeast(1)
    val nh = (h * scale).toInt().coerceAtLeast(1)
    ensureEmitBuffers(nw, nh)
    val dst = emitY!!
    var di = 0
    for (y in 0 until nh) {
      val sy = (y * h) / nh
      val srcRow = sy * w
      for (x in 0 until nw) {
        dst[di++] = src[srcRow + (x * w) / nw]
      }
    }
    return Triple(nw, nh, dst)
  }

  private fun ensureEmitBuffers(w: Int, h: Int) {
    val n = w * h
    if (emitY == null || emitY!!.size < n || emitW != w || emitH != h) {
      emitY = ByteArray(n)
      emitW = w
      emitH = h
    }
  }

  private fun extractTightY(
    buffer: ByteBuffer,
    width: Int,
    height: Int,
    rowStride: Int,
    pixelStride: Int,
    out: ByteArray,
  ) {
    buffer.rewind()
    if (pixelStride == 1 && rowStride == width) {
      buffer.get(out, 0, (width * height).coerceAtMost(buffer.remaining()))
      return
    }
    val row = ByteArray(rowStride)
    var dst = 0
    for (y in 0 until height) {
      val toRead = minOf(rowStride, buffer.remaining())
      if (toRead <= 0) break
      buffer.get(row, 0, toRead)
      if (pixelStride == 1) {
        System.arraycopy(row, 0, out, dst, width)
        dst += width
      } else {
        var x = 0
        while (x < width) {
          out[dst++] = row[x * pixelStride]
          x++
        }
      }
    }
  }

  private fun uprightSize(w: Int, h: Int, degrees: Int): Pair<Int, Int> =
    if (degrees % 180 == 0) Pair(w, h) else Pair(h, w)

  private fun rotateY(src: ByteArray, w: Int, h: Int, degrees: Int, dst: ByteArray) {
    val d = ((degrees % 360) + 360) % 360
    when (d) {
      0 -> System.arraycopy(src, 0, dst, 0, w * h)
      90 -> {
        for (y in 0 until h) {
          for (x in 0 until w) {
            dst[x * h + (h - 1 - y)] = src[y * w + x]
          }
        }
      }
      180 -> {
        val n = w * h
        for (i in 0 until n) dst[n - 1 - i] = src[i]
      }
      270 -> {
        for (y in 0 until h) {
          for (x in 0 until w) {
            dst[y + (w - 1 - x) * h] = src[y * w + x]
          }
        }
      }
      else -> System.arraycopy(src, 0, dst, 0, w * h)
    }
  }

  private fun findLifecycleOwner(): LifecycleOwner? {
    val act = appContext.currentActivity
    if (act is LifecycleOwner) return act
    var ctx: Context? = context
    while (ctx is ContextWrapper) {
      if (ctx is LifecycleOwner) return ctx
      if (ctx is Activity) return ctx as? LifecycleOwner
      ctx = ctx.baseContext
    }
    return null
  }

  companion object {
    private const val TAG = "FrameCameraView"
    private const val EMIT_MAX = 1080
    /** CameraX TransformUtils.NORMALIZED_RECT. */
    private val NORMALIZED_RECT = RectF(-1f, -1f, 1f, 1f)

    /** Same as CameraOrientationUtil.surfaceRotationToDegrees. */
    private fun surfaceRotationToDegrees(rotation: Int): Int =
      when (rotation) {
        Surface.ROTATION_0 -> 0
        Surface.ROTATION_90 -> 90
        Surface.ROTATION_180 -> 180
        Surface.ROTATION_270 -> 270
        else -> 0
      }

    /**
     * CameraX TransformUtils.getRectToRect — maps [source] → [target] with
     * [rotationDegrees] applied (CW in Android Matrix / y-down space).
     */
    private fun rectToRect(source: RectF, target: RectF, rotationDegrees: Int): Matrix {
      val matrix = Matrix()
      matrix.setRectToRect(source, NORMALIZED_RECT, Matrix.ScaleToFit.FILL)
      matrix.postRotate(rotationDegrees.toFloat())
      val restore = Matrix()
      restore.setRectToRect(NORMALIZED_RECT, target, Matrix.ScaleToFit.FILL)
      matrix.postConcat(restore)
      return matrix
    }
  }
}
