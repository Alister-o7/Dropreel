//! Minimal bindings to libmpv's client and render API, loaded at runtime.
//!
//! Loading `libmpv-2.dll` dynamically means DropReel still starts (with playback disabled and a
//! clear message) when the DLL is missing, and builds without mpv's import library.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_double, c_int, c_ulong, c_void};
use std::path::PathBuf;
use std::sync::OnceLock;

#[repr(C)]
pub struct mpv_handle {
    _private: [u8; 0],
}

#[repr(C)]
pub struct mpv_render_context {
    _private: [u8; 0],
}

#[repr(C)]
pub struct mpv_event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_end_file {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_render_param {
    pub kind: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_opengl_init_params {
    pub get_proc_address:
        Option<unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void>,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct mpv_opengl_fbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

pub const MPV_FORMAT_NONE: c_int = 0;
pub const MPV_FORMAT_FLAG: c_int = 3;
pub const MPV_FORMAT_INT64: c_int = 4;
pub const MPV_FORMAT_DOUBLE: c_int = 5;

pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

pub const MPV_END_FILE_REASON_ERROR: c_int = 4;

pub const MPV_RENDER_PARAM_INVALID: c_int = 0;
pub const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
pub const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const MPV_RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const MPV_RENDER_PARAM_FLIP_Y: c_int = 4;
pub const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;

pub const MPV_RENDER_UPDATE_FRAME: u64 = 1;

pub type Callback = unsafe extern "C" fn(ctx: *mut c_void);

pub struct MpvLib {
    _lib: libloading::Library,
    pub client_api_version: unsafe extern "C" fn() -> c_ulong,
    pub error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    pub free: unsafe extern "C" fn(*mut c_void),
    pub create: unsafe extern "C" fn() -> *mut mpv_handle,
    pub initialize: unsafe extern "C" fn(*mut mpv_handle) -> c_int,
    pub terminate_destroy: unsafe extern "C" fn(*mut mpv_handle),
    pub set_option_string:
        unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int,
    pub set_property_string:
        unsafe extern "C" fn(*mut mpv_handle, *const c_char, *const c_char) -> c_int,
    pub set_property:
        unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int,
    pub get_property:
        unsafe extern "C" fn(*mut mpv_handle, *const c_char, c_int, *mut c_void) -> c_int,
    pub get_property_string: unsafe extern "C" fn(*mut mpv_handle, *const c_char) -> *mut c_char,
    pub command: unsafe extern "C" fn(*mut mpv_handle, *mut *const c_char) -> c_int,
    pub wait_event: unsafe extern "C" fn(*mut mpv_handle, c_double) -> *mut mpv_event,
    pub observe_property: unsafe extern "C" fn(*mut mpv_handle, u64, *const c_char, c_int) -> c_int,
    pub set_wakeup_callback: unsafe extern "C" fn(*mut mpv_handle, Option<Callback>, *mut c_void),
    pub render_context_create: unsafe extern "C" fn(
        *mut *mut mpv_render_context,
        *mut mpv_handle,
        *mut mpv_render_param,
    ) -> c_int,
    pub render_context_set_update_callback:
        unsafe extern "C" fn(*mut mpv_render_context, Option<Callback>, *mut c_void),
    pub render_context_update: unsafe extern "C" fn(*mut mpv_render_context) -> u64,
    pub render_context_render:
        unsafe extern "C" fn(*mut mpv_render_context, *mut mpv_render_param) -> c_int,
    pub render_context_report_swap: unsafe extern "C" fn(*mut mpv_render_context),
    pub render_context_free: unsafe extern "C" fn(*mut mpv_render_context),
}

fn candidates() -> Vec<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["libmpv-2.dll", "mpv-2.dll", "mpv-1.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.2.dylib", "libmpv.dylib"]
    } else {
        &["libmpv.so.2", "libmpv.so"]
    };
    let mut paths = Vec::new();
    if let Some(custom) = std::env::var_os("DROPREEL_LIBMPV") {
        paths.push(PathBuf::from(custom));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        paths.extend(names.iter().map(|n| dir.join(n)));
    }
    paths.extend(names.iter().map(PathBuf::from));
    paths
}

unsafe fn load(lib: libloading::Library) -> Result<MpvLib, String> {
    macro_rules! sym {
        ($name:literal) => {
            *unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                .map_err(|e| format!("libmpv is missing {}: {e}", $name))?
        };
    }
    Ok(MpvLib {
        client_api_version: sym!("mpv_client_api_version"),
        error_string: sym!("mpv_error_string"),
        free: sym!("mpv_free"),
        create: sym!("mpv_create"),
        initialize: sym!("mpv_initialize"),
        terminate_destroy: sym!("mpv_terminate_destroy"),
        set_option_string: sym!("mpv_set_option_string"),
        set_property_string: sym!("mpv_set_property_string"),
        set_property: sym!("mpv_set_property"),
        get_property: sym!("mpv_get_property"),
        get_property_string: sym!("mpv_get_property_string"),
        command: sym!("mpv_command"),
        wait_event: sym!("mpv_wait_event"),
        observe_property: sym!("mpv_observe_property"),
        set_wakeup_callback: sym!("mpv_set_wakeup_callback"),
        render_context_create: sym!("mpv_render_context_create"),
        render_context_set_update_callback: sym!("mpv_render_context_set_update_callback"),
        render_context_update: sym!("mpv_render_context_update"),
        render_context_render: sym!("mpv_render_context_render"),
        render_context_report_swap: sym!("mpv_render_context_report_swap"),
        render_context_free: sym!("mpv_render_context_free"),
        _lib: lib,
    })
}

/// Loads libmpv once. Requires client API 2.x (mpv 0.35 or newer).
pub fn lib() -> Result<&'static MpvLib, String> {
    static LIB: OnceLock<Result<MpvLib, String>> = OnceLock::new();
    LIB.get_or_init(|| {
        let mut errors = Vec::new();
        for path in candidates() {
            // SAFETY: loading libmpv runs only its own initialisers.
            match unsafe { libloading::Library::new(&path) } {
                Ok(lib) => {
                    let mpv = unsafe { load(lib) }?;
                    let major = unsafe { (mpv.client_api_version)() } >> 16;
                    if major != 2 {
                        return Err(format!("libmpv API version {major}.x is not supported (need 2.x, mpv 0.35 or newer)"));
                    }
                    tracing::info!("loaded libmpv from {}", path.display());
                    return Ok(mpv);
                }
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }
        tracing::warn!("libmpv not found: {}", errors.join("; "));
        Err(if cfg!(windows) {
            "libmpv-2.dll was not found. Put it next to DropReel.exe to enable the built-in player.".into()
        } else {
            "libmpv was not found. Install mpv's library to enable the built-in player.".into()
        })
    })
    .as_ref()
    .map_err(Clone::clone)
}
