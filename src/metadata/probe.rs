//! Reading technical details of a video file with `ffprobe`.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::library::TrackInfo;
use crate::tools;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoStream {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: Option<String>,
    pub fps: Option<f64>,
    pub color_primaries: Option<String>,
    pub color_transfer: Option<String>,
    pub color_space: Option<String>,
    pub bit_rate: Option<u64>,
}

impl VideoStream {
    pub fn is_high_bit_depth(&self) -> bool {
        self.pix_fmt
            .as_deref()
            .is_some_and(|f| f.contains("10") || f.contains("12"))
    }

    pub fn is_hdr(&self) -> bool {
        matches!(
            self.color_transfer.as_deref(),
            Some("smpte2084" | "arib-std-b67")
        )
    }
}

/// An audio or subtitle stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OtherStream {
    pub codec: String,
    pub channels: Option<u32>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub bit_rate: Option<u64>,
}

impl OtherStream {
    pub fn to_track_info(&self) -> TrackInfo {
        TrackInfo {
            codec: self.codec.clone(),
            language: self.language.clone(),
            title: self.title.clone(),
            channels: self.channels,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub video: Option<VideoStream>,
    pub audio: Vec<OtherStream>,
    pub subtitles: Vec<OtherStream>,
}

#[derive(Deserialize)]
struct FfOutput {
    #[serde(default)]
    streams: Vec<FfStream>,
    format: Option<FfFormat>,
}

#[derive(Deserialize)]
struct FfFormat {
    duration: Option<String>,
}

#[derive(Deserialize, Default)]
struct FfStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    color_primaries: Option<String>,
    color_transfer: Option<String>,
    color_space: Option<String>,
    channels: Option<u32>,
    bit_rate: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    disposition: HashMap<String, i64>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

impl FfStream {
    fn tag(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }

    /// Bit rate from ffprobe, or from the statistics tags mkvmerge writes (`BPS`,
    /// `NUMBER_OF_BYTES`).
    fn bit_rate(&self, container_duration: Option<f64>) -> Option<u64> {
        let parse = |s: &str| s.trim().parse::<u64>().ok().filter(|&v| v > 0);
        if let Some(b) = self.bit_rate.as_deref().and_then(parse) {
            return Some(b);
        }
        if let Some(b) = self
            .tag("BPS")
            .or_else(|| self.tag("BPS-eng"))
            .and_then(parse)
        {
            return Some(b);
        }
        let bytes = self
            .tag("NUMBER_OF_BYTES")
            .or_else(|| self.tag("NUMBER_OF_BYTES-eng"))
            .and_then(parse)?;
        let duration = self
            .duration
            .as_deref()
            .and_then(parse_f64)
            .or(container_duration)
            .filter(|d| *d > 0.0)?;
        Some((bytes as f64 * 8.0 / duration) as u64)
    }

    fn other(&self, container_duration: Option<f64>) -> OtherStream {
        OtherStream {
            codec: self.codec_name.clone().unwrap_or_else(|| "unknown".into()),
            channels: self.channels,
            language: self
                .tag("language")
                .filter(|l| *l != "und")
                .map(str::to_string),
            title: self.tag("title").map(str::to_string),
            bit_rate: self.bit_rate(container_duration),
        }
    }
}

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

fn known(value: &Option<String>) -> Option<String> {
    value
        .clone()
        .filter(|v| !v.is_empty() && v != "unknown" && v != "reserved")
}

/// Parses `ffprobe -print_format json -show_format -show_streams` output.
pub fn parse_ffprobe_json(json: &str) -> Result<MediaInfo, String> {
    let out: FfOutput =
        serde_json::from_str(json).map_err(|e| format!("unexpected ffprobe output: {e}"))?;
    let duration = out
        .format
        .as_ref()
        .and_then(|f| f.duration.as_deref())
        .and_then(parse_f64)
        .filter(|d| *d > 0.0);
    let mut info = MediaInfo {
        duration,
        ..MediaInfo::default()
    };
    for s in &out.streams {
        match s.codec_type.as_deref() {
            Some("video")
                if info.video.is_none() && s.disposition.get("attached_pic") != Some(&1) =>
            {
                info.video = Some(VideoStream {
                    codec: s.codec_name.clone().unwrap_or_else(|| "unknown".into()),
                    width: s.width.unwrap_or(0),
                    height: s.height.unwrap_or(0),
                    pix_fmt: s.pix_fmt.clone(),
                    fps: s
                        .avg_frame_rate
                        .as_deref()
                        .and_then(parse_rate)
                        .or_else(|| s.r_frame_rate.as_deref().and_then(parse_rate)),
                    color_primaries: known(&s.color_primaries),
                    color_transfer: known(&s.color_transfer),
                    color_space: known(&s.color_space),
                    bit_rate: s.bit_rate(duration),
                });
            }
            Some("audio") => info.audio.push(s.other(duration)),
            Some("subtitle") => info.subtitles.push(s.other(duration)),
            _ => {}
        }
    }
    Ok(info)
}

pub fn probe(ffprobe: &Path, file: &Path) -> Result<MediaInfo, String> {
    let output = tools::command(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(file)
        .output()
        .map_err(|e| format!("could not run ffprobe: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffprobe could not read the file: {}", err.trim()));
    }
    parse_ffprobe_json(&String::from_utf8_lossy(&output.stdout))
}

/// HDR10 static metadata, formatted for x265's `master-display` / `max-cll` options.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrMetadata {
    pub master_display: Option<String>,
    pub max_cll: Option<String>,
}

#[derive(Deserialize)]
struct FfFrames {
    #[serde(default)]
    frames: Vec<FfFrame>,
}

#[derive(Deserialize)]
struct FfFrame {
    #[serde(default)]
    side_data_list: Vec<HashMap<String, serde_json::Value>>,
}

fn fraction(v: Option<&serde_json::Value>) -> Option<f64> {
    match v? {
        serde_json::Value::String(s) => match s.split_once('/') {
            Some((n, d)) => {
                let d: f64 = d.parse().ok()?;
                (d != 0.0).then_some(n.parse::<f64>().ok()? / d)
            }
            None => s.parse().ok(),
        },
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// Parses the side data of the first video frame
/// (`ffprobe -show_frames -read_intervals %+#1 -show_entries frame=side_data_list`).
pub fn parse_hdr_side_data(json: &str) -> Option<HdrMetadata> {
    let frames: FfFrames = serde_json::from_str(json).ok()?;
    let side_data = &frames.frames.first()?.side_data_list;
    let find = |kind: &str| {
        side_data
            .iter()
            .find(|d| d.get("side_data_type").and_then(|t| t.as_str()) == Some(kind))
    };
    let master_display = find("Mastering display metadata").and_then(|d| {
        // x265 wants chromaticities in units of 0.00002 and luminance in units of 0.0001.
        let c = |k: &str| fraction(d.get(k)).map(|v| (v * 50_000.0).round() as u64);
        let l = |k: &str| fraction(d.get(k)).map(|v| (v * 10_000.0).round() as u64);
        Some(format!(
            "G({},{})B({},{})R({},{})WP({},{})L({},{})",
            c("green_x")?,
            c("green_y")?,
            c("blue_x")?,
            c("blue_y")?,
            c("red_x")?,
            c("red_y")?,
            c("white_point_x")?,
            c("white_point_y")?,
            l("max_luminance")?,
            l("min_luminance")?
        ))
    });
    let max_cll = find("Content light level metadata").and_then(|d| {
        Some(format!(
            "{},{}",
            fraction(d.get("max_content"))? as u64,
            fraction(d.get("max_average"))? as u64
        ))
    });
    (master_display.is_some() || max_cll.is_some()).then_some(HdrMetadata {
        master_display,
        max_cll,
    })
}

pub fn probe_hdr(ffprobe: &Path, file: &Path) -> Option<HdrMetadata> {
    let output = tools::command(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-read_intervals",
            "%+#1",
            "-show_frames",
            "-show_entries",
            "frame=side_data_list",
            "-print_format",
            "json",
        ])
        .arg(file)
        .output()
        .ok()?;
    parse_hdr_side_data(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MKV: &str = r#"{
      "streams": [
        {"index":0,"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
         "pix_fmt":"yuv420p","avg_frame_rate":"24000/1001","disposition":{"attached_pic":0},
         "tags":{"BPS":"9000000"}},
        {"index":1,"codec_type":"audio","codec_name":"ac3","channels":6,
         "tags":{"language":"eng","BPS-eng":"640000","title":"Surround"}},
        {"index":2,"codec_type":"audio","codec_name":"dts","channels":6,
         "tags":{"language":"und","NUMBER_OF_BYTES":"900000000"}},
        {"index":3,"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"eng"}},
        {"index":4,"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}
      ],
      "format": {"duration":"7200.000000","size":"9000000000"}
    }"#;

    #[test]
    fn parses_streams_and_bitrates() {
        let info = parse_ffprobe_json(MKV).unwrap();
        assert_eq!(info.duration, Some(7200.0));
        let v = info.video.unwrap();
        assert_eq!((v.codec.as_str(), v.width, v.height), ("h264", 1920, 1080));
        assert!((v.fps.unwrap() - 23.976).abs() < 0.01);
        assert_eq!(v.bit_rate, Some(9_000_000));
        assert!(!v.is_high_bit_depth());
        assert_eq!(info.audio.len(), 2);
        assert_eq!(info.audio[0].bit_rate, Some(640_000));
        assert_eq!(info.audio[0].language.as_deref(), Some("eng"));
        assert_eq!(info.audio[1].language, None, "'und' means unknown");
        assert_eq!(
            info.audio[1].bit_rate,
            Some(1_000_000),
            "from NUMBER_OF_BYTES / duration"
        );
        assert_eq!(info.subtitles.len(), 1);
        assert_eq!(info.subtitles[0].bit_rate, None);
    }

    #[test]
    fn hdr_side_data_is_converted_to_x265_units() {
        let json = r#"{"frames":[{"side_data_list":[
          {"side_data_type":"Mastering display metadata",
           "red_x":"34000/50000","red_y":"16000/50000","green_x":"13250/50000","green_y":"34500/50000",
           "blue_x":"7500/50000","blue_y":"3000/50000","white_point_x":"15635/50000","white_point_y":"16450/50000",
           "min_luminance":"50/10000","max_luminance":"10000000/10000"},
          {"side_data_type":"Content light level metadata","max_content":1000,"max_average":400}
        ]}]}"#;
        let hdr = parse_hdr_side_data(json).unwrap();
        assert_eq!(
            hdr.master_display.as_deref(),
            Some("G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,50)")
        );
        assert_eq!(hdr.max_cll.as_deref(), Some("1000,400"));
        assert!(parse_hdr_side_data(r#"{"frames":[{"side_data_list":[]}]}"#).is_none());
    }
}
