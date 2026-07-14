use crate::blur::RasterDirection4;
use crate::{ImgProcError, ImgProcResult};
use qrkit_image::{Gray8Image, Gray8View, Gray8ViewMut, Size};

/// Border handling for one-dimensional restoration kernels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineBorderMode {
    /// Repeat the first or last sample on a scan line.
    #[default]
    Replicate,
    /// Reflect samples at the line boundary without repeating the edge.
    Reflect,
    /// Use a fixed grayscale value outside the image.
    Constant(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VanCittertConfig {
    pub theta_radians: f64,
    pub blur_length: usize,
    pub iterations: usize,
    /// Relaxation in `[0, 1]`, quantized to Q16 for deterministic execution.
    pub relaxation: f64,
    pub border: LineBorderMode,
}

impl VanCittertConfig {
    pub fn raster_direction(&self) -> RasterDirection4 {
        RasterDirection4::from_angle(self.theta_radians)
    }
}

impl Default for VanCittertConfig {
    fn default() -> Self {
        Self {
            theta_radians: 0.0,
            blur_length: 3,
            iterations: 3,
            relaxation: 1.0,
            border: LineBorderMode::Replicate,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalUnsharpConfig {
    pub theta_radians: f64,
    pub blur_length: usize,
    /// Sharpening gain, quantized to Q16 for deterministic execution.
    pub gain: f64,
    pub border: LineBorderMode,
}

impl Default for DirectionalUnsharpConfig {
    fn default() -> Self {
        Self {
            theta_radians: 0.0,
            blur_length: 3,
            gain: 1.5,
            border: LineBorderMode::Replicate,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestorationMetadata {
    pub raster_direction: RasterDirection4,
}

/// Reusable scratch storage shared by line-restoration operators.
#[derive(Debug, Default)]
pub struct DeblurWorkspace {
    observed: Vec<i32>,
    estimate: Vec<i32>,
    mean: Vec<i32>,
    line_indices: Vec<usize>,
}

pub fn van_cittert_line(
    src: Gray8View<'_>,
    config: &VanCittertConfig,
) -> ImgProcResult<Gray8Image> {
    let mut output = Gray8Image::new(src.size())?;
    let mut workspace = DeblurWorkspace::default();
    van_cittert_line_into(src, output.view_mut(), config, &mut workspace)?;
    Ok(output)
}

/// Apply Van Cittert line-PSF restoration in O(pixels * iterations),
/// independent of the blur length, using a running-sum line filter.
pub fn van_cittert_line_into(
    src: Gray8View<'_>,
    mut dst: Gray8ViewMut<'_>,
    config: &VanCittertConfig,
    workspace: &mut DeblurWorkspace,
) -> ImgProcResult<RestorationMetadata> {
    validate_common(
        src.size(),
        dst.size(),
        config.theta_radians,
        config.blur_length,
    )?;
    if config.iterations == 0
        || !config.relaxation.is_finite()
        || !(0.0..=1.0).contains(&config.relaxation)
    {
        return Err(ImgProcError::InvalidConfiguration);
    }
    let relaxation_q16 = quantize_q16(config.relaxation)?;
    prepare_workspace(src, workspace)?;
    workspace.estimate.copy_from_slice(&workspace.observed);
    let direction = config.raster_direction();
    for _ in 0..config.iterations {
        directional_box_mean(
            &workspace.estimate,
            src.width(),
            src.height(),
            direction,
            config.blur_length,
            config.border,
            &mut workspace.mean,
            &mut workspace.line_indices,
        );
        for index in 0..workspace.estimate.len() {
            let residual = workspace.observed[index] - workspace.mean[index];
            workspace.estimate[index] += apply_q16(residual, relaxation_q16);
        }
    }
    write_clamped(&workspace.estimate, &mut dst);
    Ok(RestorationMetadata {
        raster_direction: direction,
    })
}

pub fn directional_unsharp_line(
    src: Gray8View<'_>,
    config: &DirectionalUnsharpConfig,
) -> ImgProcResult<Gray8Image> {
    let mut output = Gray8Image::new(src.size())?;
    let mut workspace = DeblurWorkspace::default();
    directional_unsharp_line_into(src, output.view_mut(), config, &mut workspace)?;
    Ok(output)
}

pub fn directional_unsharp_line_into(
    src: Gray8View<'_>,
    mut dst: Gray8ViewMut<'_>,
    config: &DirectionalUnsharpConfig,
    workspace: &mut DeblurWorkspace,
) -> ImgProcResult<RestorationMetadata> {
    validate_common(
        src.size(),
        dst.size(),
        config.theta_radians,
        config.blur_length,
    )?;
    if !config.gain.is_finite() || !(0.0..=8.0).contains(&config.gain) {
        return Err(ImgProcError::InvalidConfiguration);
    }
    let gain_q16 = quantize_q16(config.gain)?;
    prepare_workspace(src, workspace)?;
    let direction = RasterDirection4::from_angle(config.theta_radians);
    directional_box_mean(
        &workspace.observed,
        src.width(),
        src.height(),
        direction,
        config.blur_length,
        config.border,
        &mut workspace.mean,
        &mut workspace.line_indices,
    );
    for index in 0..workspace.observed.len() {
        let high_frequency = workspace.observed[index] - workspace.mean[index];
        workspace.estimate[index] = workspace.observed[index] + apply_q16(high_frequency, gain_q16);
    }
    write_clamped(&workspace.estimate, &mut dst);
    Ok(RestorationMetadata {
        raster_direction: direction,
    })
}

fn validate_common(
    source: Size,
    destination: Size,
    theta_radians: f64,
    blur_length: usize,
) -> ImgProcResult<()> {
    if source != destination {
        return Err(ImgProcError::DimensionMismatch);
    }
    let max_reasonable = source
        .width
        .max(source.height)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or(ImgProcError::InvalidConfiguration)?;
    if blur_length < 3
        || blur_length.is_multiple_of(2)
        || blur_length > max_reasonable
        || !theta_radians.is_finite()
    {
        return Err(ImgProcError::InvalidConfiguration);
    }
    Ok(())
}

fn prepare_workspace(src: Gray8View<'_>, workspace: &mut DeblurWorkspace) -> ImgProcResult<()> {
    let len = src
        .width()
        .checked_mul(src.height())
        .ok_or(ImgProcError::InvalidConfiguration)?;
    workspace.observed.resize(len, 0);
    workspace.estimate.resize(len, 0);
    workspace.mean.resize(len, 0);
    for y in 0..src.height() {
        for x in 0..src.width() {
            workspace.observed[y * src.width() + x] = src.get(x, y) as i32;
        }
    }
    Ok(())
}

fn quantize_q16(value: f64) -> ImgProcResult<i64> {
    let scaled = (value * 65_536.0).round();
    if !scaled.is_finite() || scaled < 0.0 || scaled > i64::MAX as f64 {
        return Err(ImgProcError::InvalidConfiguration);
    }
    Ok(scaled as i64)
}

fn apply_q16(value: i32, factor_q16: i64) -> i32 {
    ((value as i64 * factor_q16) / 65_536) as i32
}

fn write_clamped(values: &[i32], dst: &mut Gray8ViewMut<'_>) {
    let width = dst.width();
    for y in 0..dst.height() {
        for x in 0..width {
            dst.set(x, y, values[y * width + x].clamp(0, 255) as u8);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn directional_box_mean(
    src: &[i32],
    width: usize,
    height: usize,
    direction: RasterDirection4,
    length: usize,
    border: LineBorderMode,
    dst: &mut [i32],
    line_indices: &mut Vec<usize>,
) {
    let mut process_line = |start_x: usize, start_y: usize| {
        line_indices.clear();
        let (dx, dy) = direction.step();
        let (mut x, mut y) = (start_x as isize, start_y as isize);
        while x >= 0 && x < width as isize && y >= 0 && y < height as isize {
            line_indices.push(y as usize * width + x as usize);
            x += dx;
            y += dy;
        }
        running_line_mean(src, line_indices, length, border, dst);
    };

    match direction {
        RasterDirection4::Horizontal => {
            for y in 0..height {
                process_line(0, y);
            }
        }
        RasterDirection4::Vertical => {
            for x in 0..width {
                process_line(x, 0);
            }
        }
        RasterDirection4::DownRight => {
            for x in 0..width {
                process_line(x, 0);
            }
            for y in 1..height {
                process_line(0, y);
            }
        }
        RasterDirection4::DownLeft => {
            for x in 0..width {
                process_line(x, 0);
            }
            for y in 1..height {
                process_line(width - 1, y);
            }
        }
    }
}

fn running_line_mean(
    src: &[i32],
    indices: &[usize],
    length: usize,
    border: LineBorderMode,
    dst: &mut [i32],
) {
    if indices.is_empty() {
        return;
    }
    let half = (length / 2) as isize;
    let sample = |position: isize| -> i32 {
        let mapped = border_index(position, indices.len(), border);
        match mapped {
            Some(index) => src[indices[index]],
            None => match border {
                LineBorderMode::Constant(value) => value as i32,
                _ => unreachable!("non-constant borders always map an index"),
            },
        }
    };
    let mut sum = 0i64;
    for offset in -half..=half {
        sum += sample(offset) as i64;
    }
    for position in 0..indices.len() as isize {
        dst[indices[position as usize]] = sum.div_euclid(length as i64) as i32;
        sum += sample(position + half + 1) as i64 - sample(position - half) as i64;
    }
}

fn border_index(position: isize, length: usize, border: LineBorderMode) -> Option<usize> {
    match border {
        LineBorderMode::Replicate => Some(position.clamp(0, length as isize - 1) as usize),
        LineBorderMode::Constant(_) => {
            (position >= 0 && position < length as isize).then_some(position as usize)
        }
        LineBorderMode::Reflect if length == 1 => Some(0),
        LineBorderMode::Reflect => {
            let period = 2 * length as isize - 2;
            let reflected = position.rem_euclid(period);
            Some(if reflected < length as isize {
                reflected as usize
            } else {
                (period - reflected) as usize
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_image_is_invariant_for_every_border() {
        let data = vec![120; 7 * 5];
        let src = Gray8View::new(&data, 7, 5, 7).unwrap();
        for border in [
            LineBorderMode::Replicate,
            LineBorderMode::Reflect,
            LineBorderMode::Constant(120),
        ] {
            let output = van_cittert_line(
                src,
                &VanCittertConfig {
                    border,
                    ..VanCittertConfig::default()
                },
            )
            .unwrap();
            assert_eq!(output.as_slice(), data);
        }
    }

    #[test]
    fn default_matches_the_original_scanner_kernel() {
        let mut data = vec![30; 11 * 7];
        data[3 * 11 + 5] = 240;
        data[3 * 11 + 6] = 180;
        let src = Gray8View::new(&data, 11, 7, 11).unwrap();
        let expected = crate::internal::van_cittert_directional(&src, 0.0, 3).0;
        let actual = van_cittert_line(src, &VanCittertConfig::default()).unwrap();
        assert_eq!(actual.as_slice(), expected);
    }

    #[test]
    fn rejects_even_kernel_and_mismatched_output() {
        let data = vec![0; 9];
        let src = Gray8View::new(&data, 3, 3, 3).unwrap();
        let mut output = Gray8Image::new(Size::new(2, 2)).unwrap();
        let mut workspace = DeblurWorkspace::default();
        assert_eq!(
            van_cittert_line_into(
                src,
                output.view_mut(),
                &VanCittertConfig::default(),
                &mut workspace
            ),
            Err(ImgProcError::DimensionMismatch)
        );
        let mut output = Gray8Image::new(Size::new(3, 3)).unwrap();
        let bad = VanCittertConfig {
            blur_length: 4,
            ..VanCittertConfig::default()
        };
        assert_eq!(
            van_cittert_line_into(src, output.view_mut(), &bad, &mut workspace),
            Err(ImgProcError::InvalidConfiguration)
        );
    }

    #[test]
    fn metadata_reports_the_rasterized_direction() {
        let data = vec![0; 25];
        let src = Gray8View::new(&data, 5, 5, 5).unwrap();
        let mut output = Gray8Image::new(src.size()).unwrap();
        let mut workspace = DeblurWorkspace::default();
        let metadata = directional_unsharp_line_into(
            src,
            output.view_mut(),
            &DirectionalUnsharpConfig {
                theta_radians: std::f64::consts::FRAC_PI_2,
                ..DirectionalUnsharpConfig::default()
            },
            &mut workspace,
        )
        .unwrap();
        assert_eq!(metadata.raster_direction, RasterDirection4::Vertical);
    }
}
