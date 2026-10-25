//! Messages from background threads to the UI thread.

use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;

use crate::library::MovieId;
use crate::library::import::{ImportProgress, ImportReport};
use crate::metadata::tmdb::TmdbMovie;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub enum AppEvent {
    /// The database changed; reload the movie list.
    LibraryChanged,
    ImportProgress(ImportProgress),
    ImportFinished {
        report: Result<ImportReport, String>,
        interactive: bool,
    },
    /// New files appeared in a watched folder.
    WatchedFiles(Vec<PathBuf>),
    TmdbResults {
        search_id: u64,
        result: Result<Vec<TmdbMovie>, String>,
    },
    TmdbApplied {
        movie_id: MovieId,
        result: Result<(), String>,
    },
    /// A show was looked up on TMDb; carries its title.
    ShowMetadataFetched {
        result: Result<String, String>,
    },
    Toast(ToastKind, String),
    EncodePreview(Box<crate::encode::EncodePreview>),
    Encode(crate::encode::EncodeEvent),
}

/// Sends events to the UI and wakes it up.
#[derive(Clone)]
pub struct EventSink {
    tx: Sender<AppEvent>,
    ctx: egui::Context,
}

impl EventSink {
    pub fn new(ctx: egui::Context) -> (Self, Receiver<AppEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (Self { tx, ctx }, rx)
    }

    pub fn send(&self, event: AppEvent) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }
}
