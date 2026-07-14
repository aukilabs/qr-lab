use crate::internal;
use qrkit_image::Gray8View;

/// One of the four raster directions used by QRKit's integer line kernels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterDirection4 {
    Horizontal,
    DownRight,
    Vertical,
    DownLeft,
}

impl RasterDirection4 {
    pub const fn step(self) -> (isize, isize) {
        match self {
            Self::Horizontal => (1, 0),
            Self::DownRight => (1, 1),
            Self::Vertical => (0, 1),
            Self::DownLeft => (-1, 1),
        }
    }

    pub fn from_angle(theta_radians: f64) -> Self {
        match internal::snap_dir(theta_radians) {
            (1, 0) => Self::Horizontal,
            (1, 1) => Self::DownRight,
            (0, 1) => Self::Vertical,
            _ => Self::DownLeft,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionEstimate {
    pub theta_radians: f64,
    pub confidence: f64,
    pub raster_direction: RasterDirection4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineLengthConfig {
    pub minimum_transitions: usize,
    pub minimum_amplitude: u8,
}

impl Default for LineLengthConfig {
    fn default() -> Self {
        Self {
            minimum_transitions: 8,
            minimum_amplitude: 24,
        }
    }
}

/// Estimate line-blur direction and tensor anisotropy from Sobel gradients.
pub fn estimate_line_direction(src: Gray8View<'_>) -> DirectionEstimate {
    let (theta_radians, confidence) = internal::structure_tensor_blur_direction(&src);
    DirectionEstimate {
        theta_radians,
        confidence,
        raster_direction: RasterDirection4::from_angle(theta_radians),
    }
}

/// Estimate line-PSF length from median 20–80% edge-rise widths.
pub fn estimate_line_length(src: Gray8View<'_>, theta_radians: f64) -> Option<f64> {
    estimate_line_length_with(src, theta_radians, &LineLengthConfig::default())
}

pub fn estimate_line_length_with(
    src: Gray8View<'_>,
    theta_radians: f64,
    config: &LineLengthConfig,
) -> Option<f64> {
    if config.minimum_transitions == 0 || config.minimum_amplitude == 0 {
        return None;
    }
    internal::edge_rise_extent_with(
        &src,
        theta_radians,
        config.minimum_transitions,
        config.minimum_amplitude as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_length_configuration_fails_without_sampling() {
        let data = [128u8; 9];
        let source = Gray8View::new(&data, 3, 3, 3).unwrap();
        assert_eq!(
            estimate_line_length_with(
                source,
                0.0,
                &LineLengthConfig {
                    minimum_transitions: 0,
                    minimum_amplitude: 24,
                }
            ),
            None
        );
    }
}
