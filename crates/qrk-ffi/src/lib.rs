//! C ABI (+ Android JNI) surface for `qrk-core`.
//!
//! Mobile consumers never call `qrk-core` directly: they load `libqrk_ffi`
//! (Android `.so` / iOS staticlib in an xcframework) and use the C entry
//! points in `include/qrk.h`. Android also gets a thin JNI wrapper so the
//! Expo module can pass a `ByteArray` without hand-rolling pointer glue.
//!
//! All heap strings returned across the C ABI are UTF-8, NUL-terminated,
//! and must be freed with [`qrk_free_string`].

use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::ptr;
use std::slice;

use qrk_core::{
    downscaled_dims, scan, DecodedCode, Detections, LumaView, ScanOptions, StageTimings,
};
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
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
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
        return Err(format!("qrk_scan_luma: width/height must be non-zero (got {w}x{h})"));
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
    use qrk_core::luma_from_rgba;

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
}
