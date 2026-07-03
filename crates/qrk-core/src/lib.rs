#![forbid(unsafe_code)]

mod consts;
mod finder;
mod homography;
mod luma;
#[cfg(test)]
mod testpaint;
mod tiles;
mod triplet;

pub use finder::{find_finders, FinderCandidate};
pub use homography::PerspectiveTransform;
pub use luma::{luma_from_rgba, LumaError, LumaView};
pub use tiles::TileGrid;
pub use triplet::{group_triplets, TripletCandidate};
