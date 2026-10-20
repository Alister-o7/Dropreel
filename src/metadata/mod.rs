//! Titles, posters and technical details for movies.

pub mod filename;
pub mod probe;
pub mod thumbnail;
pub mod tmdb;

use std::path::{Path, PathBuf};

use crate::library::import::LibraryEnv;
use crate::library::{Db, MovieId};
use tmdb::{TmdbClient, TmdbMovie};

/// A fresh poster file name. The timestamp makes the image URI change when a poster is
/// replaced, so the UI does not show a cached old image.
fn new_poster_path(posters: &Path, id: MovieId, kind: &str) -> PathBuf {
    let stamp = chrono::Utc::now().timestamp_millis();
    posters.join(format!("{id}-{kind}-{stamp}.jpg"))
}

/// Deletes a poster we created ourselves (never anything outside the cache folder).
pub fn delete_cached_poster(posters: &Path, poster: Option<&Path>) {
    if let Some(p) = poster
        && p.starts_with(posters)
    {
        let _ = std::fs::remove_file(p);
    }
}

fn replace_poster(env: &LibraryEnv, db: &Db, id: MovieId, new: &Path) -> anyhow::Result<()> {
    let old = db.movie(id)?.and_then(|m| m.poster);
    db.set_poster(id, Some(new))?;
    if old.as_deref() != Some(new) {
        delete_cached_poster(&env.posters_dir, old.as_deref());
    }
    Ok(())
}

/// Saves a TMDb match: title, year, plot and poster.
pub fn apply_tmdb_match(
    env: &LibraryEnv,
    db: &Db,
    id: MovieId,
    client: &TmdbClient,
    m: &TmdbMovie,
) -> Result<(), String> {
    db.set_tmdb_metadata(
        id,
        m.id,
        &m.title,
        m.year(),
        m.overview.as_deref().filter(|o| !o.is_empty()),
    )
    .map_err(|e| e.to_string())?;
    if let Some(poster) = &m.poster_path {
        let out = new_poster_path(&env.posters_dir, id, "tmdb");
        client.download_poster(poster, &out)?;
        replace_poster(env, db, id, &out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Grabs a frame as the poster.
pub fn generate_thumbnail(
    env: &LibraryEnv,
    db: &Db,
    id: MovieId,
    file: &Path,
    duration: Option<f64>,
    hdr: bool,
) -> Result<(), String> {
    let ffmpeg = env.tools.ffmpeg()?;
    let out = new_poster_path(&env.posters_dir, id, "frame");
    thumbnail::grab_frame(ffmpeg, file, duration, hdr, &out)?;
    replace_poster(env, db, id, &out).map_err(|e| e.to_string())
}

/// Metadata for a newly imported movie: TMDb if a key is set, otherwise (or when TMDb has no
/// poster) a frame grab. `hdr` makes the frame grab tone-map. Returns a warning if something
/// went wrong.
pub fn fetch_for_new_movie(env: &LibraryEnv, db: &Db, id: MovieId, hdr: bool) -> Option<String> {
    let movie = db.movie(id).ok()??;
    let mut warning = None;
    let mut has_poster = false;
    if !env.tmdb_key.is_empty() {
        let result = TmdbClient::new(&env.tmdb_key).and_then(|client| {
            match client.find_best(&movie.title, movie.year)? {
                Some(m) => {
                    has_poster = m.poster_path.is_some();
                    apply_tmdb_match(env, db, id, &client, &m)
                }
                None => Ok(()),
            }
        });
        if let Err(e) = result {
            warning = Some(e);
            has_poster = false;
        }
    }
    if !has_poster
        && env.tools.ffmpeg.is_some()
        && let Err(e) = generate_thumbnail(env, db, id, &movie.path, movie.duration, hdr)
    {
        warning.get_or_insert(e);
    }
    warning
}
