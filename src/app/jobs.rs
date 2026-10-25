//! Work that runs off the UI thread.

use std::path::PathBuf;

use super::events::{AppEvent, EventSink};
use crate::library::import::{self, LibraryEnv};
use crate::library::{Db, MovieId};
use crate::metadata;
use crate::metadata::tmdb::{TmdbClient, TmdbMovie};

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        tracing::error!("could not start {name} thread: {e}");
    }
}

/// Imports `paths`. `interactive` imports (drag & drop, Add buttons) report everything,
/// background ones (watched folders) stay quiet unless something was added.
pub fn import(env: LibraryEnv, events: EventSink, paths: Vec<PathBuf>, interactive: bool) {
    spawn("import", move || {
        let progress_events = events.clone();
        let change_events = events.clone();
        let report = import::run_import(
            &env,
            &paths,
            interactive,
            &move |p| progress_events.send(AppEvent::ImportProgress(p)),
            &move || change_events.send(AppEvent::LibraryChanged),
        )
        .map_err(|e| format!("{e:#}"));
        if let Err(e) = &report {
            tracing::error!("import failed: {e}");
        }
        events.send(AppEvent::ImportFinished {
            report,
            interactive,
        });
    });
}

/// Re-grabs low-resolution frame posters and downloads small TMDb posters again, from older
/// versions, refreshing the library as each one is done.
pub fn upgrade_posters(env: LibraryEnv, events: EventSink) {
    spawn("poster-upgrade", move || {
        let db = match Db::open(&env.db_path) {
            Ok(db) => db,
            Err(e) => {
                tracing::warn!("could not open the library to upgrade posters: {e:#}");
                return;
            }
        };
        let n = metadata::upgrade_frame_posters(&env, &db, &|| {
            events.send(AppEvent::LibraryChanged);
        });
        if n > 0 {
            tracing::info!("upgraded {n} frame posters to a higher resolution");
        }
        let n = metadata::upgrade_tmdb_posters(&env, &db, &|| {
            events.send(AppEvent::LibraryChanged);
        });
        if n > 0 {
            tracing::info!("downloaded {n} TMDb posters again at a higher resolution");
        }
    });
}

pub fn tmdb_search(
    key: String,
    events: EventSink,
    search_id: u64,
    query: String,
    year: Option<i32>,
) {
    spawn("tmdb-search", move || {
        let result = TmdbClient::new(&key).and_then(|c| c.search(&query, year));
        events.send(AppEvent::TmdbResults { search_id, result });
    });
}

pub fn tmdb_apply(env: LibraryEnv, events: EventSink, movie_id: MovieId, choice: TmdbMovie) {
    spawn("tmdb-apply", move || {
        let result = Db::open(&env.db_path)
            .map_err(|e| e.to_string())
            .and_then(|db| {
                let client = TmdbClient::new(&env.tmdb_key)?;
                metadata::apply_tmdb_match(&env, &db, movie_id, &client, &choice)
            });
        events.send(AppEvent::TmdbApplied { movie_id, result });
    });
}
