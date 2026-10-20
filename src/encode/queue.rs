//! The background re-encode queue: one job at a time, with progress, cancel and verification.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::args::{self, EncodeSpec, Pass};
use super::plan::{self, EncodePlan, PlanInput};
use super::progress::ProgressParser;
use super::{Encoder, EncoderDetector, verify};
use crate::library::{Db, MovieId, import};
use crate::metadata::probe;
use crate::settings::{EncoderChoice, Settings};
use crate::tools::{self, Tools};

/// Extra free space required on the target drive beyond the planned output size.
const FREE_SPACE_MARGIN: u64 = 500_000_000;

/// What a job needs to know; refreshed from Settings whenever they change.
#[derive(Clone, Debug)]
pub struct EncodeConfig {
    pub db_path: PathBuf,
    pub tools: Tools,
    pub encoder: EncoderChoice,
    pub x265_preset: String,
    pub threshold_bytes: u64,
    pub target_bytes: u64,
}

impl EncodeConfig {
    pub fn new(db_path: PathBuf, tools: Tools, settings: &Settings) -> Self {
        Self {
            db_path,
            tools,
            encoder: settings.encoder,
            x265_preset: settings.x265_preset.clone(),
            threshold_bytes: settings.threshold_bytes(),
            target_bytes: settings.target_bytes(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncodeJob {
    pub movie_id: MovieId,
    pub title: String,
    pub source: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Preparing,
    /// x265's first (analysis) pass.
    Analyzing,
    Encoding,
    Verifying,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing",
            Self::Analyzing => "Analyzing (pass 1 of 2)",
            Self::Encoding => "Encoding",
            Self::Verifying => "Verifying",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncodeStatus {
    pub movie_id: MovieId,
    pub title: String,
    pub stage: Stage,
    /// Overall progress, 0.0–1.0.
    pub fraction: f32,
    pub eta: Option<Duration>,
    /// Encoding speed relative to real time.
    pub speed: Option<f64>,
    pub encoder: Option<Encoder>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncodeOutcome {
    pub movie_id: MovieId,
    pub title: String,
    pub original: PathBuf,
    pub original_size: u64,
    pub output: PathBuf,
    pub output_size: u64,
    pub encoder: Encoder,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EncodeEvent {
    Finished(EncodeOutcome),
    Failed {
        movie_id: MovieId,
        title: String,
        error: String,
    },
    Cancelled {
        movie_id: MovieId,
        title: String,
    },
}

/// Where a movie is in the queue.
#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Queued { position: usize },
    Running(EncodeStatus),
}

#[derive(Default)]
struct State {
    queued: VecDeque<EncodeJob>,
    running: Option<EncodeStatus>,
    cancel_running: bool,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    config: Mutex<EncodeConfig>,
    detector: Arc<EncoderDetector>,
}

type EventFn = Arc<dyn Fn(EncodeEvent) + Send + Sync>;
type RepaintFn = Arc<dyn Fn() + Send + Sync>;

pub struct EncodeQueue {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl EncodeQueue {
    /// `on_event` receives finished/failed/cancelled jobs; `repaint` is called on progress.
    pub fn new(
        config: EncodeConfig,
        detector: Arc<EncoderDetector>,
        on_event: impl Fn(EncodeEvent) + Send + Sync + 'static,
        repaint: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            config: Mutex::new(config),
            detector,
        });
        let worker_shared = shared.clone();
        let (on_event, repaint): (EventFn, RepaintFn) = (Arc::new(on_event), Arc::new(repaint));
        let worker = std::thread::Builder::new()
            .name("encode-queue".into())
            .spawn(move || worker_loop(worker_shared, on_event, repaint))
            .ok();
        Self { shared, worker }
    }

    pub fn set_config(&self, config: EncodeConfig) {
        *self.shared.config.lock().unwrap() = config;
    }

    /// Adds a job. Returns false if that movie is already queued or encoding.
    pub fn enqueue(&self, job: EncodeJob) -> bool {
        let mut state = self.shared.state.lock().unwrap();
        let busy = state
            .running
            .as_ref()
            .is_some_and(|r| r.movie_id == job.movie_id)
            || state.queued.iter().any(|j| j.movie_id == job.movie_id);
        if busy {
            return false;
        }
        state.queued.push_back(job);
        self.shared.wake.notify_all();
        true
    }

    /// Removes a queued job or stops the running one.
    pub fn cancel(&self, movie_id: MovieId) {
        let mut state = self.shared.state.lock().unwrap();
        state.queued.retain(|j| j.movie_id != movie_id);
        if state
            .running
            .as_ref()
            .is_some_and(|r| r.movie_id == movie_id)
        {
            state.cancel_running = true;
        }
    }

    pub fn job_state(&self, movie_id: MovieId) -> Option<JobState> {
        let state = self.shared.state.lock().unwrap();
        if let Some(r) = state.running.as_ref().filter(|r| r.movie_id == movie_id) {
            return Some(JobState::Running(r.clone()));
        }
        state
            .queued
            .iter()
            .position(|j| j.movie_id == movie_id)
            .map(|position| JobState::Queued { position })
    }

    /// The running job and the titles waiting behind it.
    pub fn snapshot(&self) -> (Option<EncodeStatus>, Vec<(MovieId, String)>) {
        let state = self.shared.state.lock().unwrap();
        (
            state.running.clone(),
            state
                .queued
                .iter()
                .map(|j| (j.movie_id, j.title.clone()))
                .collect(),
        )
    }

    pub fn is_busy(&self) -> bool {
        let state = self.shared.state.lock().unwrap();
        state.running.is_some() || !state.queued.is_empty()
    }

    /// Cancels everything and waits for the worker (and ffmpeg) to stop.
    pub fn shutdown(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.shutdown = true;
            state.cancel_running = true;
            state.queued.clear();
            self.shared.wake.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for EncodeQueue {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn worker_loop(shared: Arc<Shared>, on_event: EventFn, repaint: RepaintFn) {
    loop {
        let job = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(job) = state.queued.pop_front() {
                    state.cancel_running = false;
                    state.running = Some(EncodeStatus {
                        movie_id: job.movie_id,
                        title: job.title.clone(),
                        stage: Stage::Preparing,
                        fraction: 0.0,
                        eta: None,
                        speed: None,
                        encoder: None,
                    });
                    break job;
                }
                state = shared.wake.wait(state).unwrap();
            }
        };
        repaint();
        let config = shared.config.lock().unwrap().clone();
        let report = |update: &dyn Fn(&mut EncodeStatus)| {
            if let Some(status) = shared.state.lock().unwrap().running.as_mut() {
                update(status);
            }
            repaint();
        };
        let cancelled = || {
            let state = shared.state.lock().unwrap();
            state.cancel_running || state.shutdown
        };
        tracing::info!("encode started: {}", job.source.display());
        let result = run_job(&config, &shared.detector, &job, &report, &cancelled);
        shared.state.lock().unwrap().running = None;
        let event = match result {
            Ok(outcome) => {
                tracing::info!(
                    "encode finished: {} -> {}",
                    job.source.display(),
                    outcome.output.display()
                );
                EncodeEvent::Finished(outcome)
            }
            Err(JobError::Cancelled) => {
                tracing::info!("encode cancelled: {}", job.source.display());
                EncodeEvent::Cancelled {
                    movie_id: job.movie_id,
                    title: job.title.clone(),
                }
            }
            Err(JobError::Failed(error)) => {
                tracing::warn!("encode failed: {}: {error}", job.source.display());
                EncodeEvent::Failed {
                    movie_id: job.movie_id,
                    title: job.title.clone(),
                    error,
                }
            }
        };
        on_event(event);
        repaint();
    }
}

#[derive(Debug)]
enum JobError {
    Cancelled,
    Failed(String),
}

impl From<String> for JobError {
    fn from(e: String) -> Self {
        Self::Failed(e)
    }
}

impl From<&str> for JobError {
    fn from(e: &str) -> Self {
        Self::Failed(e.to_string())
    }
}

/// Deletes temporary files unless disarmed.
struct TempFiles(Vec<PathBuf>);

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `Movie.mkv` → `Movie.hevc.mkv` (or `Movie.hevc (2).mkv` if that exists).
pub fn output_path(source: &Path) -> PathBuf {
    let dir = source.parent().unwrap_or(Path::new("."));
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "movie".into());
    let mut candidate = dir.join(format!("{stem}.hevc.mkv"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}.hevc ({n}).mkv"));
        n += 1;
    }
    candidate
}

fn part_path(output: &Path) -> PathBuf {
    let mut name = output.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// Everything shown before the user confirms an encode.
#[derive(Clone, Debug)]
pub struct EncodePreview {
    pub movie_id: MovieId,
    pub title: String,
    pub source: PathBuf,
    pub source_size: u64,
    pub duration: Option<f64>,
    pub video: Option<String>,
    pub audio_tracks: usize,
    pub subtitle_tracks: usize,
    pub plan: Result<EncodePlan, String>,
    pub encoder: Result<Encoder, String>,
    pub output: PathBuf,
    pub free_space: Option<u64>,
    pub target_bytes: u64,
}

impl EncodePreview {
    /// The reason the encode cannot start, if any.
    pub fn blocker(&self) -> Option<String> {
        if let Err(e) = &self.plan {
            return Some(e.clone());
        }
        if let Err(e) = &self.encoder {
            return Some(e.clone());
        }
        match self.free_space {
            Some(free) if free < self.target_bytes + FREE_SPACE_MARGIN => Some(format!(
                "Not enough free space next to the movie: {} free, about {} needed.",
                crate::util::format_bytes(free),
                crate::util::format_bytes(self.target_bytes + FREE_SPACE_MARGIN)
            )),
            _ => None,
        }
    }
}

/// Probes the file, plans the bitrate and picks the encoder (runs a short test encode, so call
/// it off the UI thread).
pub fn preview(
    config: &EncodeConfig,
    detector: &EncoderDetector,
    movie_id: MovieId,
    title: &str,
    source: &Path,
) -> EncodePreview {
    let source_size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
    let media = config
        .tools
        .ffprobe()
        .and_then(|ffprobe| probe::probe(ffprobe, source));
    let plan = media
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|m| PlanInput::from_media(m, config.target_bytes))
        .and_then(|input| plan::plan(&input));
    let encoder = config
        .tools
        .ffmpeg()
        .and_then(|ffmpeg| detector.select(ffmpeg, config.encoder));
    let media = media.ok();
    EncodePreview {
        movie_id,
        title: title.to_string(),
        source: source.to_path_buf(),
        source_size,
        duration: media.as_ref().and_then(|m| m.duration),
        video: media
            .as_ref()
            .and_then(|m| m.video.as_ref())
            .map(|v| format!("{} {}×{}", v.codec, v.width, v.height)),
        audio_tracks: media.as_ref().map_or(0, |m| m.audio.len()),
        subtitle_tracks: media.as_ref().map_or(0, |m| m.subtitles.len()),
        plan,
        encoder,
        output: output_path(source),
        free_space: source
            .parent()
            .and_then(|dir| fs4::available_space(dir).ok()),
        target_bytes: config.target_bytes,
    }
}

type Report<'a> = &'a dyn Fn(&dyn Fn(&mut EncodeStatus));

fn run_job(
    config: &EncodeConfig,
    detector: &EncoderDetector,
    job: &EncodeJob,
    report: Report<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<EncodeOutcome, JobError> {
    let started = Instant::now();
    let ffmpeg = config.tools.ffmpeg()?.to_path_buf();
    let ffprobe = config.tools.ffprobe()?.to_path_buf();
    let original_size = std::fs::metadata(&job.source)
        .map_err(|e| format!("Cannot read the movie file: {e}"))?
        .len();
    if original_size <= config.threshold_bytes {
        return Err(JobError::Failed(
            "The file is already under the size limit.".into(),
        ));
    }
    let media = probe::probe(&ffprobe, &job.source)?;
    let video = media.video.clone().ok_or("The file has no video track.")?;
    let duration = media.duration.ok_or("The file's duration is unknown.")?;
    let plan = plan::plan(&PlanInput::from_media(&media, config.target_bytes)?)?;
    let encoder = detector.select(&ffmpeg, config.encoder)?;
    report(&|s| s.encoder = Some(encoder));

    let dir = job
        .source
        .parent()
        .ok_or("The movie has no parent folder.")?;
    if let Ok(free) = fs4::available_space(dir)
        && free < config.target_bytes + FREE_SPACE_MARGIN
    {
        return Err(JobError::Failed(format!(
            "Not enough free space on the drive ({} free).",
            crate::util::format_bytes(free)
        )));
    }
    if cancelled() {
        return Err(JobError::Cancelled);
    }

    let output = output_path(&job.source);
    let part = part_path(&output);
    let stats_dir = std::env::temp_dir();
    // Unique per job, so two encodes (or two DropReel instances) never share x265 stats.
    static JOB_COUNTER: AtomicU64 = AtomicU64::new(0);
    let job_no = JOB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stats_name = format!(
        "dropreel-x265-{}-{}-{job_no}.log",
        std::process::id(),
        job.movie_id
    );
    let mut temp = TempFiles(vec![
        part.clone(),
        stats_dir.join(&stats_name),
        stats_dir.join(format!("{stats_name}.cutree")),
        stats_dir.join(format!("{stats_name}.temp")),
        stats_dir.join(format!("{stats_name}.cutree.temp")),
    ]);
    let hdr = (encoder == Encoder::X265 && video.is_hdr())
        .then(|| probe::probe_hdr(&ffprobe, &job.source))
        .flatten();
    let subtitle_codecs: Vec<String> = media.subtitles.iter().map(|s| s.codec.clone()).collect();
    let spec = EncodeSpec {
        input: &job.source,
        output: &part,
        encoder,
        video_kbps: plan.video_kbps,
        x265_preset: &config.x265_preset,
        video: &video,
        hdr: hdr.as_ref(),
        subtitle_codecs: &subtitle_codecs,
        stats_file: &stats_name,
    };

    for &pass in args::passes(encoder) {
        let (base, weight, stage) = match pass {
            Pass::First => (0.0, 0.3, Stage::Analyzing),
            Pass::Second => (0.3, 0.7, Stage::Encoding),
            Pass::Single => (0.0, 1.0, Stage::Encoding),
        };
        report(&|s| {
            s.stage = stage;
            s.fraction = base as f32;
        });
        run_ffmpeg(
            &ffmpeg,
            args::ffmpeg_args(&spec, pass),
            &stats_dir,
            cancelled,
            &|fraction, speed| {
                let overall = base + weight * fraction;
                let elapsed = started.elapsed();
                let eta = (overall > 0.01).then(|| elapsed.mul_f64((1.0 - overall) / overall));
                report(&|s| {
                    s.fraction = overall as f32;
                    s.speed = speed;
                    s.eta = eta;
                });
            },
            duration,
        )?;
    }

    report(&|s| {
        s.stage = Stage::Verifying;
        s.fraction = 1.0;
        s.eta = None;
    });
    let output_size = std::fs::metadata(&part)
        .map_err(|e| format!("The encoded file is missing: {e}"))?
        .len();
    verify::check_output(
        output_size,
        config.threshold_bytes,
        duration,
        verify::probe_duration(&ffprobe, &part),
    )?;
    std::fs::rename(&part, &output)
        .map_err(|e| format!("Could not rename the finished file: {e}"))?;
    temp.0.retain(|p| p != &part);

    let info = import::read_file_info(&output, &config.tools)?;
    Db::open(&config.db_path)
        .and_then(|db| db.replace_file(job.movie_id, &info))
        .map_err(|e| {
            format!(
                "The encode finished ({}) but the library could not be updated: {e:#}",
                output.display()
            )
        })?;
    Ok(EncodeOutcome {
        movie_id: job.movie_id,
        title: job.title.clone(),
        original: job.source.clone(),
        original_size,
        output,
        output_size,
        encoder,
        elapsed: started.elapsed(),
    })
}

/// Runs ffmpeg, reporting `-progress` updates as (fraction of `duration`, speed).
fn run_ffmpeg(
    ffmpeg: &Path,
    args: Vec<OsString>,
    cwd: &Path,
    cancelled: &dyn Fn() -> bool,
    on_progress: &dyn Fn(f64, Option<f64>),
    duration: f64,
) -> Result<(), JobError> {
    let mut child = tools::background_command(ffmpeg)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start ffmpeg: {e}"))?;
    let stdout = child.stdout.take().ok_or("no ffmpeg stdout")?;
    let stderr = child.stderr.take().ok_or("no ffmpeg stderr")?;
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let errors = Arc::new(Mutex::new(VecDeque::<String>::new()));
    let errors_writer = errors.clone();
    let stderr_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let mut e = errors_writer.lock().unwrap();
            e.push_back(line);
            if e.len() > 20 {
                e.pop_front();
            }
        }
    });

    let mut parser = ProgressParser::default();
    loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(JobError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                if parser.feed(&line) {
                    on_progress(parser.fraction(duration), parser.speed);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("ffmpeg did not finish: {e}"))?;
    let _ = stderr_thread.join();
    if status.success() {
        Ok(())
    } else {
        let tail: Vec<String> = errors.lock().unwrap().iter().cloned().collect();
        let detail = tail
            .iter()
            .rev()
            .take(4)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        Err(JobError::Failed(
            format!("ffmpeg failed ({status}).\n{detail}")
                .trim()
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_names_never_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Movie (2019).mkv");
        assert_eq!(
            output_path(&source),
            dir.path().join("Movie (2019).hevc.mkv")
        );
        std::fs::write(dir.path().join("Movie (2019).hevc.mkv"), b"x").unwrap();
        assert_eq!(
            output_path(&source),
            dir.path().join("Movie (2019).hevc (2).mkv")
        );
        assert_eq!(
            part_path(&dir.path().join("a.hevc.mkv")),
            dir.path().join("a.hevc.mkv.part")
        );
    }
}
