//! Checking a finished encode before it replaces anything.

use std::path::Path;

use crate::tools;
use crate::util::format_bytes;

/// Allowed difference between source and output duration.
pub const DURATION_TOLERANCE: f64 = 1.0;

/// Decides whether an encode is good: under the limit, same duration, readable.
pub fn check_output(
    output_size: u64,
    limit_bytes: u64,
    source_duration: f64,
    probed_duration: Result<f64, String>,
) -> Result<(), String> {
    if output_size >= limit_bytes {
        return Err(format!(
            "The encoded file is {}, which is still not under {}. Try the x265 (CPU) encoder in Settings for more accurate sizing.",
            format_bytes(output_size),
            format_bytes(limit_bytes)
        ));
    }
    let duration =
        probed_duration.map_err(|e| format!("The encoded file could not be read back: {e}"))?;
    if (duration - source_duration).abs() > DURATION_TOLERANCE {
        return Err(format!(
            "The encoded file is {:.1} s long but the original is {:.1} s.",
            duration, source_duration
        ));
    }
    Ok(())
}

/// Reads the container duration with ffprobe; any error output counts as a failure.
pub fn probe_duration(ffprobe: &Path, file: &Path) -> Result<f64, String> {
    let output = tools::command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(file)
        .output()
        .map_err(|e| format!("could not run ffprobe: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() || !stderr.trim().is_empty() {
        return Err(stderr.trim().to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "ffprobe reported no duration".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: u64 = 5_000_000_000;

    #[test]
    fn good_output_passes() {
        assert!(check_output(4_700_000_000, LIMIT, 7200.0, Ok(7200.4)).is_ok());
    }

    #[test]
    fn oversized_output_fails() {
        let err = check_output(5_000_000_000, LIMIT, 7200.0, Ok(7200.0)).unwrap_err();
        assert!(err.contains("not under"), "{err}");
    }

    #[test]
    fn duration_mismatch_fails() {
        assert!(check_output(1, LIMIT, 7200.0, Ok(7198.5)).is_err());
        assert!(check_output(1, LIMIT, 7200.0, Ok(7199.0)).is_ok());
    }

    #[test]
    fn unreadable_output_fails() {
        let err = check_output(1, LIMIT, 7200.0, Err("Invalid data found".into())).unwrap_err();
        assert!(err.contains("Invalid data found"));
    }
}
