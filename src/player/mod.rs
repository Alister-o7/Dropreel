//! The built-in video player (libmpv rendering into an OpenGL texture shown by egui).

mod ffi;
mod mpv;

pub use mpv::{Player, PlayerEvent, Track, TrackKind, is_software_renderer};
