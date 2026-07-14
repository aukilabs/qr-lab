//! QR detection, decoding, robust recovery, and temporal scanning for QRKit.

// `deny` (not `forbid`) so the aarch64 NEON hot-path module can
// `#![allow(unsafe_code)]` for `std::arch::aarch64` intrinsics. Every other
// module stays safe, and the SIMD paths are tested against scalar fallbacks.
#![deny(unsafe_code)]
#![allow(rustdoc::private_intra_doc_links)]

mod alignment;
mod bitmatrix;
mod consts;
mod decode;
mod downscale;
mod enhance;
mod facade;
mod finder;
mod homography;
mod ladder;
mod luma;
#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
mod neon;
mod refine;
mod sample;
mod scan;
mod scanner;
#[cfg(test)]
mod testpaint;
mod tiles;
mod trace;
mod triplet;
mod version;

pub use bitmatrix::{decode_bits, BitMatrix, DecodeFailure, DecodedPayload};
pub use decode::DecodedCode;
pub use downscale::{downscale_luma, downscaled_dims};
pub use facade::{Scanner, ScannerConfig};
pub use finder::{find_finders, FinderCandidate};
pub use homography::PerspectiveTransform;
pub use ladder::{
    scan_robust, scan_robust_debug, RobustCode, RobustDebug, RobustDetections, ScanConfig,
    ScanSession, SessionConfig, VariantKind, VariantRecord, VariantSnapshot,
};
#[doc(hidden)]
pub use ladder::{scan_robust_with_kernel, UpscaleKernel};
pub use luma::{luma_from_rgba, LumaError, LumaView};
pub use scan::{scan, scan_traced, ScanOptions};
pub use scanner::{detect, detect_traced, detect_with, Detections, StageClock, StageTimings};
pub use tiles::{BinarizeSpec, TileGrid};
#[cfg(feature = "debug-trace")]
pub use trace::TileTrace;
pub use trace::Trace;
pub use triplet::{group_triplets, TripletCandidate};
