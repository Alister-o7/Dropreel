//! Safe wrapper around one libmpv instance plus the texture it renders into.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::glow::{self, HasContext as _};

use super::ffi::{self, MpvLib};

pub type GetProcAddress = Arc<dyn Fn(&CStr) -> *const c_void + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: i64,
    pub kind: TrackKind,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
    pub channels: Option<i64>,
    pub external: bool,
    pub selected: bool,
}

impl Track {
    /// e.g. "eng · ac3 5.1 · Commentary"
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(lang) = &self.lang {
            parts.push(lang.clone());
        }
        if let Some(codec) = &self.codec {
            let ch = match self.channels {
                Some(1) => " mono",
                Some(2) => " stereo",
                Some(6) => " 5.1",
                Some(8) => " 7.1",
                _ => "",
            };
            parts.push(format!("{codec}{ch}"));
        }
        if let Some(title) = &self.title {
            parts.push(title.clone());
        }
        if self.external {
            parts.push("external".into());
        }
        if parts.is_empty() {
            format!("Track {}", self.id)
        } else {
            parts.join(" · ")
        }
    }

    /// A compact label without the track title, e.g. "eng · ac3 5.1" (for the menu button).
    pub fn short_label(&self) -> String {
        let mut parts = Vec::new();
        if let Some(lang) = &self.lang {
            parts.push(lang.clone());
        }
        if let Some(codec) = &self.codec {
            let ch = match self.channels {
                Some(1) => " mono",
                Some(2) => " stereo",
                Some(6) => " 5.1",
                Some(8) => " 7.1",
                _ => "",
            };
            parts.push(format!("{codec}{ch}"));
        }
        if parts.is_empty() {
            format!("Track {}", self.id)
        } else {
            parts.join(" · ")
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    FileLoaded,
    /// Tracks were added, removed or (de)selected; get them with `Player::tracks`.
    TracksChanged,
    /// The file could not be played.
    Error(String),
}

// Reply ids for observed properties.
const OBS_TIME_POS: u64 = 1;
const OBS_DURATION: u64 = 2;
const OBS_PAUSE: u64 = 3;
const OBS_VOLUME: u64 = 4;
const OBS_MUTE: u64 = 5;
const OBS_EOF: u64 = 6;
const OBS_TRACKS: u64 = 7;

/// Properties logged by [`Player::log_diagnostics`].
const DIAGNOSTIC_PROPS: [&str; 17] = [
    "hwdec-current",
    "video-sync",
    "interpolation",
    "display-fps",
    "estimated-display-fps",
    "container-fps",
    "vsync-ratio",
    "vsync-jitter",
    "frame-drop-count",
    "decoder-frame-drop-count",
    "vo-delayed-frame-count",
    "mistimed-frame-count",
    "video-params/pixelformat",
    "dwidth",
    "dheight",
    "osd-width",
    "osd-height",
];

/// Calls into mpv from the UI thread slower than this are logged.
const SLOW_CALL: Duration = Duration::from_millis(50);
/// A file that has no picture this long after it was asked for is logged.
const SLOW_LOAD: Duration = Duration::from_secs(3);

/// Which file mpv's file events are about. After `load`, mpv still reports on the file it is
/// leaving, and on a file replaced before it even started (the next episode asked for twice in
/// quick succession); those reports must not be taken for the new file's.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gate {
    /// No file asked for, or it was stopped.
    Idle,
    /// `loadfile` went out as this request; waiting for mpv to say which playlist entry it made.
    AwaitingReply(u64),
    /// Waiting for that entry to start (`None`: mpv didn't say, so whichever starts next).
    Expecting(Option<i64>),
    /// Events are about the file asked for.
    Current,
}

/// What an asynchronous request to mpv was, for handling its reply.
enum Request {
    Load,
    Command(String),
    Set(String),
    Diagnostics { batch: u64, index: usize },
}

/// Diagnostics values on their way back from mpv.
struct DiagnosticsBatch {
    id: u64,
    values: Vec<Option<String>>,
    remaining: usize,
    stats: String,
}

/// When each step of loading the current file happened, for the log.
struct LoadTimeline {
    at: Instant,
    start_file: Option<Duration>,
    loaded: Option<Duration>,
    restart: Option<Duration>,
    /// mpv's video output waiting for us to render (it gives up after 200 ms and drops the frame).
    render_waits: u32,
    longest_frame: Duration,
    reported: bool,
}

impl LoadTimeline {
    fn new() -> Self {
        Self {
            at: Instant::now(),
            start_file: None,
            loaded: None,
            restart: None,
            render_waits: 0,
            longest_frame: Duration::ZERO,
            reported: false,
        }
    }

    fn summary(&self) -> String {
        let ms = |d: Option<Duration>| {
            d.map_or("-".to_string(), |d| {
                format!("{:.0} ms", d.as_secs_f64() * 1000.0)
            })
        };
        format!(
            "started {}, loaded {}, playing {}, picture {}; mpv waited on rendering {} times, longest UI frame {:.0} ms",
            ms(self.start_file),
            ms(self.loaded),
            ms(self.restart),
            ms(Some(self.at.elapsed())),
            self.render_waits,
            self.longest_frame.as_secs_f64() * 1000.0,
        )
    }
}

/// A seek asked for before the file finished loading. mpv refuses seeks until then, so it is
/// kept and applied on `FILE_LOADED`.
#[derive(Clone, Copy, Debug)]
enum PendingSeek {
    To { seconds: f64, exact: bool },
    By(f64),
}

/// Playback state mirrored from mpv's property-change events.
#[derive(Clone, Debug, PartialEq)]
struct PlaybackState {
    position: Option<f64>,
    duration: Option<f64>,
    paused: bool,
    volume: f64,
    muted: bool,
    eof: bool,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            position: None,
            duration: None,
            paused: false,
            volume: 100.0,
            muted: false,
            eof: false,
        }
    }
}

impl PlaybackState {
    /// Applies one property change.
    fn apply(&mut self, id: u64, prop: &ffi::mpv_event_property) {
        let has_value = prop.format != ffi::MPV_FORMAT_NONE && !prop.data.is_null();
        // SAFETY: mpv sets `data` to a value of the format we observed the property with.
        let f64_value = || {
            has_value
                .then(|| unsafe { *(prop.data as *const f64) })
                .filter(|v| v.is_finite())
        };
        let flag_value = || has_value && unsafe { *(prop.data as *const c_int) } != 0;
        match id {
            OBS_TIME_POS => self.position = f64_value(),
            OBS_DURATION => self.duration = f64_value(),
            OBS_PAUSE => self.paused = flag_value(),
            OBS_VOLUME => self.volume = f64_value().unwrap_or(self.volume),
            OBS_MUTE => self.muted = flag_value(),
            OBS_EOF => self.eof = flag_value(),
            _ => {}
        }
    }
}

/// Frame-pacing numbers for the log, collected since the current file was loaded.
#[derive(Default)]
struct RenderStats {
    last_frame: Option<Instant>,
    /// Time since the frame before this one.
    last_gap: Option<Duration>,
    frames: u64,
    /// UI frames that took noticeably longer than one display refresh.
    late_frames: u64,
    longest_frame: Duration,
    renders: u64,
    render_time: Duration,
    longest_render: Duration,
}

impl RenderStats {
    fn frame(&mut self, refresh_interval: Duration, playing: bool) {
        let now = Instant::now();
        self.last_gap = self.last_frame.map(|prev| now - prev);
        if let Some(prev) = self.last_frame.replace(now)
            && playing
        {
            let dt = now - prev;
            // Gaps longer than a second are pauses or window moves, not stutter.
            if dt < Duration::from_secs(1) {
                self.frames += 1;
                self.longest_frame = self.longest_frame.max(dt);
                if dt > refresh_interval.mul_f64(1.5) {
                    self.late_frames += 1;
                }
            }
        }
    }

    fn rendered(&mut self, took: Duration) {
        self.renders += 1;
        self.render_time += took;
        self.longest_render = self.longest_render.max(took);
    }

    fn summary(&self) -> String {
        let avg_render = if self.renders > 0 {
            self.render_time.as_secs_f64() * 1000.0 / self.renders as f64
        } else {
            0.0
        };
        format!(
            "ui frames {} (late {}, longest {:.1} ms), mpv renders {} (avg {avg_render:.2} ms, longest {:.1} ms)",
            self.frames,
            self.late_frames,
            self.longest_frame.as_secs_f64() * 1000.0,
            self.renders,
            self.longest_render.as_secs_f64() * 1000.0,
        )
    }
}

struct RenderTarget {
    fbo: glow::Framebuffer,
    texture: glow::Texture,
    texture_id: egui::TextureId,
    size: [i32; 2],
}

pub struct Player {
    lib: &'static MpvLib,
    handle: *mut ffi::mpv_handle,
    render: *mut ffi::mpv_render_context,
    // Kept alive for as long as mpv may call back into them.
    _get_proc: Option<Box<GetProcAddress>>,
    _waker: Box<Waker>,
    target: Option<RenderTarget>,
    /// mpv's own values for the options in SIMPLE_SCALING, to restore high-quality scaling.
    default_scaling: Vec<(&'static str, String)>,
    state: PlaybackState,
    /// Whether the current file has loaded, so mpv accepts seeks.
    loaded: bool,
    /// Whether playback of the current file has started (mpv's PLAYBACK_RESTART after the load):
    /// from then on, frames are the new file's.
    started: bool,
    /// Whether a frame of the current file has been rendered: until then the render target
    /// (and mpv's video output, which a new file doesn't clear) still holds the previous file's
    /// last frame.
    has_picture: bool,
    pending_seek: Option<PendingSeek>,
    display_fps: Option<f64>,
    stats: RenderStats,
    gate: Gate,
    /// The track list, as mpv last sent it.
    tracks: Vec<Track>,
    /// Render on the next frame even if mpv has no new frame (see PLAYBACK_RESTART).
    force_render: bool,
    next_request: u64,
    requests: HashMap<u64, Request>,
    diagnostics: Vec<DiagnosticsBatch>,
    last_diagnostics: Option<String>,
    timeline: Option<LoadTimeline>,
}

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: ctx is the boxed GetProcAddress owned by the Player; name is a C string from mpv.
    let get = unsafe { &*(ctx as *const GetProcAddress) };
    get(unsafe { CStr::from_ptr(name) }) as *mut c_void
}

/// Wakes the UI when mpv has news. mpv calls back from its own threads while holding its own
/// locks, and egui's `request_repaint` takes egui's context lock: if the UI thread held that lock
/// while waiting on mpv, each would wait for the other. So the callback only signals a channel,
/// and a thread of our own asks egui to repaint.
struct Waker {
    tx: crossbeam_channel::Sender<()>,
}

impl Waker {
    fn new(ctx: egui::Context) -> Self {
        // One pending wake is enough: the next frame picks up everything that happened.
        let (tx, rx) = crossbeam_channel::bounded::<()>(1);
        let spawned = std::thread::Builder::new()
            .name("mpv-wake".into())
            .spawn(move || {
                // Ends when the Player, and with it the sender, is dropped.
                for () in rx {
                    ctx.request_repaint();
                }
            });
        if let Err(e) = spawned {
            tracing::error!("could not start the mpv wake-up thread: {e}");
        }
        Self { tx }
    }
}

unsafe extern "C" fn wake_up(ctx: *mut c_void) {
    // SAFETY: ctx is the boxed Waker owned by the Player, which outlives mpv's callbacks.
    // try_send never blocks, and mpv forbids calling mpv functions here, which we don't.
    let _ = unsafe { &*(ctx as *const Waker) }.tx.try_send(());
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl Player {
    /// Creates an mpv instance that renders through the app's OpenGL context. Must be called
    /// on the UI thread with that context current (e.g. in eframe's app creator).
    pub fn new(get_proc: GetProcAddress, ctx: egui::Context) -> Result<Self, String> {
        let mut player = Self::create(ctx, &[("vo", "libmpv")], Some(Box::new(get_proc)))?;
        let api = cstr("opengl");
        let get_proc_ctx = player
            ._get_proc
            .as_deref()
            .map_or(std::ptr::null_mut(), |g| {
                g as *const GetProcAddress as *mut c_void
            });
        let mut init = ffi::mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address),
            get_proc_address_ctx: get_proc_ctx,
        };
        let mut params = [
            ffi::mpv_render_param {
                kind: ffi::MPV_RENDER_PARAM_API_TYPE,
                data: api.as_ptr() as *mut c_void,
            },
            ffi::mpv_render_param {
                kind: ffi::MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut _ as *mut c_void,
            },
            ffi::mpv_render_param {
                kind: ffi::MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        let mut render = std::ptr::null_mut();
        let rc = unsafe {
            (player.lib.render_context_create)(&mut render, player.handle, params.as_mut_ptr())
        };
        player.check(rc, "create the mpv OpenGL renderer")?;
        player.render = render;
        let wake = &*player._waker as *const Waker as *mut c_void;
        unsafe { (player.lib.render_context_set_update_callback)(render, Some(wake_up), wake) };
        Ok(player)
    }

    /// Creates an mpv instance that decodes and plays in real time without drawing a picture or
    /// playing sound. Everything except `render` works as usual; used by tests.
    pub fn headless(ctx: egui::Context) -> Result<Self, String> {
        Self::create(ctx, &[("vo", "null"), ("ao", "null")], None)
    }

    /// `outputs` are the video/audio output options.
    fn create(
        ctx: egui::Context,
        outputs: &[(&str, &str)],
        get_proc: Option<Box<GetProcAddress>>,
    ) -> Result<Self, String> {
        let lib = ffi::lib()?;
        let handle = unsafe { (lib.create)() };
        if handle.is_null() {
            return Err("mpv could not be created".into());
        }
        let mut player = Self {
            lib,
            handle,
            render: std::ptr::null_mut(),
            _get_proc: get_proc,
            _waker: Box::new(Waker::new(ctx)),
            target: None,
            default_scaling: Vec::new(),
            state: PlaybackState::default(),
            loaded: false,
            started: false,
            has_picture: false,
            pending_seek: None,
            display_fps: None,
            stats: RenderStats::default(),
            gate: Gate::Idle,
            tracks: Vec::new(),
            force_render: false,
            next_request: 1,
            requests: HashMap::new(),
            diagnostics: Vec::new(),
            last_diagnostics: None,
            timeline: None,
        };
        for (name, value) in outputs {
            player.option(name, value);
        }
        for (name, value) in [
            ("hwdec", "auto-safe"),
            ("keep-open", "yes"),
            ("idle", "yes"),
            ("osc", "no"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("terminal", "no"),
            ("config", "no"),
            ("ytdl", "no"),
            ("sub-auto", "fuzzy"),
            ("audio-display", "no"),
            // We render without blocking (see `render`), so mpv must hand over each frame at its
            // display time rather than ahead of it.
            ("video-timing-offset", "0"),
        ] {
            player.option(name, value);
        }
        // Troubleshooting: DROPREEL_MPV_OPTS="hwdec=no,scale=bilinear" passes extra mpv options.
        if let Ok(extra) = std::env::var("DROPREEL_MPV_OPTS") {
            for pair in extra.split(',').filter(|p| !p.is_empty()) {
                let (name, value) = pair.split_once('=').unwrap_or((pair, "yes"));
                player.option(name.trim(), value.trim());
            }
        }
        // Troubleshooting: DROPREEL_MPV_LOG=C:\path\mpv.log writes mpv's full log.
        if let Some(log) = std::env::var_os("DROPREEL_MPV_LOG") {
            tracing::warn!(
                "DROPREEL_MPV_LOG is set: mpv writes its log to {} (only for troubleshooting; \
                 some mpv builds can hang while writing it)",
                log.to_string_lossy()
            );
            player.option("log-file", &log.to_string_lossy());
        }
        player.check(unsafe { (lib.initialize)(handle) }, "initialize mpv")?;
        // mpv's messages come in as events and go to our log (see `log_message`); mpv drops
        // them rather than wait when we fall behind.
        let level = cstr("v");
        let rc = unsafe { (lib.request_log_messages)(handle, level.as_ptr()) };
        if rc < 0 {
            tracing::warn!("mpv log messages: {}", player.error_text(rc));
        }
        if let Some(version) = player.get_string("mpv-version") {
            tracing::info!("{version}");
        }

        player.default_scaling = SIMPLE_SCALING
            .iter()
            .map(|(name, _)| (*name, player.get_string(name).unwrap_or_default()))
            .collect();

        for (id, name, format) in [
            (OBS_TIME_POS, "time-pos", ffi::MPV_FORMAT_DOUBLE),
            (OBS_DURATION, "duration", ffi::MPV_FORMAT_DOUBLE),
            (OBS_PAUSE, "pause", ffi::MPV_FORMAT_FLAG),
            (OBS_VOLUME, "volume", ffi::MPV_FORMAT_DOUBLE),
            (OBS_MUTE, "mute", ffi::MPV_FORMAT_FLAG),
            (OBS_EOF, "eof-reached", ffi::MPV_FORMAT_FLAG),
            (OBS_TRACKS, "track-list", ffi::MPV_FORMAT_NODE),
        ] {
            player.observe(id, name, format);
        }

        let wake = &*player._waker as *const Waker as *mut c_void;
        unsafe { (lib.set_wakeup_callback)(handle, Some(wake_up), wake) };
        Ok(player)
    }

    fn error_text(&self, code: c_int) -> String {
        unsafe { CStr::from_ptr((self.lib.error_string)(code)) }
            .to_string_lossy()
            .into_owned()
    }

    fn check(&self, code: c_int, what: &str) -> Result<(), String> {
        if code < 0 {
            Err(format!("Could not {what}: {}", self.error_text(code)))
        } else {
            Ok(())
        }
    }

    fn option(&self, name: &str, value: &str) {
        let rc = unsafe {
            (self.lib.set_option_string)(self.handle, cstr(name).as_ptr(), cstr(value).as_ptr())
        };
        if rc < 0 {
            tracing::warn!("mpv option {name}={value}: {}", self.error_text(rc));
        }
    }

    // Everything below goes to mpv asynchronously. The UI thread is also the thread that
    // renders mpv's video, and mpv's video output waits for it to render (up to 200 ms per
    // frame) while mpv's core waits for the video output, for instance while a new file opens.
    // A call that waited for the core in that moment would stall the whole window until those
    // waits ran out, two or three times per file. mpv runs the requests in the order sent.

    fn request(&mut self, request: Request) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        self.requests.insert(id, request);
        id
    }

    /// Logs a request mpv refused outright (it then sends no reply).
    fn sent(&mut self, id: u64, rc: c_int, what: &str) -> Result<(), String> {
        if rc < 0 {
            self.requests.remove(&id);
            let e = format!("mpv {what}: {}", self.error_text(rc));
            tracing::warn!("{e}");
            return Err(e);
        }
        Ok(())
    }

    fn command_as(&mut self, args: &[&str], request: Request) -> Result<u64, String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let id = self.request(request);
        let rc = timed("command", || unsafe {
            (self.lib.command_async)(self.handle, id, ptrs.as_mut_ptr())
        });
        self.sent(id, rc, args.first().unwrap_or(&"command"))?;
        Ok(id)
    }

    fn command(&mut self, args: &[&str]) {
        let name = args.first().unwrap_or(&"command").to_string();
        let _ = self.command_as(args, Request::Command(name));
    }

    fn set(&mut self, name: &str, format: c_int, data: *mut c_void) {
        let id = self.request(Request::Set(name.to_string()));
        let rc = timed("set", || unsafe {
            (self.lib.set_property_async)(self.handle, id, cstr(name).as_ptr(), format, data)
        });
        let _ = self.sent(id, rc, &format!("set {name}"));
    }

    fn set_string(&mut self, name: &str, value: &str) {
        let value = cstr(value);
        let mut ptr = value.as_ptr();
        self.set(
            name,
            ffi::MPV_FORMAT_STRING,
            &mut ptr as *mut *const c_char as *mut c_void,
        );
    }

    fn set_flag(&mut self, name: &str, on: bool) {
        let mut v: c_int = on.into();
        self.set(
            name,
            ffi::MPV_FORMAT_FLAG,
            &mut v as *mut c_int as *mut c_void,
        );
    }

    fn set_f64(&mut self, name: &str, value: f64) {
        let mut v = value;
        self.set(
            name,
            ffi::MPV_FORMAT_DOUBLE,
            &mut v as *mut f64 as *mut c_void,
        );
    }

    /// Reads a property synchronously: only while setting up, before anything plays.
    fn get_string(&self, name: &str) -> Option<String> {
        let ptr = unsafe { (self.lib.get_property_string)(self.handle, cstr(name).as_ptr()) };
        if ptr.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.lib.free)(ptr as *mut c_void) };
        Some(s).filter(|s| !s.is_empty())
    }

    /// Switches between mpv's default high-quality scalers and simple bilinear scaling.
    pub fn set_high_quality_scaling(&mut self, high_quality: bool) {
        if high_quality {
            for (name, value) in self.default_scaling.clone() {
                if !value.is_empty() {
                    self.set_string(name, &value);
                }
            }
        } else {
            for (name, value) in SIMPLE_SCALING {
                self.set_string(name, value);
            }
        }
    }

    /// Smooth motion: sync video to the display refresh (resampling audio to match) and blend
    /// frames, which removes the judder of e.g. 24 fps films on a 60 Hz screen. Needs the
    /// refresh rate from `set_display_fps`; without it mpv keeps timing video to audio.
    pub fn set_smooth_motion(&mut self, on: bool) {
        self.set_string("video-sync", if on { "display-resample" } else { "audio" });
        self.set_string("interpolation", if on { "yes" } else { "no" });
    }

    /// Tells mpv the refresh rate of the monitor the video is on.
    pub fn set_display_fps(&mut self, fps: Option<f64>) {
        let changed = match (self.display_fps, fps) {
            (Some(a), Some(b)) => (a - b).abs() > 0.01,
            (a, b) => a.is_some() != b.is_some(),
        };
        if changed {
            self.display_fps = fps;
            tracing::info!("display refresh rate: {fps:?} Hz");
            self.set_string(
                "display-fps-override",
                &format!("{:.4}", fps.unwrap_or(0.0)),
            );
        }
    }

    /// Logs one line describing decoding and frame timing, for diagnosing choppy playback. The
    /// values are those of the file playing now; the line is logged once mpv has sent them all
    /// (by `poll_events`), and is then also available from `last_diagnostics`.
    pub fn log_diagnostics(&mut self) {
        let batch = self.next_request;
        let mut sent = 0;
        for (index, name) in DIAGNOSTIC_PROPS.iter().enumerate() {
            let id = self.request(Request::Diagnostics { batch, index });
            let rc = unsafe {
                (self.lib.get_property_async)(
                    self.handle,
                    id,
                    cstr(name).as_ptr(),
                    ffi::MPV_FORMAT_STRING,
                )
            };
            if self.sent(id, rc, &format!("get {name}")).is_ok() {
                sent += 1;
            }
        }
        self.diagnostics.push(DiagnosticsBatch {
            id: batch,
            values: vec![None; DIAGNOSTIC_PROPS.len()],
            remaining: sent,
            stats: self.stats.summary(),
        });
        self.finish_diagnostics();
    }

    /// The last line logged by `log_diagnostics`.
    pub fn last_diagnostics(&self) -> Option<&str> {
        self.last_diagnostics.as_deref()
    }

    fn diagnostics_value(&mut self, batch: u64, index: usize, value: Option<String>) {
        if let Some(b) = self.diagnostics.iter_mut().find(|b| b.id == batch) {
            b.values[index] = value;
            b.remaining = b.remaining.saturating_sub(1);
        }
        self.finish_diagnostics();
    }

    fn finish_diagnostics(&mut self) {
        while let Some(i) = self.diagnostics.iter().position(|b| b.remaining == 0) {
            let b = self.diagnostics.remove(i);
            let mut parts: Vec<String> = DIAGNOSTIC_PROPS
                .iter()
                .zip(b.values)
                .map(|(name, v)| format!("{name}={}", v.unwrap_or_else(|| "-".into())))
                .collect();
            parts.push(b.stats);
            let line = parts.join(", ");
            tracing::info!("playback stats: {line}");
            self.last_diagnostics = Some(line);
        }
    }

    // ----- playback -----

    /// Starts playing `path`, optionally from `start` seconds.
    pub fn load(&mut self, path: &Path, start: Option<f64>) -> Result<(), String> {
        // "start" applies to the next file loaded; reset it so later files start at 0.
        self.set_string(
            "start",
            &start.map_or("none".to_string(), |s| format!("{s:.3}")),
        );
        if let Some(t) = self.timeline.take().filter(|t| !t.reported) {
            tracing::info!("left the file before its picture: {}", t.summary());
        }
        self.state.position = start;
        self.state.duration = None;
        self.state.eof = false;
        self.loaded = false;
        self.started = false;
        self.has_picture = false;
        self.pending_seek = None;
        self.force_render = false;
        self.tracks.clear();
        self.stats = RenderStats::default();
        let id = self.command_as(
            &["loadfile", &path.to_string_lossy(), "replace"],
            Request::Load,
        )?;
        self.gate = Gate::AwaitingReply(id);
        self.timeline = Some(LoadTimeline::new());
        // Unpause only once the new file is queued. At the end of a file (keep-open), mpv pauses
        // again as soon as it's unpaused, and that pause would carry over to the new file. mpv
        // would send no change for it either (it last told us "paused"), so the app would think
        // the new file is playing while it sits on its first frame.
        self.set_paused(false);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.loaded = false;
        self.started = false;
        self.has_picture = false;
        self.pending_seek = None;
        self.force_render = false;
        self.tracks.clear();
        self.gate = Gate::Idle;
        self.timeline = None;
        self.command(&["stop"]);
    }

    // The getters below return values mpv pushed to us as property-change events (see
    // `poll_events`), so the UI never waits on mpv's core lock while drawing a frame.

    pub fn paused(&self) -> bool {
        self.state.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.state.paused = paused;
        self.set_flag("pause", paused);
    }

    pub fn position(&self) -> Option<f64> {
        self.state.position
    }

    pub fn duration(&self) -> Option<f64> {
        self.state.duration.filter(|d| *d > 0.0)
    }

    /// Whether [`render`](Self::render) has drawn a frame of the current file yet.
    pub fn has_picture(&self) -> bool {
        self.has_picture
    }

    /// Whether playback of the current file has started (its first frame is on its way).
    pub fn started(&self) -> bool {
        self.started
    }

    pub fn eof_reached(&self) -> bool {
        self.state.eof
    }

    /// Seeks to `seconds`. `exact` is slower but lands precisely; keyframe seeks are used while
    /// dragging the seek bar.
    pub fn seek(&mut self, seconds: f64, exact: bool) {
        if !self.loaded {
            // Replaces anything queued: this is where the user wants to be.
            self.pending_seek = Some(PendingSeek::To { seconds, exact });
            return;
        }
        let mode = if exact {
            "absolute+exact"
        } else {
            "absolute+keyframes"
        };
        self.command(&["seek", &format!("{seconds:.3}"), mode]);
    }

    pub fn seek_relative(&mut self, seconds: f64) {
        if !self.loaded {
            self.pending_seek = Some(match self.pending_seek {
                Some(PendingSeek::To { seconds: to, exact }) => PendingSeek::To {
                    seconds: (to + seconds).max(0.0),
                    exact,
                },
                Some(PendingSeek::By(by)) => PendingSeek::By(by + seconds),
                None => PendingSeek::By(seconds),
            });
            return;
        }
        self.command(&["seek", &format!("{seconds:.3}"), "relative"]);
    }

    pub fn volume(&self) -> f64 {
        self.state.volume
    }

    pub fn set_volume(&mut self, volume: f64) {
        let volume = volume.clamp(0.0, 130.0);
        self.state.volume = volume;
        self.set_f64("volume", volume);
    }

    pub fn muted(&self) -> bool {
        self.state.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.state.muted = muted;
        self.set_flag("mute", muted);
    }

    /// The current file's tracks (updated before each `TracksChanged`).
    pub fn tracks(&self) -> Vec<Track> {
        self.tracks.clone()
    }

    /// Selects an audio or subtitle track; `None` turns it off.
    pub fn select_track(&mut self, kind: TrackKind, id: Option<i64>) {
        let prop = match kind {
            TrackKind::Audio => "aid",
            TrackKind::Subtitle => "sid",
            TrackKind::Video => "vid",
        };
        self.set_string(prop, &id.map_or("no".to_string(), |id| id.to_string()));
        // Shown straight away; mpv's track list follows.
        for t in self.tracks.iter_mut().filter(|t| t.kind == kind) {
            t.selected = Some(t.id) == id;
        }
    }

    fn observe(&self, id: u64, name: &str, format: c_int) {
        let rc =
            unsafe { (self.lib.observe_property)(self.handle, id, cstr(name).as_ptr(), format) };
        if rc < 0 {
            tracing::warn!("mpv observe {name}: {}", self.error_text(rc));
        }
    }

    /// Drains mpv's event queue, updating the cached playback state. Never blocks.
    pub fn poll_events(&mut self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        loop {
            let ev = unsafe { &*(self.lib.wait_event)(self.handle, 0.0) };
            let current = self.gate == Gate::Current;
            match ev.event_id {
                ffi::MPV_EVENT_NONE => break,
                ffi::MPV_EVENT_START_FILE if !ev.data.is_null() => {
                    let start = unsafe { &*(ev.data as *const ffi::mpv_event_start_file) };
                    if let Gate::Expecting(entry) = self.gate
                        && entry.is_none_or(|e| e == start.playlist_entry_id)
                    {
                        self.gate = Gate::Current;
                        self.mark(|t| &mut t.start_file);
                    }
                }
                ffi::MPV_EVENT_FILE_LOADED if current => {
                    self.loaded = true;
                    self.mark(|t| &mut t.loaded);
                    match self.pending_seek.take() {
                        Some(PendingSeek::To { seconds, exact }) => self.seek(seconds, exact),
                        Some(PendingSeek::By(by)) => self.seek_relative(by),
                        None => {}
                    }
                    events.push(PlayerEvent::FileLoaded);
                }
                // After a load (or a seek): from here on, frames belong to the current file.
                ffi::MPV_EVENT_PLAYBACK_RESTART if current && self.loaded => {
                    if !self.started {
                        self.mark(|t| &mut t.restart);
                    }
                    self.started = true;
                    // mpv drew the file's first frame before saying so, while it still counted
                    // as the previous file's; when paused, no other frame follows to count.
                    self.force_render = true;
                }
                ffi::MPV_EVENT_END_FILE if current && !ev.data.is_null() => {
                    let end = unsafe { &*(ev.data as *const ffi::mpv_event_end_file) };
                    if end.reason == ffi::MPV_END_FILE_REASON_ERROR {
                        events.push(PlayerEvent::Error(self.error_text(end.error)));
                    }
                }
                ffi::MPV_EVENT_PROPERTY_CHANGE if !ev.data.is_null() => {
                    let prop = unsafe { &*(ev.data as *const ffi::mpv_event_property) };
                    if ev.reply_userdata == OBS_TRACKS {
                        if prop.format == ffi::MPV_FORMAT_NODE && !prop.data.is_null() {
                            // SAFETY: observed as MPV_FORMAT_NODE; valid until the next event.
                            self.tracks =
                                parse_tracks(unsafe { &*(prop.data as *const ffi::mpv_node) });
                            events.push(PlayerEvent::TracksChanged);
                        }
                    } else {
                        self.state.apply(ev.reply_userdata, prop);
                    }
                }
                ffi::MPV_EVENT_COMMAND_REPLY
                | ffi::MPV_EVENT_SET_PROPERTY_REPLY
                | ffi::MPV_EVENT_GET_PROPERTY_REPLY => self.reply(ev, &mut events),
                ffi::MPV_EVENT_LOG_MESSAGE if !ev.data.is_null() => {
                    let msg = unsafe { &*(ev.data as *const ffi::mpv_event_log_message) };
                    self.log_message(msg);
                }
                ffi::MPV_EVENT_SHUTDOWN => break,
                _ => {}
            }
        }
        self.check_slow_load();
        events
    }

    fn reply(&mut self, ev: &ffi::mpv_event, events: &mut Vec<PlayerEvent>) {
        let Some(request) = self.requests.remove(&ev.reply_userdata) else {
            return;
        };
        let failed = (ev.error < 0).then(|| self.error_text(ev.error));
        match request {
            Request::Load => {
                if Gate::AwaitingReply(ev.reply_userdata) != self.gate {
                    return; // A load since replaced by another.
                }
                if let Some(e) = failed {
                    self.gate = Gate::Idle;
                    events.push(PlayerEvent::Error(e));
                    return;
                }
                // mpv says which playlist entry the file became (0.33 and newer); its START_FILE
                // tells us when mpv has moved on to it.
                let entry = (!ev.data.is_null())
                    .then(|| unsafe { &(*(ev.data as *const ffi::mpv_event_command)).result })
                    .and_then(|result| node_map_get(result, "playlist_entry_id"))
                    .and_then(node_i64);
                self.gate = Gate::Expecting(entry);
            }
            Request::Command(name) => {
                if let Some(e) = failed {
                    tracing::debug!("mpv {name}: {e}");
                }
            }
            Request::Set(name) => {
                if let Some(e) = failed {
                    tracing::debug!("mpv set {name}: {e}");
                }
            }
            Request::Diagnostics { batch, index } => {
                let value = (failed.is_none() && !ev.data.is_null())
                    .then(|| unsafe { &*(ev.data as *const ffi::mpv_event_property) })
                    .filter(|p| p.format == ffi::MPV_FORMAT_STRING && !p.data.is_null())
                    .and_then(|p| unsafe { c_string(*(p.data as *const *const c_char)) });
                self.diagnostics_value(batch, index, value);
            }
        }
    }

    /// Passes mpv's warnings and general messages to our log, plus the detailed ones that say
    /// how a file is loading and whether rendering kept up.
    fn log_message(&mut self, msg: &ffi::mpv_event_log_message) {
        let (Some(prefix), Some(text)) = (unsafe { c_string(msg.prefix) }, unsafe {
            c_string(msg.text)
        }) else {
            return;
        };
        let text = text.trim_end();
        if text.contains("not being called or stuck")
            && let Some(t) = self.timeline.as_mut()
        {
            t.render_waits += 1;
        }
        if msg.log_level <= ffi::MPV_LOG_LEVEL_WARN {
            tracing::warn!("mpv [{prefix}] {text}");
        } else if msg.log_level <= ffi::MPV_LOG_LEVEL_INFO
            || [
                "not being called",
                "first video frame after restart",
                "playback restart complete",
                "reconfig to",
                "Opening done",
                "hwdec",
            ]
            .iter()
            .any(|s| text.contains(s))
            || prefix == "ao"
        {
            tracing::info!("mpv [{prefix}] {text}");
        }
    }

    /// Records when a step of loading the current file happened.
    fn mark(&mut self, step: impl FnOnce(&mut LoadTimeline) -> &mut Option<Duration>) {
        if let Some(t) = self.timeline.as_mut() {
            let at = t.at.elapsed();
            step(t).get_or_insert(at);
        }
    }

    /// The first picture of the current file is on screen (or, headless, playback started).
    fn report_load(&mut self) {
        if let Some(t) = self.timeline.as_mut().filter(|t| !t.reported) {
            t.reported = true;
            tracing::info!("file ready: {}", t.summary());
        }
    }

    fn check_slow_load(&mut self) {
        if self.render.is_null() && self.started {
            self.report_load();
        }
        if let Some(t) = self.timeline.as_mut()
            && !t.reported
            && t.at.elapsed() > SLOW_LOAD
        {
            t.reported = true;
            tracing::warn!(
                "no picture {:.1} s after loading (loaded {}, started {}, paused {}): {}",
                t.at.elapsed().as_secs_f64(),
                self.loaded,
                self.started,
                self.state.paused,
                t.summary()
            );
        }
    }

    // ----- rendering -----

    /// Renders the current frame into a texture of `size` pixels and returns it for egui. Call
    /// once per frame from the UI thread while the player is visible.
    pub fn render(
        &mut self,
        gl: &glow::Context,
        frame: &mut eframe::Frame,
        size: [i32; 2],
    ) -> Option<egui::TextureId> {
        if self.render.is_null() {
            return None;
        }
        let size = [size[0].max(1), size[1].max(1)];
        // Called once per displayed frame: tells mpv the previous frame reached the screen (eframe
        // swaps with vsync at the end of each frame), so it can time frames against the display's
        // refresh. The UI repaints every vsync while playing.
        timed("report_swap", || unsafe {
            (self.lib.render_context_report_swap)(self.render)
        });
        let refresh = Duration::from_secs_f64(1.0 / self.display_fps.unwrap_or(60.0));
        self.stats
            .frame(refresh, !self.state.paused && self.state.position.is_some());
        if let (Some(t), Some(prev)) = (self.timeline.as_mut(), self.stats.last_gap) {
            t.longest_frame = t.longest_frame.max(prev);
        }
        let resized = self.ensure_target(gl, frame, size).ok()?;
        let (target_fbo, texture_id) = self.target.as_ref().map(|t| (t.fbo, t.texture_id))?;
        let flags = timed("update", || unsafe {
            (self.lib.render_context_update)(self.render)
        });
        if resized || flags & ffi::MPV_RENDER_UPDATE_FRAME != 0 || self.force_render {
            self.force_render = false;
            unsafe {
                let previous = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
                reset_gl_state(gl);
                let mut fbo = ffi::mpv_opengl_fbo {
                    fbo: target_fbo.0.get() as c_int,
                    w: size[0],
                    h: size[1],
                    internal_format: 0,
                };
                // Row 0 of the texture is the top of the picture, which is what egui expects.
                let mut flip: c_int = 0;
                // Never wait inside the UI frame for the frame's display time: that wait plus
                // egui's own vsync wait made frames miss refreshes, worst at high resolutions.
                let mut block: c_int = 0;
                let mut params = [
                    ffi::mpv_render_param {
                        kind: ffi::MPV_RENDER_PARAM_OPENGL_FBO,
                        data: &mut fbo as *mut _ as *mut c_void,
                    },
                    ffi::mpv_render_param {
                        kind: ffi::MPV_RENDER_PARAM_FLIP_Y,
                        data: &mut flip as *mut _ as *mut c_void,
                    },
                    ffi::mpv_render_param {
                        kind: ffi::MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                        data: &mut block as *mut _ as *mut c_void,
                    },
                    ffi::mpv_render_param {
                        kind: ffi::MPV_RENDER_PARAM_INVALID,
                        data: std::ptr::null_mut(),
                    },
                ];
                let started = Instant::now();
                timed("render", || {
                    (self.lib.render_context_render)(self.render, params.as_mut_ptr())
                });
                self.stats.rendered(started.elapsed());
                // Only once the new file's playback has started: a redraw between "loaded" and
                // the first new frame still shows the previous file's last frame.
                if self.started && !self.has_picture {
                    self.has_picture = true;
                    self.report_load();
                }
                gl.bind_framebuffer(
                    glow::FRAMEBUFFER,
                    NonZeroU32::new(previous as u32).map(glow::NativeFramebuffer),
                );
            }
        }
        Some(texture_id)
    }

    /// Creates or resizes the render texture. Returns true if it changed.
    fn ensure_target(
        &mut self,
        gl: &glow::Context,
        frame: &mut eframe::Frame,
        size: [i32; 2],
    ) -> Result<bool, String> {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return Ok(false);
        }
        unsafe {
            let previous = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
            let (texture, fbo, texture_id) = match self.target.take() {
                Some(t) => (t.texture, t.fbo, t.texture_id),
                None => {
                    let texture = gl.create_texture()?;
                    let fbo = gl.create_framebuffer()?;
                    (texture, fbo, frame.register_native_glow_texture(texture))
                }
            };
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(texture),
                0,
            );
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.bind_framebuffer(
                glow::FRAMEBUFFER,
                NonZeroU32::new(previous as u32).map(glow::NativeFramebuffer),
            );
            self.target = Some(RenderTarget {
                fbo,
                texture,
                texture_id,
                size,
            });
        }
        Ok(true)
    }

    /// Frees the GL objects and mpv. Call from `on_exit` with the GL context current.
    pub fn destroy(&mut self, gl: Option<&glow::Context>) {
        if let (Some(gl), Some(t)) = (gl, self.target.take()) {
            unsafe {
                gl.delete_framebuffer(t.fbo);
                gl.delete_texture(t.texture);
            }
        }
        self.shutdown();
    }

    fn shutdown(&mut self) {
        unsafe {
            if !self.render.is_null() {
                (self.lib.render_context_free)(self.render);
                self.render = std::ptr::null_mut();
            }
            if !self.handle.is_null() {
                (self.lib.set_wakeup_callback)(self.handle, None, std::ptr::null_mut());
                (self.lib.terminate_destroy)(self.handle);
                self.handle = std::ptr::null_mut();
            }
        }
    }
}

/// Runs a call into mpv, logging it if it held up the UI thread.
fn timed<T>(what: &str, call: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = call();
    let took = started.elapsed();
    if took > SLOW_CALL {
        tracing::warn!(
            "mpv {what} took {:.0} ms on the UI thread",
            took.as_secs_f64() * 1000.0
        );
    }
    result
}

/// Copies a C string from mpv; `None` for null or empty.
unsafe fn c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    Some(s).filter(|s| !s.is_empty())
}

/// The items of an mpv node list (array or map), with their keys for a map.
fn node_items(node: &ffi::mpv_node) -> Vec<(Option<String>, &ffi::mpv_node)> {
    if node.format != ffi::MPV_FORMAT_NODE_ARRAY && node.format != ffi::MPV_FORMAT_NODE_MAP {
        return Vec::new();
    }
    // SAFETY: for these formats mpv sets `list`, with `num` values (and keys for a map).
    let list = unsafe { node.u.list };
    if list.is_null() {
        return Vec::new();
    }
    let list = unsafe { &*list };
    (0..list.num.max(0) as usize)
        .map(|i| unsafe {
            let key = (node.format == ffi::MPV_FORMAT_NODE_MAP && !list.keys.is_null())
                .then(|| c_string(*list.keys.add(i)))
                .flatten();
            (key, &*list.values.add(i))
        })
        .collect()
}

fn node_map_get<'a>(node: &'a ffi::mpv_node, key: &str) -> Option<&'a ffi::mpv_node> {
    if node.format != ffi::MPV_FORMAT_NODE_MAP {
        return None;
    }
    node_items(node)
        .into_iter()
        .find(|(k, _)| k.as_deref() == Some(key))
        .map(|(_, v)| v)
}

fn node_i64(node: &ffi::mpv_node) -> Option<i64> {
    // SAFETY: the union member read matches the node's format.
    match node.format {
        ffi::MPV_FORMAT_INT64 => Some(unsafe { node.u.int64 }),
        ffi::MPV_FORMAT_DOUBLE => Some(unsafe { node.u.double } as i64),
        _ => None,
    }
}

fn node_str(node: &ffi::mpv_node) -> Option<String> {
    (node.format == ffi::MPV_FORMAT_STRING)
        .then(|| unsafe { c_string(node.u.string) })
        .flatten()
}

fn node_flag(node: &ffi::mpv_node) -> bool {
    node.format == ffi::MPV_FORMAT_FLAG && unsafe { node.u.flag } != 0
}

/// Reads mpv's `track-list` property.
fn parse_tracks(list: &ffi::mpv_node) -> Vec<Track> {
    node_items(list)
        .into_iter()
        .filter_map(|(_, track)| {
            let get = |key| node_map_get(track, key);
            let kind = match get("type").and_then(node_str)?.as_str() {
                "video" => TrackKind::Video,
                "audio" => TrackKind::Audio,
                "sub" => TrackKind::Subtitle,
                _ => return None,
            };
            Some(Track {
                id: get("id").and_then(node_i64)?,
                kind,
                title: get("title").and_then(node_str),
                lang: get("lang").and_then(node_str),
                codec: get("codec").and_then(node_str),
                channels: get("demux-channel-count").and_then(node_i64),
                external: get("external").is_some_and(node_flag),
                selected: get("selected").is_some_and(node_flag),
            })
        })
        .collect()
}

/// libmpv expects GL_SCISSOR_TEST and GL_BLEND at their defaults (off) when rendering, but egui
/// leaves both enabled after painting. egui turns them back on itself when it paints.
unsafe fn reset_gl_state(gl: &glow::Context) {
    unsafe {
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
    }
}

/// Scaler options that mpv's defaults are swapped for when high-quality scaling is off.
const SIMPLE_SCALING: [(&str, &str); 4] = [
    ("scale", "bilinear"),
    ("cscale", "bilinear"),
    ("dscale", "bilinear"),
    ("dither-depth", "no"),
];

/// True for software OpenGL implementations, which are too slow for (and in Mesa's llvmpipe,
/// render black with) mpv's default multi-tap scalers.
pub fn is_software_renderer(renderer: &str) -> bool {
    let r = renderer.to_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swiftshader",
        "gdi generic",
        "basic render",
        "software",
    ]
    .iter()
    .any(|s| r.contains(s))
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_track_label_drops_long_titles() {
        let t = Track {
            id: 1,
            kind: TrackKind::Audio,
            title: Some(
                "GalaxyRG265 - Drive.2011.1080p.BluRay.DDP5.1.x265.10bit-GalaxyRG265".into(),
            ),
            lang: Some("eng".into()),
            codec: Some("eac3".into()),
            channels: Some(6),
            external: false,
            selected: true,
        };
        assert_eq!(t.short_label(), "eng · eac3 5.1");
        assert!(t.label().contains("GalaxyRG265"));
    }

    #[test]
    fn detects_software_renderers() {
        assert!(is_software_renderer("llvmpipe (LLVM 20.1.2, 256 bits)"));
        assert!(is_software_renderer("GDI Generic"));
        assert!(!is_software_renderer("NVIDIA GeForce RTX 4070/PCIe/SSE2"));
        assert!(!is_software_renderer("AMD Radeon RX 7800 XT"));
        assert!(!is_software_renderer("Intel(R) UHD Graphics 770"));
    }
}
