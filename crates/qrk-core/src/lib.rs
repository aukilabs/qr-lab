//! Backward-compatible facade for the original `qrk-core` package.
//!
//! New applications should depend on `qrkit` for the umbrella API or on a
//! focused `qrkit-*` crate. Existing source code can continue importing the
//! complete scanner from `qrk_core` during the compatibility cycle.

#![forbid(unsafe_code)]

pub use qrkit_geometry as geometry;
pub use qrkit_image as image;
pub use qrkit_imgproc as imgproc;
pub use qrkit_qr as qr;
#[doc(inline)]
pub use qrkit_qr::DecodedCode;
#[doc(inline)]
pub use qrkit_qr::PerspectiveTransform;
#[cfg(feature = "debug-trace")]
#[doc(inline)]
pub use qrkit_qr::TileTrace;
#[doc(inline)]
pub use qrkit_qr::Trace;
#[doc(inline)]
pub use qrkit_qr::{decode_bits, BitMatrix, DecodeFailure, DecodedPayload};
#[doc(inline)]
pub use qrkit_qr::{detect, detect_traced, detect_with, Detections, StageClock, StageTimings};
#[doc(inline)]
pub use qrkit_qr::{downscale_luma, downscaled_dims};
#[doc(inline)]
pub use qrkit_qr::{find_finders, FinderCandidate};
#[doc(inline)]
pub use qrkit_qr::{group_triplets, TripletCandidate};
#[doc(inline)]
pub use qrkit_qr::{luma_from_rgba, LumaError, LumaView};
#[doc(inline)]
pub use qrkit_qr::{scan, scan_traced, ScanOptions};
#[doc(inline)]
pub use qrkit_qr::{
    scan_robust, scan_robust_debug, RobustCode, RobustDebug, RobustDetections, ScanConfig,
    ScanSession, SessionConfig, VariantKind, VariantRecord, VariantSnapshot,
};
#[doc(hidden)]
pub use qrkit_qr::{scan_robust_with_kernel, UpscaleKernel};
#[doc(inline)]
pub use qrkit_qr::{BinarizeSpec, TileGrid};

// New facade types are additive to the compatibility surface.
#[doc(inline)]
pub use qrkit_qr::{Scanner, ScannerConfig};
