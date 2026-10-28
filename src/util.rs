//! Small formatting and time helpers shared across the app.

use std::path::Path;

/// One decimal gigabyte. The 5 GB limit is measured in decimal units (5 × 10⁹ bytes).
pub const GB: u64 = 1_000_000_000;

/// Formats a byte count as e.g. "7.82 GB" or "640 MB".
/// The app's version as people see it. Cargo's versions have three numbers, so a fourth goes in
/// as build metadata ("0.3.5+1") and is shown as "0.3.5.1".
pub fn app_version() -> String {
    display_version(env!("CARGO_PKG_VERSION"))
}

fn display_version(version: &str) -> String {
    version.replace('+', ".")
}

pub fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes >= GB {
        format!("{:.2} GB", b / GB as f64)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", b / 1e6)
    } else if bytes >= 1_000 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

/// Formats seconds as "1:02:03" or "2:03".
pub fn format_duration(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds.round() as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Current time as Unix seconds.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Formats a Unix timestamp as a local date, e.g. "2026-10-20".
pub fn format_date(unix: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => {
            t.format("%Y-%m-%d").to_string()
        }
        chrono::LocalResult::None => String::from("?"),
    }
}

/// The useful part of ffmpeg/ffprobe error output: the last message, without the
/// "[matroska,webm @ 0x55d…]" source prefix or the file path. Empty input stays empty.
pub fn ffmpeg_error(stderr: &str) -> String {
    let Some(line) = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()) else {
        return String::new();
    };
    let mut line = line;
    if line.starts_with('[')
        && let Some(end) = line.find("] ")
        && line[..end].contains(" @ ")
    {
        line = &line[end + 2..];
    }
    // "C:\movies\x.mkv: Invalid data found when processing input"
    if let Some((before, after)) = line.rsplit_once(": ")
        && (before.contains('/') || before.contains('\\'))
    {
        line = after;
    }
    line.to_string()
}

/// A `file://` URI that egui's file loader understands on this platform.
pub fn file_uri(path: &Path) -> String {
    if cfg!(windows) {
        format!("file:///{}", path.display())
    } else {
        format!("file://{}", path.display())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_fourth_version_number_is_shown_with_a_dot() {
        assert_eq!(super::display_version("0.3.5+1"), "0.3.5.1");
        assert_eq!(super::display_version("0.3.6"), "0.3.6");
    }

    use super::*;

    #[test]
    fn ffmpeg_errors_are_tidied() {
        let probe = "[mov,mp4,m4a,3gp,3g2,mj2 @ 0x560772cb6640] moov atom not found\n/tmp/in/Empty.mp4: Invalid data found when processing input\n";
        assert_eq!(
            ffmpeg_error(probe),
            "Invalid data found when processing input"
        );
        let mux = "[image2 @ 0x55] Could not open file : x.jpg\n[out#0/image2 @ 0x56] Error muxing a packet";
        assert_eq!(ffmpeg_error(mux), "Error muxing a packet");
        assert_eq!(
            ffmpeg_error("C:\\Movies\\a b.mkv: No such file or directory"),
            "No such file or directory"
        );
        assert_eq!(ffmpeg_error("  \n"), "");
        assert_eq!(ffmpeg_error("plain message"), "plain message");
    }

    #[test]
    fn bytes_are_formatted_in_decimal_units() {
        assert_eq!(format_bytes(5 * GB), "5.00 GB");
        assert_eq!(format_bytes(7_823_000_000), "7.82 GB");
        assert_eq!(format_bytes(640_000_000), "640 MB");
        assert_eq!(format_bytes(12), "12 B");
    }

    #[test]
    fn durations_are_formatted_as_clock_time() {
        assert_eq!(format_duration(4354.4), "1:12:34");
        assert_eq!(format_duration(123.0), "2:03");
        assert_eq!(format_duration(f64::NAN), "0:00");
    }
}
