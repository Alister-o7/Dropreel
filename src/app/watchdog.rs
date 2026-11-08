//! Notices when the UI thread is stuck in one frame, and says so in the log from a thread of
//! its own: a freeze leaves a trace even if it never ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long one frame may take before each warning.
const WARN_AFTER: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
];

struct Shared {
    epoch: Instant,
    /// When the frame in progress started, in ms since `epoch` plus one; 0 between frames.
    frame_started: AtomicU64,
}

impl Shared {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64 + 1
    }
}

pub struct Watchdog {
    shared: Arc<Shared>,
}

impl Watchdog {
    pub fn start() -> Self {
        let shared = Arc::new(Shared {
            epoch: Instant::now(),
            frame_started: AtomicU64::new(0),
        });
        let watched = Arc::downgrade(&shared);
        let spawned = std::thread::Builder::new()
            .name("ui-watchdog".into())
            .spawn(move || {
                // The frame last warned about, and how many warnings it has had.
                let mut warned: Option<(u64, usize)> = None;
                loop {
                    std::thread::sleep(Duration::from_millis(250));
                    // Ends with the app.
                    let Some(shared) = watched.upgrade() else {
                        return;
                    };
                    let started = shared.frame_started.load(Ordering::Relaxed);
                    let now = shared.now();
                    if let Some((frame, _)) = warned
                        && frame != started
                    {
                        let took = Duration::from_millis(now.saturating_sub(frame));
                        tracing::warn!(
                            "the UI thread is responding again after about {:.1} s",
                            took.as_secs_f64()
                        );
                        warned = None;
                    }
                    if started == 0 {
                        continue;
                    }
                    let stuck = Duration::from_millis(now.saturating_sub(started));
                    let count = warned.map_or(0, |(_, n)| n);
                    if WARN_AFTER.get(count).is_some_and(|after| stuck >= *after) {
                        tracing::warn!(
                            "the UI thread has been stuck in one frame for {:.1} s",
                            stuck.as_secs_f64()
                        );
                        warned = Some((started, count + 1));
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("could not start the UI watchdog: {e}");
        }
        Self { shared }
    }

    pub fn frame_started(&self) {
        let now = self.shared.now();
        self.shared.frame_started.store(now, Ordering::Relaxed);
    }

    pub fn frame_ended(&self) {
        self.shared.frame_started.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_tracked() {
        let w = Watchdog::start();
        w.frame_started();
        assert_ne!(w.shared.frame_started.load(Ordering::Relaxed), 0);
        w.frame_ended();
        assert_eq!(w.shared.frame_started.load(Ordering::Relaxed), 0);
    }
}
