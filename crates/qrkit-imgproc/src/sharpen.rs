use crate::{ImgProcError, ImgProcResult};
use qrkit_image::{Gray8Image, Gray8View, Gray8ViewMut};

#[derive(Debug, Default)]
pub struct SharpenWorkspace {
    horizontal: Vec<u16>,
}

/// Apply the scanner's deterministic `[1,4,6,4,1]/16`, amount-1 unsharp mask.
pub fn fixed_binomial_unsharp(src: Gray8View<'_>) -> ImgProcResult<Gray8Image> {
    let mut output = Gray8Image::new(src.size())?;
    let mut workspace = SharpenWorkspace::default();
    fixed_binomial_unsharp_into(src, output.view_mut(), &mut workspace)?;
    Ok(output)
}

pub fn fixed_binomial_unsharp_into(
    src: Gray8View<'_>,
    mut dst: Gray8ViewMut<'_>,
    workspace: &mut SharpenWorkspace,
) -> ImgProcResult<()> {
    if src.size() != dst.size() {
        return Err(ImgProcError::DimensionMismatch);
    }
    let (width, height) = (src.width(), src.height());
    workspace.horizontal.resize(width * height, 0);
    for y in 0..height {
        let row = src.row(y);
        for x in 0..width {
            let at = |offset: isize| {
                row[(x as isize + offset).clamp(0, width as isize - 1) as usize] as u16
            };
            workspace.horizontal[y * width + x] =
                at(-2) + 4 * at(-1) + 6 * at(0) + 4 * at(1) + at(2);
        }
    }
    for y in 0..height {
        for x in 0..width {
            let at = |offset: isize| {
                let row = (y as isize + offset).clamp(0, height as isize - 1) as usize;
                workspace.horizontal[row * width + x] as u32
            };
            let blurred = (at(-2) + 4 * at(-1) + 6 * at(0) + 4 * at(1) + at(2) + 128) >> 8;
            let sharpened = 2 * src.get(x, y) as i32 - blurred as i32;
            dst.set(x, y, sharpened.clamp(0, 255) as u8);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_operator_matches_scanner_kernel() {
        let (width, height) = (17, 11);
        let data: Vec<u8> = (0..width * height)
            .map(|index| ((index * 53 + index / width * 7) & 255) as u8)
            .collect();
        let source = Gray8View::new(&data, width, height, width).unwrap();
        let expected = crate::internal::unsharp_mask(&source).0;
        let actual = fixed_binomial_unsharp(source).unwrap();
        assert_eq!(actual.as_slice(), expected);
    }
}
