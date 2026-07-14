//! Geometry and subpixel sampling shared by QRKit vision pipelines.

#![forbid(unsafe_code)]

mod homography;
mod line;
mod sampling;

pub use homography::PerspectiveTransform;
pub use line::{fit_line_tls, fit_line_tls_weighted, intersect_lines, FitLineError, Line2, Point2};
pub use sampling::{sample_bilinear, BorderMode};

/// Four points in top-left, top-right, bottom-right, bottom-left order.
pub type Quad = [[f64; 2]; 4];
