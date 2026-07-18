use crate::{ImgProcError, ImgProcResult};
use qr_lab_image::{Gray8Image, Gray8View, Gray8ViewMut};

/// Threshold calculation for [`TileThresholdGrid`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TileThresholdMethod {
    /// Threshold at the midrange of neighborhood min/max.
    Midrange,
    /// Local Sauvola threshold using neighborhood mean and stddev.
    Sauvola {
        /// Sauvola `k` parameter.
        k: f64,
        /// Dynamic range used to normalize the local stddev.
        dynamic_range: f64,
    },
}

/// Configuration for a tile-local adaptive threshold surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileThresholdConfig {
    /// Tile width in pixels.
    pub tile_width: usize,
    /// Tile height in pixels.
    pub tile_height: usize,
    /// Neighborhood radius in tiles for min/max or Sauvola aggregation.
    pub neighborhood_radius: usize,
    /// Additive offset applied after the base threshold is computed.
    pub threshold_offset: i16,
    /// Minimum (max − min) contrast; lower-contrast tiles are marked skip.
    pub contrast_floor: u8,
    /// Threshold estimator.
    pub method: TileThresholdMethod,
}

impl Default for TileThresholdConfig {
    fn default() -> Self {
        Self {
            tile_width: 16,
            tile_height: 16,
            neighborhood_radius: 1,
            threshold_offset: 0,
            contrast_floor: 12,
            method: TileThresholdMethod::Midrange,
        }
    }
}

/// Per-tile threshold and low-contrast mask usable by arbitrary detectors.
#[derive(Clone, Debug)]
pub struct TileThresholdGrid {
    tiles_x: usize,
    tiles_y: usize,
    tile_width: usize,
    tile_height: usize,
    thresholds: Vec<u8>,
    skip: Vec<bool>,
}

impl TileThresholdGrid {
    /// Build a tile threshold grid for `src`.
    pub fn build(src: Gray8View<'_>, config: TileThresholdConfig) -> ImgProcResult<Self> {
        validate_tile_config(config)?;
        let tiles_x = src.width().div_ceil(config.tile_width);
        let tiles_y = src.height().div_ceil(config.tile_height);
        let tile_count = tiles_x
            .checked_mul(tiles_y)
            .ok_or(ImgProcError::InvalidConfiguration)?;
        let mut minima = vec![255u8; tile_count];
        let mut maxima = vec![0u8; tile_count];
        let use_sauvola = matches!(config.method, TileThresholdMethod::Sauvola { .. });
        let mut sums = use_sauvola.then(|| vec![0u64; tile_count]);
        let mut sums_sq = use_sauvola.then(|| vec![0u64; tile_count]);
        let mut counts = use_sauvola.then(|| vec![0u64; tile_count]);

        for tile_y in 0..tiles_y {
            let y0 = tile_y * config.tile_height;
            let y1 = (y0 + config.tile_height).min(src.height());
            for y in y0..y1 {
                let row = src.row(y);
                for tile_x in 0..tiles_x {
                    let x0 = tile_x * config.tile_width;
                    let x1 = (x0 + config.tile_width).min(src.width());
                    let pixels = &row[x0..x1];
                    let index = tile_y * tiles_x + tile_x;
                    #[cfg(target_arch = "aarch64")]
                    if pixels.len() == 16 {
                        let block: &[u8; 16] = pixels.try_into().expect("16-byte tile row");
                        let (minimum, maximum) = crate::neon::min_max_u8x16(block);
                        minima[index] = minima[index].min(minimum);
                        maxima[index] = maxima[index].max(maximum);
                        if let (Some(sums), Some(sums_sq), Some(counts)) =
                            (&mut sums, &mut sums_sq, &mut counts)
                        {
                            sums[index] += crate::neon::sum_u8x16(block) as u64;
                            sums_sq[index] += crate::neon::sumsq_u8x16(block);
                            counts[index] += 16;
                        }
                        continue;
                    }
                    if use_sauvola {
                        let sums = sums.as_mut().expect("Sauvola sums");
                        let sums_sq = sums_sq.as_mut().expect("Sauvola squared sums");
                        let counts = counts.as_mut().expect("Sauvola counts");
                        for &pixel in pixels {
                            minima[index] = minima[index].min(pixel);
                            maxima[index] = maxima[index].max(pixel);
                            sums[index] += pixel as u64;
                            sums_sq[index] += pixel as u64 * pixel as u64;
                            counts[index] += 1;
                        }
                    } else {
                        for &pixel in pixels {
                            minima[index] = minima[index].min(pixel);
                            maxima[index] = maxima[index].max(pixel);
                        }
                    }
                }
            }
        }

        let mut thresholds = vec![0u8; tile_count];
        let mut skip = vec![false; tile_count];
        for tile_y in 0..tiles_y {
            for tile_x in 0..tiles_x {
                let mut minimum = 255u8;
                let mut maximum = 0u8;
                let (mut sum, mut sum_sq, mut count) = (0u64, 0u64, 0u64);
                let min_y = tile_y.saturating_sub(config.neighborhood_radius);
                let max_y = tile_y
                    .saturating_add(config.neighborhood_radius)
                    .min(tiles_y - 1);
                let min_x = tile_x.saturating_sub(config.neighborhood_radius);
                let max_x = tile_x
                    .saturating_add(config.neighborhood_radius)
                    .min(tiles_x - 1);
                for neighbor_y in min_y..=max_y {
                    for neighbor_x in min_x..=max_x {
                        let index = neighbor_y * tiles_x + neighbor_x;
                        minimum = minimum.min(minima[index]);
                        maximum = maximum.max(maxima[index]);
                        if let (Some(sums), Some(sums_sq), Some(counts)) =
                            (&sums, &sums_sq, &counts)
                        {
                            sum += sums[index];
                            sum_sq += sums_sq[index];
                            count += counts[index];
                        }
                    }
                }
                let base = match config.method {
                    TileThresholdMethod::Midrange => ((minimum as u16 + maximum as u16) / 2) as i16,
                    TileThresholdMethod::Sauvola { k, dynamic_range } => {
                        let mean = sum as f64 / count as f64;
                        let variance = sum_sq as f64 / count as f64 - mean * mean;
                        (mean * (1.0 + k * (variance.max(0.0).sqrt() / dynamic_range - 1.0)))
                            .round()
                            .clamp(0.0, 255.0) as i16
                    }
                };
                let index = tile_y * tiles_x + tile_x;
                thresholds[index] = (base + config.threshold_offset).clamp(0, 255) as u8;
                skip[index] = maximum - minimum < config.contrast_floor;
            }
        }

        Ok(Self {
            tiles_x,
            tiles_y,
            tile_width: config.tile_width,
            tile_height: config.tile_height,
            thresholds,
            skip,
        })
    }

    /// Number of tiles along the X axis.
    pub const fn tiles_x(&self) -> usize {
        self.tiles_x
    }

    /// Number of tiles along the Y axis.
    pub const fn tiles_y(&self) -> usize {
        self.tiles_y
    }

    /// Per-tile thresholds in row-major tile order.
    #[inline]
    pub fn thresholds(&self) -> &[u8] {
        &self.thresholds
    }

    /// Per-tile low-contrast skip flags in row-major tile order.
    #[inline]
    pub fn skip_mask(&self) -> &[bool] {
        &self.skip
    }

    /// Threshold for the tile containing pixel `(x, y)`.
    #[inline]
    pub fn threshold_at(&self, x: usize, y: usize) -> u8 {
        self.thresholds[(y / self.tile_height) * self.tiles_x + x / self.tile_width]
    }

    /// Whether the tile containing pixel `(x, y)` is low-contrast.
    #[inline]
    pub fn is_skip(&self, x: usize, y: usize) -> bool {
        self.skip[(y / self.tile_height) * self.tiles_x + x / self.tile_width]
    }

    /// True when every tile on the row containing `y` is marked skip.
    #[inline]
    pub fn row_all_skip(&self, y: usize) -> bool {
        let base = (y / self.tile_height) * self.tiles_x;
        self.skip[base..base + self.tiles_x]
            .iter()
            .all(|value| *value)
    }
}

fn validate_tile_config(config: TileThresholdConfig) -> ImgProcResult<()> {
    if config.tile_width == 0 || config.tile_height == 0 {
        return Err(ImgProcError::InvalidConfiguration);
    }
    if let TileThresholdMethod::Sauvola { k, dynamic_range } = config.method {
        if !k.is_finite() || !dynamic_range.is_finite() || dynamic_range <= 0.0 {
            return Err(ImgProcError::InvalidConfiguration);
        }
    }
    Ok(())
}

/// Configuration for whole-image Sauvola binarization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SauvolaConfig {
    /// Half-window radius in pixels.
    pub radius: usize,
    /// Sauvola `k` scaled by 1000 (e.g. 200 → 0.2).
    pub k_milli: u32,
    /// Dynamic range used to normalize the local stddev.
    pub dynamic_range: u32,
    /// Output value for pixels classified as dark.
    pub dark_value: u8,
    /// Output value for pixels classified as light.
    pub light_value: u8,
}

impl Default for SauvolaConfig {
    fn default() -> Self {
        Self {
            radius: 8,
            k_milli: 200,
            dynamic_range: 128,
            dark_value: 0,
            light_value: 255,
        }
    }
}

/// Reusable integral-image scratch storage for Sauvola.
#[derive(Debug, Default)]
pub struct ThresholdWorkspace {
    sum: Vec<u64>,
    sum_sq: Vec<u64>,
}

/// Binarize `src` with Sauvola into a newly allocated image.
pub fn sauvola(src: Gray8View<'_>, config: &SauvolaConfig) -> ImgProcResult<Gray8Image> {
    let mut output = Gray8Image::new(src.size())?;
    let mut workspace = ThresholdWorkspace::default();
    sauvola_into(src, output.view_mut(), config, &mut workspace)?;
    Ok(output)
}

/// Binarize `src` with Sauvola into caller-owned storage.
pub fn sauvola_into(
    src: Gray8View<'_>,
    mut dst: Gray8ViewMut<'_>,
    config: &SauvolaConfig,
    workspace: &mut ThresholdWorkspace,
) -> ImgProcResult<()> {
    if src.size() != dst.size() {
        return Err(ImgProcError::DimensionMismatch);
    }
    if config.radius == 0 || config.k_milli > 1_000 || config.dynamic_range == 0 {
        return Err(ImgProcError::InvalidConfiguration);
    }
    let integral_width = src.width() + 1;
    let integral_height = src.height() + 1;
    let integral_len = integral_width
        .checked_mul(integral_height)
        .ok_or(ImgProcError::InvalidConfiguration)?;
    workspace.sum.clear();
    workspace.sum.resize(integral_len, 0);
    workspace.sum_sq.clear();
    workspace.sum_sq.resize(integral_len, 0);
    for y in 0..src.height() {
        let mut row_sum = 0u64;
        let mut row_sum_sq = 0u64;
        for x in 0..src.width() {
            let value = src.get(x, y) as u64;
            row_sum += value;
            row_sum_sq += value * value;
            let index = (y + 1) * integral_width + x + 1;
            workspace.sum[index] = workspace.sum[index - integral_width] + row_sum;
            workspace.sum_sq[index] = workspace.sum_sq[index - integral_width] + row_sum_sq;
        }
    }
    for y in 0..src.height() {
        let y0 = y.saturating_sub(config.radius);
        let y1 = (y + config.radius + 1).min(src.height());
        for x in 0..src.width() {
            let x0 = x.saturating_sub(config.radius);
            let x1 = (x + config.radius + 1).min(src.width());
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            let sum = rect_sum(&workspace.sum, integral_width, x0, y0, x1, y1);
            let sum_sq = rect_sum(&workspace.sum_sq, integral_width, x0, y0, x1, y1);
            let mean = sum / count;
            let variance = (sum_sq / count).saturating_sub(mean * mean);
            let stddev = integer_sqrt(variance);
            let scale = 1_000u64 - config.k_milli as u64
                + config.k_milli as u64 * stddev / config.dynamic_range as u64;
            let threshold = mean * scale / 1_000;
            dst.set(
                x,
                y,
                if src.get(x, y) as u64 <= threshold {
                    config.dark_value
                } else {
                    config.light_value
                },
            );
        }
    }
    Ok(())
}

fn rect_sum(integral: &[u64], stride: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> u64 {
    integral[y1 * stride + x1] + integral[y0 * stride + x0]
        - integral[y0 * stride + x1]
        - integral[y1 * stride + x0]
}

fn integer_sqrt(value: u64) -> u64 {
    if value < 2 {
        return value;
    }
    let mut x = value;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_image_thresholds_deterministically() {
        let data = vec![100; 25];
        let src = Gray8View::new(&data, 5, 5, 5).unwrap();
        let output = sauvola(src, &SauvolaConfig::default()).unwrap();
        assert!(output.as_slice().iter().all(|&value| value == 255));
    }

    #[test]
    fn strided_input_matches_tight_input() {
        let tight = [20, 20, 220, 20, 20, 220, 20, 20, 220];
        let mut padded = vec![0; 15];
        for y in 0..3 {
            padded[y * 5..y * 5 + 3].copy_from_slice(&tight[y * 3..y * 3 + 3]);
        }
        let a = sauvola(
            Gray8View::new(&tight, 3, 3, 3).unwrap(),
            &SauvolaConfig {
                radius: 1,
                ..SauvolaConfig::default()
            },
        )
        .unwrap();
        let b = sauvola(
            Gray8View::new(&padded, 3, 3, 5).unwrap(),
            &SauvolaConfig {
                radius: 1,
                ..SauvolaConfig::default()
            },
        )
        .unwrap();
        assert_eq!(a, b);
    }
}
