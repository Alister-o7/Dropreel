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

/// Kind in the file names of TMDb posters, which says their size: older versions saved
/// 500-pixel posters as `-tmdb-`.
const TMDB_KIND: &str = "tmdb780";

/// Whether a poster is a frame grabbed from the movie (rather than artwork from TMDb).
pub fn is_frame_poster(poster: &Path) -> bool {
    poster
        .file_name()
        .is_some_and(|name| name.to_string_lossy().contains("-frame-"))
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
        let out = new_poster_path(&env.posters_dir, id, TMDB_KIND);
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

/// Re-grabs frame posters made smaller than [`thumbnail::FRAME_WIDTH`] (older versions grabbed
/// 480-pixel frames), when the movie itself is wider. Calls `on_upgrade` after each one and
/// returns how many were upgraded.
pub fn upgrade_frame_posters(env: &LibraryEnv, db: &Db, on_upgrade: &dyn Fn()) -> usize {
    if env.tools.ffmpeg.is_none() {
        return 0;
    }
    let movies = match db.all_movies() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("could not list movies to upgrade posters: {e:#}");
            return 0;
        }
    };
    let mut upgraded = 0;
    for movie in movies {
        let Some(poster) = movie.poster.as_deref() else {
            continue;
        };
        if !is_frame_poster(poster) || !poster.starts_with(&env.posters_dir) || !movie.path.exists()
        {
            continue;
        }
        let Ok((width, _)) = image::image_dimensions(poster) else {
            continue;
        };
        let video_width = movie.width.unwrap_or(0);
        if width >= thumbnail::FRAME_WIDTH || video_width <= width {
            continue;
        }
        // HDR isn't stored, so ask the file again: an HDR frame needs tone-mapping.
        let hdr = env
            .tools
            .ffprobe
            .as_deref()
            .and_then(|ffprobe| probe::probe(ffprobe, &movie.path).ok())
            .and_then(|media| media.video)
            .is_some_and(|v| v.is_hdr());
        match generate_thumbnail(env, db, movie.id, &movie.path, movie.duration, hdr) {
            Ok(()) => {
                upgraded += 1;
                on_upgrade();
            }
            Err(e) => tracing::warn!("could not upgrade the poster of \"{}\": {e}", movie.title),
        }
    }
    upgraded
}

/// Downloads TMDb posters again at the current size where older versions saved smaller ones. Calls `on_upgrade` after each one and returns how many were upgraded.
pub fn upgrade_tmdb_posters(env: &LibraryEnv, db: &Db, on_upgrade: &dyn Fn()) -> usize {
    if env.tmdb_key.is_empty() {
        return 0;
    }
    let movies = match db.all_movies() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("could not list movies to upgrade posters: {e:#}");
            return 0;
        }
    };
    let stale: Vec<_> = movies
        .into_iter()
        .filter(|m| {
            m.tmdb_id.is_some()
                && m.poster.as_deref().is_some_and(|p| {
                    let name = p.file_name().unwrap_or_default().to_string_lossy();
                    // Only our own downloads, from before posters were named by their size.
                    p.starts_with(&env.posters_dir) && name.contains("-tmdb-")
                })
        })
        .collect();
    if stale.is_empty() {
        return 0;
    }
    let client = match TmdbClient::new(&env.tmdb_key) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("could not upgrade TMDb posters: {e}");
            return 0;
        }
    };
    let mut upgraded = 0;
    for movie in stale {
        let Some(tmdb_id) = movie.tmdb_id else {
            continue;
        };
        let result = client.poster_path(tmdb_id).and_then(|path| {
            let Some(path) = path else { return Ok(false) };
            let out = new_poster_path(&env.posters_dir, movie.id, TMDB_KIND);
            client.download_poster(&path, &out)?;
            replace_poster(env, db, movie.id, &out).map_err(|e| e.to_string())?;
            Ok(true)
        });
        match result {
            Ok(true) => {
                upgraded += 1;
                on_upgrade();
            }
            Ok(false) => {}
            Err(e) => {
                tracing::warn!("could not upgrade the poster of \"{}\": {e}", movie.title);
                // Offline or a bad key: the rest would fail the same way.
                if e.contains("request failed") || e.contains("rejected") {
                    break;
                }
            }
        }
    }
    upgraded
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
