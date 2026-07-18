use crate::consts::{CONTRAST_FLOOR, TILE};
use crate::LumaView;
use qr_lab_imgproc::threshold::{TileThresholdConfig, TileThresholdGrid, TileThresholdMethod};

/// QR scanner compatibility configuration for the reusable tile threshold grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BinarizeSpec {
    /// Additive offset applied to each tile threshold.
    pub threshold_offset: i16,
    /// Minimum (max − min) contrast before a tile is marked skip.
    pub contrast_floor: u8,
    /// Use Sauvola instead of midrange tile thresholds when `true`.
    pub sauvola: bool,
}

impl Default for BinarizeSpec {
    fn default() -> Self {
        Self {
            threshold_offset: 0,
            contrast_floor: CONTRAST_FLOOR,
            sauvola: false,
        }
    }
}

/// QR-facing wrapper preserving the original `TileGrid` API while delegating
/// threshold construction to `qr-lab-imgproc`.
pub struct TileGrid {
    /// Number of tiles along the X axis.
    pub tiles_x: usize,
    /// Number of tiles along the Y axis.
    pub tiles_y: usize,
    inner: TileThresholdGrid,
}

impl TileGrid {
    /// Build a tile grid with the default binarization specification.
    pub fn build(view: &LumaView<'_>) -> Self {
        Self::build_with(view, BinarizeSpec::default())
    }

    /// Build a tile grid with an explicit [`BinarizeSpec`].
    pub fn build_with(view: &LumaView<'_>, spec: BinarizeSpec) -> Self {
        let method = if spec.sauvola {
            TileThresholdMethod::Sauvola {
                k: 0.2,
                dynamic_range: 128.0,
            }
        } else {
            TileThresholdMethod::Midrange
        };
        let inner = TileThresholdGrid::build(
            *view,
            TileThresholdConfig {
                tile_width: TILE,
                tile_height: TILE,
                neighborhood_radius: 1,
                threshold_offset: spec.threshold_offset,
                contrast_floor: spec.contrast_floor,
                method,
            },
        )
        .expect("scanner tile configuration is valid");
        Self {
            tiles_x: inner.tiles_x(),
            tiles_y: inner.tiles_y(),
            inner,
        }
    }

    /// Threshold for the tile containing pixel `(x, y)`.
    #[inline]
    pub fn threshold_at(&self, x: usize, y: usize) -> u8 {
        self.inner.thresholds()[(y / TILE) * self.tiles_x + x / TILE]
    }

    /// Whether the tile containing pixel `(x, y)` is low-contrast.
    #[inline]
    pub fn is_skip(&self, x: usize, y: usize) -> bool {
        self.inner.skip_mask()[(y / TILE) * self.tiles_x + x / TILE]
    }

    /// True when every tile on the row containing `y` is marked skip.
    pub fn row_all_skip(&self, y: usize) -> bool {
        let base = (y / TILE) * self.tiles_x;
        self.inner.skip_mask()[base..base + self.tiles_x]
            .iter()
            .all(|value| *value)
    }

    #[cfg(feature = "debug-trace")]
    pub(crate) fn to_trace(&self) -> crate::trace::TileTrace {
        crate::trace::TileTrace {
            tiles_x: self.tiles_x,
            tiles_y: self.tiles_y,
            thresholds: self.inner.thresholds().to_vec(),
            skip: self.inner.skip_mask().to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, pixel: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        (0..width * height)
            .map(|index| pixel(index % width, index / width))
            .collect()
    }

    #[test]
    fn flat_image_is_all_skip() {
        let data = image(64, 48, |_, _| 128);
        let view = LumaView::new(&data, 64, 48, 64).unwrap();
        let grid = TileGrid::build(&view);
        assert_eq!((grid.tiles_x, grid.tiles_y), (4, 3));
        for y in (0..48).step_by(7) {
            assert!(grid.row_all_skip(y));
            for x in (0..64).step_by(7) {
                assert!(grid.is_skip(x, y));
                assert_eq!(grid.threshold_at(x, y), 128);
            }
        }
    }

    #[test]
    fn contrast_tile_thresholds_midpoint_and_dilates() {
        let data = image(64, 48, |x, _| if x < 32 { 20 } else { 220 });
        let view = LumaView::new(&data, 64, 48, 64).unwrap();
        let grid = TileGrid::build(&view);
        assert!(!grid.is_skip(32, 24));
        assert_eq!(grid.threshold_at(32, 24), 120);
        assert!(!grid.is_skip(16, 24));
        assert_eq!(grid.threshold_at(16, 24), 120);
        assert!(grid.is_skip(48, 24));
        assert!(grid.is_skip(0, 24));
    }

    #[test]
    fn ragged_edges_are_handled() {
        let data = image(70, 30, |x, y| if x >= 64 && y >= 16 { 200 } else { 50 });
        let view = LumaView::new(&data, 70, 30, 70).unwrap();
        let grid = TileGrid::build(&view);
        assert_eq!((grid.tiles_x, grid.tiles_y), (5, 2));
        assert!(!grid.is_skip(69, 29));
        assert_eq!(grid.threshold_at(69, 29), 125);
    }
}
