//! Geometry and subpixel sampling shared by QR Lab vision pipelines.
//!
//! This crate provides:
//! - projective homographies ([`PerspectiveTransform`])
//! - total-least-squares line fitting ([`fit_line_tls`])
//! - bilinear sampling with explicit border modes ([`sample_bilinear`])

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod homography;
mod line;
mod sampling;

pub use homography::PerspectiveTransform;
pub use line::{fit_line_tls, fit_line_tls_weighted, intersect_lines, FitLineError, Line2, Point2};
pub use sampling::{sample_bilinear, BorderMode};

/// Four points in top-left, top-right, bottom-right, bottom-left order.
pub type Quad = [[f64; 2]; 4];
