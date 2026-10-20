//! Safe wrapper around one libmpv instance plus the texture it renders into.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;

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
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    FileLoaded,
    /// The file could not be played.
    Error(String),
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
    _get_proc: Box<GetProcAddress>,
    _wake_ctx: Box<egui::Context>,
    target: Option<RenderTarget>,
    /// mpv's own values for the options in SIMPLE_SCALING, to restore high-quality scaling.
    default_scaling: Vec<(&'static str, String)>,
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
        let lib = ffi::lib()?;
        let handle = unsafe { (lib.create)() };
        if handle.is_null() {
            return Err("mpv could not be created".into());
        }
        let mut player = Self {
            lib,
            handle,
            render: std::ptr::null_mut(),
            _get_proc: Box::new(get_proc),
            _wake_ctx: Box::new(ctx),
            target: None,
            default_scaling: Vec::new(),
        };
        for (name, value) in [
            ("vo", "libmpv"),
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

        let api = cstr("opengl");
        let mut init = ffi::mpv_opengl_init_params {
            get_proc_address: Some(get_proc_address),
            get_proc_address_ctx: &*player._get_proc as *const GetProcAddress as *mut c_void,
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
        let rc = unsafe { (lib.render_context_create)(&mut render, handle, params.as_mut_ptr()) };
        player.check(rc, "create the mpv OpenGL renderer")?;
        player.render = render;

        player.default_scaling = SIMPLE_SCALING
            .iter()
            .map(|(name, _)| (*name, player.get_string(name).unwrap_or_default()))
            .collect();

        let wake = &*player._wake_ctx as *const egui::Context as *mut c_void;
        unsafe {
            (lib.render_context_set_update_callback)(render, Some(wake_up), wake);
            (lib.set_wakeup_callback)(handle, Some(wake_up), wake);
        }
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

    fn get_f64(&self, name: &str) -> Option<f64> {
        let mut v: f64 = 0.0;
        let rc = unsafe {
            (self.lib.get_property)(
                self.handle,
                cstr(name).as_ptr(),
                ffi::MPV_FORMAT_DOUBLE,
                &mut v as *mut f64 as *mut c_void,
            )
        };
        (rc >= 0 && v.is_finite()).then_some(v)
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

    // ----- playback -----

    /// Starts playing `path`, optionally from `start` seconds.
    pub fn load(&mut self, path: &Path, start: Option<f64>) -> Result<(), String> {
        // "start" applies to the next file loaded; reset it so later files start at 0.
        self.set_string(
            "start",
            &start.map_or("none".to_string(), |s| format!("{s:.3}")),
        );
        self.set_flag("pause", false);
        self.command(&["loadfile", &path.to_string_lossy(), "replace"])
    }

    pub fn stop(&mut self) {
        let _ = self.command(&["stop"]);
    }

    pub fn paused(&self) -> bool {
        self.get_flag("pause")
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.set_flag("pause", paused);
    }

    pub fn position(&self) -> Option<f64> {
        self.get_f64("time-pos")
    }

    pub fn duration(&self) -> Option<f64> {
        self.get_f64("duration").filter(|d| *d > 0.0)
    }

    pub fn eof_reached(&self) -> bool {
        self.get_flag("eof-reached")
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
        self.get_f64("volume").unwrap_or(100.0)
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.set_f64("volume", volume.clamp(0.0, 130.0));
    }

    pub fn muted(&self) -> bool {
        self.get_flag("mute")
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.set_flag("mute", muted);
    }

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

    /// Drains mpv's event queue.
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
        let size = [size[0].max(1), size[1].max(1)];
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
                        kind: ffi::MPV_RENDER_PARAM_INVALID,
                        data: std::ptr::null_mut(),
                    },
                ];
                (self.lib.render_context_render)(self.render, params.as_mut_ptr());
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
    fn detects_software_renderers() {
        assert!(is_software_renderer("llvmpipe (LLVM 20.1.2, 256 bits)"));
        assert!(is_software_renderer("GDI Generic"));
        assert!(!is_software_renderer("NVIDIA GeForce RTX 4070/PCIe/SSE2"));
        assert!(!is_software_renderer("AMD Radeon RX 7800 XT"));
        assert!(!is_software_renderer("Intel(R) UHD Graphics 770"));
    }
}
