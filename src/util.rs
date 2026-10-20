//! Small formatting and time helpers shared across the app.

use std::path::Path;

/// One decimal gigabyte. The 5 GB limit is measured in decimal units (5 × 10⁹ bytes).
pub const GB: u64 = 1_000_000_000;

/// Formats a byte count as e.g. "7.82 GB" or "640 MB".
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
    use super::*;

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
