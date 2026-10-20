//! Parsing ffmpeg's `-progress` output.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProgressParser {
    /// Seconds of output written so far.
    pub out_time: Option<f64>,
    /// Encoding speed relative to real time (e.g. 2.5 = 2.5×).
    pub speed: Option<f64>,
    pub finished: bool,
}

impl ProgressParser {
    /// Feeds one `key=value` line. Returns true at the end of each progress block.
    pub fn feed(&mut self, line: &str) -> bool {
        let Some((key, value)) = line.trim().split_once('=') else {
            return false;
        };
        let value = value.trim();
        match key {
            // Despite the name, `out_time_ms` is also in microseconds.
            "out_time_us" | "out_time_ms" => {
                if let Ok(us) = value.parse::<i64>()
                    && us >= 0
                {
                    self.out_time = Some(us as f64 / 1_000_000.0);
                }
            }
            "speed" => {
                self.speed = value
                    .trim_end_matches('x')
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v > 0.0);
            }
            "progress" => {
                self.finished = value == "end";
                return true;
            }
            _ => {}
        }
        false
    }

    /// Fraction of `duration` done, 0.0–1.0.
    pub fn fraction(&self, duration: f64) -> f64 {
        match self.out_time {
            Some(t) if duration > 0.0 => (t / duration).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_blocks() {
        let mut p = ProgressParser::default();
        let block = "frame=240\nfps=48.0\nout_time_us=10000000\nout_time=00:00:10.000000\nspeed=2.05x\nprogress=continue\n";
        let ends: Vec<bool> = block.lines().map(|l| p.feed(l)).collect();
        assert_eq!(ends.iter().filter(|e| **e).count(), 1);
        assert_eq!(p.out_time, Some(10.0));
        assert_eq!(p.speed, Some(2.05));
        assert!(!p.finished);
        assert!((p.fraction(100.0) - 0.1).abs() < 1e-9);

        p.feed("out_time_us=N/A");
        p.feed("speed=N/A");
        assert_eq!(p.out_time, Some(10.0), "N/A keeps the last value");
        assert_eq!(p.speed, None);
        p.feed("progress=end");
        assert!(p.finished);
    }
}
