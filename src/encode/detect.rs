//! Choosing an HEVC encoder: the first GPU encoder that works, else x265.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::settings::EncoderChoice;
use crate::tools;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Encoder {
    Nvenc,
    Amf,
    Qsv,
    X265,
}

/// GPU encoders in the order we try them.
pub const GPU_PREFERENCE: [Encoder; 3] = [Encoder::Nvenc, Encoder::Amf, Encoder::Qsv];

impl Encoder {
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::Nvenc => "hevc_nvenc",
            Self::Amf => "hevc_amf",
            Self::Qsv => "hevc_qsv",
            Self::X265 => "libx265",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Nvenc => "NVIDIA NVENC (GPU)",
            Self::Amf => "AMD AMF (GPU)",
            Self::Qsv => "Intel Quick Sync (GPU)",
            Self::X265 => "x265 (CPU)",
        }
    }

    fn from_choice(choice: EncoderChoice) -> Option<Self> {
        match choice {
            EncoderChoice::Auto => None,
            EncoderChoice::Nvenc => Some(Self::Nvenc),
            EncoderChoice::Amf => Some(Self::Amf),
            EncoderChoice::Qsv => Some(Self::Qsv),
            EncoderChoice::X265 => Some(Self::X265),
        }
    }
}

/// Picks the encoder for `choice`. `usable` says whether an encoder works on this PC.
pub fn select_encoder(
    choice: EncoderChoice,
    mut usable: impl FnMut(Encoder) -> bool,
) -> Result<Encoder, String> {
    match Encoder::from_choice(choice) {
        Some(enc) if usable(enc) => Ok(enc),
        Some(enc) => Err(format!(
            "{} is selected in Settings but does not work on this PC (or this ffmpeg build lacks {}). Choose another encoder.",
            enc.label(),
            enc.ffmpeg_name()
        )),
        None => GPU_PREFERENCE
            .into_iter()
            .chain([Encoder::X265])
            .find(|&enc| usable(enc))
            .ok_or_else(|| {
                "This ffmpeg build has no HEVC encoder (libx265, NVENC, AMF or QSV).".to_string()
            }),
    }
}

/// Tests encoders with a tiny encode and caches the answers per ffmpeg binary.
#[derive(Default)]
pub struct EncoderDetector {
    cache: Mutex<HashMap<(PathBuf, Encoder), bool>>,
    listed: Mutex<HashMap<PathBuf, String>>,
}

impl EncoderDetector {
    pub fn select(&self, ffmpeg: &Path, choice: EncoderChoice) -> Result<Encoder, String> {
        select_encoder(choice, |enc| self.is_usable(ffmpeg, enc))
    }

    pub fn is_usable(&self, ffmpeg: &Path, enc: Encoder) -> bool {
        let key = (ffmpeg.to_path_buf(), enc);
        if let Some(&ok) = self.cache.lock().unwrap().get(&key) {
            return ok;
        }
        // A GPU encoder being compiled in does not mean the hardware exists, so after the cheap
        // listing check we run a real 3-frame encode.
        let ok = self.is_listed(ffmpeg, enc) && test_encode(ffmpeg, enc);
        tracing::info!("encoder {} usable: {ok}", enc.ffmpeg_name());
        self.cache.lock().unwrap().insert(key, ok);
        ok
    }

    fn is_listed(&self, ffmpeg: &Path, enc: Encoder) -> bool {
        let mut listed = self.listed.lock().unwrap();
        let text = listed.entry(ffmpeg.to_path_buf()).or_insert_with(|| {
            tools::command(ffmpeg)
                .args(["-hide_banner", "-encoders"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        });
        text.split_whitespace()
            .any(|word| word == enc.ffmpeg_name())
    }
}

fn test_encode(ffmpeg: &Path, enc: Encoder) -> bool {
    tools::command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=256x256:r=24",
            "-frames:v",
            "3",
            "-c:v",
        ])
        .arg(enc.ffmpeg_name())
        .args(["-f", "null", "-"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(available: &'static [Encoder]) -> impl FnMut(Encoder) -> bool {
        move |e| available.contains(&e)
    }

    #[test]
    fn auto_prefers_nvenc_then_amf_then_qsv_then_x265() {
        use Encoder::*;
        let auto = EncoderChoice::Auto;
        assert_eq!(
            select_encoder(auto, only(&[Nvenc, Amf, Qsv, X265])),
            Ok(Nvenc)
        );
        assert_eq!(select_encoder(auto, only(&[Amf, Qsv, X265])), Ok(Amf));
        assert_eq!(select_encoder(auto, only(&[Qsv, X265])), Ok(Qsv));
        assert_eq!(select_encoder(auto, only(&[X265])), Ok(X265));
        assert!(select_encoder(auto, only(&[])).is_err());
    }

    #[test]
    fn auto_tests_encoders_in_order_and_stops_at_the_first_that_works() {
        let mut tried = Vec::new();
        let picked = select_encoder(EncoderChoice::Auto, |e| {
            tried.push(e);
            e == Encoder::Qsv
        });
        assert_eq!(picked, Ok(Encoder::Qsv));
        assert_eq!(tried, vec![Encoder::Nvenc, Encoder::Amf, Encoder::Qsv]);
    }

    #[test]
    fn explicit_choice_is_used_or_reported() {
        assert_eq!(
            select_encoder(EncoderChoice::X265, only(&[Encoder::Nvenc, Encoder::X265])),
            Ok(Encoder::X265)
        );
        let err = select_encoder(EncoderChoice::Nvenc, only(&[Encoder::X265])).unwrap_err();
        assert!(err.contains("NVENC"), "{err}");
    }
}
