//! Cached lens rectification for grayscale working images.

use crate::{ImgProcError, ImgProcResult};
use qr_lab_geometry::{sample_bilinear, BorderMode, CameraCalibration};
use qr_lab_image::{Gray8View, Gray8ViewMut, Size};

/// Reusable output-to-input pixel map for a fixed calibration and output size.
///
/// Coordinates use the scanner's per-axis scaling convention: an output pixel
/// `(x,y)` corresponds to full rectified `(x/sx,y/sy)`. Map storage is eight
/// bytes per output pixel. Only map construction evaluates the lens model;
/// subsequent frames reuse it. Unsupported/out-of-frame samples use a caller's
/// constant border value (normally white for a QR background).
pub struct UndistortMap {
    source_size: Size,
    output_size: Size,
    points: Vec<[f32; 2]>,
}

impl UndistortMap {
    /// Build a map for `output_size`; reject empty/overflowing dimensions.
    pub fn new(calibration: &CameraCalibration, output_size: Size) -> ImgProcResult<Self> {
        let area = output_size.area()?;
        let source_size = calibration.image_size();
        let sx = output_size.width as f64 / source_size.width as f64;
        let sy = output_size.height as f64 / source_size.height as f64;
        let mut points = Vec::new();
        points
            .try_reserve_exact(area)
            .map_err(|_| ImgProcError::InvalidDestinationSize)?;
        for y in 0..output_size.height {
            for x in 0..output_size.width {
                let p = calibration.rectified_to_source([x as f64 / sx, y as f64 / sy]);
                points.push(
                    p.map(|p| [p[0] as f32, p[1] as f32])
                        .unwrap_or([f32::NAN; 2]),
                );
            }
        }
        Ok(Self {
            source_size,
            output_size,
            points,
        })
    }

    /// Dimensions of the required input image.
    pub const fn source_size(&self) -> Size {
        self.source_size
    }

    /// Dimensions of the rectified working image.
    pub const fn output_size(&self) -> Size {
        self.output_size
    }

    /// Bilinearly rectify a frame into a caller-owned destination.
    ///
    /// Source and destination may have row padding. Rejects a size mismatch
    /// before writing anything; missing samples are filled with `border`.
    pub fn remap_into(
        &self,
        source: Gray8View<'_>,
        mut output: Gray8ViewMut<'_>,
        border: u8,
    ) -> ImgProcResult<()> {
        if source.size() != self.source_size || output.size() != self.output_size {
            return Err(ImgProcError::DimensionMismatch);
        }
        for y in 0..self.output_size.height {
            let row = output.row_mut(y);
            for (x, value) in row.iter_mut().enumerate() {
                let [px, py] = self.points[y * self.output_size.width + x];
                *value = sample_bilinear(source, px as f64, py as f64, BorderMode::Reject)
                    .map(|v| v.round() as u8)
                    .unwrap_or(border);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qr_lab_geometry::{CameraIntrinsics, LensDistortion};
    use qr_lab_image::Gray8Image;

    #[test]
    fn identity_preserves_strided_pixels_and_rejects_wrong_size() {
        let size = Size::new(3, 2);
        let c = CameraCalibration::new(
            size,
            CameraIntrinsics {
                fx: 2.0,
                fy: 2.0,
                cx: 1.0,
                cy: 0.5,
            },
            LensDistortion::None,
        )
        .unwrap();
        let map = UndistortMap::new(&c, size).unwrap();
        let bytes = [1, 2, 3, 99, 4, 5, 6];
        let source = Gray8View::new(&bytes, 3, 2, 4).unwrap();
        let mut output = Gray8Image::new(size).unwrap();
        map.remap_into(source, output.view_mut(), 255).unwrap();
        assert_eq!(output.as_slice(), &[1, 2, 3, 4, 5, 6]);
        let wrong = Gray8Image::new(Size::new(2, 2)).unwrap();
        assert_eq!(
            map.remap_into(wrong.view(), output.view_mut(), 255),
            Err(ImgProcError::DimensionMismatch)
        );
        assert_eq!(output.as_slice(), &[1, 2, 3, 4, 5, 6]);
        assert!(UndistortMap::new(&c, Size::new(0, 2)).is_err());
    }

    #[test]
    fn wider_rectified_view_fills_missing_samples_and_preserves_output_padding() {
        let size = Size::new(3, 3);
        let k = CameraIntrinsics {
            fx: 2.0,
            fy: 2.0,
            cx: 1.0,
            cy: 1.0,
        };
        let c = CameraCalibration::new(size, k, LensDistortion::None)
            .unwrap()
            .with_rectified_intrinsics(CameraIntrinsics {
                fx: 1.0,
                fy: 1.0,
                ..k
            })
            .unwrap();
        let map = UndistortMap::new(&c, size).unwrap();
        let pixels = [0; 9];
        let source = Gray8View::new(&pixels, 3, 3, 3).unwrap();
        let mut output = [73; 12];
        map.remap_into(
            source,
            Gray8ViewMut::new(&mut output, 3, 3, 4).unwrap(),
            255,
        )
        .unwrap();
        assert_eq!(
            output,
            [255, 255, 255, 73, 255, 0, 255, 73, 255, 255, 255, 73]
        );
    }
}
