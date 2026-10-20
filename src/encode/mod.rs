//! Re-encoding large movies to HEVC.

pub mod args;
pub mod detect;
pub mod plan;
pub mod progress;
pub mod queue;
pub mod verify;

pub use detect::{Encoder, EncoderDetector};
pub use queue::{
    EncodeConfig, EncodeEvent, EncodeJob, EncodeOutcome, EncodePreview, EncodeQueue, EncodeStatus,
    JobState, Stage,
};
