#![forbid(unsafe_code)]

mod bitmatrix;
mod consts;
mod finder;
mod homography;
mod luma;
mod scanner;
#[cfg(test)]
mod testpaint;
mod tiles;
mod trace;
mod triplet;
mod version;

pub use bitmatrix::{decode_bits, BitMatrix, DecodeFailure, DecodedPayload};
pub use finder::{find_finders, FinderCandidate};
pub use homography::PerspectiveTransform;
pub use luma::{luma_from_rgba, LumaError, LumaView};
pub use scanner::{detect, detect_traced, detect_with, Detections, StageClock, StageTimings};
pub use tiles::TileGrid;
pub use trace::Trace;
#[cfg(feature = "debug-trace")]
pub use trace::TileTrace;
pub use triplet::{group_triplets, TripletCandidate};
