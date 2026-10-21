//! The refresh rate of the monitor showing the window, which mpv needs to sync video to the
//! display (vo_libmpv cannot find it out on its own).

use std::time::{Duration, Instant};

/// How often the window's monitor is checked again (it can be moved or its mode changed).
const RECHECK: Duration = Duration::from_secs(2);

/// Tracks the refresh rate of the window's monitor, re-checking it every few seconds.
#[derive(Default)]
pub struct RefreshRate {
    checked: Option<Instant>,
    hz: Option<f64>,
}

impl RefreshRate {
    /// The current refresh rate in Hz, if the platform can tell.
    pub fn poll(&mut self, frame: &eframe::Frame) -> Option<f64> {
        if self.checked.is_none_or(|t| t.elapsed() >= RECHECK) {
            self.checked = Some(Instant::now());
            self.hz = query(frame);
        }
        self.hz
    }
}

/// Windows reports fractional NTSC rates rounded down (59 for 59.94 Hz), so turn those back into
/// the real rate.
#[cfg_attr(not(windows), allow(dead_code))]
fn exact_rate(reported_hz: u32) -> Option<f64> {
    match reported_hz {
        0 | 1 => None, // "hardware default"
        hz if [24, 30, 48, 60, 72, 120, 144, 240].contains(&(hz + 1)) => {
            Some(f64::from(hz + 1) * 1000.0 / 1001.0)
        }
        hz => Some(f64::from(hz)),
    }
}

#[cfg(windows)]
fn query(frame: &eframe::Frame) -> Option<f64> {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::Graphics::Gdi::{
        DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW, GetMonitorInfoW,
        MONITOR_DEFAULTTONEAREST, MONITORINFOEXW, MonitorFromWindow,
    };

    let RawWindowHandle::Win32(handle) = frame.window_handle().ok()?.as_raw() else {
        return None;
    };
    // SAFETY: plain Win32 queries on our own window, with correctly sized out structs.
    unsafe {
        let monitor = MonitorFromWindow(handle.hwnd.get() as _, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(monitor, &mut info.monitorInfo) == 0 {
            return None;
        }
        let mut mode: DEVMODEW = std::mem::zeroed();
        mode.dmSize = size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(info.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) == 0 {
            return None;
        }
        exact_rate(mode.dmDisplayFrequency)
    }
}

#[cfg(not(windows))]
fn query(_frame: &eframe::Frame) -> Option<f64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_fractional_rates() {
        assert_eq!(exact_rate(60), Some(60.0));
        assert_eq!(exact_rate(144), Some(144.0));
        assert!((exact_rate(59).unwrap() - 59.94).abs() < 0.01);
        assert!((exact_rate(23).unwrap() - 23.976).abs() < 0.01);
        assert!((exact_rate(119).unwrap() - 119.88).abs() < 0.01);
        assert_eq!(exact_rate(0), None);
        assert_eq!(exact_rate(1), None);
    }
}
