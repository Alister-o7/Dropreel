//! User settings, stored as TOML in `settings.toml`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util::GB;

/// Which HEVC encoder to use for re-encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncoderChoice {
    /// Use the first working GPU encoder (NVENC → AMF → QSV), else x265 on the CPU.
    #[default]
    Auto,
    Nvenc,
    Amf,
    Qsv,
    X265,
}

impl EncoderChoice {
    pub const ALL: [EncoderChoice; 5] = [Self::Auto, Self::Nvenc, Self::Amf, Self::Qsv, Self::X265];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (GPU if available, else x265)",
            Self::Nvenc => "NVIDIA NVENC (GPU)",
            Self::Amf => "AMD AMF (GPU)",
            Self::Qsv => "Intel Quick Sync (GPU)",
            Self::X265 => "x265 (CPU, best quality per GB)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

pub const X265_PRESETS: [&str; 7] = [
    "ultrafast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folders whose new files are imported automatically.
    pub watched_folders: Vec<PathBuf>,
    /// TMDb API key (v3 key or v4 read access token). Empty disables TMDb.
    pub tmdb_api_key: String,
    pub ffmpeg_path: Option<PathBuf>,
    pub ffprobe_path: Option<PathBuf>,
    pub encoder: EncoderChoice,
    pub x265_preset: String,
    /// Files larger than this many decimal GB get a re-encode suggestion.
    pub threshold_gb: f64,
    pub theme: ThemeChoice,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            watched_folders: Vec::new(),
            tmdb_api_key: String::new(),
            ffmpeg_path: None,
            ffprobe_path: None,
            encoder: EncoderChoice::Auto,
            x265_preset: "medium".into(),
            threshold_gb: 5.0,
            theme: ThemeChoice::Dark,
        }
    }
}

impl Settings {
    /// Loads settings, falling back to defaults. The second value describes a problem with the
    /// file, if there was one, so the UI can tell the user.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(settings) => (settings.sanitized(), None),
                Err(err) => (
                    Self::default(),
                    Some(format!("settings.toml is invalid and was ignored: {err}")),
                ),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(err) => (
                Self::default(),
                Some(format!("could not read settings.toml: {err}")),
            ),
        }
    }

    /// Writes the settings atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn sanitized(mut self) -> Self {
        if !self.threshold_gb.is_finite() || self.threshold_gb < 0.5 {
            self.threshold_gb = 5.0;
        }
        if !X265_PRESETS.contains(&self.x265_preset.as_str()) {
            self.x265_preset = "medium".into();
        }
        self.tmdb_api_key = self.tmdb_api_key.trim().to_string();
        self
    }

    /// Size above which a re-encode is suggested.
    pub fn threshold_bytes(&self) -> u64 {
        (self.threshold_gb * GB as f64) as u64
    }

    /// Size the encoder aims for: 95% of the threshold (4.75 GB for 5 GB), leaving headroom.
    pub fn target_bytes(&self) -> u64 {
        (self.threshold_bytes() as f64 * 0.95) as u64
    }

    pub fn tmdb_enabled(&self) -> bool {
        !self.tmdb_api_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let settings = Settings {
            watched_folders: vec![PathBuf::from("D:/Movies")],
            tmdb_api_key: "abc".into(),
            encoder: EncoderChoice::Nvenc,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let (loaded, warning) = Settings::load(&path);
        assert_eq!(loaded, settings);
        assert!(warning.is_none());
    }

    #[test]
    fn missing_file_gives_defaults_and_invalid_file_warns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        assert_eq!(Settings::load(&path), (Settings::default(), None));
        std::fs::write(&path, "threshold_gb = \"lots\"").unwrap();
        let (loaded, warning) = Settings::load(&path);
        assert_eq!(loaded, Settings::default());
        assert!(warning.is_some());
    }

    #[test]
    fn target_is_95_percent_of_threshold() {
        let settings = Settings::default();
        assert_eq!(settings.threshold_bytes(), 5_000_000_000);
        assert_eq!(settings.target_bytes(), 4_750_000_000);
    }
}
