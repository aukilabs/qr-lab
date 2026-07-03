//! Pinned detection constants. Every value must state a principled
//! derivation (QR geometry or established practice) — never a value tuned
//! to make a fixture pass (see Plan 2 "No overfitting" constraint).

/// Tile edge for local min/max thresholding (AprilTag's tile-extrema
/// scheme, scaled to full working resolution). Thresholds are drawn from
/// the 3×3-dilated neighborhood, i.e. an effective 48px window: the
/// smallest decodable finder (7 modules × ≥2px = 14px ≈ one tile) always
/// contributes both ink and background extrema to every tile it overlaps,
/// while tiles stay small enough to track illumination gradients.
pub const TILE: usize = 16;

/// Minimum tile-neighborhood contrast (max−min) to consider content.
/// A usable module edge needs ink/background separation well above sensor
/// noise: typical 8-bit luma sensor read noise is a few gray levels
/// (σ≈2 in our fixture model and commonly cited for phone camera sensors)
/// ⇒ 6σ ≈ 12 gray levels; below this a tile cannot contain a decodable
/// transition. The σ≈2 figure is drawn from the fixture generator, not a
/// measurement of real camera hardware — to be validated against real
/// captures in Plan 5.
pub const CONTRAST_FLOOR: u8 = 12;

/// Scan every 2nd row. The 1:1:3:1:1 cross-section only appears where a
/// horizontal scanline crosses the finder's 3-module-tall dark core band
/// (the inner square) — not anywhere across the full 7-module height, a
/// scanline outside that band still crosses ink/space but not in that
/// proportion. At the ≥2px/module decodability floor that band is ≥6px
/// tall (6 consecutive rows), and any 6 consecutive rows contain exactly
/// 3 even-indexed and 3 odd-indexed rows, so step 2 guarantees ≥3 scan
/// rows land inside the band regardless of its vertical offset. The
/// binding constraint is downstream: `find_finders` keeps only
/// candidates with `hits >= 2` (the merge gate), so ≥3 hit chances
/// leaves one spare above that floor rather than sitting exactly on it.
pub const ROW_STEP: usize = 2;
