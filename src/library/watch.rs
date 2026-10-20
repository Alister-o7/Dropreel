//! Watching folders for new movie files.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};

use super::scan;

/// How long a file's size must stay unchanged before we import it, so files that are still
/// being copied are not imported half-written.
pub const SETTLE_TIME: Duration = Duration::from_secs(3);

pub struct FolderWatcher {
    _watcher: notify::RecommendedWatcher,
    stop: Arc<AtomicBool>,
}

impl Drop for FolderWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl FolderWatcher {
    /// Starts watching `folders`. `on_ready` receives batches of new video files once they have
    /// finished copying. Returns the watcher plus one error message per folder that could not be
    /// watched.
    pub fn start(
        folders: &[PathBuf],
        settle: Duration,
        on_ready: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> notify::Result<(Self, Vec<String>)> {
        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })?;
        let mut errors = Vec::new();
        for folder in folders {
            if let Err(e) = watcher.watch(folder, RecursiveMode::Recursive) {
                errors.push(format!("Cannot watch {}: {e}", folder.display()));
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        std::thread::Builder::new()
            .name("folder-watcher".into())
            .spawn(move || watch_loop(rx, stop_flag, settle, on_ready))
            .map_err(|e| notify::Error::generic(&e.to_string()))?;
        Ok((
            Self {
                _watcher: watcher,
                stop,
            },
            errors,
        ))
    }
}

struct Pending {
    size: u64,
    changed: Instant,
}

fn watch_loop(
    rx: mpsc::Receiver<notify::Result<notify::Event>>,
    stop: Arc<AtomicBool>,
    settle: Duration,
    on_ready: impl Fn(Vec<PathBuf>),
) {
    let mut pending: HashMap<PathBuf, Pending> = HashMap::new();
    let tick = (settle / 3).clamp(Duration::from_millis(50), Duration::from_secs(1));
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(tick) {
            Ok(Ok(event)) if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) => {
                for path in event.paths {
                    let files = if path.is_dir() {
                        scan::collect_videos(&[path])
                    } else if scan::is_video_file(&path) {
                        vec![path]
                    } else {
                        vec![]
                    };
                    for file in files {
                        let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                        pending.insert(
                            file,
                            Pending {
                                size,
                                changed: Instant::now(),
                            },
                        );
                    }
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!("folder watch error: {e}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let mut ready = Vec::new();
        pending.retain(|path, p| {
            let Ok(meta) = std::fs::metadata(path) else {
                return false;
            };
            if meta.len() != p.size {
                p.size = meta.len();
                p.changed = Instant::now();
                return true;
            }
            // On Windows a file that is still being copied cannot be opened.
            if p.changed.elapsed() >= settle && std::fs::File::open(path).is_ok() {
                ready.push(path.clone());
                return false;
            }
            true
        });
        if !ready.is_empty() {
            on_ready(ready);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_new_video_files_after_they_settle() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::channel();
        let (_watcher, errors) = FolderWatcher::start(
            &[dir.path().to_path_buf()],
            Duration::from_millis(300),
            move |files| {
                let _ = tx.send(files);
            },
        )
        .unwrap();
        assert!(errors.is_empty());
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(dir.path().join("readme.txt"), b"ignored").unwrap();
        std::fs::write(dir.path().join("New Movie (2020).mkv"), b"video").unwrap();

        let files = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("watcher reported nothing");
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("New Movie (2020).mkv"));
    }
}
