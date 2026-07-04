//! Packed bit matrix + rqrr `BitGrid` adapter + top-level decode entry point.
//!
//! `BitMatrix` stores one bit per QR module, packed row-major into `u32`
//! words (`ceil(dim/32)` words per row) so it stays cheap to carry around in
//! a per-frame trace (see the plan's trace-compactness constraint: packed
//! `Vec<u32>` rows, not `Vec<bool>`).
//!
//! Decoding itself is delegated to `rqrr`, a pure-Rust QR bit-matrix decoder
//! (Reed-Solomon error correction + ISO 18004 data-stream parsing). We only
//! provide the `rqrr::BitGrid` glue and independent mirrored-orientation
//! detection (rqrr handles mirrored *content* itself but does not report
//! which orientation it used).

/// A packed, square bit matrix: one bit per QR module.
///
/// Coordinates are `(x, y)` = `(column, row)`, matching the `qrcode` crate's
/// `code[(x, y)]` indexing used to build matrices in tests.
pub struct BitMatrix {
    /// Row/column count (QR codes are always square).
    pub dim: usize,
    /// Row-major packed storage: `ceil(dim/32)` `u32` words per row.
    words: Vec<u32>,
}

impl BitMatrix {
    /// Allocate an all-zero (all-light) matrix of size `dim x dim`.
    pub fn new(dim: usize) -> Self {
        let words_per_row = dim.div_ceil(32).max(1);
        BitMatrix {
            dim,
            words: vec![0u32; words_per_row * dim],
        }
    }

    /// Number of `u32` words used to store one row.
    pub fn words_per_row(&self) -> usize {
        self.dim.div_ceil(32).max(1)
    }

    /// Read the module at column `x`, row `y`. `true` = dark.
    pub fn get(&self, x: usize, y: usize) -> bool {
        let word = self.words[y * self.words_per_row() + x / 32];
        (word >> (x % 32)) & 1 != 0
    }

    /// Set the module at column `x`, row `y`. `true` = dark.
    pub fn set(&mut self, x: usize, y: usize, v: bool) {
        let idx = y * self.words_per_row() + x / 32;
        let bit = 1u32 << (x % 32);
        if v {
            self.words[idx] |= bit;
        } else {
            self.words[idx] &= !bit;
        }
    }

    /// The raw packed row-major words, for trace serialization.
    pub fn words(&self) -> &[u32] {
        &self.words
    }
}

// `rqrr::BitGrid::bit` takes `(y, x)` (row, then column) — verified against
// rqrr 0.10.1 source (`src/lib.rs`): `fn bit(&self, y: usize, x: usize) -> bool`.
// We adapt by delegating to our own `(x, y)`-ordered `get`.
impl rqrr::BitGrid for &BitMatrix {
    fn size(&self) -> usize {
        self.dim
    }

    fn bit(&self, y: usize, x: usize) -> bool {
        self.get(x, y)
    }
}

/// A view over a `BitMatrix` with x/y swapped, used by the mirrored-
/// orientation detection in `is_mirrored`.
///
/// rqrr 0.10.1's own `MirroredGrid` has a private inner field (no public
/// constructor), so it cannot be built outside the `rqrr` crate. This is the
/// brief's documented fallback: a local wrapper with identical semantics
/// (swap the two axes before delegating to the direct `BitGrid` read).
struct Transposed<'a>(&'a BitMatrix);

impl rqrr::BitGrid for Transposed<'_> {
    fn size(&self) -> usize {
        self.0.dim
    }

    fn bit(&self, y: usize, x: usize) -> bool {
        self.0.get(y, x)
    }
}

/// A successfully decoded QR payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPayload {
    /// Decoded content, as UTF-8 text (rqrr rejects non-UTF-8 output).
    pub payload: String,
    /// Same content as raw bytes.
    pub payload_bytes: Vec<u8>,
    /// QR version (1..=40).
    pub version: u32,
    /// Error-correction level: one of `'L'`, `'M'`, `'Q'`, `'H'`, or `'?'`
    /// if it could not be determined.
    pub ecc: char,
    /// `true` when the grid's true reading orientation is x/y-swapped —
    /// i.e. the source image was mirrored. Determined independently of
    /// content decoding; see `is_mirrored`.
    pub mirrored: bool,
}

/// Why `decode_bits` failed, mapped from rqrr's `DeQRError` for trace
/// visibility (the underlying enum has finer-grained variants; these four
/// buckets are what's useful to show in a scan trace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeFailure {
    /// Format-info bits (error-correction level + mask) could not be read
    /// or corrected at either of the two redundant locations.
    Format,
    /// The grid size did not correspond to a valid QR version, or (for
    /// v >= 7) the version-info bits could not be read/corrected.
    Version,
    /// Data error-correction failed: too many errors to correct with the
    /// declared error-correction level.
    Ecc,
    /// The (ECC-corrected) data stream itself was malformed: an unknown
    /// data-segment type, a bit-length mismatch, or non-UTF-8 output.
    Content,
}

/// Map rqrr's error type onto our four-bucket `DecodeFailure`.
///
/// Verified against rqrr 0.10.1's `DeQRError` (`src/lib.rs`): the full set of
/// nine variants is `IoError`, `DataUnderflow`, `DataOverflow`,
/// `UnknownDataType`, `DataEcc`, `FormatEcc`, `InvalidVersion`,
/// `InvalidGridSize`, `EncodingError`.
fn map_err(e: rqrr::DeQRError) -> DecodeFailure {
    use rqrr::DeQRError::*;
    match e {
        FormatEcc => DecodeFailure::Format,
        InvalidVersion | InvalidGridSize => DecodeFailure::Version,
        DataEcc => DecodeFailure::Ecc,
        DataUnderflow | DataOverflow | UnknownDataType | EncodingError | IoError => {
            DecodeFailure::Content
        }
    }
}

/// Map rqrr's raw `ecc_level` (the 2-bit QR format-info field) to the
/// conventional letter grade.
///
/// Verified against rqrr 0.10.1's `read_format` (`src/decode.rs`): it stores
/// the format info's top two bits (after BCH(15,5) correction) unmodified as
/// `ecc_level`. Per ISO 18004 Table 25 (format-info EC-level indicators):
/// `01 = L`, `00 = M`, `11 = Q`, `10 = H`. rqrr does not expose the letter
/// grade itself, only this raw 2-bit field, so we do the mapping here.
fn ecc_char(ecc_level: u16) -> char {
    match ecc_level {
        0b01 => 'L',
        0b00 => 'M',
        0b11 => 'Q',
        0b10 => 'H',
        _ => '?', // unreachable: ecc_level is masked to 2 bits by rqrr
    }
}

/// Detect whether the grid is mirrored (must be read with x/y swapped).
///
/// Orientation must NOT be inferred from `Grid::decode` success/failure:
/// rqrr 0.10.1's `decode::decode` internally retries the mirrored reading
/// (`src/decode.rs`: `_decode(code)` falling back to
/// `_decode(&MirroredGrid(code))`), so direct and transposed calls always
/// agree and carry zero orientation signal. Nor is mere format-info
/// *validity* per orientation enough: empirically the transposed 15-bit
/// format read (same cells, scrambled order) frequently corrects to some
/// *other* valid BCH(15,5) codeword — e.g. a true (L, mask 2) reads as a
/// "valid" (Q, mask 6) when transposed.
///
/// Two signals, checked in order:
///
/// 1. **Format metadata match against the RS-verified truth** (primary).
///    `verified` is the `(ecc_level, mask)` from `Grid::decode`'s returned
///    `MetaData` — rqrr reports the metadata of whichever orientation
///    passed the full Reed-Solomon data check (`codestream_ecc`), i.e. the
///    true one; a wrong-orientation full decode would require transposed
///    data codewords to satisfy RS, which garbage data does not.
///    `Grid::get_raw_data` -> `decode::get_raw` -> `read_format` has NO
///    mirror retry (verified in rqrr 0.10.1 source), so it reads each
///    orientation's format info as laid out. The orientation whose format
///    read yields `verified` — and whose counterpart doesn't — is the true
///    one, even when the counterpart's read happens to be BCH-valid noise.
/// 2. **ISO 18004 dark module** (tiebreaker, for when signal 1 ties).
///    Every QR code has one unconditionally dark module at
///    (row = 4*version + 9, col = 8). Its transpose position
///    (row 8, col = 4*version + 9) lands in the content-dependent second
///    format-info copy, so comparing the two cells breaks the tie whenever
///    they differ; the true orientation always has its dark module set.
///
/// If both signals tie the grid is reported as not mirrored — at that
/// point the two orientations are genuinely indistinguishable at the bit
/// level (coincidentally matching format reads AND equal dark cells).
fn is_mirrored(m: &BitMatrix, verified: (u16, u16), version: u32) -> bool {
    fn format_meta<G: rqrr::BitGrid>(grid: G) -> Option<(u16, u16)> {
        rqrr::Grid::new(grid)
            .get_raw_data()
            .ok()
            .map(|(meta, _)| (meta.ecc_level, meta.mask))
    }
    let direct_matches = format_meta(m) == Some(verified);
    let transposed_matches = format_meta(Transposed(m)) == Some(verified);
    match (direct_matches, transposed_matches) {
        (true, false) => false,
        (false, true) => true,
        _ => {
            let dm = 4 * version as usize + 9; // dark module row (== dim - 8)
            let direct_dark = m.get(8, dm); // (row = dm, col = 8)
            let transposed_dark = m.get(dm, 8); // transposed reading's dark module
            !direct_dark && transposed_dark
        }
    }
}

/// Decode a QR payload from a sampled bit matrix.
///
/// Content decoding is a single `rqrr::Grid::decode` call — rqrr itself
/// retries the mirrored reading internally, so a mirrored matrix decodes
/// to the correct payload without any orientation handling on our side.
/// The `mirrored` flag is then determined independently by `is_mirrored`
/// (rqrr does not report which orientation its internal retry used).
pub fn decode_bits(m: &BitMatrix) -> Result<DecodedPayload, DecodeFailure> {
    let (meta, payload) = rqrr::Grid::new(m).decode().map_err(map_err)?;
    let version = meta.version.0 as u32;
    Ok(DecodedPayload {
        payload_bytes: payload.as_bytes().to_vec(),
        payload,
        version,
        ecc: ecc_char(meta.ecc_level),
        mirrored: is_mirrored(m, (meta.ecc_level, meta.mask), version),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qr_matrix(payload: &str, version: i16, ecc: qrcode::EcLevel) -> BitMatrix {
        let code = qrcode::QrCode::with_version(
            payload.as_bytes(), qrcode::Version::Normal(version), ecc).unwrap();
        let dim = code.width();
        let mut m = BitMatrix::new(dim);
        for y in 0..dim {
            for x in 0..dim {
                m.set(x, y, code[(x, y)] == qrcode::Color::Dark);
            }
        }
        m
    }

    #[test]
    fn packing_round_trips() {
        let mut m = BitMatrix::new(41); // v6: crosses a u32 word boundary
        m.set(0, 0, true); m.set(31, 3, true); m.set(32, 3, true); m.set(40, 40, true);
        assert!(m.get(0, 0) && m.get(31, 3) && m.get(32, 3) && m.get(40, 40));
        assert!(!m.get(1, 0) && !m.get(33, 3));
    }

    #[test]
    fn decodes_v1_and_v7_and_v40() {
        for (v, payload) in [(1i16, "Q:test:1"), (7, "HTTPS://R8.HR/O9MKM1ZO3W5"),
                             (40, &"x".repeat(1000) as &str)] {
            let m = qr_matrix(payload, v, qrcode::EcLevel::M);
            let d = decode_bits(&m).unwrap_or_else(|e| panic!("v{v}: {e:?}"));
            assert_eq!(d.payload, payload, "v{v}");
            assert_eq!(d.version as i16, v);
            assert!(!d.mirrored);
        }
    }

    /// Orientation must be detected reliably across versions, ECC levels,
    /// and payloads — not by luck of a single mask choice. 100 combos, each
    /// checked in both orientations (200 mirrored-flag assertions).
    #[test]
    fn mirrored_flag_sweep() {
        let ecc_levels = [
            (qrcode::EcLevel::L, "L"),
            (qrcode::EcLevel::M, "M"),
            (qrcode::EcLevel::Q, "Q"),
            (qrcode::EcLevel::H, "H"),
        ];
        let mut failures = Vec::new();
        for v in 1i16..=5 {
            for (ecc, ecc_name) in ecc_levels {
                for i in 0..5 {
                    // Compact but distinct per combo: v1-H byte capacity is
                    // only 7, so the payload must stay short.
                    let payload = format!("Q{v}{ecc_name}{i}");
                    let m = qr_matrix(&payload, v, ecc);
                    let mut t = BitMatrix::new(m.dim);
                    for y in 0..m.dim {
                        for x in 0..m.dim {
                            t.set(y, x, m.get(x, y));
                        }
                    }
                    let d = decode_bits(&m).unwrap_or_else(|e| panic!("{payload} direct: {e:?}"));
                    let dt =
                        decode_bits(&t).unwrap_or_else(|e| panic!("{payload} transposed: {e:?}"));
                    assert_eq!(d.payload, payload, "{payload} direct payload");
                    assert_eq!(dt.payload, payload, "{payload} transposed payload");
                    if d.mirrored {
                        failures.push(format!("{payload}: direct claims mirrored"));
                    }
                    if !dt.mirrored {
                        failures.push(format!("{payload}: transposed missed mirrored"));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} / 200 orientation assertions failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn garbage_fails_cleanly() {
        let mut m = BitMatrix::new(21);
        for y in 0..21 { for x in 0..21 { m.set(x, y, (x * 31 + y * 17) % 3 == 0); } }
        assert!(decode_bits(&m).is_err()); // no panic
    }
}
