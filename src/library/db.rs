//! SQLite storage for the library.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use rusqlite::{Connection, OptionalExtension as _, Row, params};
use serde::{Deserialize, Serialize};

use crate::util::now_unix;

pub type MovieId = i64;
pub type TagId = i64;

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
];

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
    poster_path, tmdb_id";

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
    })
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
        Ok(self.conn.last_insert_rowid())
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
    /// resume position.
    pub fn replace_file(&self, id: MovieId, f: &FileInfo) -> anyhow::Result<()> {
        self.conn.execute(
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
}
