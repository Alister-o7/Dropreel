//! Notices when the UI thread is stuck in one frame, and says so in the log from a thread of
//! its own: a freeze leaves a trace even if it never ends.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How long one frame may take before each warning.
const WARN_AFTER: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
];

/// Native dialogs (file pickers) open now: they block the UI thread on purpose.
static MODALS: AtomicU32 = AtomicU32::new(0);
/// When the last native dialog closed (see `now`).
static MODAL_CLOSED: AtomicU64 = AtomicU64::new(0);

/// Milliseconds since the first call, plus one (so 0 can mean "never").
fn now() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// Runs `dialog`, a native dialog that blocks the UI thread until the user closes it, without
/// the watchdog taking that for a freeze.
pub fn modal<T>(dialog: impl FnOnce() -> T) -> T {
    MODALS.fetch_add(1, Ordering::Relaxed);
    let result = dialog();
    MODAL_CLOSED.store(now(), Ordering::Relaxed);
    MODALS.fetch_sub(1, Ordering::Relaxed);
    result
}

struct Shared {
    /// When the frame in progress started (see `now`); 0 between frames.
    frame_started: AtomicU64,
}

pub struct Watchdog {
    shared: Arc<Shared>,
}

impl Watchdog {
    pub fn start() -> Self {
        let shared = Arc::new(Shared {
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
                    let now = now();
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
                    if started == 0 || MODALS.load(Ordering::Relaxed) > 0 {
                        continue;
                    }
                    // Time in a native dialog doesn't count.
                    let since = started.max(MODAL_CLOSED.load(Ordering::Relaxed));
                    let stuck = Duration::from_millis(now.saturating_sub(since));
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
        self.shared.frame_started.store(now(), Ordering::Relaxed);
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

    #[test]
    fn time_in_a_native_dialog_is_not_counted() {
        let before = now();
        let inside = modal(|| {
            assert!(MODALS.load(Ordering::Relaxed) > 0);
            std::thread::sleep(Duration::from_millis(5));
            42
        });
        assert_eq!(inside, 42);
        assert!(MODAL_CLOSED.load(Ordering::Relaxed) > before);
    }
}
