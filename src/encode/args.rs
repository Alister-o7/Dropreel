//! Building ffmpeg command lines for the re-encode.

use std::ffi::OsString;
use std::path::Path;

use super::Encoder;
use crate::metadata::probe::{HdrMetadata, VideoStream};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    /// GPU encoders: one pass (NVENC does its own internal multipass).
    Single,
    /// x265 two-pass: analysis pass, output discarded.
    First,
    Second,
}

pub struct EncodeSpec<'a> {
    pub input: &'a Path,
    /// Where ffmpeg writes (the temporary `.part` file).
    pub output: &'a Path,
    pub encoder: Encoder,
    pub video_kbps: u32,
    pub x265_preset: &'a str,
    /// Encode in 10-bit even when the source is 8-bit (smoother gradients, less banding).
    pub always_ten_bit: bool,
    pub video: &'a VideoStream,
    pub hdr: Option<&'a HdrMetadata>,
    /// Codec of each subtitle stream, in order.
    pub subtitle_codecs: &'a [String],
    /// x265 stats file name. ffmpeg runs with the temp folder as working directory, so this is a
    /// bare name (x265 option strings use `:` as a separator, which breaks on `C:\`).
    pub stats_file: &'a str,
}

pub fn passes(encoder: Encoder) -> &'static [Pass] {
    match encoder {
        Encoder::X265 => &[Pass::First, Pass::Second],
        _ => &[Pass::Single],
    }
}

fn s(v: impl Into<String>) -> OsString {
    OsString::from(v.into())
}

fn video_args(spec: &EncodeSpec<'_>, pass: Pass) -> Vec<OsString> {
    let k = spec.video_kbps;
    let rate_control = || -> Vec<OsString> {
        vec![
            s("-b:v"),
            s(format!("{k}k")),
            s("-maxrate"),
            s(format!("{}k", k * 3 / 2)),
            s("-bufsize"),
            s(format!("{}k", k * 2)),
        ]
    };
    let ten_bit = spec.always_ten_bit || spec.video.is_high_bit_depth();
    let mut a: Vec<OsString> = vec![s("-c:v"), s(spec.encoder.ffmpeg_name())];
    match spec.encoder {
        Encoder::X265 => {
            a.extend([
                s("-preset"),
                s(spec.x265_preset),
                s("-b:v"),
                s(format!("{k}k")),
            ]);
            a.extend([
                s("-pix_fmt"),
                s(if ten_bit { "yuv420p10le" } else { "yuv420p" }),
            ]);
            let pass_no = if pass == Pass::First { 1 } else { 2 };
            let mut params = format!("pass={pass_no}:stats={}:log-level=error", spec.stats_file);
            if spec.video.is_hdr() {
                let transfer = spec.video.color_transfer.as_deref().unwrap_or("smpte2084");
                params.push_str(&format!(
                    ":colorprim=bt2020:transfer={transfer}:colormatrix=bt2020nc:repeat-headers=1"
                ));
                if let Some(hdr) = spec.hdr {
                    params.push_str(":hdr10=1:hdr10-opt=1");
                    if let Some(md) = &hdr.master_display {
                        params.push_str(&format!(":master-display={md}"));
                    }
                    if let Some(cll) = &hdr.max_cll {
                        params.push_str(&format!(":max-cll={cll}"));
                    }
                }
            }
            a.extend([s("-x265-params"), s(params)]);
        }
        Encoder::Nvenc => {
            a.extend(
                [
                    "-preset",
                    "p5",
                    "-tune",
                    "hq",
                    "-rc",
                    "vbr",
                    "-multipass",
                    "fullres",
                    "-spatial-aq",
                    "1",
                ]
                .map(s),
            );
            a.extend(rate_control());
        }
        Encoder::Amf => {
            a.extend(["-quality", "quality", "-rc", "vbr_peak"].map(s));
            a.extend(rate_control());
        }
        Encoder::Qsv => {
            a.extend(["-preset", "slow"].map(s));
            a.extend(rate_control());
        }
    }
    if ten_bit && spec.encoder != Encoder::X265 {
        a.extend(["-profile:v", "main10", "-pix_fmt", "p010le"].map(s));
    }
    // Keep colour signalling (BT.2020/PQ for HDR) on every encoder.
    for (flag, value) in [
        ("-color_primaries", &spec.video.color_primaries),
        ("-color_trc", &spec.video.color_transfer),
        ("-colorspace", &spec.video.color_space),
    ] {
        if let Some(v) = value {
            a.extend([s(flag), s(v.as_str())]);
        }
    }
    a
}

/// Full ffmpeg argument list for one pass.
pub fn ffmpeg_args(spec: &EncodeSpec<'_>, pass: Pass) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-nostats",
        "-i",
    ]
    .map(s)
    .into();
    a.push(spec.input.as_os_str().to_owned());
    // `V` (capital) skips cover-art pictures that some MP4s carry as video streams.
    a.extend(["-map", "0:V:0"].map(s));
    if pass == Pass::First {
        a.extend(video_args(spec, pass));
        a.extend(["-an", "-sn", "-dn", "-f", "null", "-"].map(s));
        return a;
    }
    a.extend(
        [
            "-map",
            "0:a?",
            "-map",
            "0:s?",
            "-map",
            "0:t?",
            "-map_chapters",
            "0",
            "-map_metadata",
            "0",
        ]
        .map(s),
    );
    a.extend(video_args(spec, pass));
    a.extend(["-c:a", "copy", "-c:s", "copy", "-c:t", "copy"].map(s));
    // MP4 text subtitles cannot be stored in Matroska as-is; convert just those to SRT.
    for (i, codec) in spec.subtitle_codecs.iter().enumerate() {
        if codec == "mov_text" {
            a.extend([s(format!("-c:s:{i}")), s("srt")]);
        }
    }
    a.extend(["-max_muxing_queue_size", "4096", "-f", "matroska"].map(s));
    a.push(spec.output.as_os_str().to_owned());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(pix_fmt: &str, transfer: Option<&str>) -> VideoStream {
        VideoStream {
            codec: "h264".into(),
            width: 3840,
            height: 2160,
            pix_fmt: Some(pix_fmt.into()),
            fps: Some(24.0),
            color_primaries: transfer.map(|_| "bt2020".into()),
            color_transfer: transfer.map(Into::into),
            color_space: transfer.map(|_| "bt2020nc".into()),
            bit_rate: None,
        }
    }

    fn joined(args: &[OsString]) -> String {
        args.iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn spec<'a>(
        encoder: Encoder,
        v: &'a VideoStream,
        subs: &'a [String],
        hdr: Option<&'a HdrMetadata>,
    ) -> EncodeSpec<'a> {
        EncodeSpec {
            input: Path::new("in.mkv"),
            output: Path::new("out.hevc.mkv.part"),
            encoder,
            video_kbps: 4000,
            x265_preset: "medium",
            always_ten_bit: false,
            video: v,
            hdr,
            subtitle_codecs: subs,
            stats_file: "stats.log",
        }
    }

    #[test]
    fn x265_uses_two_passes_with_a_shared_stats_file() {
        let v = video("yuv420p", None);
        let sp = spec(Encoder::X265, &v, &[], None);
        assert_eq!(passes(Encoder::X265), &[Pass::First, Pass::Second]);
        let first = joined(&ffmpeg_args(&sp, Pass::First));
        assert!(
            first.contains("-x265-params pass=1:stats=stats.log"),
            "{first}"
        );
        assert!(first.ends_with("-an -sn -dn -f null -"), "{first}");
        let second = joined(&ffmpeg_args(&sp, Pass::Second));
        assert!(second.contains("pass=2:stats=stats.log"));
        assert!(second.contains("-map 0:a? -map 0:s? -map 0:t? -map_chapters 0"));
        assert!(second.contains("-c:a copy -c:s copy"));
        assert!(second.contains("-pix_fmt yuv420p "));
        assert!(second.ends_with("-f matroska out.hevc.mkv.part"));
    }

    #[test]
    fn best_quality_encodes_eight_bit_sources_in_ten_bit() {
        let v = video("yuv420p", None);
        let mut sp = spec(Encoder::X265, &v, &[], None);
        sp.always_ten_bit = true;
        let a = joined(&ffmpeg_args(&sp, Pass::Second));
        assert!(a.contains("-pix_fmt yuv420p10le"), "{a}");
        sp.encoder = Encoder::Nvenc;
        let a = joined(&ffmpeg_args(&sp, Pass::Single));
        assert!(a.contains("-profile:v main10 -pix_fmt p010le"), "{a}");
    }

    #[test]
    fn nvenc_ten_bit_hdr_keeps_main10_and_colour_tags() {
        let v = video("yuv420p10le", Some("smpte2084"));
        let a = joined(&ffmpeg_args(
            &spec(Encoder::Nvenc, &v, &[], None),
            Pass::Single,
        ));
        assert!(a.contains("-c:v hevc_nvenc"));
        assert!(a.contains("-b:v 4000k -maxrate 6000k -bufsize 8000k"));
        assert!(a.contains("-profile:v main10 -pix_fmt p010le"));
        assert!(a.contains("-color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc"));
        assert_eq!(passes(Encoder::Nvenc), &[Pass::Single]);
    }

    #[test]
    fn x265_hdr_passes_mastering_metadata() {
        let v = video("yuv420p10le", Some("smpte2084"));
        let hdr = HdrMetadata {
            master_display: Some("G(1,2)B(3,4)R(5,6)WP(7,8)L(9,10)".into()),
            max_cll: Some("1000,400".into()),
        };
        let a = joined(&ffmpeg_args(
            &spec(Encoder::X265, &v, &[], Some(&hdr)),
            Pass::Second,
        ));
        assert!(a.contains("-pix_fmt yuv420p10le"));
        assert!(a.contains(":hdr10=1"));
        assert!(a.contains(":master-display=G(1,2)B(3,4)R(5,6)WP(7,8)L(9,10):max-cll=1000,400"));
    }

    #[test]
    fn mp4_text_subtitles_are_converted_for_matroska() {
        let v = video("yuv420p", None);
        let subs = vec!["subrip".to_string(), "mov_text".to_string()];
        let a = joined(&ffmpeg_args(
            &spec(Encoder::Amf, &v, &subs, None),
            Pass::Single,
        ));
        assert!(a.contains("-c:s copy -c:t copy -c:s:1 srt"), "{a}");
        assert!(!a.contains("-c:s:0 srt"));
    }
}
