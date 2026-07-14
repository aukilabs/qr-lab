//! Reusable grayscale image-processing operators for QRKit.
//!
//! The public modules provide checked, composable APIs. The hidden `internal`
//! module temporarily preserves byte-identical scanner kernels while the
//! compatibility scanner is migrated.

#![deny(unsafe_code)]
#![allow(rustdoc::private_intra_doc_links)]

use qrkit_image::{ImageError, LumaError};
use std::error::Error;
use std::fmt;

pub mod blur;
pub mod deblur;
pub mod illumination;
#[doc(hidden)]
pub mod internal;
pub mod morphology;
#[cfg(target_arch = "aarch64")]
#[doc(hidden)]
#[allow(unsafe_code)]
pub mod neon;
pub mod resize;
pub mod sharpen;
pub mod threshold;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImgProcError {
    Image(ImageError),
    DimensionMismatch,
    InvalidDestinationSize,
    InvalidKernel,
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

pub type ImgProcResult<T> = Result<T, ImgProcError>;
