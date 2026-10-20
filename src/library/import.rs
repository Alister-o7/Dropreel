//! Adding files to the library.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;

use super::db::FileInfo;
use super::{Db, MovieId, NewMovie, fingerprint, scan};
use crate::metadata::{self, filename, probe};
use crate::tools::Tools;

/// What the import (and metadata) code needs to know about the environment.
#[derive(Clone, Debug)]
pub struct LibraryEnv {
    pub db_path: PathBuf,
    pub posters_dir: PathBuf,
    pub tools: Tools,
    /// Empty when TMDb is disabled.
    pub tmdb_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportProgress {
    Scanning,
    Reading { done: usize, total: usize },
    Metadata { done: usize, total: usize },
}

#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    pub added: Vec<String>,
    /// Missing movies found at a new location: (title, new path).
    pub relinked: Vec<(String, PathBuf)>,
    /// Files that were not added, with the reason (e.g. duplicates).
    pub skipped: Vec<(PathBuf, String)>,
    /// Files that were added, but with a problem (e.g. ffprobe could not read them).
    pub warnings: Vec<(PathBuf, String)>,
    pub failed: Vec<(PathBuf, String)>,
    /// One-off notes about the whole import.
    pub notes: Vec<String>,
}

impl ImportReport {
    /// True when there is more to say than "N movies added".
    pub fn needs_attention(&self) -> bool {
        !(self.relinked.is_empty()
            && self.skipped.is_empty()
            && self.warnings.is_empty()
            && self.failed.is_empty()
            && self.notes.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && !self.needs_attention()
    }

    pub fn summary(&self) -> String {
        let plural = |n: usize, one: &str, many: &str| {
            if n == 1 {
                format!("1 {one}")
            } else {
                format!("{n} {many}")
            }
        };
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!(
                "added {}",
                plural(self.added.len(), "movie", "movies")
            ));
        }
        if !self.relinked.is_empty() {
            parts.push(format!(
                "found {}",
                plural(self.relinked.len(), "moved movie", "moved movies")
            ));
        }
        if !self.skipped.is_empty() {
            parts.push(format!(
                "skipped {}",
                plural(self.skipped.len(), "file", "files")
            ));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} failed", self.failed.len()));
        }
        let text = if parts.is_empty() {
            "nothing new to add".to_string()
        } else {
            parts.join(", ")
        };
        let mut chars = text.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect())
            .unwrap_or_default()
    }
}

struct Candidate {
    info: FileInfo,
    hdr: bool,
    warning: Option<String>,
}

/// Reads size, fingerprint and (if ffprobe is available) technical details of a file.
fn read_candidate(path: &Path, tools: &Tools) -> Result<Candidate, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("cannot read file: {e}"))?
        .len();
    let fingerprint =
        fingerprint::fingerprint(path).map_err(|e| format!("cannot read file: {e}"))?;
    let mut info = FileInfo {
        path: path.to_path_buf(),
        size,
        fingerprint,
        ..FileInfo::default()
    };
    let mut hdr = false;
    let mut warning = None;
    if let Some(ffprobe) = &tools.ffprobe {
        match probe::probe(ffprobe, path) {
            Ok(media) => {
                info.duration = media.duration;
                if let Some(v) = &media.video {
                    info.video_codec = Some(v.codec.clone());
                    info.width = Some(v.width).filter(|w| *w > 0);
                    info.height = Some(v.height).filter(|h| *h > 0);
                    hdr = v.is_hdr();
                }
                info.audio_tracks = media
                    .audio
                    .iter()
                    .map(probe::OtherStream::to_track_info)
                    .collect();
                info.subtitle_tracks = media
                    .subtitles
                    .iter()
                    .map(probe::OtherStream::to_track_info)
                    .collect();
            }
            Err(e) => warning = Some(e),
        }
    }
    Ok(Candidate { info, hdr, warning })
}

/// File details for an existing file (used after re-encoding).
pub fn read_file_info(path: &Path, tools: &Tools) -> Result<FileInfo, String> {
    read_candidate(path, tools).map(|c| c.info)
}

fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 4)
}

/// Imports the video files in `paths` (files or folders).
///
/// Files already in the library are reported as skipped when `report_known` is set; background
/// scans of watched folders pass `false` so they stay quiet. A file whose fingerprint matches a
/// movie whose file has gone missing re-links that movie instead of adding a new one.
pub fn run_import(
    env: &LibraryEnv,
    paths: &[PathBuf],
    report_known: bool,
    progress: &(dyn Fn(ImportProgress) + Sync),
    on_change: &(dyn Fn() + Sync),
) -> anyhow::Result<ImportReport> {
    progress(ImportProgress::Scanning);
    let files: BTreeSet<PathBuf> = scan::collect_videos(paths)
        .iter()
        .map(|p| scan::normalize(p))
        .collect();
    let db = Db::open(&env.db_path)?;
    let known: HashSet<PathBuf> = db.all_paths()?.into_iter().collect();
    let ignored: HashSet<PathBuf> = db.ignored_paths()?.into_iter().collect();

    let mut report = ImportReport::default();
    let mut todo = Vec::new();
    for file in files {
        if ignored.contains(&file) {
            // Removed by the user: only an explicit drop/add brings it back.
            if !report_known {
                continue;
            }
            db.unignore_path(&file)?;
        }
        if known.contains(&file) {
            if report_known {
                report.skipped.push((file, "already in the library".into()));
            }
        } else {
            todo.push(file);
        }
    }
    if todo.is_empty() {
        return Ok(report);
    }
    if env.tools.ffprobe.is_none() {
        report.notes.push(
            "ffprobe was not found, so duration, codec and track details are unknown.".into(),
        );
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(worker_count())
        .build()?;
    let total = todo.len();
    let done = AtomicUsize::new(0);
    let candidates: Vec<(PathBuf, Result<Candidate, String>)> = pool.install(|| {
        todo.par_iter()
            .map(|path| {
                let result = read_candidate(path, &env.tools);
                progress(ImportProgress::Reading {
                    done: done.fetch_add(1, Ordering::Relaxed) + 1,
                    total,
                });
                (path.clone(), result)
            })
            .collect()
    });

    // Database decisions happen one at a time so duplicates inside one drop are caught too.
    let mut new_movies: Vec<(MovieId, PathBuf, bool)> = Vec::new();
    for (path, candidate) in candidates {
        let c = match candidate {
            Ok(c) => c,
            Err(e) => {
                report.failed.push((path, e));
                continue;
            }
        };
        let matches = db.find_by_fingerprint(&c.info.fingerprint)?;
        if let Some((id, _, title)) = matches.iter().find(|(_, p, _)| !p.exists()) {
            db.relink(*id, &path, c.info.size, &c.info.fingerprint)?;
            report.relinked.push((title.clone(), path));
            continue;
        }
        if let Some((_, existing, title)) = matches.first() {
            report.skipped.push((
                path,
                format!("duplicate of \"{title}\" ({})", existing.display()),
            ));
            continue;
        }
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (title, year) = filename::parse_title_year(&stem);
        match db.insert_movie(&NewMovie {
            file: c.info,
            title: title.clone(),
            year,
        }) {
            Ok(id) => {
                report.added.push(title);
                if let Some(w) = c.warning {
                    report.warnings.push((path.clone(), w));
                }
                new_movies.push((id, path, c.hdr));
            }
            // Another import added the same path in the meantime.
            Err(_) if db.find_by_path(&path).ok().flatten().is_some() => {
                if report_known {
                    report.skipped.push((path, "already in the library".into()));
                }
            }
            Err(e) => report.failed.push((path, format!("database error: {e}"))),
        }
    }
    if !new_movies.is_empty() || !report.relinked.is_empty() {
        on_change();
    }

    let total = new_movies.len();
    let done = AtomicUsize::new(0);
    let warnings: Vec<(PathBuf, String)> = pool.install(|| {
        new_movies
            .par_iter()
            .map_init(
                || Db::open(&env.db_path),
                |db, (id, path, hdr)| {
                    let warning = match db {
                        Ok(db) => metadata::fetch_for_new_movie(env, db, *id, *hdr),
                        Err(e) => Some(e.to_string()),
                    };
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(ImportProgress::Metadata { done: n, total });
                    if n.is_multiple_of(8) {
                        on_change();
                    }
                    warning.map(|w| (path.clone(), w))
                },
            )
            .flatten()
            .collect()
    });
    report.warnings.extend(warnings);
    on_change();
    Ok(report)
}

/// Points a movie at a file the user located by hand.
pub fn relink_manually(env: &LibraryEnv, id: MovieId, path: &Path) -> Result<(), String> {
    let path = scan::normalize(path);
    if !scan::is_video_file(&path) {
        return Err("That is not a supported video file.".into());
    }
    let db = Db::open(&env.db_path).map_err(|e| e.to_string())?;
    if let Some(other) = db.find_by_path(&path).map_err(|e| e.to_string())?
        && other != id
    {
        return Err("That file is already in the library as another movie.".into());
    }
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    let fp = fingerprint::fingerprint(&path).map_err(|e| e.to_string())?;
    db.relink(id, &path, size, &fp).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(dir: &Path) -> LibraryEnv {
        let posters = dir.join("posters");
        std::fs::create_dir_all(&posters).unwrap();
        // No tools: keeps the test independent of ffmpeg being installed.
        LibraryEnv {
            db_path: dir.join("library.db"),
            posters_dir: posters,
            tools: Tools::default(),
            tmdb_key: String::new(),
        }
    }

    fn import(env: &LibraryEnv, paths: &[PathBuf]) -> ImportReport {
        run_import(env, paths, true, &|_| {}, &|| {}).unwrap()
    }

    #[test]
    fn imports_files_and_skips_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let movies = dir.path().join("movies");
        std::fs::create_dir_all(movies.join("sub")).unwrap();
        std::fs::write(movies.join("The.Matrix.1999.1080p.mkv"), b"matrix").unwrap();
        std::fs::write(movies.join("sub/Copy of matrix.mkv"), b"matrix").unwrap();
        std::fs::write(movies.join("Alien (1979).mp4"), b"alien").unwrap();
        std::fs::write(movies.join("notes.txt"), b"x").unwrap();

        let report = import(&env, std::slice::from_ref(&movies));
        assert_eq!(report.added.len(), 2, "{report:?}");
        assert_eq!(report.summary(), "Added 2 movies, skipped 1 file");
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].1.starts_with("duplicate of"));
        assert!(!report.notes.is_empty(), "missing ffprobe is mentioned");

        let db = Db::open(&env.db_path).unwrap();
        let mut titles: Vec<_> = db
            .all_movies()
            .unwrap()
            .into_iter()
            .map(|m| (m.title, m.year))
            .collect();
        titles.sort();
        assert_eq!(
            titles,
            vec![
                ("Alien".to_string(), Some(1979)),
                ("The Matrix".to_string(), Some(1999))
            ]
        );

        // Importing again reports the files as already known.
        let again = import(&env, &[movies]);
        assert!(again.added.is_empty());
        assert_eq!(again.skipped.len(), 3);
    }

    #[test]
    fn removed_movies_stay_removed_from_background_scans() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let folder = dir.path().join("watched");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Heat.1995.mkv"), b"heat").unwrap();
        import(&env, std::slice::from_ref(&folder));
        let db = Db::open(&env.db_path).unwrap();
        db.remove_movie(db.all_movies().unwrap()[0].id).unwrap();

        let background =
            run_import(&env, std::slice::from_ref(&folder), false, &|_| {}, &|| {}).unwrap();
        assert!(background.is_empty(), "{background:?}");
        let explicit = import(&env, &[folder]);
        assert_eq!(explicit.added.len(), 1);
        assert!(db.ignored_paths().unwrap().is_empty());
    }

    #[test]
    fn moved_files_are_relinked_by_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let env = env(dir.path());
        let original = dir.path().join("Heat.1995.mkv");
        std::fs::write(&original, b"heat contents").unwrap();
        import(&env, std::slice::from_ref(&original));

        let moved_dir = dir.path().join("elsewhere");
        std::fs::create_dir_all(&moved_dir).unwrap();
        let moved = moved_dir.join("Heat renamed.mkv");
        std::fs::rename(&original, &moved).unwrap();

        let report = run_import(&env, &[moved_dir], false, &|_| {}, &|| {}).unwrap();
        assert_eq!(report.relinked.len(), 1, "{report:?}");
        assert_eq!(report.summary(), "Found 1 moved movie");
        assert!(report.added.is_empty());
        let movies = Db::open(&env.db_path).unwrap().all_movies().unwrap();
        assert_eq!(movies.len(), 1);
        assert_eq!(movies[0].path, scan::normalize(&moved));
        assert_eq!(movies[0].title, "Heat");
    }
}
