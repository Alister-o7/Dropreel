//! Finding and launching the bundled `ffmpeg.exe` / `ffprobe.exe`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::settings::Settings;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tools {
    pub ffmpeg: Option<PathBuf>,
    pub ffprobe: Option<PathBuf>,
}

impl Tools {
    /// Looks for the tools in this order: the path set in Settings, next to `DropReel.exe`,
    /// then on `PATH`.
    pub fn locate(settings: &Settings) -> Self {
        Self {
            ffmpeg: find_tool("ffmpeg", settings.ffmpeg_path.as_deref()),
            ffprobe: find_tool("ffprobe", settings.ffprobe_path.as_deref()),
        }
    }

    pub fn ffmpeg(&self) -> Result<&Path, String> {
        self.ffmpeg
            .as_deref()
            .ok_or_else(|| "ffmpeg was not found. Put ffmpeg.exe next to DropReel.exe or set its path in Settings.".into())
    }

    pub fn ffprobe(&self) -> Result<&Path, String> {
        self.ffprobe
            .as_deref()
            .ok_or_else(|| "ffprobe was not found. Put ffprobe.exe next to DropReel.exe or set its path in Settings.".into())
    }
}

fn find_tool(name: &str, configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured.filter(|p| !p.as_os_str().is_empty()) {
        // An explicit setting wins; if it is wrong we report "not found" rather than silently
        // using a different binary.
        return path.is_file().then(|| path.to_path_buf());
    }
    let file_name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        let candidate = dir.join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    which::which(name).ok()
}

/// Like [`command`], but at below-normal CPU priority on Windows so a long encode doesn't make
/// the UI or playback stutter.
pub fn background_command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = command(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    cmd
}

/// A `Command` that never opens a console window on Windows and has no stdin.
pub fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
