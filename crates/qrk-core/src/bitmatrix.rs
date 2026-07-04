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

/// The 7x7 finder pattern's spec-fixed polarity at relative in-block module
/// offset `(r, c)` (0-indexed, ISO 18004 §6.3.3): the outer 1-module dark
/// border ring, a 1-module light ring inside it, and the 3x3 dark center —
///
/// ```text
/// 1111111
/// 1000001
/// 1011101
/// 1011101
/// 1011101
/// 1000001
/// 1111111
/// ```
///
/// payload-independent by construction, so every one of these cells is
/// ground truth regardless of what the code encodes — the same structure
/// `finder.rs`'s detector matches against (its 1:1:3:1:1 cross-section
/// ratio), transcribed here as a per-cell dark/light mask instead.
fn finder_module_is_dark(r: usize, c: usize) -> bool {
    r == 0 || r == 6 || c == 0 || c == 6 || ((2..=4).contains(&r) && (2..=4).contains(&c))
}

/// Module-space `(x0, y0)` top-left origin of each of the three 7x7 finder
/// blocks (TL, TR, BL) for a `dim`-sized grid. Valid for every QR dimension
/// (`dim >= 21`): `dim - 7 >= 14 > 7`, so the TL block (`0..7`) and the
/// TR/BL blocks (`dim-7..dim`) never overlap.
fn finder_block_origins(dim: usize) -> [(usize, usize); 3] {
    [(0, 0), (dim - 7, 0), (0, dim - 7)]
}

/// Plan 4B Fix B's per-code reference threshold: the midpoint between the
/// mean RAW gray of the three finder blocks' known-dark modules and the
/// mean of their known-light modules (provenance: ISO 18004 §6.3.3's
/// finder pattern is spec-fixed and payload-independent, so these module
/// positions are ground truth no matter what the code encodes; precedent:
/// the predecessor GPU scanner's `gray1`/`gray2` referencing). Unlike the
/// tile-threshold path (a local min/max per `consts::TILE`-px tile), this
/// is a single scalar for the whole candidate, immune to a tile whose
/// *local* extrema — bright floor, dark shelving, real-video scene
/// content dilating into the tile — happen to fall outside the code's own
/// actual ink/paper gray levels.
///
/// `grays` is `dim*dim` row-major (`y*dim+x`), `f32::NAN` marking an
/// out-of-image sample (excluded from both means). `None` when no dark or
/// no light sample was available anywhere in any finder block — should not
/// happen for a candidate that reached sampling at all (a finder pattern is
/// what got it here), but defensive rather than dividing by zero.
fn finder_reference_threshold(grays: &[f32], dim: usize) -> Option<f32> {
    let (mut dark_sum, mut dark_n) = (0.0f64, 0u32);
    let (mut light_sum, mut light_n) = (0.0f64, 0u32);
    for (bx, by) in finder_block_origins(dim) {
        for r in 0..7usize {
            for c in 0..7usize {
                let v = grays[(by + r) * dim + (bx + c)];
                if v.is_nan() {
                    continue;
                }
                if finder_module_is_dark(r, c) {
                    dark_sum += v as f64;
                    dark_n += 1;
                } else {
                    light_sum += v as f64;
                    light_n += 1;
                }
            }
        }
    }
    if dark_n == 0 || light_n == 0 {
        return None;
    }
    Some(((dark_sum / dark_n as f64 + light_sum / light_n as f64) / 2.0) as f32)
}

/// AprilTag-style `decode_sharpening` (provenance: AprilTag's
/// `quad_decode.c` default unsharp-mask pass — see `consts::SHARPEN_K`)
/// applied to a raw per-module gray grid: for an interior cell (all four
/// neighbors available), `v' = v + K*(4v - N - S - E - W)/4`; at a grid
/// edge or where a neighbor's own sample was out-of-image (`NAN`), the
/// missing neighbor(s) are simply excluded — `v' = v + K*(v - mean(the
/// neighbors that ARE available))`, which is exactly the interior formula
/// when all four exist, never a neighbor treated as zero. A `NAN` input
/// cell (out-of-image itself) stays `NAN` (nothing to sharpen).
fn decode_sharpen(grays: &[f32], dim: usize, k: f32) -> Vec<f32> {
    let at = |x: isize, y: isize| -> Option<f32> {
        if x < 0 || y < 0 || x as usize >= dim || y as usize >= dim {
            return None;
        }
        let v = grays[y as usize * dim + x as usize];
        (!v.is_nan()).then_some(v)
    };
    let mut out = vec![f32::NAN; dim * dim];
    for y in 0..dim {
        for x in 0..dim {
            let v = grays[y * dim + x];
            if v.is_nan() {
                continue;
            }
            let (xi, yi) = (x as isize, y as isize);
            let neighbors: Vec<f32> =
                [at(xi, yi - 1), at(xi, yi + 1), at(xi - 1, yi), at(xi + 1, yi)]
                    .into_iter()
                    .flatten()
                    .collect();
            out[y * dim + x] = if neighbors.is_empty() {
                v
            } else {
                let mean = neighbors.iter().sum::<f32>() / neighbors.len() as f32;
                v + k * (v - mean)
            };
        }
    }
    out
}

/// Plan 4B Fix B: build an alternative bit matrix from a candidate's raw
/// per-module gray grid (`sample.rs`'s `SampledGrid::grays`) when the
/// tile-threshold `bits` failed to decode — real-video-capture robustness
/// against scattered bit errors from (1) scene-driven tile-threshold drift
/// and (2) inter-module blur crosstalk (see this fix's investigation
/// report). Order matters and was the one the controller's probe verified:
/// [`decode_sharpen`] runs first on the RAW grays (sharpening a boolean
/// doesn't mean anything, so this is the only sensible order, and it is the
/// order actually measured), THEN [`finder_reference_threshold`]'s scalar
/// (itself computed from the RAW, unsharpened grays — the finder pattern's
/// spec-fixed cells are the reference, not something to sharpen first) is
/// applied to the sharpened values to binarize.
///
/// `None` only when the reference threshold itself is undefined (see
/// [`finder_reference_threshold`]) — sampling that produced no usable
/// finder-block evidence at all.
pub(crate) fn build_reference_threshold_bits(
    grays: &[f32],
    dim: usize,
    inverted: bool,
) -> Option<BitMatrix> {
    let threshold = finder_reference_threshold(grays, dim)?;
    let sharpened = decode_sharpen(grays, dim, crate::consts::SHARPEN_K);
    let mut bits = BitMatrix::new(dim);
    for y in 0..dim {
        for x in 0..dim {
            let v = sharpened[y * dim + x];
            let dark = !v.is_nan() && (v < threshold) != inverted;
            bits.set(x, y, dark);
        }
    }
    Some(bits)
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

    // --- Plan 4B Fix B: reference-threshold + sharpening decode round ---

    /// `finder_reference_threshold` must average each polarity's samples
    /// (not just pick one, and not assume an even split between the two
    /// values used below — a 7x7 finder pattern has 33 dark and 16 light
    /// cells, an odd/even split that can't alternate two values 50/50), then
    /// return the exact midpoint of the two TRUE means: every finder-block
    /// known-dark module alternates {28, 32}, every known-light module
    /// alternates {195, 205} — this test computes the actual resulting means
    /// from the same assignment (not a hand-assumed round number), so it
    /// stays correct regardless of exactly how the alternation falls out.
    #[test]
    fn finder_reference_threshold_is_midpoint_of_known_dark_and_light_means() {
        let dim = 21usize;
        let mut grays = vec![f32::NAN; dim * dim];
        let (mut dark_sum, mut dark_n) = (0.0f64, 0u32);
        let (mut light_sum, mut light_n) = (0.0f64, 0u32);
        let mut toggle = false;
        for (bx, by) in finder_block_origins(dim) {
            for r in 0..7usize {
                for c in 0..7usize {
                    let dark = finder_module_is_dark(r, c);
                    let v: f32 = if dark {
                        if toggle { 32.0 } else { 28.0 }
                    } else if toggle {
                        205.0
                    } else {
                        195.0
                    };
                    toggle = !toggle;
                    grays[(by + r) * dim + (bx + c)] = v;
                    if dark {
                        dark_sum += v as f64;
                        dark_n += 1;
                    } else {
                        light_sum += v as f64;
                        light_n += 1;
                    }
                }
            }
        }
        let expected = ((dark_sum / dark_n as f64 + light_sum / light_n as f64) / 2.0) as f32;
        // Sanity-check the fixture itself actually exercises two well-
        // separated polarities (not a degenerate all-equal grid).
        assert!(expected > 50.0 && expected < 250.0, "fixture sanity: {expected}");

        let threshold = finder_reference_threshold(&grays, dim)
            .expect("all three finder blocks fully populated: threshold must be defined");
        assert!(
            (threshold - expected).abs() < 1e-4,
            "expected midpoint of the true dark/light means ({expected}), got {threshold}"
        );
    }

    /// `None` when a polarity has no sample at all anywhere (defensive path
    /// — should not occur for a real candidate that reached sampling, since
    /// finder patterns are what got it there).
    #[test]
    fn finder_reference_threshold_none_when_a_polarity_is_entirely_missing() {
        let dim = 21usize;
        let grays = vec![f32::NAN; dim * dim]; // nothing sampled at all
        assert!(finder_reference_threshold(&grays, dim).is_none());
    }

    /// Hand-computed 3x3 grid, `K = 0.25`:
    /// ```text
    /// 10 20 10
    /// 20 100 20
    /// 10 20 10
    /// ```
    /// Center (1,1)=100 has all 4 neighbors, each 20: mean=20,
    /// v' = 100 + 0.25*(100-20) = 120.
    /// Top-mid edge (1,0)=20 has 3 neighbors (S=100, E=10, W=10, no N):
    /// mean=(100+10+10)/3=40, v' = 20 + 0.25*(20-40) = 15.
    /// Corner (0,0)=10 has 2 neighbors (S=20, E=20, no N/W):
    /// mean=20, v' = 10 + 0.25*(10-20) = 7.5.
    #[test]
    fn decode_sharpen_matches_hand_computed_3x3_including_edges() {
        let dim = 3usize;
        #[rustfmt::skip]
        let grays: Vec<f32> = vec![
            10.0, 20.0, 10.0,
            20.0, 100.0, 20.0,
            10.0, 20.0, 10.0,
        ];
        let sharpened = decode_sharpen(&grays, dim, 0.25);
        let at = |x: usize, y: usize| sharpened[y * dim + x];
        assert!((at(1, 1) - 120.0).abs() < 1e-4, "center: got {}", at(1, 1));
        assert!((at(1, 0) - 15.0).abs() < 1e-4, "top-mid edge: got {}", at(1, 0));
        assert!((at(0, 0) - 7.5).abs() < 1e-4, "corner: got {}", at(0, 0));
    }

    /// A `NAN` (out-of-image) input cell must stay `NAN` — nothing to
    /// sharpen — and must not be pulled into a neighboring cell's mean as
    /// if it were a real zero-ish sample.
    #[test]
    fn decode_sharpen_leaves_nan_cells_untouched_and_excludes_them_from_neighbors() {
        let dim = 3usize;
        #[rustfmt::skip]
        let grays: Vec<f32> = vec![
            f32::NAN, 20.0, 10.0,
            20.0, 100.0, 20.0,
            10.0, 20.0, 10.0,
        ];
        let sharpened = decode_sharpen(&grays, dim, 0.25);
        let at = |x: usize, y: usize| sharpened[y * dim + x];
        assert!(at(0, 0).is_nan(), "OOB input cell must stay NAN");
        // (1,0)'s neighbors are N=NAN(excluded), S=100, E=10, no W(edge):
        // mean=(100+10)/2=55, v' = 20 + 0.25*(20-55) = 11.25.
        assert!((at(1, 0) - 11.25).abs() < 1e-4, "got {}", at(1, 0));
    }

    /// Full round-trip smoke test: a v2 code's own true dark/light module
    /// grays (30.0 / 220.0, well-separated, no blur) fed through
    /// `build_reference_threshold_bits` must reproduce the exact same bit
    /// matrix `qrcode` rendered (sharpening a clean, well-separated grid is
    /// a no-op at every interior/edge cell — see `decode_sharpen`'s own
    /// tests for why it never flips a cell's polarity) and decode.
    #[test]
    fn build_reference_threshold_bits_recovers_a_clean_grid_from_true_grays() {
        let code = qrcode::QrCode::with_version(
            b"REFBITS", qrcode::Version::Normal(2), qrcode::EcLevel::H,
        )
        .unwrap();
        let dim = code.width();
        // "True" grays: 30.0 for dark modules, 220.0 for light — a clean,
        // well-separated synthetic grid with no scene-driven threshold
        // drift and no blur, so the reference-threshold path alone (no
        // sharpening needed) must recover it exactly.
        let mut grays = vec![0.0f32; dim * dim];
        for y in 0..dim {
            for x in 0..dim {
                grays[y * dim + x] =
                    if code[(x, y)] == qrcode::Color::Dark { 30.0 } else { 220.0 };
            }
        }
        let refbits = build_reference_threshold_bits(&grays, dim, false)
            .expect("finder blocks are well-populated dark/light: threshold must be defined");
        for y in 0..dim {
            for x in 0..dim {
                assert_eq!(
                    refbits.get(x, y),
                    code[(x, y)] == qrcode::Color::Dark,
                    "mismatch at ({x},{y})"
                );
            }
        }
        let decoded = decode_bits(&refbits).expect("clean re-thresholded grid must decode");
        assert_eq!(decoded.payload, "REFBITS");
    }

    /// Inverted-polarity coverage for `build_reference_threshold_bits`
    /// (review finding: the `inv_*` fixtures never trigger the refbits
    /// retry, so the `inverted = true` branch had zero coverage): an
    /// INVERTED (light-on-dark) code's ink modules read HIGH grays and its
    /// background LOW. The finder-derived reference threshold still lands
    /// between the two polarity means regardless of which side is ink
    /// (the midpoint formula is symmetric), and the `(v < threshold) !=
    /// inverted` binarization must then map high-gray ink cells back to
    /// `true` (ink) exactly as the tile-threshold path does. Plus a
    /// false-polarity control: the SAME gray grid with `inverted = false`
    /// must come out fully complemented (every ink cell reads light and
    /// every background cell reads dark) — pinning that the flag, not some
    /// accident of the threshold, is what carries the polarity.
    #[test]
    fn build_reference_threshold_bits_handles_inverted_polarity() {
        let code = qrcode::QrCode::with_version(
            b"INVREF", qrcode::Version::Normal(2), qrcode::EcLevel::H,
        )
        .unwrap();
        let dim = code.width();
        // Inverted render: spec-ink (Dark) modules are painted LIGHT
        // (220.0) on a dark (30.0) background.
        let mut grays = vec![0.0f32; dim * dim];
        for y in 0..dim {
            for x in 0..dim {
                grays[y * dim + x] =
                    if code[(x, y)] == qrcode::Color::Dark { 220.0 } else { 30.0 };
            }
        }

        // Correct polarity flag: bit-exact recovery + decode.
        let refbits = build_reference_threshold_bits(&grays, dim, true)
            .expect("finder blocks fully populated: threshold must be defined");
        for y in 0..dim {
            for x in 0..dim {
                assert_eq!(
                    refbits.get(x, y),
                    code[(x, y)] == qrcode::Color::Dark,
                    "inverted=true mismatch at ({x},{y})"
                );
            }
        }
        let decoded = decode_bits(&refbits).expect("inverted re-thresholded grid must decode");
        assert_eq!(decoded.payload, "INVREF");

        // False-polarity control: same grays, inverted=false — every module
        // must come out complemented (ink cells light, background dark),
        // proving the polarity flows through the flag and nothing else.
        let wrong = build_reference_threshold_bits(&grays, dim, false)
            .expect("threshold is polarity-independent: still defined");
        for y in 0..dim {
            for x in 0..dim {
                assert_eq!(
                    wrong.get(x, y),
                    code[(x, y)] != qrcode::Color::Dark,
                    "inverted=false control not complemented at ({x},{y})"
                );
            }
        }
    }
}
