//! Grabbing a frame from the movie to use as its poster when TMDb is not used.

use std::path::Path;

use crate::tools;

/// Tone-maps HDR (PQ/HLG) frames to SDR so the thumbnail is not washed out. Needs an ffmpeg
/// build with zimg; if that fails we fall back to a plain grab.
const TONEMAP: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p,";

/// Writes a JPEG frame from about 10% into the movie to `out`.
pub fn grab_frame(
    ffmpeg: &Path,
    file: &Path,
    duration: Option<f64>,
    hdr: bool,
    out: &Path,
) -> Result<(), String> {
    let at = duration.map(|d| d * 0.10).unwrap_or(30.0);
    let mut attempts: Vec<(f64, &str)> = Vec::new();
    if hdr {
        attempts.push((at, TONEMAP));
    }
    attempts.push((at, ""));
    attempts.push((0.0, ""));

    let mut last_error = String::new();
    for (seek, filter) in attempts {
        let _ = std::fs::remove_file(out);
        let output = tools::command(ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-ss",
                &format!("{seek:.2}"),
                "-i",
            ])
            .arg(file)
            .args([
                "-frames:v",
                "1",
                "-vf",
                &format!("{filter}scale=480:-2"),
                "-q:v",
                "3",
                "-f",
                "image2",
            ])
            .arg(out)
            .output()
            .map_err(|e| format!("could not run ffmpeg: {e}"))?;
        let written = std::fs::metadata(out).map(|m| m.len() > 0).unwrap_or(false);
        if output.status.success() && written {
            return Ok(());
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    }
    Err(format!("could not grab a frame: {last_error}"))
}
