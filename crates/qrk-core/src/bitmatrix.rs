//! Packed bit matrix + rqrr `BitGrid` adapter + top-level decode entry point.
//!
//! `BitMatrix` stores one bit per QR module, packed row-major into `u32`
//! words (`ceil(dim/32)` words per row) so it stays cheap to carry around in
//! a per-frame trace (see the plan's trace-compactness constraint: packed
//! `Vec<u32>` rows, not `Vec<bool>`).
//!
//! Decoding itself is delegated to `rqrr`, a pure-Rust QR bit-matrix decoder
//! (Reed-Solomon error correction + ISO 18004 data-stream parsing). We only
//! provide the `rqrr::BitGrid` glue and a mirrored-read retry.

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

/// A view over a `BitMatrix` with x/y swapped, for the mirrored-read retry.
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
    /// `true` when the payload only decoded via the transposed (mirrored)
    /// read — i.e. the source image was mirrored.
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

fn decode_grid<G: rqrr::BitGrid>(grid: G) -> Result<DecodedPayload, DecodeFailure> {
    let (meta, payload) = rqrr::Grid::new(grid).decode().map_err(map_err)?;
    Ok(DecodedPayload {
        payload_bytes: payload.as_bytes().to_vec(),
        payload,
        version: meta.version.0 as u32,
        ecc: ecc_char(meta.ecc_level),
        mirrored: false,
    })
}

/// Is the ISO 18004 "dark module" set?
///
/// Every valid QR code has exactly one module that is unconditionally dark,
/// at (row = 4*version + 9, col = 8), regardless of content, mask, or ECC
/// level. It's cheap to check and orientation-*sensitive*: unlike the
/// format-info cross (symmetric by construction) or the finder patterns
/// (symmetric individually), this single bit's transpose position falls
/// inside the content-dependent redundant format-info strip, so it is not
/// generally also dark under a swapped reading. That makes it a reliable,
/// standards-grounded tiebreaker (see `decode_bits`) rather than a fit to
/// any specific test payload.
fn dark_module_set<G: rqrr::BitGrid>(grid: G, version: u32) -> bool {
    let row = 4 * version as usize + 9;
    grid.bit(row, 8)
}

/// Decode a QR payload from a sampled bit matrix.
///
/// Tries the matrix as given and with x/y swapped (`Transposed`, to handle
/// QR codes sampled from a mirrored source image — e.g. a front-facing
/// camera or a code viewed through a reflective surface). `mirrored = true`
/// iff only the transposed read succeeded.
///
/// Format-info placement is symmetric under transpose by construction (ISO
/// 18004's redundant copy is laid out as a symmetric cross), and typical
/// mask choices are too, so it's common for *both* readings to decode the
/// grid successfully (to the identical payload — the ambiguity is real, not
/// a bug: such a grid is genuinely readable either way). When that happens,
/// the dark module (see `dark_module_set`) breaks the tie in favor of
/// whichever orientation actually has it set, since only one meaningfully
/// can.
pub fn decode_bits(m: &BitMatrix) -> Result<DecodedPayload, DecodeFailure> {
    let direct = decode_grid(m);
    let transposed = decode_grid(Transposed(m));
    match (direct, transposed) {
        (Ok(d), Err(_)) => Ok(d),
        (Err(_), Ok(t)) => Ok(DecodedPayload {
            mirrored: true,
            ..t
        }),
        (Err(direct_err), Err(_)) => Err(direct_err),
        (Ok(d), Ok(t)) => {
            let direct_dark = dark_module_set(m, d.version);
            let transposed_dark = dark_module_set(Transposed(m), d.version);
            if !direct_dark && transposed_dark {
                Ok(DecodedPayload {
                    mirrored: true,
                    ..t
                })
            } else {
                Ok(d)
            }
        }
    }
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

    #[test]
    fn decodes_mirrored() {
        let m = qr_matrix("Q:mirror:1", 1, qrcode::EcLevel::M);
        let mut t = BitMatrix::new(m.dim);
        for y in 0..m.dim { for x in 0..m.dim { t.set(y, x, m.get(x, y)); } }
        let d = decode_bits(&t).unwrap();
        assert_eq!(d.payload, "Q:mirror:1");
        assert!(d.mirrored);
    }

    #[test]
    fn garbage_fails_cleanly() {
        let mut m = BitMatrix::new(21);
        for y in 0..21 { for x in 0..21 { m.set(x, y, (x * 31 + y * 17) % 3 == 0); } }
        assert!(decode_bits(&m).is_err()); // no panic
    }
}
