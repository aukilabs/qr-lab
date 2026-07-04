#![forbid(unsafe_code)]

mod alignment;
mod bitmatrix;
mod consts;
mod decode;
mod downscale;
mod finder;
mod homography;
mod luma;
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
pub use finder::{find_finders, FinderCandidate};
pub use homography::PerspectiveTransform;
pub use luma::{luma_from_rgba, LumaError, LumaView};
pub use scan::{scan, scan_traced, ScanOptions};
pub use scanner::{detect, detect_traced, detect_with, Detections, StageClock, StageTimings};
pub use tiles::TileGrid;
pub use trace::Trace;
#[cfg(feature = "debug-trace")]
pub use trace::TileTrace;
pub use triplet::{group_triplets, TripletCandidate};
