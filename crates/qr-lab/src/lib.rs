//! Umbrella API for QR Lab.
//!
//! Re-exports the focused crates so application code can depend on a single
//! package:
//!
//! - [`image`] — grayscale views and owned buffers
//! - [`geometry`] — homographies, line fitting, sampling
//! - [`imgproc`] — thresholding, morphology, restoration
//! - [`qr`] — QR detection, decoding, robust recovery, sessions
//!
//! The complete scanner types ([`Scanner`], [`ScannerConfig`], …) are also
//! re-exported at the crate root for a short import path.
//!
//! # Example
//!
//! ```
//! use qr_lab::image::Gray8View;
//! use qr_lab::{Scanner, ScannerConfig};
//!
//! let pixels = vec![255u8; 64 * 64];
//! let frame = Gray8View::new(&pixels, 64, 64, 64).unwrap();
//! let mut scanner = Scanner::new(ScannerConfig::robust_fast());
//! let _result = scanner.scan(&frame);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub use qr_lab_geometry as geometry;
pub use qr_lab_image as image;
pub use qr_lab_imgproc as imgproc;
pub use qr_lab_qr as qr;
pub use qr_lab_qr::*;
