//! Pinned detection constants. Every value must state a principled
//! derivation (QR geometry or established practice) — never a value tuned
//! to make a fixture pass (see Plan 2 "No overfitting" constraint).
#![allow(dead_code)] // consumed from Task 3 (tiles.rs) onward; remove then.

/// Tile edge for local min/max thresholding (AprilTag's tile-extrema
/// scheme, scaled to full working resolution). Thresholds are drawn from
/// the 3×3-dilated neighborhood, i.e. an effective 48px window: the
/// smallest decodable finder (7 modules × ≥2px = 14px ≈ one tile) always
/// contributes both ink and background extrema to every tile it overlaps,
/// while tiles stay small enough to track illumination gradients.
pub const TILE: usize = 16;

/// Minimum tile-neighborhood contrast (max−min) to consider content.
/// A usable module edge needs ink/background separation well above sensor
/// noise (σ≈2 ⇒ 6σ ≈ 12 gray levels); below this a tile cannot contain a
/// decodable transition.
pub const CONTRAST_FLOOR: u8 = 12;

/// Scan every 2nd row: a finder is 7 modules tall and modules must be
/// ≥2px to be decodable, so the smallest real finder spans ≥14 rows —
/// step 2 guarantees ≥7 chances to hit its 1:1:3:1:1 cross-section.
pub const ROW_STEP: usize = 2;
