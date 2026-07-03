/// Borrowed view over an 8-bit luma (grayscale) image with row stride.
/// The scanner's only input type: zero-copy over camera Y planes.
#[derive(Clone, Copy)]
pub struct LumaView<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LumaError {
    EmptyDimensions,
    StrideTooSmall,
    BufferTooSmall,
}

impl<'a> LumaView<'a> {
    pub fn new(
        data: &'a [u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, LumaError> {
        if width == 0 || height == 0 {
            return Err(LumaError::EmptyDimensions);
        }
        if stride < width {
            return Err(LumaError::StrideTooSmall);
        }
        let needed = stride
            .checked_mul(height - 1)
            .and_then(|n| n.checked_add(width))
            .ok_or(LumaError::BufferTooSmall)?;
        if data.len() < needed {
            return Err(LumaError::BufferTooSmall);
        }
        Ok(Self { data, width, height, stride })
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        debug_assert!(x < self.width && y < self.height);
        self.data[y * self.stride + x]
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn row(&self, y: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + self.width]
    }
}

pub fn luma_from_rgba(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    assert_eq!(rgba.len(), width * height * 4, "rgba buffer size mismatch");
    rgba.chunks_exact(4)
        .map(|p| {
            ((77 * p[0] as u32 + 150 * p[1] as u32 + 29 * p[2] as u32 + 128)
                >> 8) as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_tight_buffer() {
        let data = vec![7u8; 4 * 3];
        let v = LumaView::new(&data, 4, 3, 4).unwrap();
        assert_eq!(v.width(), 4);
        assert_eq!(v.height(), 3);
        assert_eq!(v.get(3, 2), 7);
    }

    #[test]
    fn accepts_padded_stride_and_indexes_through_it() {
        // 3 rows, width 4, stride 6; mark (0, row) with the row index.
        let mut data = vec![0u8; 6 * 2 + 4];
        data[0] = 10;
        data[6] = 11;
        data[12] = 12;
        let v = LumaView::new(&data, 4, 3, 6).unwrap();
        assert_eq!(v.get(0, 0), 10);
        assert_eq!(v.get(0, 1), 11);
        assert_eq!(v.get(0, 2), 12);
    }

    #[test]
    fn rejects_stride_smaller_than_width() {
        assert!(matches!(
            LumaView::new(&[0; 100], 8, 4, 6),
            Err(LumaError::StrideTooSmall)
        ));
    }

    #[test]
    fn rejects_short_buffer() {
        // Needs stride*(h-1)+width = 6*2+4 = 16 bytes; give 15.
        assert!(matches!(
            LumaView::new(&[0; 15], 4, 3, 6),
            Err(LumaError::BufferTooSmall)
        ));
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert!(matches!(
            LumaView::new(&[0; 16], 0, 3, 4),
            Err(LumaError::EmptyDimensions)
        ));
    }

    #[test]
    fn rgba_conversion_bt601() {
        let rgba = [255, 255, 255, 255,  0, 0, 0, 255,  255, 0, 0, 255];
        let y = luma_from_rgba(&rgba, 3, 1);
        assert_eq!(y, vec![255, 0, 77]); // (77*255+128)>>8 = 77
    }

    #[test]
    #[should_panic]
    fn rgba_wrong_len_panics() {
        luma_from_rgba(&[0; 10], 3, 1);
    }
}
