//! Safe wrapper around one libmpv instance plus the texture it renders into.

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
    /// Tracks were added, removed or (de)selected; re-read them with `Player::tracks`.
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
    /// Applies one property change. Returns true if the track list changed.
    fn apply(&mut self, id: u64, prop: &ffi::mpv_event_property) -> bool {
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
            OBS_TRACKS => return true,
            _ => {}
        }
        false
    }
}

/// Frame-pacing numbers for the log, collected since the current file was loaded.
#[derive(Default)]
struct RenderStats {
    last_frame: Option<Instant>,
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
    _wake_ctx: Box<egui::Context>,
    target: Option<RenderTarget>,
    /// mpv's own values for the options in SIMPLE_SCALING, to restore high-quality scaling.
    default_scaling: Vec<(&'static str, String)>,
    state: PlaybackState,
    display_fps: Option<f64>,
    stats: RenderStats,
}

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: ctx is the boxed GetProcAddress owned by the Player; name is a C string from mpv.
    let get = unsafe { &*(ctx as *const GetProcAddress) };
    get(unsafe { CStr::from_ptr(name) }) as *mut c_void
}

unsafe extern "C" fn wake_up(ctx: *mut c_void) {
    // SAFETY: ctx is the boxed egui::Context owned by the Player. request_repaint is thread-safe,
    // and mpv forbids calling mpv functions here, which we don't.
    unsafe { &*(ctx as *const egui::Context) }.request_repaint();
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
        let wake = &*player._wake_ctx as *const egui::Context as *mut c_void;
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
            _wake_ctx: Box::new(ctx),
            target: None,
            default_scaling: Vec::new(),
            state: PlaybackState::default(),
            display_fps: None,
            stats: RenderStats::default(),
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
        // Troubleshooting: DROPREEL_MPV_LOG=C:\path\mpv.log writes mpv's own log.
        if let Some(log) = std::env::var_os("DROPREEL_MPV_LOG") {
            player.option("log-file", &log.to_string_lossy());
        }
        player.check(unsafe { (lib.initialize)(handle) }, "initialize mpv")?;

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
            (OBS_TRACKS, "track-list", ffi::MPV_FORMAT_NONE),
        ] {
            player.observe(id, name, format);
        }

        let wake = &*player._wake_ctx as *const egui::Context as *mut c_void;
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

    fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|a| a.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let rc = unsafe { (self.lib.command)(self.handle, ptrs.as_mut_ptr()) };
        self.check(rc, &format!("run {}", args.first().unwrap_or(&"command")))
    }

    fn set_string(&self, name: &str, value: &str) {
        let rc = unsafe {
            (self.lib.set_property_string)(self.handle, cstr(name).as_ptr(), cstr(value).as_ptr())
        };
        if rc < 0 {
            tracing::debug!("mpv set {name}={value}: {}", self.error_text(rc));
        }
    }

    fn get_i64(&self, name: &str) -> Option<i64> {
        let mut v: i64 = 0;
        let rc = unsafe {
            (self.lib.get_property)(
                self.handle,
                cstr(name).as_ptr(),
                ffi::MPV_FORMAT_INT64,
                &mut v as *mut i64 as *mut c_void,
            )
        };
        (rc >= 0).then_some(v)
    }

    fn get_flag(&self, name: &str) -> bool {
        let mut v: c_int = 0;
        let rc = unsafe {
            (self.lib.get_property)(
                self.handle,
                cstr(name).as_ptr(),
                ffi::MPV_FORMAT_FLAG,
                &mut v as *mut c_int as *mut c_void,
            )
        };
        rc >= 0 && v != 0
    }

    fn set_flag(&self, name: &str, on: bool) {
        let mut v: c_int = on.into();
        unsafe {
            (self.lib.set_property)(
                self.handle,
                cstr(name).as_ptr(),
                ffi::MPV_FORMAT_FLAG,
                &mut v as *mut c_int as *mut c_void,
            );
        }
    }

    fn set_f64(&self, name: &str, value: f64) {
        let mut v = value;
        unsafe {
            (self.lib.set_property)(
                self.handle,
                cstr(name).as_ptr(),
                ffi::MPV_FORMAT_DOUBLE,
                &mut v as *mut f64 as *mut c_void,
            );
        }
    }

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
            for (name, value) in &self.default_scaling {
                if !value.is_empty() {
                    self.set_string(name, value);
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

    /// One log line describing decoding and frame timing, for diagnosing choppy playback.
    pub fn diagnostics(&self) -> String {
        let props = [
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
        let mut parts: Vec<String> = props
            .iter()
            .map(|name| {
                let value = self.get_string(name).unwrap_or_else(|| "-".into());
                format!("{name}={value}")
            })
            .collect();
        parts.push(self.stats.summary());
        parts.join(", ")
    }

    // ----- playback -----

    /// Starts playing `path`, optionally from `start` seconds.
    pub fn load(&mut self, path: &Path, start: Option<f64>) -> Result<(), String> {
        // "start" applies to the next file loaded; reset it so later files start at 0.
        self.set_string(
            "start",
            &start.map_or("none".to_string(), |s| format!("{s:.3}")),
        );
        self.set_paused(false);
        self.state.position = start;
        self.state.duration = None;
        self.state.eof = false;
        self.stats = RenderStats::default();
        self.command(&["loadfile", &path.to_string_lossy(), "replace"])
    }

    pub fn stop(&mut self) {
        let _ = self.command(&["stop"]);
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

    pub fn eof_reached(&self) -> bool {
        self.state.eof
    }

    /// Seeks to `seconds`. `exact` is slower but lands precisely; keyframe seeks are used while
    /// dragging the seek bar.
    pub fn seek(&mut self, seconds: f64, exact: bool) {
        let mode = if exact {
            "absolute+exact"
        } else {
            "absolute+keyframes"
        };
        let _ = self.command(&["seek", &format!("{seconds:.3}"), mode]);
    }

    pub fn seek_relative(&mut self, seconds: f64) {
        let _ = self.command(&["seek", &format!("{seconds:.3}"), "relative"]);
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

    /// Reads the track list (synchronously; call only when `TracksChanged` arrives).
    pub fn tracks(&self) -> Vec<Track> {
        let count = self.get_i64("track-list/count").unwrap_or(0);
        (0..count)
            .filter_map(|i| {
                let p = |key: &str| format!("track-list/{i}/{key}");
                let kind = match self.get_string(&p("type"))?.as_str() {
                    "video" => TrackKind::Video,
                    "audio" => TrackKind::Audio,
                    "sub" => TrackKind::Subtitle,
                    _ => return None,
                };
                Some(Track {
                    id: self.get_i64(&p("id"))?,
                    kind,
                    title: self.get_string(&p("title")),
                    lang: self.get_string(&p("lang")),
                    codec: self.get_string(&p("codec")),
                    channels: self.get_i64(&p("demux-channel-count")),
                    external: self.get_flag(&p("external")),
                    selected: self.get_flag(&p("selected")),
                })
            })
            .collect()
    }

    /// Selects an audio or subtitle track; `None` turns it off.
    pub fn select_track(&mut self, kind: TrackKind, id: Option<i64>) {
        let prop = match kind {
            TrackKind::Audio => "aid",
            TrackKind::Subtitle => "sid",
            TrackKind::Video => "vid",
        };
        self.set_string(prop, &id.map_or("no".to_string(), |id| id.to_string()));
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
            match ev.event_id {
                ffi::MPV_EVENT_NONE => break,
                ffi::MPV_EVENT_FILE_LOADED => events.push(PlayerEvent::FileLoaded),
                ffi::MPV_EVENT_END_FILE if !ev.data.is_null() => {
                    let end = unsafe { &*(ev.data as *const ffi::mpv_event_end_file) };
                    if end.reason == ffi::MPV_END_FILE_REASON_ERROR {
                        events.push(PlayerEvent::Error(self.error_text(end.error)));
                    }
                }
                ffi::MPV_EVENT_PROPERTY_CHANGE if !ev.data.is_null() => {
                    let prop = unsafe { &*(ev.data as *const ffi::mpv_event_property) };
                    if self.state.apply(ev.reply_userdata, prop) {
                        events.push(PlayerEvent::TracksChanged);
                    }
                }
                ffi::MPV_EVENT_SHUTDOWN => break,
                _ => {}
            }
        }
        events
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
        unsafe { (self.lib.render_context_report_swap)(self.render) };
        let refresh = Duration::from_secs_f64(1.0 / self.display_fps.unwrap_or(60.0));
        self.stats
            .frame(refresh, !self.state.paused && self.state.position.is_some());
        let resized = self.ensure_target(gl, frame, size).ok()?;
        let target = self.target.as_ref()?;
        let flags = unsafe { (self.lib.render_context_update)(self.render) };
        if resized || flags & ffi::MPV_RENDER_UPDATE_FRAME != 0 {
            unsafe {
                let previous = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
                reset_gl_state(gl);
                let mut fbo = ffi::mpv_opengl_fbo {
                    fbo: target.fbo.0.get() as c_int,
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
                (self.lib.render_context_render)(self.render, params.as_mut_ptr());
                self.stats.rendered(started.elapsed());
                gl.bind_framebuffer(
                    glow::FRAMEBUFFER,
                    NonZeroU32::new(previous as u32).map(glow::NativeFramebuffer),
                );
            }
        }
        Some(target.texture_id)
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
