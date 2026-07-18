//! Backward-compatible facade for the original `qr-lab-core` package.
//!
//! New applications should depend on [`qr_lab`](https://docs.rs/qr-lab) for
//! the umbrella API or on a focused `qr-lab-*` crate. Existing source code can
//! continue importing the complete scanner from `qr_lab_core` during the
//! compatibility cycle.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub use qr_lab_geometry as geometry;
pub use qr_lab_image as image;
pub use qr_lab_imgproc as imgproc;
pub use qr_lab_qr as qr;
#[doc(inline)]
pub use qr_lab_qr::DecodedCode;
#[doc(inline)]
pub use qr_lab_qr::PerspectiveTransform;
#[cfg(feature = "debug-trace")]
#[doc(inline)]
pub use qr_lab_qr::TileTrace;
#[doc(inline)]
pub use qr_lab_qr::Trace;
#[doc(inline)]
pub use qr_lab_qr::{decode_bits, BitMatrix, DecodeFailure, DecodedPayload};
#[doc(inline)]
pub use qr_lab_qr::{detect, detect_traced, detect_with, Detections, StageClock, StageTimings};
#[doc(inline)]
pub use qr_lab_qr::{downscale_luma, downscaled_dims};
#[doc(inline)]
pub use qr_lab_qr::{find_finders, FinderCandidate};
#[doc(inline)]
pub use qr_lab_qr::{group_triplets, TripletCandidate};
#[doc(inline)]
pub use qr_lab_qr::{luma_from_rgba, LumaError, LumaView};
#[doc(inline)]
pub use qr_lab_qr::{scan, scan_traced, ScanOptions};
#[doc(inline)]
pub use qr_lab_qr::{
    scan_robust, scan_robust_debug, RobustCode, RobustDebug, RobustDetections, ScanConfig,
    ScanSession, SessionConfig, VariantKind, VariantRecord, VariantSnapshot,
};
#[doc(hidden)]
pub use qr_lab_qr::{scan_robust_with_kernel, UpscaleKernel};
#[doc(inline)]
pub use qr_lab_qr::{BinarizeSpec, TileGrid};

// New facade types are additive to the compatibility surface.
#[doc(inline)]
pub use qr_lab_qr::{Scanner, ScannerConfig};
