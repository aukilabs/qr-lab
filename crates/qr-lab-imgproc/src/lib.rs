//! Reusable grayscale image-processing operators for QR Lab.
//!
//! Public modules cover thresholding, morphology, illumination correction,
//! blur estimation, directional restoration, sharpening, and resizing. The
//! hidden `internal` module preserves byte-identical scanner kernels used by
//! the QR pipeline.

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![allow(rustdoc::private_intra_doc_links)]

use qr_lab_image::{ImageError, LumaError};
use std::error::Error;
use std::fmt;

/// Line-blur direction estimation and PSF-length measurement.
pub mod blur;
/// Directional Van Cittert and unsharp restoration along line PSFs.
pub mod deblur;
/// Multiplicative illumination normalization (background division).
pub mod illumination;
#[doc(hidden)]
pub mod internal;
/// Separable grayscale morphological operators.
pub mod morphology;
#[cfg(target_arch = "aarch64")]
#[doc(hidden)]
#[allow(unsafe_code)]
pub mod neon;
/// Resize, downscale, and working-resolution helpers.
pub mod resize;
/// Fixed binomial unsharp masking used by the scanner ladder.
pub mod sharpen;
/// Tile-local adaptive thresholds and Sauvola binarization.
pub mod threshold;

/// Errors produced by image-processing operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImgProcError {
    /// An underlying image layout or ROI error.
    Image(ImageError),
    /// Source and destination dimensions differ when they must match.
    DimensionMismatch,
    /// Destination width or height is zero.
    InvalidDestinationSize,
    /// Morphological or filter kernel size is invalid.
    InvalidKernel,
    /// Operator configuration is out of range or non-finite.
    InvalidConfiguration,
}

impl fmt::Display for ImgProcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Image(error) => return error.fmt(f),
            Self::DimensionMismatch => "source and destination dimensions do not match",
            Self::InvalidDestinationSize => "destination dimensions must be non-zero",
            Self::InvalidKernel => "kernel size must be odd and at least three",
            Self::InvalidConfiguration => "image-processing configuration is invalid",
        };
        f.write_str(message)
    }
}

impl Error for ImgProcError {}

impl From<ImageError> for ImgProcError {
    fn from(value: ImageError) -> Self {
        Self::Image(value)
    }
}

impl From<LumaError> for ImgProcError {
    fn from(value: LumaError) -> Self {
        Self::Image(ImageError::Layout(value))
    }
}

/// Convenience alias for image-processing results.
pub type ImgProcResult<T> = Result<T, ImgProcError>;
