//! One Dropreel per data folder. A second launch (another video opened from Explorer while
//! Dropreel runs) hands its files to the running one, which plays them in its window, and exits.
//!
//! The running Dropreel holds a lock on `instance.lock` and listens on a local port, which it
//! writes with a random token to `instance.port`. Only someone who can read the data folder
//! knows the token, so other local programs can't make Dropreel open files.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const LOCK_FILE: &str = "instance.lock";
const PORT_FILE: &str = "instance.port";
/// How long a second launch keeps trying to reach the first (which may be starting up too).
const FORWARD_FOR: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(3);
/// A request is a list of paths; anything bigger isn't one.
const MAX_REQUEST: u64 = 1 << 20;

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    /// As text: JSON can't carry a path that isn't valid Unicode (mpv gets text anyway).
    paths: Vec<String>,
}

pub enum Instance {
    /// This is the only Dropreel: it takes files from later launches.
    Primary(Server),
    /// Another Dropreel took the files; this one should exit.
    Forwarded,
    /// The other Dropreel couldn't be reached (or the lock couldn't be used): run on our own.
    Alone,
}

/// Becomes the running Dropreel for the data folder `root`, or hands `paths` to the one already
/// running.
pub fn acquire(root: &Path, paths: &[PathBuf]) -> Instance {
    let started = Instant::now();
    let mut last_error = String::new();
    while started.elapsed() < FORWARD_FOR {
        let lock = match OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(LOCK_FILE))
        {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("could not open the instance lock: {e}");
                return Instance::Alone;
            }
        };
        match lock.try_lock() {
            Ok(()) => {
                return match Server::start(root, lock) {
                    Ok(server) => Instance::Primary(server),
                    Err(e) => {
                        tracing::warn!("could not listen for other launches: {e:#}");
                        Instance::Alone
                    }
                };
            }
            Err(std::fs::TryLockError::WouldBlock) => match forward(root, paths) {
                Ok(()) => {
                    tracing::info!("handed {paths:?} to the Dropreel already running");
                    return Instance::Forwarded;
                }
                // It may still be starting up (no port file yet), so try again.
                Err(e) => last_error = format!("{e:#}"),
            },
            Err(std::fs::TryLockError::Error(e)) => {
                tracing::warn!("could not lock the instance lock: {e}");
                return Instance::Alone;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    tracing::warn!("could not reach the Dropreel already running ({last_error}); starting another");
    Instance::Alone
}

fn forward(root: &Path, paths: &[PathBuf]) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(root.join(PORT_FILE))?;
    let (port, token) = text
        .trim()
        .split_once(' ')
        .ok_or_else(|| anyhow::anyhow!("{PORT_FILE} is malformed"))?;
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port.parse::<u16>()?));
    let mut stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    // The running Dropreel brings its window to the front: allow it to (Windows only lets the
    // program the user is working with take the foreground).
    allow_foreground();
    let request = Request {
        token: token.to_string(),
        paths: paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    };
    let mut line = serde_json::to_vec(&request)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    anyhow::ensure!(reply.trim() == "ok", "unexpected reply {reply:?}");
    Ok(())
}

#[cfg(windows)]
fn allow_foreground() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    // SAFETY: no pointers; failure only means the window may not come to the front.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

#[cfg(not(windows))]
fn allow_foreground() {}

pub struct Server {
    /// Files from later launches, in order, held until the app takes them.
    received: mpsc::Receiver<Vec<PathBuf>>,
    accepting: Arc<AtomicBool>,
}

impl Server {
    /// Listens straight away: a launch while this one is still starting up gets its answer at
    /// once, and its files are kept until `serve`.
    fn start(root: &Path, lock: File) -> anyhow::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let token = new_token();
        let (tx, received) = mpsc::channel();
        let accepting = Arc::new(AtomicBool::new(true));
        let gate = accepting.clone();
        let expected = token.clone();
        std::thread::Builder::new()
            .name("other-launches".into())
            .spawn(move || {
                // Held for as long as Dropreel runs; the system releases it when the process
                // ends.
                let _lock = lock;
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    if let Err(e) = answer(stream, &expected, &gate, &tx) {
                        tracing::warn!("ignored a request from another launch: {e:#}");
                    }
                }
            })?;
        let path = root.join(PORT_FILE);
        let tmp = path.with_extension("port.tmp");
        std::fs::write(&tmp, format!("{port} {token}"))?;
        std::fs::rename(&tmp, &path)?;
        Ok(Self {
            received,
            accepting,
        })
    }

    /// Set to false while Dropreel is closing: later launches then get no answer, wait for this
    /// one to exit, and open their files themselves.
    pub fn accepting(&self) -> Arc<AtomicBool> {
        self.accepting.clone()
    }

    /// Hands the files of each later launch to `open`, on a thread of its own, for as long as
    /// Dropreel runs.
    pub fn serve(self, open: impl Fn(Vec<PathBuf>) + Send + 'static) {
        let received = self.received;
        let spawned = std::thread::Builder::new()
            .name("open-requests".into())
            .spawn(move || {
                for paths in received {
                    open(paths);
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("could not take files from other launches: {e}");
        }
    }
}

fn answer(
    stream: TcpStream,
    token: &str,
    accepting: &AtomicBool,
    queue: &mpsc::Sender<Vec<PathBuf>>,
) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_REQUEST)).read_line(&mut line)?;
    let request: Request = serde_json::from_str(&line)?;
    anyhow::ensure!(request.token == token, "wrong token");
    // Closing: no answer, so the other launch waits for this one to exit.
    anyhow::ensure!(accepting.load(Ordering::SeqCst), "closing");
    queue.send(request.paths.into_iter().map(PathBuf::from).collect())?;
    (&stream).write_all(b"ok\n")?;
    Ok(())
}

/// Hard to guess for anyone who can't read the data folder.
fn new_token() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&std::process::id().to_le_bytes());
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.update(&t.as_nanos().to_le_bytes());
    }
    // Seeded randomly by the system for each process; a stack address adds ASLR's randomness.
    let local = 0u8;
    hasher.update(&(&local as *const u8 as usize).to_le_bytes());
    let random = std::collections::hash_map::RandomState::new();
    hasher.update(&std::hash::BuildHasher::hash_one(&random, 0u8).to_le_bytes());
    hasher.finalize().to_hex()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_second_launch_hands_its_files_to_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let Instance::Primary(server) = acquire(dir.path(), &[]) else {
            panic!("the first launch should run on its own");
        };
        let (tx, rx) = mpsc::channel();
        server.serve(move |paths| tx.send(paths).unwrap());

        let files = vec![PathBuf::from("/videos/a b.mkv")];
        assert!(matches!(acquire(dir.path(), &files), Instance::Forwarded));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), files);
        assert!(matches!(acquire(dir.path(), &[]), Instance::Forwarded));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn requests_without_the_token_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let Instance::Primary(server) = acquire(dir.path(), &[]) else {
            panic!("the first launch should run on its own");
        };
        let (tx, rx) = mpsc::channel();
        server.serve(move |paths| tx.send(paths).unwrap());
        let text = std::fs::read_to_string(dir.path().join(PORT_FILE)).unwrap();
        let port: u16 = text.split_once(' ').unwrap().0.parse().unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .write_all(b"{\"token\":\"guess\",\"paths\":[\"/etc/passwd\"]}\n")
            .unwrap();
        let mut reply = String::new();
        let _ = BufReader::new(stream).read_line(&mut reply);
        assert_eq!(reply, "");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn files_sent_before_serve_are_kept_and_none_while_closing() {
        let dir = tempfile::tempdir().unwrap();
        let Instance::Primary(server) = acquire(dir.path(), &[]) else {
            panic!("the first launch should run on its own");
        };
        // Still starting up: answered at once, kept for later.
        let early = vec![PathBuf::from("/videos/early.mkv")];
        assert!(matches!(acquire(dir.path(), &early), Instance::Forwarded));
        let accepting = server.accepting();
        let (tx, rx) = mpsc::channel();
        server.serve(move |paths| tx.send(paths).unwrap());
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), early);
        // Closing: the request isn't taken (the launch would wait to become the only one).
        accepting.store(false, Ordering::SeqCst);
        assert!(forward(dir.path(), &early).is_err());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn different_data_folders_run_side_by_side() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let first = acquire(a.path(), &[]);
        assert!(matches!(first, Instance::Primary(_)));
        assert!(matches!(acquire(b.path(), &[]), Instance::Primary(_)));
        drop(first);
    }
}
