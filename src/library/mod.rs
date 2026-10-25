//! The movie library: database, importing, scanning and folder watching.

pub mod db;
pub mod fingerprint;
pub mod import;
pub mod scan;
pub mod watch;

pub use db::{Db, Movie, MovieId, NewMovie, Show, ShowId, Tag, TagId, TrackInfo};
