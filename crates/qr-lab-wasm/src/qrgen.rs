//! In-browser QR *generation* (Plan 5 Task 5) — the mirror image of the
//! rest of this crate, which only ever *decodes*. The debug UI's 3D-scene
//! mode needs a real, decodable QR bit matrix to texture a plane with, so
//! it can compare `qr_lab::scan`'s live `refined_corners` against an
//! analytically known ground truth as the camera orbits.
//!
//! Gated behind the `qr-gen` cargo feature (see this crate's `Cargo.toml`):
//! the `qrcode` crate is an optional dependency, pulled in ONLY by that
//! feature, so the mobile-relevant default build (and `qr-lab-core` itself,
//! which never depends on `qrcode` outside its own `dev-dependencies`)
//! stays free of it. `scripts/build-wasm.sh` always builds the debug UI's
//! wasm package WITH `--features qr-gen`; `scripts/check-wasm.sh` checks
//! both configurations (with and without) so a build that accidentally
//! makes `qr-gen` load-bearing for the default target would be caught.
//!
//! Split into two layers, same pattern as `lib.rs`'s `scan_rgba`:
//! `generate_qr_bits` is plain Rust (`Result<BitMatrix, String>`, no
//! `JsValue`) so it's natively unit-testable with a plain `cargo test`;
//! `generate_qr` is the thin `#[wasm_bindgen]` wrapper the debug UI calls,
//! converting the matrix to the same packed `{dim, words}` shape
//! `BitsTrace` already uses on the wire (see `qr_lab::trace::BitsTrace`),
//! so the debug UI's existing bit-matrix-unpacking code (`overlays/layers/
//! bits.ts`) can be reused as-is for rendering the generated QR's texture.

use qr_lab::BitMatrix;
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Serializable envelope for `generate_qr`'s return value — same packed
/// shape as `qr_lab::trace::BitsTrace` (row-major `u32` words, `dim`
/// wide) so the debug UI can reuse its existing bit-matrix TS helpers.
#[derive(Serialize)]
#[doc(hidden)]
pub struct GeneratedQr {
    pub dim: u32,
    pub words: Vec<u32>,
}

/// Generate a QR bit matrix for `payload` and pack it into a `BitMatrix`.
/// Plain Rust — no `wasm_bindgen`/`JsValue` — so it's directly unit
/// testable (see the `tests` module below, which round-trips the result
/// through `qr_lab::decode_bits`) without needing a wasm runtime.
///
/// `version`: `0` means auto-select the smallest version that fits
/// `payload` at the requested `ecc` (via `qrcode::QrCode::
/// with_error_correction_level`); `1..=40` requests that exact version
/// (via `qrcode::QrCode::with_version`), erroring if `payload` doesn't
/// fit. `ecc`: `0..=3` for L/M/Q/H (the same order `qr_lab::bitmatrix`'s
/// `ecc_char` reports them in) — chosen over a `char` param purely for a
/// simpler JS call site (a plain number, no string marshaling).
pub(crate) fn generate_qr_bits(payload: &str, version: u32, ecc: u8) -> Result<BitMatrix, String> {
    let ec_level = match ecc {
        0 => qrcode::EcLevel::L,
        1 => qrcode::EcLevel::M,
        2 => qrcode::EcLevel::Q,
        3 => qrcode::EcLevel::H,
        other => {
            return Err(format!(
                "generate_qr: ecc must be 0..=3 (L/M/Q/H), got {other}"
            ))
        }
    };

    let code = if version == 0 {
        qrcode::QrCode::with_error_correction_level(payload.as_bytes(), ec_level)
    } else {
        if version > 40 {
            return Err(format!(
                "generate_qr: version must be 0 (auto) or 1..=40, got {version}"
            ));
        }
        qrcode::QrCode::with_version(
            payload.as_bytes(),
            qrcode::Version::Normal(version as i16),
            ec_level,
        )
    }
    .map_err(|e| {
        format!(
            "generate_qr: {e:?} (payload {} bytes, version {version}, ecc {ecc})",
            payload.len()
        )
    })?;

    let dim = code.width();
    let mut m = BitMatrix::new(dim);
    for y in 0..dim {
        for x in 0..dim {
            m.set(x, y, code[(x, y)] == qrcode::Color::Dark);
        }
    }
    Ok(m)
}

/// wasm binding: generate a QR for `payload` and return `{dim, words}` (via
/// `serde-wasm-bindgen`) as a plain JS object, or `Err` with a descriptive
/// message when `payload` doesn't fit the requested version/ecc, or `ecc`
/// is out of range. See `generate_qr_bits` for the parameter contract.
#[wasm_bindgen]
pub fn generate_qr(payload: &str, version: u32, ecc: u8) -> Result<JsValue, JsValue> {
    let m = generate_qr_bits(payload, version, ecc).map_err(|e| JsValue::from_str(&e))?;
    serde_wasm_bindgen::to_value(&GeneratedQr {
        dim: m.dim as u32,
        words: m.words().to_vec(),
    })
    .map_err(JsValue::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core round-trip gate: whatever `generate_qr_bits` produces must
    /// decode back through our OWN decoder (`qr_lab::decode_bits`, the
    /// same one `qr_lab::decode`'s sample+decode stage calls) to exactly
    /// the payload that went in. This is the one test that matters for
    /// "is the generated matrix actually a valid QR" — everything else
    /// (dim, version auto-selection) is secondary.
    #[test]
    fn generate_qr_bits_round_trips_through_decode_bits() {
        let m = generate_qr_bits("HELLO WORLD", 0, 1 /* M */).unwrap();
        let decoded = qr_lab::decode_bits(&m).expect("generated matrix should decode");
        assert_eq!(decoded.payload, "HELLO WORLD");
        assert_eq!(decoded.ecc, 'M');
    }

    #[test]
    fn generate_qr_bits_round_trips_at_fixed_version() {
        // Version 5-M can hold this payload comfortably; with_version
        // should honor the exact version requested rather than picking a
        // smaller one.
        let m = generate_qr_bits("subpixel refinement plan 5 task 5", 5, 3 /* H */).unwrap();
        assert_eq!(m.dim, 4 * 5 + 17); // QR dimension formula: 4*version + 17
        let decoded = qr_lab::decode_bits(&m).expect("generated matrix should decode");
        assert_eq!(decoded.payload, "subpixel refinement plan 5 task 5");
        assert_eq!(decoded.version, 5);
        assert_eq!(decoded.ecc, 'H');
    }

    #[test]
    fn generate_qr_bits_round_trips_across_all_ecc_levels() {
        for (ecc, want) in [(0u8, 'L'), (1, 'M'), (2, 'Q'), (3, 'H')] {
            let m = generate_qr_bits("ECC SWEEP", 0, ecc).unwrap();
            let decoded = qr_lab::decode_bits(&m).expect("generated matrix should decode");
            assert_eq!(decoded.payload, "ECC SWEEP");
            assert_eq!(decoded.ecc, want, "ecc index {ecc}");
        }
    }

    #[test]
    fn generate_qr_bits_rejects_out_of_range_ecc() {
        let err = generate_qr_bits("X", 0, 4).err().unwrap();
        assert!(
            err.contains("ecc must be 0..=3"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn generate_qr_bits_rejects_out_of_range_version() {
        let err = generate_qr_bits("X", 41, 0).err().unwrap();
        assert!(
            err.contains("version must be 0"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn generate_qr_bits_rejects_payload_too_large_for_requested_version() {
        // Version 1-H holds at most 17 alphanumeric chars; this payload is
        // deliberately far larger and must error rather than panic.
        let huge: String = "A".repeat(200);
        let err = generate_qr_bits(&huge, 1, 3).err().unwrap();
        assert!(err.starts_with("generate_qr:"), "unexpected message: {err}");
    }
}
