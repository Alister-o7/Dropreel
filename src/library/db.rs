//! SQLite storage for the library.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use rusqlite::{Connection, OptionalExtension as _, Row, params};
use serde::{Deserialize, Serialize};

use crate::metadata::filename::{self, EpisodeName};
use crate::util::now_unix;

pub type MovieId = i64;
pub type TagId = i64;
pub type ShowId = i64;

/// Schema migrations. Entry `i` upgrades the schema from version `i` to `i + 1`; the current
/// version is stored in SQLite's `user_version`. Never edit an entry that has shipped — add a
/// new one.
const MIGRATIONS: &[&str] = &[
    // v1: movies
    "CREATE TABLE movies (
        id            INTEGER PRIMARY KEY,
        path          TEXT NOT NULL UNIQUE,
        title         TEXT NOT NULL,
        year          INTEGER,
        plot          TEXT,
        size          INTEGER NOT NULL,
        duration      REAL,
        video_codec   TEXT,
        width         INTEGER,
        height        INTEGER,
        audio_tracks  TEXT NOT NULL DEFAULT '[]',
        subtitle_tracks TEXT NOT NULL DEFAULT '[]',
        fingerprint   TEXT NOT NULL,
        date_added    INTEGER NOT NULL,
        last_played   INTEGER,
        resume_pos    REAL,
        watched       INTEGER NOT NULL DEFAULT 0,
        poster_path   TEXT,
        tmdb_id       INTEGER
    );
    CREATE INDEX movies_fingerprint ON movies(fingerprint);",
    // v2: tags
    "CREATE TABLE tags (
        id   INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE
    );
    CREATE TABLE movie_tags (
        movie_id INTEGER NOT NULL REFERENCES movies(id) ON DELETE CASCADE,
        tag_id   INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
        PRIMARY KEY (movie_id, tag_id)
    );",
    // v3: files the user removed, so watched-folder scans do not add them back
    "CREATE TABLE ignored_paths (path TEXT PRIMARY KEY);",
    // v4: shows. An episode is a row in `movies` that belongs to a show.
    "CREATE TABLE shows (
        id          INTEGER PRIMARY KEY,
        title       TEXT NOT NULL UNIQUE COLLATE NOCASE,
        year        INTEGER,
        plot        TEXT,
        poster_path TEXT,
        tmdb_id     INTEGER
    );
    ALTER TABLE movies ADD COLUMN show_id INTEGER REFERENCES shows(id) ON DELETE SET NULL;
    ALTER TABLE movies ADD COLUMN season INTEGER;
    ALTER TABLE movies ADD COLUMN episode INTEGER;
    CREATE INDEX movies_show ON movies(show_id);",
];

/// The schema version that introduced shows; upgrading past it sorts existing episodes into shows.
const SHOWS_VERSION: usize = 4;

pub const SCHEMA_VERSION: usize = MIGRATIONS.len();

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub codec: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u32>,
}

impl TrackInfo {
    /// e.g. "eng · ac3 5.1 · Commentary"
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(lang) = &self.language {
            parts.push(lang.clone());
        }
        let mut codec = self.codec.clone();
        if let Some(ch) = self.channels {
            codec.push_str(match ch {
                1 => " mono",
                2 => " stereo",
                6 => " 5.1",
                8 => " 7.1",
                _ => "",
            });
        }
        parts.push(codec);
        if let Some(title) = &self.title {
            parts.push(title.clone());
        }
        parts.join(" · ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Movie {
    pub id: MovieId,
    pub path: PathBuf,
    pub title: String,
    pub year: Option<i32>,
    pub plot: Option<String>,
    pub size: u64,
    pub duration: Option<f64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio_tracks: Vec<TrackInfo>,
    pub subtitle_tracks: Vec<TrackInfo>,
    pub fingerprint: String,
    pub date_added: i64,
    pub last_played: Option<i64>,
    pub resume_pos: Option<f64>,
    pub watched: bool,
    pub poster: Option<PathBuf>,
    pub tmdb_id: Option<i64>,
    /// Set for an episode of a show; its `title` is then the episode's own title.
    pub show_id: Option<ShowId>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

impl Movie {
    /// "S01E02" for an episode.
    pub fn episode_code(&self) -> Option<String> {
        Some(format!("S{:02}E{:02}", self.season?, self.episode?))
    }
}

/// A show: its episodes are the movies with its `show_id`.
#[derive(Clone, Debug, PartialEq)]
pub struct Show {
    pub id: ShowId,
    pub title: String,
    pub year: Option<i32>,
    pub plot: Option<String>,
    pub poster: Option<PathBuf>,
    pub tmdb_id: Option<i64>,
}

/// Technical details of a file on disk, as stored for a movie.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub size: u64,
    pub fingerprint: String,
    pub duration: Option<f64>,
    pub video_codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio_tracks: Vec<TrackInfo>,
    pub subtitle_tracks: Vec<TrackInfo>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewMovie {
    pub file: FileInfo,
    pub title: String,
    pub year: Option<i32>,
    /// Set when the file is an episode of a show.
    pub episode: Option<EpisodeName>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
}

pub struct Db {
    conn: Connection,
}

fn path_to_sql(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn tracks_to_sql(tracks: &[TrackInfo]) -> String {
    serde_json::to_string(tracks).unwrap_or_else(|_| "[]".into())
}

fn tracks_from_sql(text: String) -> Vec<TrackInfo> {
    serde_json::from_str(&text).unwrap_or_default()
}

const MOVIE_COLUMNS: &str =
    "id, path, title, year, plot, size, duration, video_codec, width, height,
    audio_tracks, subtitle_tracks, fingerprint, date_added, last_played, resume_pos, watched,
    poster_path, tmdb_id, show_id, season, episode";

fn movie_from_row(row: &Row<'_>) -> rusqlite::Result<Movie> {
    Ok(Movie {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        title: row.get(2)?,
        year: row.get(3)?,
        plot: row.get(4)?,
        size: row.get::<_, i64>(5)?.max(0) as u64,
        duration: row.get(6)?,
        video_codec: row.get(7)?,
        width: row.get(8)?,
        height: row.get(9)?,
        audio_tracks: tracks_from_sql(row.get(10)?),
        subtitle_tracks: tracks_from_sql(row.get(11)?),
        fingerprint: row.get(12)?,
        date_added: row.get(13)?,
        last_played: row.get(14)?,
        resume_pos: row.get(15)?,
        watched: row.get(16)?,
        poster: row.get::<_, Option<String>>(17)?.map(PathBuf::from),
        tmdb_id: row.get(18)?,
        show_id: row.get(19)?,
        season: row.get(20)?,
        episode: row.get(21)?,
    })
}

fn show_from_row(row: &Row<'_>) -> rusqlite::Result<Show> {
    Ok(Show {
        id: row.get(0)?,
        title: row.get(1)?,
        year: row.get(2)?,
        plot: row.get(3)?,
        poster: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
        tmdb_id: row.get(5)?,
    })
}

/// An episode's title in the library: its own, or "Episode 3".
fn episode_title(ep: &EpisodeName) -> String {
    ep.title
        .clone()
        .unwrap_or_else(|| format!("Episode {}", ep.episode))
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("could not open the library database {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        // WAL lets the import/encode threads write while the UI reads.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> anyhow::Result<usize> {
        let v: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        Ok(v.max(0) as usize)
    }

    fn migrate(&mut self) -> anyhow::Result<()> {
        let current = self.schema_version()?;
        if current > SCHEMA_VERSION {
            bail!(
                "the library database was created by a newer version of DropReel (schema v{current}, this version supports v{SCHEMA_VERSION})"
            );
        }
        for (version, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("database migration to v{} failed", version + 1))?;
            tx.pragma_update(None, "user_version", (version + 1) as i64)?;
            tx.commit()?;
        }
        if current < SHOWS_VERSION && SCHEMA_VERSION >= SHOWS_VERSION {
            self.sort_episodes_into_shows()?;
        }
        Ok(())
    }

    /// Libraries from before shows: files named like episodes become episodes of their show.
    fn sort_episodes_into_shows(&self) -> anyhow::Result<()> {
        let rows: Vec<(MovieId, PathBuf)> = {
            let mut stmt = self.conn.prepare("SELECT id, path FROM movies")?;
            stmt.query_map([], |r| {
                Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?)))
            })?
            .collect::<Result<_, _>>()?
        };
        let tx = self.conn.unchecked_transaction()?;
        for (id, path) in rows {
            if let Some(ep) = filename::parse_episode(&path) {
                // Movie metadata matched from TMDb doesn't belong to an episode.
                self.set_episode(id, &ep)?;
                tx.execute(
                    "UPDATE movies SET tmdb_id = NULL, plot = NULL WHERE id = ?1",
                    [id],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // ----- movies -----

    pub fn all_movies(&self) -> anyhow::Result<Vec<Movie>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {MOVIE_COLUMNS} FROM movies"))?;
        let movies = stmt
            .query_map([], movie_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(movies)
    }

    pub fn movie(&self, id: MovieId) -> anyhow::Result<Option<Movie>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {MOVIE_COLUMNS} FROM movies WHERE id = ?1"),
                [id],
                movie_from_row,
            )
            .optional()?)
    }

    pub fn find_by_path(&self, path: &Path) -> anyhow::Result<Option<MovieId>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM movies WHERE path = ?1",
                [path_to_sql(path)],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// All stored paths, for skipping known files quickly during an import.
    pub fn all_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM movies")?;
        let paths = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|p| p.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(paths)
    }

    /// Movies with this fingerprint, as `(id, path, title)`.
    pub fn find_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> anyhow::Result<Vec<(MovieId, PathBuf, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, title FROM movies WHERE fingerprint = ?1")?;
        let rows = stmt
            .query_map([fingerprint], |r| {
                Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?), r.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_movie(&self, movie: &NewMovie) -> anyhow::Result<MovieId> {
        let f = &movie.file;
        self.conn.execute(
            "INSERT INTO movies (path, title, year, size, duration, video_codec, width, height,
                audio_tracks, subtitle_tracks, fingerprint, date_added)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                path_to_sql(&f.path),
                movie.title,
                movie.year,
                f.size as i64,
                f.duration,
                f.video_codec,
                f.width,
                f.height,
                tracks_to_sql(&f.audio_tracks),
                tracks_to_sql(&f.subtitle_tracks),
                f.fingerprint,
                now_unix(),
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        if let Some(ep) = &movie.episode {
            self.set_episode(id, ep)?;
        }
        Ok(id)
    }

    /// Points a movie at a new location of the same file.
    pub fn relink(
        &self,
        id: MovieId,
        path: &Path,
        size: u64,
        fingerprint: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET path = ?2, size = ?3, fingerprint = ?4 WHERE id = ?1",
            params![id, path_to_sql(path), size as i64, fingerprint],
        )?;
        Ok(())
    }

    /// Replaces the file behind a movie (after a re-encode), keeping its metadata, tags and
    /// resume position. A watched folder may already have added the new file as a movie of its
    /// own; that entry is dropped. The old file is remembered as removed, so watched folders
    /// don't add it back as a second copy of the movie (an explicit add still can).
    pub fn replace_file(&self, id: MovieId, f: &FileInfo) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO ignored_paths (path) SELECT path FROM movies WHERE id = ?1",
            [id],
        )?;
        tx.execute(
            "DELETE FROM movies WHERE path = ?2 AND id != ?1",
            params![id, path_to_sql(&f.path)],
        )?;
        tx.execute(
            "UPDATE movies SET path = ?2, size = ?3, fingerprint = ?4, duration = ?5,
                video_codec = ?6, width = ?7, height = ?8, audio_tracks = ?9, subtitle_tracks = ?10
             WHERE id = ?1",
            params![
                id,
                path_to_sql(&f.path),
                f.size as i64,
                f.fingerprint,
                f.duration,
                f.video_codec,
                f.width,
                f.height,
                tracks_to_sql(&f.audio_tracks),
                tracks_to_sql(&f.subtitle_tracks),
            ],
        )?;
        tx.execute(
            "DELETE FROM ignored_paths WHERE path = ?1",
            [path_to_sql(&f.path)],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_poster(&self, id: MovieId, poster: Option<&Path>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET poster_path = ?2 WHERE id = ?1",
            params![id, poster.map(path_to_sql)],
        )?;
        Ok(())
    }

    pub fn set_title_year(
        &self,
        id: MovieId,
        title: &str,
        year: Option<i32>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET title = ?2, year = ?3 WHERE id = ?1",
            params![id, title, year],
        )?;
        Ok(())
    }

    pub fn set_tmdb_metadata(
        &self,
        id: MovieId,
        tmdb_id: i64,
        title: &str,
        year: Option<i32>,
        plot: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET tmdb_id = ?2, title = ?3, year = ?4, plot = ?5 WHERE id = ?1",
            params![id, tmdb_id, title, year, plot],
        )?;
        Ok(())
    }

    pub fn set_resume(&self, id: MovieId, position: Option<f64>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET resume_pos = ?2 WHERE id = ?1",
            params![id, position],
        )?;
        Ok(())
    }

    pub fn mark_played(&self, id: MovieId) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET last_played = ?2 WHERE id = ?1",
            params![id, now_unix()],
        )?;
        Ok(())
    }

    pub fn set_watched(&self, id: MovieId, watched: bool) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET watched = ?2 WHERE id = ?1",
            params![id, watched],
        )?;
        Ok(())
    }

    // ----- shows -----

    pub fn all_shows(&self) -> anyhow::Result<Vec<Show>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, year, plot, poster_path, tmdb_id FROM shows")?;
        let shows = stmt
            .query_map([], show_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(shows)
    }

    pub fn show(&self, id: ShowId) -> anyhow::Result<Option<Show>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, title, year, plot, poster_path, tmdb_id FROM shows WHERE id = ?1",
                [id],
                show_from_row,
            )
            .optional()?)
    }

    /// The show called `title` (ignoring case), created if it's new.
    pub fn show_named(&self, title: &str, year: Option<i32>) -> anyhow::Result<ShowId> {
        let existing: Option<ShowId> = self
            .conn
            .query_row("SELECT id FROM shows WHERE title = ?1", [title], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(id) = existing {
            if year.is_some() {
                self.conn.execute(
                    "UPDATE shows SET year = ?2 WHERE id = ?1 AND year IS NULL",
                    params![id, year],
                )?;
            }
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO shows (title, year) VALUES (?1, ?2)",
            params![title, year],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Makes a movie an episode of its show (created if needed).
    pub fn set_episode(&self, id: MovieId, ep: &EpisodeName) -> anyhow::Result<ShowId> {
        let show = self.show_named(&ep.show, ep.year)?;
        self.conn.execute(
            "UPDATE movies SET show_id = ?2, season = ?3, episode = ?4, title = ?5, year = NULL
             WHERE id = ?1",
            params![id, show, ep.season, ep.episode, episode_title(ep)],
        )?;
        Ok(show)
    }

    pub fn set_show_poster(&self, id: ShowId, poster: Option<&Path>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE shows SET poster_path = ?2 WHERE id = ?1",
            params![id, poster.map(path_to_sql)],
        )?;
        Ok(())
    }

    pub fn set_show_tmdb_metadata(
        &self,
        id: ShowId,
        tmdb_id: i64,
        year: Option<i32>,
        plot: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE shows SET tmdb_id = ?2, year = COALESCE(?3, year), plot = ?4 WHERE id = ?1",
            params![id, tmdb_id, year, plot],
        )?;
        Ok(())
    }

    /// Updates an episode's title (from TMDb) and plot.
    pub fn set_episode_details(
        &self,
        id: MovieId,
        title: &str,
        plot: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE movies SET title = ?2, plot = ?3 WHERE id = ?1",
            params![id, title, plot],
        )?;
        Ok(())
    }

    /// Deletes shows with no episodes left, returning their cached posters.
    pub fn remove_empty_shows(&self) -> anyhow::Result<Vec<PathBuf>> {
        let posters: Vec<PathBuf> = {
            let mut stmt = self.conn.prepare(
                "SELECT poster_path FROM shows WHERE poster_path IS NOT NULL
                 AND id NOT IN (SELECT show_id FROM movies WHERE show_id IS NOT NULL)",
            )?;
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .map(|p| p.map(PathBuf::from))
                .collect::<Result<_, _>>()?
        };
        self.conn.execute(
            "DELETE FROM shows WHERE id NOT IN (SELECT show_id FROM movies WHERE show_id IS NOT NULL)",
            [],
        )?;
        Ok(posters)
    }

    /// Removes a movie from the library. The file on disk is not touched. Its path is remembered
    /// so background scans of watched folders do not add it again.
    pub fn remove_movie(&self, id: MovieId) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO ignored_paths (path) SELECT path FROM movies WHERE id = ?1",
            [id],
        )?;
        self.conn
            .execute("DELETE FROM movies WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn ignored_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM ignored_paths")?;
        let paths = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|p| p.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(paths)
    }

    /// Forgets that a path was removed (the user added it again explicitly).
    pub fn unignore_path(&self, path: &Path) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM ignored_paths WHERE path = ?1",
            [path_to_sql(path)],
        )?;
        Ok(())
    }

    // ----- tags -----

    pub fn all_tags(&self) -> anyhow::Result<Vec<Tag>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name FROM tags ORDER BY name COLLATE NOCASE")?;
        let tags = stmt
            .query_map([], |r| {
                Ok(Tag {
                    id: r.get(0)?,
                    name: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tags)
    }

    /// Creates a tag, or returns the existing one with the same name (case-insensitive).
    pub fn create_tag(&self, name: &str) -> anyhow::Result<TagId> {
        let name = name.trim();
        if name.is_empty() {
            bail!("tag names cannot be empty");
        }
        self.conn
            .execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])?;
        Ok(self
            .conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [name], |r| r.get(0))?)
    }

    /// Renames a tag. Renaming it to another tag's name (ignoring case) merges the two: its movies
    /// join that tag and this one goes away. Returns the tag that now has the name.
    pub fn rename_tag(&self, id: TagId, name: &str) -> anyhow::Result<TagId> {
        let name = name.trim();
        if name.is_empty() {
            bail!("tag names cannot be empty");
        }
        let other: Option<TagId> = self
            .conn
            .query_row(
                "SELECT id FROM tags WHERE name = ?1 AND id != ?2",
                params![name, id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(other) = other else {
            self.conn
                .execute("UPDATE tags SET name = ?2 WHERE id = ?1", params![id, name])?;
            return Ok(id);
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO movie_tags (movie_id, tag_id)
             SELECT movie_id, ?2 FROM movie_tags WHERE tag_id = ?1",
            [id, other],
        )?;
        tx.execute("DELETE FROM tags WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(other)
    }

    pub fn delete_tag(&self, id: TagId) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM tags WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_movie_tag(&self, movie: MovieId, tag: TagId, on: bool) -> anyhow::Result<()> {
        if on {
            self.conn.execute(
                "INSERT OR IGNORE INTO movie_tags (movie_id, tag_id) VALUES (?1, ?2)",
                [movie, tag],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM movie_tags WHERE movie_id = ?1 AND tag_id = ?2",
                [movie, tag],
            )?;
        }
        Ok(())
    }

    pub fn movie_tags(&self) -> anyhow::Result<HashMap<MovieId, BTreeSet<TagId>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT movie_id, tag_id FROM movie_tags")?;
        let mut map: HashMap<MovieId, BTreeSet<TagId>> = HashMap::new();
        for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
            let (movie, tag) = row?;
            map.entry(movie).or_default().insert(tag);
        }
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(path: &str, fingerprint: &str) -> NewMovie {
        NewMovie {
            file: FileInfo {
                path: PathBuf::from(path),
                size: 6_000_000_000,
                fingerprint: fingerprint.into(),
                duration: Some(7200.0),
                video_codec: Some("h264".into()),
                width: Some(1920),
                height: Some(1080),
                audio_tracks: vec![TrackInfo {
                    codec: "ac3".into(),
                    language: Some("eng".into()),
                    title: None,
                    channels: Some(6),
                }],
                subtitle_tracks: vec![],
            },
            title: "Movie".into(),
            year: Some(2019),
            episode: None,
        }
    }

    #[test]
    fn fresh_database_is_migrated_to_the_latest_schema() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(db.all_movies().unwrap().is_empty());
        assert!(db.all_tags().unwrap().is_empty());
    }

    #[test]
    fn reopening_keeps_data_and_does_not_rerun_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let id = Db::open(&path)
            .unwrap()
            .insert_movie(&sample("C:/a.mkv", "fp"))
            .unwrap();
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(db.movie(id).unwrap().unwrap().title, "Movie");
    }

    #[test]
    fn upgrading_from_v1_keeps_existing_movies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO movies (path, title, size, fingerprint, date_added) VALUES ('C:/old.mkv', 'Old', 1, 'f', 0)",
                [],
            )
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        let movies = db.all_movies().unwrap();
        assert_eq!(movies.len(), 1);
        assert_eq!(movies[0].title, "Old");
        let tag = db.create_tag("Favourites").unwrap();
        db.set_movie_tag(movies[0].id, tag, true).unwrap();
    }

    #[test]
    fn database_from_a_newer_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();
        let err = Db::open(&path).err().unwrap();
        assert!(err.to_string().contains("newer version"), "{err}");
    }

    #[test]
    fn movies_round_trip_and_can_be_updated() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_movie(&sample("C:/a.mkv", "fp1")).unwrap();
        let movie = db.movie(id).unwrap().unwrap();
        assert_eq!(movie.audio_tracks[0].channels, Some(6));
        assert_eq!(db.find_by_path(Path::new("C:/a.mkv")).unwrap(), Some(id));
        assert_eq!(db.find_by_fingerprint("fp1").unwrap().len(), 1);

        db.set_resume(id, Some(123.5)).unwrap();
        db.relink(id, Path::new("D:/moved.mkv"), 5, "fp2").unwrap();
        let movie = db.movie(id).unwrap().unwrap();
        assert_eq!(movie.path, PathBuf::from("D:/moved.mkv"));
        assert_eq!(movie.resume_pos, Some(123.5));

        // A duplicate path is rejected by the UNIQUE constraint.
        assert!(db.insert_movie(&sample("D:/moved.mkv", "x")).is_err());
    }

    #[test]
    fn a_re_encode_replaces_the_file_without_leaving_a_second_copy() {
        let db = Db::open_in_memory().unwrap();
        let id = db
            .insert_movie(&sample("G:/Films/Heat.mkv", "big"))
            .unwrap();
        db.set_resume(id, Some(600.0)).unwrap();
        // A watched folder spotted the new file first and added it as a movie of its own.
        let early = db
            .insert_movie(&sample("G:/Films/Heat.hevc.mkv", "small"))
            .unwrap();

        let mut encoded = sample("G:/Films/Heat.hevc.mkv", "small").file;
        encoded.size = 4_000_000_000;
        db.replace_file(id, &encoded).unwrap();

        let movies = db.all_movies().unwrap();
        assert_eq!(movies.len(), 1, "{movies:?}");
        assert_eq!(movies[0].id, id);
        assert_eq!(movies[0].size, 4_000_000_000);
        assert_eq!(movies[0].resume_pos, Some(600.0));
        assert!(db.movie(early).unwrap().is_none());
        // Watched folders won't add the original back while it's still on disk.
        assert_eq!(
            db.ignored_paths().unwrap(),
            vec![PathBuf::from("G:/Films/Heat.mkv")]
        );
    }

    fn episode_of(show: &str, season: u32, episode: u32) -> EpisodeName {
        EpisodeName {
            show: show.into(),
            year: None,
            season,
            episode,
            title: None,
        }
    }

    #[test]
    fn episodes_belong_to_their_show() {
        let db = Db::open_in_memory().unwrap();
        let mut first = sample("D:/TV/Dark.S01E01.mkv", "a");
        first.episode = Some(EpisodeName {
            title: Some("Secrets".into()),
            ..episode_of("Dark", 1, 1)
        });
        let mut second = sample("D:/TV/dark.s01e02.mkv", "b");
        second.episode = Some(episode_of("dark", 1, 2));
        let a = db.insert_movie(&first).unwrap();
        let b = db.insert_movie(&second).unwrap();
        let movie = db.insert_movie(&sample("D:/Movies/Heat.mkv", "c")).unwrap();

        let shows = db.all_shows().unwrap();
        assert_eq!(shows.len(), 1, "show names ignore case");
        assert_eq!(shows[0].title, "Dark");
        let a = db.movie(a).unwrap().unwrap();
        let b = db.movie(b).unwrap().unwrap();
        assert_eq!(a.show_id, Some(shows[0].id));
        assert_eq!(
            (a.title.as_str(), a.episode_code().as_deref()),
            ("Secrets", Some("S01E01"))
        );
        assert_eq!(b.title, "Episode 2");
        assert_eq!(db.movie(movie).unwrap().unwrap().show_id, None);

        // A show goes once its last episode does.
        db.set_show_poster(shows[0].id, Some(Path::new("/p/dark.jpg")))
            .unwrap();
        db.remove_movie(a.id).unwrap();
        assert!(db.remove_empty_shows().unwrap().is_empty());
        db.remove_movie(b.id).unwrap();
        assert_eq!(
            db.remove_empty_shows().unwrap(),
            vec![PathBuf::from("/p/dark.jpg")]
        );
        assert!(db.all_shows().unwrap().is_empty());
    }

    #[test]
    fn upgrading_sorts_existing_episodes_into_shows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        {
            let conn = Connection::open(&path).unwrap();
            for sql in &MIGRATIONS[..3] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 3).unwrap();
            for (file, title) in [
                ("D:/TV/Severance/Season 1/S01E02.mkv", "S01E02"),
                (
                    "D:/TV/Severance.S01E01.Good.News.About.Hell.mkv",
                    "Severance",
                ),
                ("D:/Movies/Heat.1995.mkv", "Heat"),
            ] {
                conn.execute(
                    "INSERT INTO movies (path, title, size, fingerprint, date_added, tmdb_id)
                     VALUES (?1, ?2, 1, ?1, 0, 42)",
                    [file, title],
                )
                .unwrap();
            }
        }
        let db = Db::open(&path).unwrap();
        let shows = db.all_shows().unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].title, "Severance");
        let mut movies = db.all_movies().unwrap();
        movies.sort_by_key(|m| (m.season, m.episode));
        let summary: Vec<_> = movies
            .iter()
            .map(|m| (m.title.as_str(), m.episode_code(), m.tmdb_id))
            .collect();
        assert_eq!(
            summary,
            [
                ("Heat", None, Some(42)),
                ("Good News About Hell", Some("S01E01".into()), None),
                ("Episode 2", Some("S01E02".into()), None),
            ]
        );
    }

    #[test]
    fn tags_are_case_insensitive_and_removed_with_their_movie() {
        let db = Db::open_in_memory().unwrap();
        let id = db.insert_movie(&sample("C:/a.mkv", "fp")).unwrap();
        let a = db.create_tag("Sci-Fi").unwrap();
        assert_eq!(db.create_tag("sci-fi").unwrap(), a);
        db.set_movie_tag(id, a, true).unwrap();
        assert!(db.movie_tags().unwrap()[&id].contains(&a));
        db.remove_movie(id).unwrap();
        assert!(db.movie_tags().unwrap().is_empty());
        assert_eq!(db.ignored_paths().unwrap(), vec![PathBuf::from("C:/a.mkv")]);
        assert_eq!(db.all_tags().unwrap().len(), 1);
    }

    #[test]
    fn renaming_a_tag_to_an_existing_name_merges_them() {
        let db = Db::open_in_memory().unwrap();
        let a = db.insert_movie(&sample("C:/a.mkv", "fa")).unwrap();
        let b = db.insert_movie(&sample("C:/b.mkv", "fb")).unwrap();
        let scifi = db.create_tag("Sci-Fi").unwrap();
        let space = db.create_tag("Space").unwrap();
        db.set_movie_tag(a, scifi, true).unwrap();
        db.set_movie_tag(a, space, true).unwrap();
        db.set_movie_tag(b, space, true).unwrap();

        // A plain rename keeps the tag and its movies.
        assert_eq!(db.rename_tag(scifi, "  Science fiction ").unwrap(), scifi);
        assert_eq!(db.all_tags().unwrap()[0].name, "Science fiction");
        assert!(db.rename_tag(scifi, " ").is_err());

        // Renaming "Space" to an existing name (any case) moves its movies there.
        assert_eq!(db.rename_tag(space, "science FICTION").unwrap(), scifi);
        let tags = db.all_tags().unwrap();
        assert_eq!(tags.len(), 1);
        let links = db.movie_tags().unwrap();
        assert_eq!(links[&a], BTreeSet::from([scifi]));
        assert_eq!(links[&b], BTreeSet::from([scifi]));

        db.delete_tag(scifi).unwrap();
        assert!(db.all_tags().unwrap().is_empty());
        assert!(db.movie_tags().unwrap().is_empty());
    }
}
