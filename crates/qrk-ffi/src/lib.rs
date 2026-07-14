//! C ABI (+ Android JNI) surface for QRKit.
//!
//! Mobile consumers never call `qrk-core` directly: they load `libqrk_ffi`
//! (Android `.so` / iOS staticlib in an xcframework) and use the C entry
//! points in `include/qrk.h`. Android also gets a thin JNI wrapper so the
//! Expo module can pass a `ByteArray` without hand-rolling pointer glue.
//!
//! All heap strings returned across the C ABI are UTF-8, NUL-terminated,
//! and must be freed with [`qrk_free_string`].

use std::ffi::CString;
use std::mem::size_of;
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::slice;

use qrkit::image::{Gray8View, Gray8ViewMut};
use qrkit::imgproc::blur::estimate_line_direction;
use qrkit::imgproc::deblur::{
    van_cittert_line_into, DeblurWorkspace, LineBorderMode, VanCittertConfig,
};
use qrkit::imgproc::illumination::{
    background_divide_into, BackgroundDivideConfig, IlluminationWorkspace,
};
use qrkit::{downscaled_dims, scan, DecodedCode, Detections, LumaView, ScanOptions, StageTimings};
use serde::Serialize;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// JSON envelope (camelCase for the Expo/TS side)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FfiResult {
    scan_width: u32,
    scan_height: u32,
    source_scale: f64,
    codes: Vec<FfiCode>,
    timings: FfiTimings,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FfiCode {
    payload: String,
    /// Base64 of the raw payload bytes (binary-safe).
    payload_bytes_b64: String,
    version: u32,
    /// Single-character ECC level (`L`/`M`/`Q`/`H`/`?`).
    ecc: String,
    mirrored: bool,
    inverted: bool,
    dimension: u32,
    /// Module-region corners TL/TR/BR/BL in WORKING pixels.
    corners: [[f64; 2]; 4],
    /// Subpixel-refined corners in SOURCE pixels, when `refine` was set.
    refined_corners: Option<[[f64; 2]; 4]>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FfiTimings {
    tiles_ns: u64,
    finders_ns: u64,
    triplets_ns: u64,
    version_ns: u64,
    alignment_ns: u64,
    sample_decode_ns: u64,
    refine_ns: u64,
}

impl From<&StageTimings> for FfiTimings {
    fn from(t: &StageTimings) -> Self {
        Self {
            tiles_ns: t.tiles_ns,
            finders_ns: t.finders_ns,
            triplets_ns: t.triplets_ns,
            version_ns: t.version_ns,
            alignment_ns: t.alignment_ns,
            sample_decode_ns: t.sample_decode_ns,
            refine_ns: t.refine_ns,
        }
    }
}

impl From<&DecodedCode> for FfiCode {
    fn from(c: &DecodedCode) -> Self {
        Self {
            payload: c.payload.clone(),
            payload_bytes_b64: base64_encode(&c.payload_bytes),
            version: c.version,
            ecc: c.ecc.to_string(),
            mirrored: c.mirrored,
            inverted: c.inverted,
            dimension: c.dimension,
            corners: c.corners,
            refined_corners: c.refined_corners,
        }
    }
}

fn envelope(dets: &Detections, source_w: usize, source_h: usize, max_dim: u32) -> FfiResult {
    let (scan_w, scan_h) =
        downscaled_dims(source_w, source_h, max_dim).unwrap_or((source_w, source_h));
    FfiResult {
        scan_width: scan_w as u32,
        scan_height: scan_h as u32,
        source_scale: dets.source_scale,
        codes: dets.codes.iter().map(FfiCode::from).collect(),
        timings: FfiTimings::from(&dets.timings),
    }
}

/// Minimal base64 (no padding edge cases needed beyond standard) so we
/// don't pull an extra crate for a single field.
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn scan_luma_to_json(
    luma: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    max_dim: u32,
    refine: bool,
) -> Result<String, String> {
    let w = width as usize;
    let h = height as usize;
    let stride = stride as usize;
    if w == 0 || h == 0 {
        return Err(format!(
            "qrk_scan_luma: width/height must be non-zero (got {w}x{h})"
        ));
    }
    if stride < w {
        return Err(format!("qrk_scan_luma: stride ({stride}) < width ({w})"));
    }
    let needed = stride
        .checked_mul(h.saturating_sub(1))
        .and_then(|n| n.checked_add(w))
        .ok_or_else(|| "qrk_scan_luma: buffer size overflow".to_string())?;
    if luma.len() < needed {
        return Err(format!(
            "qrk_scan_luma: buffer too short ({} bytes, need at least {needed} for {w}x{h} stride={stride})",
            luma.len()
        ));
    }

    let view = LumaView::new(luma, w, h, stride)
        .map_err(|e| format!("qrk_scan_luma: invalid luma view: {e:?}"))?;
    let opts = ScanOptions {
        max_working_dim: max_dim,
        refine,
    };
    let dets = scan(&view, &opts);
    let env = envelope(&dets, w, h, max_dim);
    serde_json::to_string(&env).map_err(|e| format!("qrk_scan_luma: serialize failed: {e}"))
}

fn cstring_from_str(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        // JSON never contains interior NULs; if it somehow did, fall back
        // to an empty object rather than panic across the FFI boundary.
        Err(_) => match CString::new("{}") {
            Ok(c) => c.into_raw(),
            Err(_) => ptr::null_mut(),
        },
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

/// Library version string (static; do not free).
#[no_mangle]
pub extern "C" fn qrk_version() -> *const c_char {
    use std::sync::OnceLock;
    static HELD: OnceLock<CString> = OnceLock::new();
    HELD.get_or_init(|| CString::new(VERSION).unwrap_or_default())
        .as_ptr()
}

/// Scan an 8-bit grayscale frame. Returns a heap JSON string (free with
/// [`qrk_free_string`]), or NULL on invalid input / allocation failure.
///
/// # Safety
/// `luma` must point to at least `stride * (height - 1) + width` readable
/// bytes for the duration of the call. `NULL` is rejected.
#[no_mangle]
pub unsafe extern "C" fn qrk_scan_luma(
    luma: *const u8,
    width: u32,
    height: u32,
    stride: u32,
    max_dim: u32,
    refine: c_int,
) -> *mut c_char {
    if luma.is_null() || width == 0 || height == 0 || stride < width {
        return ptr::null_mut();
    }
    let stride_usize = stride as usize;
    let height_usize = height as usize;
    let width_usize = width as usize;
    let len = match stride_usize
        .checked_mul(height_usize.saturating_sub(1))
        .and_then(|n| n.checked_add(width_usize))
    {
        Some(n) => n,
        None => return ptr::null_mut(),
    };
    let slice = slice::from_raw_parts(luma, len);
    match scan_luma_to_json(slice, width, height, stride, max_dim, refine != 0) {
        Ok(json) => cstring_from_str(&json),
        Err(_) => ptr::null_mut(),
    }
}

/// Free a string returned by [`qrk_scan_luma`]. NULL-safe.
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by this crate's
/// string-returning functions and not already freed.
#[no_mangle]
pub unsafe extern "C" fn qrk_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    drop(CString::from_raw(ptr));
}

// ---------------------------------------------------------------------------
// Reusable image-operator C ABI (version 1)
// ---------------------------------------------------------------------------

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QrkStatus {
    Ok = 0,
    NullPointer = 1,
    InvalidArgument = 2,
    BufferTooSmall = 3,
    ProcessingFailed = 4,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct QrkBackgroundDivideConfigV1 {
    pub struct_size: u32,
    pub structuring_element: u32,
    pub target_luma: u8,
    pub denominator_floor: u8,
    pub reserved: [u8; 2],
}

impl Default for QrkBackgroundDivideConfigV1 {
    fn default() -> Self {
        let config = BackgroundDivideConfig::default();
        Self {
            struct_size: size_of::<Self>() as u32,
            structuring_element: config.structuring_element as u32,
            target_luma: config.target_luma,
            denominator_floor: config.denominator_floor,
            reserved: [0; 2],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct QrkVanCittertConfigV1 {
    pub struct_size: u32,
    pub blur_length: u32,
    pub iterations: u32,
    pub border_mode: u32,
    pub theta_radians: f64,
    pub relaxation: f64,
    pub border_value: u8,
    pub reserved: [u8; 7],
}

impl Default for QrkVanCittertConfigV1 {
    fn default() -> Self {
        let config = VanCittertConfig::default();
        Self {
            struct_size: size_of::<Self>() as u32,
            blur_length: config.blur_length as u32,
            iterations: config.iterations as u32,
            border_mode: 0,
            theta_radians: config.theta_radians,
            relaxation: config.relaxation,
            border_value: 0,
            reserved: [0; 7],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct QrkLineBlurEstimateV1 {
    pub struct_size: u32,
    pub has_length: u32,
    pub theta_radians: f64,
    pub confidence: f64,
    pub length_px: f64,
    pub raster_dx: i32,
    pub raster_dy: i32,
}

/// Opaque reusable scratch state for native image-operator calls.
#[derive(Default)]
pub struct QrkOperatorContext {
    illumination: IlluminationWorkspace,
    deblur: DeblurWorkspace,
    last_error: CString,
}

impl QrkOperatorContext {
    fn fail(&mut self, status: QrkStatus, message: impl AsRef<str>) -> c_int {
        self.last_error = CString::new(message.as_ref()).unwrap_or_default();
        status as c_int
    }

    fn succeed(&mut self) -> c_int {
        self.last_error = CString::default();
        QrkStatus::Ok as c_int
    }
}

#[no_mangle]
pub extern "C" fn qrk_operator_context_create() -> *mut QrkOperatorContext {
    Box::into_raw(Box::new(QrkOperatorContext::default()))
}

/// # Safety
/// `context` must be null or a pointer returned by
/// [`qrk_operator_context_create`] that has not already been destroyed.
#[no_mangle]
pub unsafe extern "C" fn qrk_operator_context_destroy(context: *mut QrkOperatorContext) {
    if !context.is_null() {
        drop(Box::from_raw(context));
    }
}

/// Return the last operator error for this context. The pointer remains valid
/// until the next call using the context or until the context is destroyed.
///
/// # Safety
/// `context` must point to a live operator context.
#[no_mangle]
pub unsafe extern "C" fn qrk_operator_last_error(
    context: *const QrkOperatorContext,
) -> *const c_char {
    if context.is_null() {
        return ptr::null();
    }
    (*context).last_error.as_ptr()
}

/// Initialize a version-1 background-division configuration.
///
/// # Safety
/// `out_config` must point to writable storage for the configuration.
#[no_mangle]
pub unsafe extern "C" fn qrk_background_divide_config_v1_default(
    out_config: *mut QrkBackgroundDivideConfigV1,
) -> c_int {
    if out_config.is_null() {
        return QrkStatus::NullPointer as c_int;
    }
    *out_config = QrkBackgroundDivideConfigV1::default();
    QrkStatus::Ok as c_int
}

/// Initialize a version-1 Van Cittert configuration.
///
/// # Safety
/// `out_config` must point to writable storage for the configuration.
#[no_mangle]
pub unsafe extern "C" fn qrk_van_cittert_config_v1_default(
    out_config: *mut QrkVanCittertConfigV1,
) -> c_int {
    if out_config.is_null() {
        return QrkStatus::NullPointer as c_int;
    }
    *out_config = QrkVanCittertConfigV1::default();
    QrkStatus::Ok as c_int
}

unsafe fn input_view<'a>(
    data: *const u8,
    width: u32,
    height: u32,
    stride: u32,
) -> Result<Gray8View<'a>, QrkStatus> {
    if data.is_null() {
        return Err(QrkStatus::NullPointer);
    }
    let len = ffi_buffer_len(width, height, stride)?;
    Gray8View::new(
        slice::from_raw_parts(data, len),
        width as usize,
        height as usize,
        stride as usize,
    )
    .map_err(|_| QrkStatus::InvalidArgument)
}

unsafe fn output_view<'a>(
    data: *mut u8,
    width: u32,
    height: u32,
    stride: u32,
) -> Result<Gray8ViewMut<'a>, QrkStatus> {
    if data.is_null() {
        return Err(QrkStatus::NullPointer);
    }
    let len = ffi_buffer_len(width, height, stride)?;
    Gray8ViewMut::new(
        slice::from_raw_parts_mut(data, len),
        width as usize,
        height as usize,
        stride as usize,
    )
    .map_err(|_| QrkStatus::InvalidArgument)
}

fn ffi_buffer_len(width: u32, height: u32, stride: u32) -> Result<usize, QrkStatus> {
    if width == 0 || height == 0 || stride < width {
        return Err(QrkStatus::InvalidArgument);
    }
    (stride as usize)
        .checked_mul(height as usize - 1)
        .and_then(|value| value.checked_add(width as usize))
        .ok_or(QrkStatus::BufferTooSmall)
}

/// Normalize uneven illumination into a caller-owned grayscale buffer.
///
/// # Safety
/// All pointers must be valid for their documented lengths for the duration of
/// the call. `src` and `dst` must not overlap.
#[no_mangle]
pub unsafe extern "C" fn qrk_background_divide_luma_v1(
    context: *mut QrkOperatorContext,
    src: *const u8,
    width: u32,
    height: u32,
    src_stride: u32,
    dst: *mut u8,
    dst_stride: u32,
    config: *const QrkBackgroundDivideConfigV1,
) -> c_int {
    if context.is_null() || config.is_null() {
        return QrkStatus::NullPointer as c_int;
    }
    let context = &mut *context;
    if (*config).struct_size < size_of::<QrkBackgroundDivideConfigV1>() as u32 {
        return context.fail(
            QrkStatus::InvalidArgument,
            "background config struct_size is too small",
        );
    }
    let source = match input_view(src, width, height, src_stride) {
        Ok(view) => view,
        Err(status) => return context.fail(status, "invalid source image layout"),
    };
    let destination = match output_view(dst, width, height, dst_stride) {
        Ok(view) => view,
        Err(status) => return context.fail(status, "invalid destination image layout"),
    };
    let config = BackgroundDivideConfig {
        structuring_element: (*config).structuring_element as usize,
        target_luma: (*config).target_luma,
        denominator_floor: (*config).denominator_floor,
    };
    match background_divide_into(source, destination, &config, &mut context.illumination) {
        Ok(()) => context.succeed(),
        Err(error) => context.fail(QrkStatus::ProcessingFailed, error.to_string()),
    }
}

/// Apply configurable line-PSF Van Cittert restoration into caller-owned
/// grayscale storage.
///
/// # Safety
/// All pointers must be valid for their documented lengths for the duration of
/// the call. `src` and `dst` must not overlap.
#[no_mangle]
pub unsafe extern "C" fn qrk_van_cittert_luma_v1(
    context: *mut QrkOperatorContext,
    src: *const u8,
    width: u32,
    height: u32,
    src_stride: u32,
    dst: *mut u8,
    dst_stride: u32,
    config: *const QrkVanCittertConfigV1,
) -> c_int {
    if context.is_null() || config.is_null() {
        return QrkStatus::NullPointer as c_int;
    }
    let context = &mut *context;
    if (*config).struct_size < size_of::<QrkVanCittertConfigV1>() as u32 {
        return context.fail(
            QrkStatus::InvalidArgument,
            "deblur config struct_size is too small",
        );
    }
    let source = match input_view(src, width, height, src_stride) {
        Ok(view) => view,
        Err(status) => return context.fail(status, "invalid source image layout"),
    };
    let destination = match output_view(dst, width, height, dst_stride) {
        Ok(view) => view,
        Err(status) => return context.fail(status, "invalid destination image layout"),
    };
    let config = VanCittertConfig {
        theta_radians: (*config).theta_radians,
        blur_length: (*config).blur_length as usize,
        iterations: (*config).iterations as usize,
        relaxation: (*config).relaxation,
        border: match (*config).border_mode {
            0 => LineBorderMode::Replicate,
            1 => LineBorderMode::Reflect,
            2 => LineBorderMode::Constant((*config).border_value),
            _ => return context.fail(QrkStatus::InvalidArgument, "unknown deblur border_mode"),
        },
    };
    match van_cittert_line_into(source, destination, &config, &mut context.deblur) {
        Ok(_) => context.succeed(),
        Err(error) => context.fail(QrkStatus::ProcessingFailed, error.to_string()),
    }
}

/// Estimate directional blur without modifying the image.
///
/// # Safety
/// `src` must describe readable image storage and `out_estimate` must point to
/// writable version-1 estimate storage with `struct_size` initialized.
#[no_mangle]
pub unsafe extern "C" fn qrk_estimate_line_blur_luma_v1(
    context: *mut QrkOperatorContext,
    src: *const u8,
    width: u32,
    height: u32,
    src_stride: u32,
    out_estimate: *mut QrkLineBlurEstimateV1,
) -> c_int {
    if context.is_null() || out_estimate.is_null() {
        return QrkStatus::NullPointer as c_int;
    }
    let context = &mut *context;
    if (*out_estimate).struct_size < size_of::<QrkLineBlurEstimateV1>() as u32 {
        return context.fail(
            QrkStatus::InvalidArgument,
            "estimate struct_size is too small",
        );
    }
    let source = match input_view(src, width, height, src_stride) {
        Ok(view) => view,
        Err(status) => return context.fail(status, "invalid source image layout"),
    };
    let direction = estimate_line_direction(source);
    let length = qrkit::imgproc::blur::estimate_line_length(source, direction.theta_radians);
    let (raster_dx, raster_dy) = direction.raster_direction.step();
    *out_estimate = QrkLineBlurEstimateV1 {
        struct_size: size_of::<QrkLineBlurEstimateV1>() as u32,
        has_length: u32::from(length.is_some()),
        theta_radians: direction.theta_radians,
        confidence: direction.confidence,
        length_px: length.unwrap_or(0.0),
        raster_dx: raster_dx as i32,
        raster_dy: raster_dy as i32,
    };
    context.succeed()
}

// ---------------------------------------------------------------------------
// Android JNI
// ---------------------------------------------------------------------------

#[cfg(target_os = "android")]
mod android {
    use super::scan_luma_to_json;
    use jni::objects::{JByteArray, JClass};
    use jni::sys::jstring;
    use jni::JNIEnv;

    /// `com.aukilabs.cpuscanner.QrkNative.nativeScanLuma`
    ///
    /// Signature:
    /// `( [B IIIIZ )Ljava/lang/String;`
    /// — luma bytes, width, height, stride, maxDim, refine.
    #[no_mangle]
    pub extern "system" fn Java_com_aukilabs_cpuscanner_QrkNative_nativeScanLuma<'local>(
        mut env: JNIEnv<'local>,
        _class: JClass<'local>,
        luma: JByteArray<'local>,
        width: i32,
        height: i32,
        stride: i32,
        max_dim: i32,
        refine: jni::sys::jboolean,
    ) -> jstring {
        let result = (|| -> Result<String, String> {
            if width <= 0 || height <= 0 || stride < width {
                return Err("invalid geometry".into());
            }
            let bytes = env
                .convert_byte_array(&luma)
                .map_err(|e| format!("byte array: {e}"))?;
            scan_luma_to_json(
                &bytes,
                width as u32,
                height as u32,
                stride as u32,
                max_dim.max(0) as u32,
                refine != 0,
            )
        })();

        match result {
            Ok(json) => env
                .new_string(json)
                .map(|s| s.into_raw())
                .unwrap_or(std::ptr::null_mut()),
            Err(msg) => {
                let _ = env.throw_new("java/lang/IllegalArgumentException", msg);
                std::ptr::null_mut()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qrkit::luma_from_rgba;

    #[test]
    fn base64_roundtrip_empty_and_hello() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn scan_emptyish_frame_returns_json() {
        // Uniform gray — no codes, but must still produce a valid envelope.
        let w = 64u32;
        let h = 48u32;
        let rgba = vec![128u8; (w * h * 4) as usize];
        let luma = luma_from_rgba(&rgba, w as usize, h as usize);
        let json = scan_luma_to_json(&luma, w, h, w, 0, false).expect("scan");
        let v: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(v["scanWidth"], w);
        assert_eq!(v["scanHeight"], h);
        assert!(v["codes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn operator_abi_uses_caller_owned_buffers_and_reusable_context() {
        let context = qrk_operator_context_create();
        assert!(!context.is_null());
        let src = vec![120u8; 7 * 5];
        let mut dst = vec![0u8; src.len()];
        let config = QrkVanCittertConfigV1::default();
        let status = unsafe {
            qrk_van_cittert_luma_v1(context, src.as_ptr(), 7, 5, 7, dst.as_mut_ptr(), 7, &config)
        };
        assert_eq!(status, QrkStatus::Ok as c_int);
        assert_eq!(src, dst);
        unsafe { qrk_operator_context_destroy(context) };
    }

    #[test]
    fn operator_abi_reports_versioned_config_errors() {
        let context = qrk_operator_context_create();
        let src = [0u8; 9];
        let mut dst = vec![0u8; 9];
        let config = QrkVanCittertConfigV1 {
            struct_size: 0,
            ..QrkVanCittertConfigV1::default()
        };
        let status = unsafe {
            qrk_van_cittert_luma_v1(context, src.as_ptr(), 3, 3, 3, dst.as_mut_ptr(), 3, &config)
        };
        assert_eq!(status, QrkStatus::InvalidArgument as c_int);
        let message = unsafe { std::ffi::CStr::from_ptr(qrk_operator_last_error(context)) };
        assert!(message.to_string_lossy().contains("struct_size"));
        unsafe { qrk_operator_context_destroy(context) };
    }
}
