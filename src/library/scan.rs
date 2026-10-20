//! Expanding dropped files and folders into the list of video files to import.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "wmv", "webm", "ts", "m2ts",
];

/// Folders that never contain the user's movies.
const SKIPPED_DIRS: &[&str] = &["$recycle.bin", "system volume information"];

pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(ext)))
}

/// Returns every video file in `paths`, recursing into folders. The result is sorted and has no
/// duplicates.
pub fn collect_videos(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = BTreeSet::new();
    for path in paths {
        if path.is_dir() {
            let walker = walkdir::WalkDir::new(path).follow_links(false).into_iter();
            let entries = walker.filter_entry(|entry| {
                !(entry.file_type().is_dir()
                    && entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| SKIPPED_DIRS.contains(&name.to_lowercase().as_str())))
            });
            for entry in entries.flatten() {
                if entry.file_type().is_file() && is_video_file(entry.path()) {
                    found.insert(entry.into_path());
                }
            }
        } else if path.is_file() && is_video_file(path) {
            found.insert(path.clone());
        }
    }
    found.into_iter().collect()
}

/// Canonical form of a path for storing in the library (absolute, real casing, no `\\?\`).
pub fn normalize(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_matched_case_insensitively() {
        assert!(is_video_file(Path::new("a/Movie.MKV")));
        assert!(is_video_file(Path::new("b.m2ts")));
        assert!(!is_video_file(Path::new("notes.txt")));
        assert!(!is_video_file(Path::new("movie.hevc.mkv.part")));
        assert!(!is_video_file(Path::new("noext")));
    }

    #[test]
    fn folders_are_scanned_recursively() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::create_dir_all(root.join("$RECYCLE.BIN")).unwrap();
        for f in [
            "one.mkv",
            "a/two.MP4",
            "a/b/three.avi",
            "a/readme.txt",
            "$RECYCLE.BIN/old.mkv",
        ] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        let single = root.join("one.mkv");
        let found = collect_videos(&[root.to_path_buf(), single]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&"three.avi"));
        assert!(!names.contains(&"old.mkv"));
    }
}
