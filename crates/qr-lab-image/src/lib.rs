//! Grayscale image storage and zero-copy views for QR Lab pipelines.
//!
//! Pixel indices use an integer-center coordinate convention: pixel `(x, y)`
//! stores the sample at continuous coordinate `(x, y)`. Views may have padded
//! row strides and can represent camera Y planes without copying.
//!
//! # Quick start
//!
//! ```
//! use qr_lab_image::{Gray8View, Gray8Image, Size};
//!
//! let image = Gray8Image::filled(Size::new(4, 3), 128).unwrap();
//! let view = image.view();
//! assert_eq!(view.get(0, 0), 128);
//! assert_eq!(view.width(), 4);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::error::Error;
use std::fmt;

/// Two-dimensional image dimensions in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Size {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
}

impl Size {
    /// Create dimensions with the given width and height.
    pub const fn new(width: usize, height: usize) -> Self {
        Self { width, height }
    }

    /// Number of pixels (`width * height`), rejecting empty or overflowing sizes.
    pub fn area(self) -> Result<usize, ImageError> {
        if self.width == 0 || self.height == 0 {
            return Err(ImageError::Layout(LumaError::EmptyDimensions));
        }
        self.width
            .checked_mul(self.height)
            .ok_or(ImageError::DimensionOverflow)
    }
}

/// A half-open image rectangle `[x, x + width) × [y, y + height)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Left edge in pixels (inclusive).
    pub x: usize,
    /// Top edge in pixels (inclusive).
    pub y: usize,
    /// Width of the region in pixels.
    pub width: usize,
    /// Height of the region in pixels.
    pub height: usize,
}

impl Rect {
    /// Create a rectangle from origin and size.
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Return the rectangle's dimensions as a [`Size`].
    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }
}

/// Errors produced while constructing or slicing grayscale images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LumaError {
    /// Width or height is zero.
    EmptyDimensions,
    /// Row stride is smaller than image width.
    StrideTooSmall,
    /// Backing buffer is shorter than the declared layout requires.
    BufferTooSmall,
}

impl fmt::Display for LumaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyDimensions => "image dimensions must be non-zero",
            Self::StrideTooSmall => "row stride is smaller than image width",
            Self::BufferTooSmall => "buffer is too small for dimensions and stride",
        };
        f.write_str(message)
    }
}

impl Error for LumaError {}

/// Errors produced by owned images, conversions, and checked ROI operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Layout validation failed for a view or buffer.
    Layout(LumaError),
    /// Owned buffer length does not match `width * height` (tight packing).
    BufferLengthMismatch,
    /// Arithmetic on dimensions overflowed `usize`.
    DimensionOverflow,
    /// Requested ROI is empty or extends outside the parent image.
    RoiOutOfBounds,
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Layout(error) => return error.fmt(f),
            Self::BufferLengthMismatch => "buffer length does not match image dimensions",
            Self::DimensionOverflow => "image dimensions overflow addressable storage",
            Self::RoiOutOfBounds => "region of interest is empty or out of bounds",
        };
        f.write_str(message)
    }
}

impl Error for ImageError {}

impl From<LumaError> for ImageError {
    fn from(value: LumaError) -> Self {
        Self::Layout(value)
    }
}

/// Borrowed, zero-copy view over an 8-bit grayscale image.
///
/// The view may use a row `stride` greater than `width` so camera buffers with
/// row padding can be scanned without copying.
#[derive(Clone, Copy, Debug)]
pub struct Gray8View<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> Gray8View<'a> {
    /// Borrow `data` as a grayscale image of the given layout.
    ///
    /// `stride` is the number of bytes between consecutive rows and must be
    /// at least `width`. Pass `width` for tightly packed buffers.
    pub fn new(
        data: &'a [u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, LumaError> {
        validate_layout(data.len(), width, height, stride)?;
        Ok(Self {
            data,
            width,
            height,
            stride,
        })
    }

    /// Read the pixel at `(x, y)`.
    ///
    /// Debug builds assert that the coordinate is in-bounds.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        debug_assert!(x < self.width && y < self.height);
        self.data[y * self.stride + x]
    }

    /// Read the pixel at `(x, y)`, or `None` if out of bounds.
    pub fn try_get(&self, x: usize, y: usize) -> Option<u8> {
        (x < self.width && y < self.height).then(|| self.get(x, y))
    }

    /// Image width in pixels.
    #[inline]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Image height in pixels.
    #[inline]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Row stride in bytes.
    #[inline]
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Image dimensions as a [`Size`].
    #[inline]
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Borrow one tightly packed row of length [`Self::width`].
    #[inline]
    pub fn row(&self, y: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + self.width]
    }

    /// The minimum backing slice needed to represent this view, including
    /// padding between rows but excluding any unused bytes after its last row.
    pub fn backing_slice(&self) -> &'a [u8] {
        let len = required_len(self.width, self.height, self.stride)
            .expect("validated image layout cannot overflow");
        &self.data[..len]
    }

    /// Create a checked, zero-copy subview.
    ///
    /// Coordinates are relative to this view, and the returned view retains
    /// the parent row stride.
    pub fn subview(&self, rect: Rect) -> Result<Self, ImageError> {
        validate_rect(self.size(), rect)?;
        let offset = rect
            .y
            .checked_mul(self.stride)
            .and_then(|value| value.checked_add(rect.x))
            .ok_or(ImageError::DimensionOverflow)?;
        Self::new(&self.data[offset..], rect.width, rect.height, self.stride)
            .map_err(ImageError::from)
    }

    /// Compatibility helper for the original scanner API.
    ///
    /// Equivalent to [`Self::subview`] with `None` on error.
    pub fn sub_view(&self, x: usize, y: usize, width: usize, height: usize) -> Option<Self> {
        self.subview(Rect::new(x, y, width, height)).ok()
    }
}

/// Borrowed, zero-copy view over a packed RGB8 image.
///
/// Pixels are consecutive red, green, and blue bytes. Rows may be padded.
#[derive(Clone, Copy, Debug)]
pub struct Rgb8View<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> Rgb8View<'a> {
    /// Borrow `data` as RGB8. `stride` is measured in bytes.
    pub fn new(
        data: &'a [u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, LumaError> {
        let row_bytes = width.checked_mul(3).ok_or(LumaError::BufferTooSmall)?;
        validate_layout(data.len(), row_bytes, height, stride)?;
        Ok(Self {
            data,
            width,
            height,
            stride,
        })
    }

    /// Image width in pixels.
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Image height in pixels.
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Row stride in bytes.
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Image dimensions.
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Borrow one row containing exactly `width * 3` RGB bytes.
    pub fn row(&self, y: usize) -> &'a [u8] {
        debug_assert!(y < self.height);
        let start = y * self.stride;
        &self.data[start..start + self.width * 3]
    }

    /// Convert to tightly packed BT.601 luminance, reusing `output`.
    pub fn write_luma(&self, output: &mut Vec<u8>) {
        let pixel_count = self
            .width
            .checked_mul(self.height)
            .expect("validated RGB8 layout cannot overflow");
        output.resize(pixel_count, 0);
        for y in 0..self.height {
            let rgb_row = self.row(y);
            let luma_row = &mut output[y * self.width..(y + 1) * self.width];
            for (rgb, luma) in rgb_row.chunks_exact(3).zip(luma_row) {
                *luma = rgb_to_luma(rgb[0], rgb[1], rgb[2]);
            }
        }
    }
}

/// Mutable, zero-copy view over an 8-bit grayscale image.
#[derive(Debug)]
pub struct Gray8ViewMut<'a> {
    data: &'a mut [u8],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> Gray8ViewMut<'a> {
    /// Borrow `data` mutably as a grayscale image of the given layout.
    pub fn new(
        data: &'a mut [u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, LumaError> {
        validate_layout(data.len(), width, height, stride)?;
        Ok(Self {
            data,
            width,
            height,
            stride,
        })
    }

    /// Image width in pixels.
    #[inline]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Image height in pixels.
    #[inline]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Row stride in bytes.
    #[inline]
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Image dimensions as a [`Size`].
    #[inline]
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Read the pixel at `(x, y)`.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        debug_assert!(x < self.width && y < self.height);
        self.data[y * self.stride + x]
    }

    /// Write `value` at `(x, y)`.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: u8) {
        debug_assert!(x < self.width && y < self.height);
        self.data[y * self.stride + x] = value;
    }

    /// Borrow one tightly packed row of length [`Self::width`].
    #[inline]
    pub fn row(&self, y: usize) -> &[u8] {
        &self.data[y * self.stride..y * self.stride + self.width]
    }

    /// Borrow one tightly packed row mutably.
    #[inline]
    pub fn row_mut(&mut self, y: usize) -> &mut [u8] {
        &mut self.data[y * self.stride..y * self.stride + self.width]
    }

    /// Reborrow as an immutable [`Gray8View`].
    pub fn as_view(&self) -> Gray8View<'_> {
        Gray8View::new(self.data, self.width, self.height, self.stride)
            .expect("validated image layout cannot become invalid")
    }

    /// Create a checked, zero-copy mutable subview of `rect`.
    pub fn subview(&mut self, rect: Rect) -> Result<Gray8ViewMut<'_>, ImageError> {
        validate_rect(self.size(), rect)?;
        let offset = rect
            .y
            .checked_mul(self.stride)
            .and_then(|value| value.checked_add(rect.x))
            .ok_or(ImageError::DimensionOverflow)?;
        Gray8ViewMut::new(
            &mut self.data[offset..],
            rect.width,
            rect.height,
            self.stride,
        )
        .map_err(ImageError::from)
    }
}

/// Owned, tightly packed 8-bit grayscale image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gray8Image {
    data: Vec<u8>,
    size: Size,
}

impl Gray8Image {
    /// Allocate a zero-filled image of the given size.
    pub fn new(size: Size) -> Result<Self, ImageError> {
        Self::filled(size, 0)
    }

    /// Allocate an image filled with `value`.
    pub fn filled(size: Size, value: u8) -> Result<Self, ImageError> {
        Ok(Self {
            data: vec![value; size.area()?],
            size,
        })
    }

    /// Take ownership of a tightly packed buffer whose length must equal
    /// `size.area()`.
    pub fn from_vec(data: Vec<u8>, size: Size) -> Result<Self, ImageError> {
        if data.len() != size.area()? {
            return Err(ImageError::BufferLengthMismatch);
        }
        Ok(Self { data, size })
    }

    /// Image dimensions.
    pub const fn size(&self) -> Size {
        self.size
    }

    /// Borrow the tightly packed pixel buffer.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// Borrow the tightly packed pixel buffer mutably.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Consume the image and return its pixel buffer.
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }

    /// Borrow as an immutable view (stride equals width).
    pub fn view(&self) -> Gray8View<'_> {
        Gray8View::new(
            &self.data,
            self.size.width,
            self.size.height,
            self.size.width,
        )
        .expect("owned image invariant")
    }

    /// Borrow as a mutable view (stride equals width).
    pub fn view_mut(&mut self) -> Gray8ViewMut<'_> {
        Gray8ViewMut::new(
            &mut self.data,
            self.size.width,
            self.size.height,
            self.size.width,
        )
        .expect("owned image invariant")
    }
}

/// Checked BT.601 integer conversion from tightly packed RGBA8 to grayscale.
///
/// Returns one luma sample per pixel using
/// `(77·R + 150·G + 29·B + 128) >> 8`.
pub fn try_luma_from_rgba(rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, ImageError> {
    let pixels = Size::new(width, height).area()?;
    let expected = pixels.checked_mul(4).ok_or(ImageError::DimensionOverflow)?;
    if rgba.len() != expected {
        return Err(ImageError::BufferLengthMismatch);
    }
    Ok(rgba
        .chunks_exact(4)
        .map(|pixel| rgb_to_luma(pixel[0], pixel[1], pixel[2]))
        .collect())
}

/// Compatibility wrapper retaining the original scanner's panicking contract.
///
/// Prefer [`try_luma_from_rgba`] in new code.
pub fn luma_from_rgba(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .expect("rgba buffer size mismatch");
    assert_eq!(rgba.len(), expected, "rgba buffer size mismatch");
    rgba.chunks_exact(4)
        .map(|pixel| rgb_to_luma(pixel[0], pixel[1], pixel[2]))
        .collect()
}

#[inline]
fn rgb_to_luma(red: u8, green: u8, blue: u8) -> u8 {
    ((77 * red as u32 + 150 * green as u32 + 29 * blue as u32 + 128) >> 8) as u8
}

/// Backward-compatible alias used by the scanner pipeline.
pub type LumaView<'a> = Gray8View<'a>;

fn required_len(width: usize, height: usize, stride: usize) -> Result<usize, LumaError> {
    stride
        .checked_mul(height - 1)
        .and_then(|value| value.checked_add(width))
        .ok_or(LumaError::BufferTooSmall)
}

fn validate_layout(
    data_len: usize,
    width: usize,
    height: usize,
    stride: usize,
) -> Result<(), LumaError> {
    if width == 0 || height == 0 {
        return Err(LumaError::EmptyDimensions);
    }
    if stride < width {
        return Err(LumaError::StrideTooSmall);
    }
    let needed = required_len(width, height, stride)?;
    if data_len < needed {
        return Err(LumaError::BufferTooSmall);
    }
    Ok(())
}

fn validate_rect(size: Size, rect: Rect) -> Result<(), ImageError> {
    if rect.width == 0 || rect.height == 0 {
        return Err(ImageError::RoiOutOfBounds);
    }
    let right = rect
        .x
        .checked_add(rect.width)
        .ok_or(ImageError::DimensionOverflow)?;
    let bottom = rect
        .y
        .checked_add(rect.height)
        .ok_or(ImageError::DimensionOverflow)?;
    if right > size.width || bottom > size.height {
        return Err(ImageError::RoiOutOfBounds);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strided_view_and_nested_roi_share_storage() {
        let mut data = vec![0; 6 * 4];
        for y in 0..4 {
            for x in 0..4 {
                data[y * 6 + x] = (10 * y + x) as u8;
            }
        }
        let view = Gray8View::new(&data, 4, 4, 6).unwrap();
        let roi = view.subview(Rect::new(1, 1, 3, 2)).unwrap();
        let nested = roi.subview(Rect::new(1, 0, 2, 2)).unwrap();
        assert_eq!(nested.stride(), 6);
        assert_eq!(nested.row(0), &[12, 13]);
        assert_eq!(nested.row(1), &[22, 23]);
    }

    #[test]
    fn mutable_view_preserves_padding() {
        let mut data = vec![99; 10];
        let mut view = Gray8ViewMut::new(&mut data, 3, 2, 5).unwrap();
        view.row_mut(1).copy_from_slice(&[1, 2, 3]);
        assert_eq!(data, [99, 99, 99, 99, 99, 1, 2, 3, 99, 99]);
    }

    #[test]
    fn checked_rgba_conversion_rejects_bad_layout() {
        assert_eq!(
            try_luma_from_rgba(&[0; 7], 2, 1),
            Err(ImageError::BufferLengthMismatch)
        );
    }

    #[test]
    fn rgba_conversion_matches_scanner_formula() {
        let rgba = [255, 255, 255, 255, 0, 0, 0, 255, 255, 0, 0, 255];
        assert_eq!(try_luma_from_rgba(&rgba, 3, 1).unwrap(), [255, 0, 77]);
    }

    #[test]
    fn rgb_view_converts_padded_rows_and_reuses_output() {
        let rgb = [255, 255, 255, 0, 0, 0, 99, 99, 255, 0, 0, 0, 255, 0, 88, 88];
        let view = Rgb8View::new(&rgb, 2, 2, 8).unwrap();
        let mut luma = vec![42; 32];
        view.write_luma(&mut luma);
        assert_eq!(luma, [255, 0, 77, 149]);
    }

    #[test]
    fn rgb_view_rejects_invalid_layouts() {
        assert_eq!(
            Rgb8View::new(&[0; 6], 2, 1, 5).unwrap_err(),
            LumaError::StrideTooSmall
        );
        assert_eq!(
            Rgb8View::new(&[0; 11], 2, 2, 6).unwrap_err(),
            LumaError::BufferTooSmall
        );
    }

    #[test]
    fn compatibility_rgba_conversion_accepts_an_empty_image() {
        assert!(luma_from_rgba(&[], 0, 0).is_empty());
    }

    #[test]
    fn dimension_and_roi_overflow_are_rejected() {
        assert_eq!(
            Gray8Image::new(Size::new(usize::MAX, 2)),
            Err(ImageError::DimensionOverflow)
        );
        let data = [0; 4];
        let view = Gray8View::new(&data, 2, 2, 2).unwrap();
        assert!(matches!(
            view.subview(Rect::new(usize::MAX, 0, 2, 1)),
            Err(ImageError::DimensionOverflow)
        ));
    }
}
