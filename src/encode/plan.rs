//! Working out the video bitrate that lands the re-encoded file under the size limit.
//!
//! Audio and subtitle tracks are copied unchanged, so their size is fixed. Whatever is left of
//! the target size after them (and a little container overhead) goes to the video track.

use crate::metadata::probe::MediaInfo;
use crate::util::{GB, format_bytes};

/// Matroska overhead (headers, cues, block framing) as a fraction of the file.
pub const CONTAINER_OVERHEAD: f64 = 0.01;

/// Below this many bits per pixel per frame, HEVC video starts to look noticeably poor.
const MIN_BITS_PER_PIXEL: f64 = 0.025;
const MIN_VIDEO_KBPS: f64 = 500.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PlanInput {
    pub duration: f64,
    pub target_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Copied tracks: (codec, channels, known bit rate in bits/s).
    pub audio: Vec<(String, Option<u32>, Option<u64>)>,
    pub subtitles: Vec<(String, Option<u64>)>,
}

impl PlanInput {
    pub fn from_media(info: &MediaInfo, target_bytes: u64) -> Result<Self, String> {
        let duration = info
            .duration
            .ok_or("The file's duration is unknown, so a bitrate cannot be planned.")?;
        let video = info.video.as_ref().ok_or("The file has no video track.")?;
        Ok(Self {
            duration,
            target_bytes,
            width: video.width,
            height: video.height,
            fps: video.fps.unwrap_or(24.0),
            audio: info
                .audio
                .iter()
                .map(|a| (a.codec.clone(), a.channels, a.bit_rate))
                .collect(),
            subtitles: info
                .subtitles
                .iter()
                .map(|s| (s.codec.clone(), s.bit_rate))
                .collect(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EncodePlan {
    pub video_kbps: u32,
    /// Size of the copied audio/subtitle tracks.
    pub passthrough_bytes: u64,
    pub target_bytes: u64,
    /// Set when the video bitrate is low for this resolution; the user must confirm.
    pub warning: Option<String>,
}

/// Typical bit rates for tracks whose rate is not stored in the file.
pub fn estimate_audio_bps(codec: &str, channels: Option<u32>) -> u64 {
    let ch = u64::from(channels.unwrap_or(2).max(1));
    match codec {
        "truehd" | "mlp" => 4_000_000,
        "dts" => 1_536_000,
        "flac" | "alac" => 600_000 * ch,
        c if c.starts_with("pcm_") => 48_000 * 24 * ch,
        "eac3" => 768_000,
        "ac3" => 640_000,
        "opus" | "vorbis" => 64_000 * ch,
        "aac" | "mp3" | "mp2" => 96_000 * ch,
        _ => 640_000,
    }
}

pub fn estimate_subtitle_bps(codec: &str) -> u64 {
    match codec {
        "hdmv_pgs_subtitle" => 40_000,
        "dvd_subtitle" | "dvb_subtitle" => 10_000,
        _ => 1_000,
    }
}

/// Lowest video bitrate (kbit/s) we consider acceptable for this resolution.
pub fn min_video_kbps(width: u32, height: u32, fps: f64) -> f64 {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps.min(60.0)
    } else {
        24.0
    };
    (f64::from(width) * f64::from(height) * fps * MIN_BITS_PER_PIXEL / 1000.0).max(MIN_VIDEO_KBPS)
}

pub fn plan(input: &PlanInput) -> Result<EncodePlan, String> {
    if !(input.duration.is_finite() && input.duration > 0.0) {
        return Err("The file's duration is unknown, so a bitrate cannot be planned.".into());
    }
    let passthrough_bps: u64 = input
        .audio
        .iter()
        .map(|(codec, ch, bps)| bps.unwrap_or_else(|| estimate_audio_bps(codec, *ch)))
        .chain(
            input
                .subtitles
                .iter()
                .map(|(codec, bps)| bps.unwrap_or_else(|| estimate_subtitle_bps(codec))),
        )
        .sum();
    let passthrough_bytes = (passthrough_bps as f64 * input.duration / 8.0) as u64;
    let usable_bytes =
        input.target_bytes as f64 * (1.0 - CONTAINER_OVERHEAD) - passthrough_bytes as f64;
    if usable_bytes <= 0.0 {
        return Err(format!(
            "The audio and subtitle tracks alone are about {} — there is no room left for video under {}.",
            format_bytes(passthrough_bytes),
            format_bytes(input.target_bytes)
        ));
    }
    let video_kbps = usable_bytes * 8.0 / input.duration / 1000.0;
    let floor = min_video_kbps(input.width, input.height, input.fps);
    let warning = (video_kbps < floor).then(|| {
        format!(
            "Only {:.1} Mbit/s is left for video after the copied audio/subtitle tracks ({}). At {}×{} this may look poor (at least {:.1} Mbit/s recommended).",
            video_kbps / 1000.0,
            format_bytes(passthrough_bytes),
            input.width,
            input.height,
            floor / 1000.0
        )
    });
    Ok(EncodePlan {
        video_kbps: video_kbps.floor() as u32,
        passthrough_bytes,
        target_bytes: input.target_bytes,
        warning,
    })
}

/// Default target: 4.75 GB, i.e. 95% of the 5 GB limit.
pub const DEFAULT_TARGET_BYTES: u64 = GB * 475 / 100;

#[cfg(test)]
mod tests {
    use super::*;

    fn input(duration: f64, audio: Vec<(String, Option<u32>, Option<u64>)>) -> PlanInput {
        PlanInput {
            duration,
            target_bytes: DEFAULT_TARGET_BYTES,
            width: 1920,
            height: 1080,
            fps: 23.976,
            audio,
            subtitles: vec![],
        }
    }

    #[test]
    fn two_hour_1080p_movie_with_ac3() {
        let p = plan(&input(7200.0, vec![("ac3".into(), Some(6), Some(640_000))])).unwrap();
        // (4.75 GB × 0.99 − 576 MB of audio) × 8 / 7200 s
        let expected = (4_750_000_000.0 * 0.99 - 576_000_000.0) * 8.0 / 7200.0 / 1000.0;
        assert_eq!(p.passthrough_bytes, 576_000_000);
        assert!(
            (f64::from(p.video_kbps) - expected).abs() < 1.0,
            "{} vs {expected}",
            p.video_kbps
        );
        assert!(p.warning.is_none());
    }

    #[test]
    fn predicted_size_stays_under_the_target() {
        let i = input(
            5400.0,
            vec![
                ("dts".into(), Some(6), None),
                ("aac".into(), Some(2), Some(160_000)),
            ],
        );
        let p = plan(&i).unwrap();
        let total =
            (f64::from(p.video_kbps) * 1000.0 * i.duration / 8.0) + p.passthrough_bytes as f64;
        assert!(total <= i.target_bytes as f64 * (1.0 - CONTAINER_OVERHEAD) + 1.0);
        assert!(total > i.target_bytes as f64 * 0.95);
    }

    #[test]
    fn unknown_bitrates_use_codec_estimates() {
        let p = plan(&input(3600.0, vec![("truehd".into(), Some(8), None)])).unwrap();
        assert_eq!(p.passthrough_bytes, 4_000_000 * 3600 / 8);
        assert_eq!(estimate_audio_bps("aac", Some(2)), 192_000);
        assert_eq!(estimate_subtitle_bps("hdmv_pgs_subtitle"), 40_000);
    }

    #[test]
    fn huge_audio_leaves_no_room() {
        let tracks = (0..4)
            .map(|_| ("truehd".to_string(), Some(8), Some(5_000_000)))
            .collect();
        let err = plan(&input(7200.0, tracks)).unwrap_err();
        assert!(err.contains("no room"), "{err}");
    }

    #[test]
    fn low_bitrate_for_4k_warns() {
        let mut i = input(
            3.0 * 3600.0,
            vec![("truehd".into(), Some(8), Some(2_500_000))],
        );
        i.width = 3840;
        i.height = 2160;
        let p = plan(&i).unwrap();
        assert!(p.warning.is_some(), "{p:?}");
    }

    #[test]
    fn unknown_duration_is_an_error() {
        assert!(plan(&input(0.0, vec![])).is_err());
        assert!(plan(&input(f64::NAN, vec![])).is_err());
    }
}
