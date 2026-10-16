//! Calibrated image-coordinate mappings (no camera pose estimation).
//!
//! Models follow OpenCV's calib3d and fisheye conventions, with zero skew.

use qr_lab_image::Size;
use std::{error::Error, fmt};

/// Pinhole intrinsics in pixels, with integer pixel centers and zero skew.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraIntrinsics {
    /// Horizontal focal length in pixels; must be positive.
    pub fx: f64,
    /// Vertical focal length in pixels; must be positive.
    pub fy: f64,
    /// Principal point's horizontal coordinate in pixels.
    pub cx: f64,
    /// Principal point's vertical coordinate in pixels.
    pub cy: f64,
}

impl CameraIntrinsics {
    fn valid(self) -> bool {
        [self.fx, self.fy, self.cx, self.cy]
            .iter()
            .all(|v| v.is_finite())
            && self.fx > 0.0
            && self.fy > 0.0
    }
}

/// Lens model applied to normalized pinhole coordinates before intrinsics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LensDistortion {
    /// Rectilinear camera, or an image already corrected for lens distortion.
    #[default]
    None,
    /// OpenCV radial/tangential model: `[k1, k2, p1, p2, k3]`.
    BrownConrady([f64; 5]),
    /// OpenCV fisheye model: `[k1, k2, k3, k4]` in the theta polynomial.
    /// Zero coefficients still describe an equidistant fisheye, not a pinhole.
    Fisheye([f64; 4]),
}

/// Invalid camera calibration parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalibrationError;

impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("calibration requires non-empty image dimensions, positive finite focal lengths, and finite principal points and distortion coefficients")
    }
}

impl Error for CalibrationError {}

/// Validated calibration for one input image size and orientation.
///
/// `rectified_to_source` maps an ideal pinhole image to the original distorted
/// pixels. The virtual rectified image has the same dimensions as the input;
/// its intrinsics default to the input intrinsics. Adjust them explicitly to
/// select a wider/narrower rectified field of view. A single pinhole view cannot
/// cover a fisheye field of view of 180 degrees or more.
///
/// Intrinsics must describe the actual input buffer after cropping, resizing,
/// or rotation. Scanners reject a dimension mismatch rather than guessing the
/// image transform. Calibration quality and model invertibility are the caller's
/// responsibility; construction validates numerical parameters only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraCalibration {
    size: Size,
    intrinsics: CameraIntrinsics,
    rectified: CameraIntrinsics,
    distortion: LensDistortion,
}

impl CameraCalibration {
    /// Validate parameters for an input frame. Both cameras initially use `intrinsics`.
    pub fn new(
        size: Size,
        intrinsics: CameraIntrinsics,
        distortion: LensDistortion,
    ) -> Result<Self, CalibrationError> {
        let finite = match distortion {
            LensDistortion::None => true,
            LensDistortion::BrownConrady(k) => k.iter().all(|v| v.is_finite()),
            LensDistortion::Fisheye(k) => k.iter().all(|v| v.is_finite()),
        };
        if size.area().is_err() || !intrinsics.valid() || !finite {
            return Err(CalibrationError);
        }
        Ok(Self {
            size,
            intrinsics,
            rectified: intrinsics,
            distortion,
        })
    }

    /// Set the virtual pinhole camera's intrinsics, keeping the input calibration.
    /// A smaller focal length increases coverage but reduces pixels per module.
    pub fn with_rectified_intrinsics(
        mut self,
        intrinsics: CameraIntrinsics,
    ) -> Result<Self, CalibrationError> {
        if !intrinsics.valid() {
            return Err(CalibrationError);
        }
        self.rectified = intrinsics;
        Ok(self)
    }

    /// Dimensions of the input and full-resolution virtual rectified image.
    pub const fn image_size(&self) -> Size {
        self.size
    }

    /// Intrinsics of the distorted input image.
    pub const fn intrinsics(&self) -> CameraIntrinsics {
        self.intrinsics
    }

    /// Intrinsics of the virtual rectified image, before working downscaling.
    pub const fn rectified_intrinsics(&self) -> CameraIntrinsics {
        self.rectified
    }

    /// Lens model and coefficients of the input camera.
    pub const fn distortion(&self) -> LensDistortion {
        self.distortion
    }

    /// Map a full-resolution rectified pixel to the original distorted image.
    ///
    /// Returns `None` for non-finite inputs/results or a non-positive fisheye
    /// radial mapping. Finite coordinates outside the image are returned so the
    /// caller can choose its border policy. This is the forward lens projection,
    /// suitable for inverse image resampling without iterative lens inversion.
    pub fn rectified_to_source(&self, point: [f64; 2]) -> Option<[f64; 2]> {
        if !point.iter().all(|v| v.is_finite()) {
            return None;
        }
        if self.distortion == LensDistortion::None && self.intrinsics == self.rectified {
            return Some(point);
        }
        let x = (point[0] - self.rectified.cx) / self.rectified.fx;
        let y = (point[1] - self.rectified.cy) / self.rectified.fy;
        let (xd, yd) = match self.distortion {
            LensDistortion::None => (x, y),
            LensDistortion::BrownConrady([k1, k2, p1, p2, k3]) => {
                let r2 = x * x + y * y;
                let radial = 1.0 + r2 * (k1 + r2 * (k2 + r2 * k3));
                (
                    x * radial + 2.0 * p1 * x * y + p2 * (r2 + 2.0 * x * x),
                    y * radial + p1 * (r2 + 2.0 * y * y) + 2.0 * p2 * x * y,
                )
            }
            LensDistortion::Fisheye([k1, k2, k3, k4]) => {
                let r = x.hypot(y);
                if !r.is_finite() {
                    return None;
                }
                if r == 0.0 {
                    (0.0, 0.0)
                } else {
                    let theta = r.atan();
                    let t2 = theta * theta;
                    let td = theta * (1.0 + t2 * (k1 + t2 * (k2 + t2 * (k3 + t2 * k4))));
                    if td <= 0.0 {
                        return None;
                    }
                    let scale = td / r;
                    (x * scale, y * scale)
                }
            }
        };
        let p = [
            self.intrinsics.fx * xd + self.intrinsics.cx,
            self.intrinsics.fy * yd + self.intrinsics.cy,
        ];
        p.iter().all(|v| v.is_finite()).then_some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn camera(distortion: LensDistortion) -> CameraCalibration {
        CameraCalibration::new(
            Size::new(640, 480),
            CameraIntrinsics {
                fx: 400.0,
                fy: 420.0,
                cx: 319.5,
                cy: 239.5,
            },
            distortion,
        )
        .unwrap()
    }

    #[test]
    fn pinhole_identity_and_optical_center() {
        for model in [
            LensDistortion::None,
            LensDistortion::BrownConrady([0.0; 5]),
            LensDistortion::Fisheye([0.0; 4]),
        ] {
            assert_eq!(
                camera(model).rectified_to_source([319.5, 239.5]),
                Some([319.5, 239.5])
            );
        }
        assert_eq!(
            camera(LensDistortion::None).rectified_to_source([0.0, 479.0]),
            Some([0.0, 479.0])
        );
    }

    #[test]
    fn fisheye_zero_coefficients_are_equidistant() {
        let p = camera(LensDistortion::Fisheye([0.0; 4]))
            .rectified_to_source([719.5, 239.5])
            .unwrap();
        assert!((p[0] - (319.5 + 400.0 * std::f64::consts::FRAC_PI_4)).abs() < 1e-10);
        assert_eq!(p[1], 239.5);
    }

    #[test]
    fn radial_tangential_coefficient_order() {
        // x=0.5, y=-0.25; reference: cv2.projectPoints, OpenCV 4.12.0.
        let p = camera(LensDistortion::BrownConrady([
            0.1, -0.02, 0.003, -0.004, 0.005,
        ]))
        .rectified_to_source([519.5, 134.5])
        .unwrap();
        assert!((p[0] - 523.789892578125).abs() < 1e-10);
        assert!((p[1] - 132.37905639648437).abs() < 1e-10);
    }

    #[test]
    fn fisheye_matches_opencv_reference() {
        // cv2.fisheye.projectPoints, OpenCV 4.12.0, normalized point (.5,-.25,1).
        let p = camera(LensDistortion::Fisheye([0.01, -0.002, 0.0003, -0.00004]))
            .rectified_to_source([519.5, 134.5])
            .unwrap();
        assert!((p[0] - 502.32017337408627).abs() < 1e-10);
        assert!((p[1] - 143.5194089786047).abs() < 1e-10);
    }

    #[test]
    fn rejects_invalid_calibration_and_points() {
        let c = camera(LensDistortion::None);
        for fx in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(CameraCalibration::new(
                c.size,
                CameraIntrinsics { fx, ..c.intrinsics },
                c.distortion
            )
            .is_err());
        }
        assert!(CameraCalibration::new(Size::new(0, 4), c.intrinsics, c.distortion).is_err());
        assert!(CameraCalibration::new(
            c.size,
            c.intrinsics,
            LensDistortion::Fisheye([f64::NAN; 4])
        )
        .is_err());
        assert_eq!(c.rectified_to_source([f64::NAN, 0.0]), None);
        assert!(c
            .with_rectified_intrinsics(CameraIntrinsics {
                fy: 0.0,
                ..c.intrinsics
            })
            .is_err());
    }
}
