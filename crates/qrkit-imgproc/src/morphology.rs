use crate::{ImgProcError, ImgProcResult};
use qrkit_image::{Gray8Image, Gray8View, Gray8ViewMut, Size};

/// Reusable storage for separable Van Herk/Gil-Werman morphology.
#[derive(Debug, Default)]
pub struct MorphologyWorkspace {
    input: Vec<u8>,
    first: Vec<u8>,
    second: Vec<u8>,
    horizontal: Vec<u8>,
    prefix: Vec<u8>,
    suffix: Vec<u8>,
}

#[derive(Clone, Copy)]
enum Operation {
    Erode,
    Dilate,
    Open,
    Close,
}

pub fn grayscale_erode(src: Gray8View<'_>, kernel_size: usize) -> ImgProcResult<Gray8Image> {
    allocate(src, kernel_size, Operation::Erode)
}

pub fn grayscale_dilate(src: Gray8View<'_>, kernel_size: usize) -> ImgProcResult<Gray8Image> {
    allocate(src, kernel_size, Operation::Dilate)
}

pub fn grayscale_open(src: Gray8View<'_>, kernel_size: usize) -> ImgProcResult<Gray8Image> {
    allocate(src, kernel_size, Operation::Open)
}

pub fn grayscale_close(src: Gray8View<'_>, kernel_size: usize) -> ImgProcResult<Gray8Image> {
    allocate(src, kernel_size, Operation::Close)
}

fn allocate(
    src: Gray8View<'_>,
    kernel_size: usize,
    operation: Operation,
) -> ImgProcResult<Gray8Image> {
    let mut output = Gray8Image::new(src.size())?;
    let mut workspace = MorphologyWorkspace::default();
    apply_into(
        src,
        output.view_mut(),
        kernel_size,
        operation,
        &mut workspace,
    )?;
    Ok(output)
}

pub fn grayscale_erode_into(
    src: Gray8View<'_>,
    dst: Gray8ViewMut<'_>,
    kernel_size: usize,
    workspace: &mut MorphologyWorkspace,
) -> ImgProcResult<()> {
    apply_into(src, dst, kernel_size, Operation::Erode, workspace)
}

pub fn grayscale_dilate_into(
    src: Gray8View<'_>,
    dst: Gray8ViewMut<'_>,
    kernel_size: usize,
    workspace: &mut MorphologyWorkspace,
) -> ImgProcResult<()> {
    apply_into(src, dst, kernel_size, Operation::Dilate, workspace)
}

pub fn grayscale_open_into(
    src: Gray8View<'_>,
    dst: Gray8ViewMut<'_>,
    kernel_size: usize,
    workspace: &mut MorphologyWorkspace,
) -> ImgProcResult<()> {
    apply_into(src, dst, kernel_size, Operation::Open, workspace)
}

pub fn grayscale_close_into(
    src: Gray8View<'_>,
    dst: Gray8ViewMut<'_>,
    kernel_size: usize,
    workspace: &mut MorphologyWorkspace,
) -> ImgProcResult<()> {
    apply_into(src, dst, kernel_size, Operation::Close, workspace)
}

fn apply_into(
    src: Gray8View<'_>,
    mut dst: Gray8ViewMut<'_>,
    kernel_size: usize,
    operation: Operation,
    workspace: &mut MorphologyWorkspace,
) -> ImgProcResult<()> {
    validate(src.size(), dst.size(), kernel_size)?;
    let MorphologyWorkspace {
        input,
        first,
        second,
        horizontal,
        prefix,
        suffix,
    } = workspace;
    input.clear();
    input.reserve(src.width() * src.height());
    for y in 0..src.height() {
        input.extend_from_slice(src.row(y));
    }

    match operation {
        Operation::Erode => morph_rect_into::<false>(
            input,
            src.width(),
            src.height(),
            kernel_size,
            first,
            horizontal,
            prefix,
            suffix,
        ),
        Operation::Dilate => morph_rect_into::<true>(
            input,
            src.width(),
            src.height(),
            kernel_size,
            first,
            horizontal,
            prefix,
            suffix,
        ),
        Operation::Open => {
            morph_rect_into::<false>(
                input,
                src.width(),
                src.height(),
                kernel_size,
                first,
                horizontal,
                prefix,
                suffix,
            );
            morph_rect_into::<true>(
                first,
                src.width(),
                src.height(),
                kernel_size,
                second,
                horizontal,
                prefix,
                suffix,
            );
        }
        Operation::Close => {
            morph_rect_into::<true>(
                input,
                src.width(),
                src.height(),
                kernel_size,
                first,
                horizontal,
                prefix,
                suffix,
            );
            morph_rect_into::<false>(
                first,
                src.width(),
                src.height(),
                kernel_size,
                second,
                horizontal,
                prefix,
                suffix,
            );
        }
    }

    let output = match operation {
        Operation::Open | Operation::Close => second,
        Operation::Erode | Operation::Dilate => first,
    };
    let width = dst.width();
    for y in 0..dst.height() {
        dst.row_mut(y)
            .copy_from_slice(&output[y * width..(y + 1) * width]);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn morph_rect_into<const IS_MAX: bool>(
    src: &[u8],
    width: usize,
    height: usize,
    kernel_size: usize,
    dst: &mut Vec<u8>,
    horizontal: &mut Vec<u8>,
    prefix: &mut Vec<u8>,
    suffix: &mut Vec<u8>,
) {
    let len = width * height;
    dst.resize(len, 0);
    horizontal.resize(len, 0);
    prefix.resize(len, 0);
    suffix.resize(len, 0);

    for y in 0..height {
        running_extremum_1d::<IS_MAX>(
            &src[y * width..(y + 1) * width],
            kernel_size,
            &mut prefix[..width],
            &mut suffix[..width],
            &mut horizontal[y * width..(y + 1) * width],
        );
    }

    let extremum = ext::<IS_MAX>;
    let radius = kernel_size / 2;
    for y in 0..height {
        let (done, current_and_after) = prefix.split_at_mut(y * width);
        let current = &mut current_and_after[..width];
        let row = &horizontal[y * width..(y + 1) * width];
        if y % kernel_size == 0 {
            current.copy_from_slice(row);
        } else {
            let previous = &done[(y - 1) * width..y * width];
            for x in 0..width {
                current[x] = extremum(previous[x], row[x]);
            }
        }
    }
    for y in (0..height).rev() {
        let (current, after) = suffix[y * width..].split_at_mut(width);
        let row = &horizontal[y * width..(y + 1) * width];
        if y % kernel_size == kernel_size - 1 || y == height - 1 {
            current.copy_from_slice(row);
        } else {
            let next = &after[..width];
            for x in 0..width {
                current[x] = extremum(next[x], row[x]);
            }
        }
    }
    for y in 0..height {
        let low = y.saturating_sub(radius);
        let high = (y + radius).min(height - 1);
        let suffix_row = &suffix[low * width..(low + 1) * width];
        let prefix_row = &prefix[high * width..(high + 1) * width];
        for x in 0..width {
            dst[y * width + x] = extremum(suffix_row[x], prefix_row[x]);
        }
    }
}

fn running_extremum_1d<const IS_MAX: bool>(
    src: &[u8],
    kernel_size: usize,
    prefix: &mut [u8],
    suffix: &mut [u8],
    output: &mut [u8],
) {
    let extremum = ext::<IS_MAX>;
    let radius = kernel_size / 2;
    for index in 0..src.len() {
        prefix[index] = if index % kernel_size == 0 {
            src[index]
        } else {
            extremum(prefix[index - 1], src[index])
        };
    }
    for index in (0..src.len()).rev() {
        suffix[index] = if index % kernel_size == kernel_size - 1 || index == src.len() - 1 {
            src[index]
        } else {
            extremum(suffix[index + 1], src[index])
        };
    }
    for (index, value) in output.iter_mut().enumerate().take(src.len()) {
        let low = index.saturating_sub(radius);
        let high = (index + radius).min(src.len() - 1);
        *value = extremum(suffix[low], prefix[high]);
    }
}

#[inline(always)]
fn ext<const IS_MAX: bool>(a: u8, b: u8) -> u8 {
    if IS_MAX {
        a.max(b)
    } else {
        a.min(b)
    }
}

fn validate(source: Size, destination: Size, kernel_size: usize) -> ImgProcResult<()> {
    if source != destination {
        return Err(ImgProcError::DimensionMismatch);
    }
    if kernel_size < 3 || kernel_size.is_multiple_of(2) {
        return Err(ImgProcError::InvalidKernel);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_close_matches_scanner_legacy_kernel() {
        let data: Vec<u8> = (0..9 * 7).map(|index| (index * 37) as u8).collect();
        let src = Gray8View::new(&data, 9, 7, 9).unwrap();
        let mut dilated = Vec::new();
        let mut expected = Vec::new();
        crate::internal::morph_rect::<true>(&data, 9, 7, 5, &mut dilated);
        crate::internal::morph_rect::<false>(&dilated, 9, 7, 5, &mut expected);
        let actual = grayscale_close(src, 5).unwrap();
        assert_eq!(actual.as_slice(), expected);
    }

    #[test]
    fn warm_workspace_preserves_capacities() {
        let data = vec![80; 32 * 24];
        let src = Gray8View::new(&data, 32, 24, 32).unwrap();
        let mut output = Gray8Image::new(src.size()).unwrap();
        let mut workspace = MorphologyWorkspace::default();
        grayscale_close_into(src, output.view_mut(), 7, &mut workspace).unwrap();
        let capacities = (
            workspace.input.capacity(),
            workspace.first.capacity(),
            workspace.second.capacity(),
            workspace.horizontal.capacity(),
            workspace.prefix.capacity(),
            workspace.suffix.capacity(),
        );
        grayscale_close_into(src, output.view_mut(), 7, &mut workspace).unwrap();
        assert_eq!(
            capacities,
            (
                workspace.input.capacity(),
                workspace.first.capacity(),
                workspace.second.capacity(),
                workspace.horizontal.capacity(),
                workspace.prefix.capacity(),
                workspace.suffix.capacity(),
            )
        );
    }
}
