use crate::morphology::{grayscale_close_into, MorphologyWorkspace};
use crate::{ImgProcError, ImgProcResult};
use qrkit_image::{Gray8Image, Gray8View, Gray8ViewMut};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackgroundDivideConfig {
    pub structuring_element: usize,
    pub target_luma: u8,
    pub denominator_floor: u8,
}

impl Default for BackgroundDivideConfig {
    fn default() -> Self {
        Self {
            structuring_element: 31,
            target_luma: 200,
            denominator_floor: 8,
        }
    }
}

#[derive(Debug, Default)]
pub struct IlluminationWorkspace {
    background: Vec<u8>,
    morphology: MorphologyWorkspace,
}

pub fn background_divide(
    src: Gray8View<'_>,
    config: &BackgroundDivideConfig,
) -> ImgProcResult<Gray8Image> {
    let mut output = Gray8Image::new(src.size())?;
    let mut workspace = IlluminationWorkspace::default();
    background_divide_into(src, output.view_mut(), config, &mut workspace)?;
    Ok(output)
}

pub fn background_divide_into(
    src: Gray8View<'_>,
    mut dst: Gray8ViewMut<'_>,
    config: &BackgroundDivideConfig,
    workspace: &mut IlluminationWorkspace,
) -> ImgProcResult<()> {
    if src.size() != dst.size() {
        return Err(ImgProcError::DimensionMismatch);
    }
    if config.structuring_element < 3
        || config.structuring_element.is_multiple_of(2)
        || config.denominator_floor == 0
    {
        return Err(ImgProcError::InvalidConfiguration);
    }
    let area = src
        .width()
        .checked_mul(src.height())
        .ok_or(ImgProcError::InvalidConfiguration)?;
    workspace.background.resize(area, 0);
    let background = Gray8ViewMut::new(
        &mut workspace.background,
        src.width(),
        src.height(),
        src.width(),
    )?;
    grayscale_close_into(
        src,
        background,
        config.structuring_element,
        &mut workspace.morphology,
    )?;
    for y in 0..src.height() {
        for x in 0..src.width() {
            let background =
                workspace.background[y * src.width() + x].max(config.denominator_floor) as u32;
            let value = src.get(x, y) as u32 * config.target_luma as u32 / background;
            dst.set(x, y, value.min(255) as u8);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_algorithm_matches_scanner_kernel_parameters() {
        let (width, height) = (64, 48);
        let data: Vec<u8> = (0..width * height)
            .map(|index| (40 + (index % width) * 3) as u8)
            .collect();
        let source = Gray8View::new(&data, width, height, width).unwrap();
        let config = BackgroundDivideConfig {
            structuring_element: 15,
            ..BackgroundDivideConfig::default()
        };
        let expected = crate::internal::background_divide(&source, 15).0;
        let actual = background_divide(source, &config).unwrap();
        assert_eq!(actual.as_slice(), expected);
    }
}
