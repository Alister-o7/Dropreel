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

/// How re-encodes trade speed for quality. The size is the same either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncodeQuality {
    /// The first working GPU encoder, else x265 at its `fast` preset.
    #[default]
    Fast,
    /// x265 on the CPU at its `slow` preset, always 10-bit: hours rather than minutes.
    Best,
    /// The encoder and x265 preset chosen in Settings.
    Custom,
}

impl EncodeQuality {
    pub const ALL: [EncodeQuality; 3] = [Self::Fast, Self::Best, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Best => "Best quality",
            Self::Custom => "Custom",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Fast => {
                "Uses your graphics card when it can: usually well under an hour for a movie."
            }
            Self::Best => {
                "x265 on the CPU at its slow preset, in 10-bit for smoother gradients. Sharper at the same size, but a movie can take several hours."
            }
            Self::Custom => "Pick the encoder and x265 preset yourself.",
        }
    }
}

/// How the player shows the time next to the seek bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeDisplay {
    /// Time played on the left, the movie's length on the right.
    #[default]
    Normal,
    /// Only the time left, on the right, like Netflix.
    Remaining,
}

impl TimeDisplay {
    pub const ALL: [TimeDisplay; 2] = [Self::Normal, Self::Remaining];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Time played and length",
            Self::Remaining => "Time left (like Netflix)",
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

/// How long the player's controls may wait before fading out, in seconds.
pub const HIDE_CONTROLS_RANGE: std::ops::RangeInclusive<f64> = 1.0..=30.0;

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
    pub quality: EncodeQuality,
    /// Used with [`EncodeQuality::Custom`].
    pub encoder: EncoderChoice,
    pub x265_preset: String,
    /// Files larger than this many decimal GB get a re-encode suggestion.
    pub threshold_gb: f64,
    pub theme: ThemeChoice,
    /// mpv's high-quality scalers. Turn off if video shows black on an unusual graphics driver.
    pub high_quality_video: bool,
    /// Sync video to the display refresh and blend frames (mpv's display-resample + interpolation).
    pub smooth_motion: bool,
    /// Seconds without mouse movement before the player's controls fade out.
    pub hide_controls_after: f64,
    pub time_display: TimeDisplay,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            watched_folders: Vec::new(),
            tmdb_api_key: String::new(),
            ffmpeg_path: None,
            ffprobe_path: None,
            quality: EncodeQuality::Fast,
            encoder: EncoderChoice::Auto,
            x265_preset: "medium".into(),
            threshold_gb: 5.0,
            theme: ThemeChoice::Dark,
            high_quality_video: true,
            // Off by default: on some displays and drivers it makes playback worse, not better.
            smooth_motion: false,
            hide_controls_after: 2.5,
            time_display: TimeDisplay::Normal,
        }
    }
}

impl Settings {
    /// Loads settings, falling back to defaults. The second value describes a problem with the
    /// file, if there was one, so the UI can tell the user.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(mut settings) => {
                    // Files from before quality presets: keep a hand-picked encoder or preset.
                    let has_quality = text
                        .parse::<toml::Table>()
                        .is_ok_and(|t| t.contains_key("quality"));
                    if !has_quality
                        && (settings.encoder != EncoderChoice::Auto
                            || settings.x265_preset != "medium")
                    {
                        settings.quality = EncodeQuality::Custom;
                    }
                    (settings.sanitized(), None)
                }
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
        self.hide_controls_after = if self.hide_controls_after.is_finite() {
            self.hide_controls_after
                .clamp(*HIDE_CONTROLS_RANGE.start(), *HIDE_CONTROLS_RANGE.end())
        } else {
            Self::default().hide_controls_after
        };
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

    /// The encoder the quality preset asks for.
    pub fn effective_encoder(&self) -> EncoderChoice {
        match self.quality {
            EncodeQuality::Fast => EncoderChoice::Auto,
            EncodeQuality::Best => EncoderChoice::X265,
            EncodeQuality::Custom => self.encoder,
        }
    }

    /// The x265 preset the quality preset asks for.
    pub fn effective_x265_preset(&self) -> &str {
        match self.quality {
            EncodeQuality::Fast => "fast",
            EncodeQuality::Best => "slow",
            EncodeQuality::Custom => &self.x265_preset,
        }
    }

    /// Whether 8-bit sources are encoded in 10-bit too (higher-bit-depth sources always are).
    pub fn always_ten_bit(&self) -> bool {
        self.quality == EncodeQuality::Best
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
    fn the_target_size_follows_the_threshold() {
        let mut settings = Settings::default();
        assert_eq!(settings.target_bytes(), 4_750_000_000);
        settings.threshold_gb = 8.0;
        assert_eq!(settings.target_bytes(), 7_600_000_000);
        settings.threshold_gb = 2.0;
        assert_eq!(settings.target_bytes(), 1_900_000_000);
    }

    #[test]
    fn quality_presets_pick_the_encoder_and_preset() {
        let mut settings = Settings::default();
        assert_eq!(settings.quality, EncodeQuality::Fast);
        assert_eq!(settings.effective_encoder(), EncoderChoice::Auto);
        assert_eq!(settings.effective_x265_preset(), "fast");
        assert!(!settings.always_ten_bit());

        settings.quality = EncodeQuality::Best;
        assert_eq!(settings.effective_encoder(), EncoderChoice::X265);
        assert_eq!(settings.effective_x265_preset(), "slow");
        assert!(settings.always_ten_bit());

        settings.quality = EncodeQuality::Custom;
        settings.encoder = EncoderChoice::Qsv;
        settings.x265_preset = "slower".into();
        assert_eq!(settings.effective_encoder(), EncoderChoice::Qsv);
        assert_eq!(settings.effective_x265_preset(), "slower");
        assert!(!settings.always_ten_bit());
    }

    #[test]
    fn older_settings_with_a_chosen_encoder_become_custom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "encoder = \"x265\"\nx265_preset = \"slower\"").unwrap();
        let (loaded, _) = Settings::load(&path);
        assert_eq!(loaded.quality, EncodeQuality::Custom);
        assert_eq!(loaded.effective_x265_preset(), "slower");

        std::fs::write(&path, "threshold_gb = 8.0").unwrap();
        assert_eq!(Settings::load(&path).0.quality, EncodeQuality::Fast);

        std::fs::write(&path, "quality = \"best\"\nencoder = \"nvenc\"").unwrap();
        assert_eq!(Settings::load(&path).0.quality, EncodeQuality::Best);
    }

    #[test]
    fn fresh_settings_and_the_hide_delay() {
        let fresh = Settings::default();
        assert!(fresh.high_quality_video);
        assert!(!fresh.smooth_motion);
        assert_eq!(fresh.hide_controls_after, 2.5);

        // Files from before the option existed get the default; odd values are clamped.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "smooth_motion = true").unwrap();
        let (old, _) = Settings::load(&path);
        assert!(old.smooth_motion, "existing choices are kept");
        assert_eq!(old.hide_controls_after, 2.5);
        for (written, expected) in [("0.2", 1.0), ("120.0", 30.0), ("7.5", 7.5)] {
            std::fs::write(&path, format!("hide_controls_after = {written}")).unwrap();
            assert_eq!(Settings::load(&path).0.hide_controls_after, expected);
        }
    }

    #[test]
    fn target_is_95_percent_of_threshold() {
        let settings = Settings::default();
        assert_eq!(settings.threshold_bytes(), 5_000_000_000);
        assert_eq!(settings.target_bytes(), 4_750_000_000);
    }
}
