//! Re-encoding large movies to HEVC.

pub mod args;
pub mod detect;
pub mod plan;
pub mod progress;
pub mod verify;

pub use detect::{Encoder, EncoderDetector};
