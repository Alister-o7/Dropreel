//! Playback tests against the real libmpv, run headless (no picture, no sound): loading,
//! resume, pause, seeking, end of file, tracks, volume, errors and every container we import.
//! Skipped when libmpv or ffmpeg are unavailable (CI sets DROPREEL_REQUIRE_MPV and
//! DROPREEL_REQUIRE_FFMPEG).

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use dropreel::player::{Player, PlayerEvent, TrackKind};
use eframe::egui;

use common::{fixtures, mpv_available};

const LOAD_TIMEOUT: Duration = Duration::from_secs(15);

struct Session {
    player: Player,
    /// Every event seen since the last `take_events`.
    events: Vec<PlayerEvent>,
}

impl Session {
    fn new() -> Self {
        Self {
            player: Player::headless(egui::Context::default()).unwrap(),
            events: Vec::new(),
        }
    }

    /// Pumps mpv's events until `cond` holds, failing the test after `timeout`.
    fn wait(&mut self, what: &str, timeout: Duration, mut cond: impl FnMut(&Self) -> bool) {
        let start = Instant::now();
        loop {
            let new = self.player.poll_events();
            self.events.extend(new);
            if cond(self) {
                return;
            }
            assert!(
                start.elapsed() < timeout,
                "timed out waiting for {what}; position {:?}, duration {:?}, paused {}, eof {}, events {:?}",
                self.player.position(),
                self.player.duration(),
                self.player.paused(),
                self.player.eof_reached(),
                self.events
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Keeps pumping events for `time`.
    fn idle(&mut self, time: Duration) {
        let end = Instant::now() + time;
        self.wait("idle", time + Duration::from_secs(5), |_| {
            Instant::now() >= end
        });
    }

    fn loaded(&self) -> bool {
        self.events.contains(&PlayerEvent::FileLoaded)
    }

    fn error(&self) -> Option<&str> {
        self.events.iter().find_map(|e| match e {
            PlayerEvent::Error(e) => Some(e.as_str()),
            _ => None,
        })
    }

    /// Loads `path` and waits until mpv has opened it and knows where it is.
    fn play(&mut self, path: &Path, start: Option<f64>) {
        self.events.clear();
        self.player.load(path, start).unwrap();
        self.wait("the file to load", LOAD_TIMEOUT, |s| {
            assert!(s.error().is_none(), "{}: {:?}", path.display(), s.error());
            s.loaded() && s.player.duration().is_some() && s.player.position().is_some()
        });
    }

    fn position(&self) -> f64 {
        self.player.position().unwrap()
    }

    /// Waits until playback has moved past `seconds`.
    fn wait_past(&mut self, seconds: f64) {
        self.wait(&format!("position {seconds}"), LOAD_TIMEOUT, |s| {
            s.player.position().is_some_and(|p| p > seconds)
        });
    }
}

fn setup() -> Option<&'static common::Fixtures> {
    let fx = fixtures()?;
    mpv_available().then_some(fx)
}

#[test]
fn plays_a_file_and_reports_duration_and_progress() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.mkv, None);
    assert!((s.player.duration().unwrap() - 6.0).abs() < 0.2);
    assert!(!s.player.paused());
    assert!(s.position() < 1.5, "started at {}", s.position());
    s.wait_past(1.0);
    assert!(!s.player.eof_reached());
}

#[test]
fn every_imported_container_and_codec_plays() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    for file in [
        &fx.mp4,
        &fx.avi,
        &fx.webm,
        &fx.ts,
        &fx.hevc10,
        &fx.silent,
        &fx.unicode,
        &fx.multitrack,
    ] {
        s.play(file, None);
        let start = s.position();
        s.wait_past(start + 0.5);
        assert!(s.error().is_none(), "{}: {:?}", file.display(), s.error());
    }
}

#[test]
fn resume_position_applies_to_that_file_only() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.long, Some(12.0));
    let p = s.position();
    assert!((11.5..13.5).contains(&p), "resumed at {p}");
    s.wait_past(12.5);

    // The next file starts from the beginning, not from the old resume point.
    s.play(&fx.long, None);
    assert!(s.position() < 1.5, "started at {}", s.position());
}

#[test]
fn pause_stops_the_clock_and_play_resumes_it() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.long, None);
    s.wait_past(0.5);

    s.player.set_paused(true);
    s.idle(Duration::from_millis(300));
    assert!(s.player.paused());
    let at = s.position();
    s.idle(Duration::from_millis(700));
    assert!(
        (s.position() - at).abs() < 0.05,
        "moved from {at} to {} while paused",
        s.position()
    );

    s.player.set_paused(false);
    s.wait_past(at + 0.4);
    assert!(!s.player.paused());
}

#[test]
fn seeking_lands_where_asked() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.long, None);

    s.player.seek(20.0, true);
    s.wait("exact seek", LOAD_TIMEOUT, |s| {
        s.player
            .position()
            .is_some_and(|p| (19.9..21.0).contains(&p))
    });

    s.player.seek_relative(-10.0);
    s.wait("relative seek back", LOAD_TIMEOUT, |s| {
        s.player
            .position()
            .is_some_and(|p| (9.5..11.5).contains(&p))
    });

    // Keyframe seeks (used while dragging the bar) land near the target (keyframe every 1 s).
    s.player.seek(5.5, false);
    s.wait("keyframe seek", LOAD_TIMEOUT, |s| {
        s.player.position().is_some_and(|p| (4.4..6.6).contains(&p))
    });

    // Seeking while paused updates the position without starting playback.
    s.player.set_paused(true);
    s.player.seek(25.0, true);
    s.wait("seek while paused", LOAD_TIMEOUT, |s| {
        s.player
            .position()
            .is_some_and(|p| (24.9..25.2).contains(&p))
    });
    s.idle(Duration::from_millis(500));
    assert!(s.player.paused());
    assert!(s.position() < 25.2);
}

#[test]
fn end_of_file_is_reported_and_the_last_frame_is_kept() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.mkv, Some(4.5));
    s.wait("end of file", LOAD_TIMEOUT, |s| s.player.eof_reached());
    assert!(s.error().is_none(), "{:?}", s.error());
    // keep-open: the file stays loaded at its end rather than unloading.
    let p = s.position();
    assert!(p > 5.5, "ended at {p}");

    // Seeking back from the end clears the flag and plays again.
    s.player.seek(1.0, true);
    s.player.set_paused(false);
    s.wait("playing again", LOAD_TIMEOUT, |s| {
        !s.player.eof_reached() && s.player.position().is_some_and(|p| p > 1.2 && p < 5.0)
    });
}

#[test]
fn seeking_past_the_end_finishes_cleanly() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.mkv, None);
    s.player.seek(999.0, true);
    s.wait("end of file", LOAD_TIMEOUT, |s| s.player.eof_reached());
    assert!(s.error().is_none(), "{:?}", s.error());
}

#[test]
fn a_truncated_download_plays_until_the_data_runs_out() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.truncated, None);
    s.wait("end of file", Duration::from_secs(20), |s| {
        s.player.eof_reached()
    });
    assert!(s.error().is_none(), "{:?}", s.error());
}

#[test]
fn audio_and_subtitle_tracks_are_listed_and_switchable() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.multitrack, None);
    let tracks = s.player.tracks();
    let of = |kind| {
        tracks
            .iter()
            .filter(|t| t.kind == kind)
            .cloned()
            .collect::<Vec<_>>()
    };
    let audio = of(TrackKind::Audio);
    let subs = of(TrackKind::Subtitle);
    assert_eq!(of(TrackKind::Video).len(), 1, "{tracks:?}");
    assert_eq!(audio.len(), 2, "{tracks:?}");
    assert_eq!(subs.len(), 2, "{tracks:?}");

    assert_eq!(audio[0].lang.as_deref(), Some("eng"));
    assert_eq!(audio[0].title.as_deref(), Some("Main"));
    assert_eq!(audio[1].title.as_deref(), Some("Commentary"));
    assert!(audio[0].selected && !audio[1].selected);
    assert_eq!(audio[0].short_label(), "eng · aac mono");
    assert!(audio[1].label().contains("Commentary"));
    assert_eq!(subs[1].lang.as_deref(), Some("ger"));
    assert!(subs.iter().all(|t| !t.external));

    // Switch audio to the commentary.
    s.events.clear();
    s.player.select_track(TrackKind::Audio, Some(audio[1].id));
    s.wait("audio switch", LOAD_TIMEOUT, |s| {
        s.events.contains(&PlayerEvent::TracksChanged)
    });
    let selected = |s: &Session, kind| {
        s.player
            .tracks()
            .into_iter()
            .filter(|t| t.kind == kind && t.selected)
            .map(|t| t.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(selected(&s, TrackKind::Audio), [audio[1].id]);

    // Turn on German subtitles, then turn subtitles off.
    s.player.select_track(TrackKind::Subtitle, Some(subs[1].id));
    s.wait("subtitles on", LOAD_TIMEOUT, |s| {
        selected(s, TrackKind::Subtitle) == [subs[1].id]
    });
    s.player.select_track(TrackKind::Subtitle, None);
    s.wait("subtitles off", LOAD_TIMEOUT, |s| {
        selected(s, TrackKind::Subtitle).is_empty()
    });
    // Playback carries on through all of that.
    let at = s.position();
    s.wait_past(at + 0.3);
}

#[test]
fn volume_is_clamped_and_mute_toggles() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.mkv, None);
    s.player.set_volume(500.0);
    assert_eq!(s.player.volume(), 130.0);
    s.player.set_volume(-3.0);
    assert_eq!(s.player.volume(), 0.0);
    s.player.set_volume(55.0);
    s.player.set_muted(true);
    s.idle(Duration::from_millis(300));
    // The cached values agree with what mpv reports back.
    assert_eq!(s.player.volume(), 55.0);
    assert!(s.player.muted());
    s.player.set_muted(false);
    s.idle(Duration::from_millis(300));
    assert!(!s.player.muted());
}

#[test]
fn unplayable_files_report_an_error_and_the_player_recovers() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    let missing = fx.dir.join("does not exist.mkv");
    for bad in [&fx.garbage, &fx.empty, &missing] {
        s.events.clear();
        s.player.load(bad, None).unwrap();
        s.wait(
            &format!("an error for {}", bad.display()),
            LOAD_TIMEOUT,
            |s| s.error().is_some(),
        );
        assert!(!s.error().unwrap().is_empty());
        assert!(!s.loaded(), "{} claimed to load", bad.display());
    }
    // A good file still plays afterwards.
    s.play(&fx.mkv, None);
    s.wait_past(0.5);
}

#[test]
fn stop_unloads_and_a_new_file_plays_afterwards() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.long, Some(10.0));
    s.player.stop();
    s.wait("unload", LOAD_TIMEOUT, |s| s.player.position().is_none());
    assert!(s.error().is_none());

    s.play(&fx.mkv, None);
    assert!((s.player.duration().unwrap() - 6.0).abs() < 0.2);
    assert!(s.position() < 1.5);
}

#[test]
fn loading_a_new_file_replaces_the_current_one() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    // Switch before the first file has even opened.
    s.player.load(&fx.long, Some(20.0)).unwrap();
    s.play(&fx.mkv, None);
    s.idle(Duration::from_millis(500));
    assert!((s.player.duration().unwrap() - 6.0).abs() < 0.2);
    assert!(s.position() < 3.0, "at {}", s.position());
}

#[test]
fn eof_from_the_previous_file_does_not_leak_into_the_next() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.mkv, Some(5.0));
    s.wait("end of file", LOAD_TIMEOUT, |s| s.player.eof_reached());
    s.play(&fx.long, None);
    s.idle(Duration::from_millis(300));
    assert!(!s.player.eof_reached());
}

#[test]
fn a_new_file_counts_as_started_only_once_its_playback_begins() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.play(&fx.mkv, Some(2.0));
    s.wait("playback to start", LOAD_TIMEOUT, |s| s.player.started());
    // Straight on to another file, with no stop in between (as for the next episode): until
    // it starts, frames may still be the old file's.
    s.player.load(&fx.long, None).unwrap();
    assert!(!s.player.started());
    assert!(!s.player.has_picture());
    s.wait("the new file to start", LOAD_TIMEOUT, |s| {
        s.player.started()
    });
    assert!((s.player.duration().unwrap() - 90.0).abs() < 1.0);
}

#[test]
fn smooth_motion_and_diagnostics_work_without_a_display() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    s.player.set_smooth_motion(true);
    s.player.set_display_fps(Some(60.0));
    s.play(&fx.mkv, None);
    s.wait_past(0.5);
    let diag = s.player.diagnostics();
    for key in [
        "hwdec-current=",
        "video-sync=display-resample",
        "frame-drop-count=",
        "ui frames",
    ] {
        assert!(diag.contains(key), "{key} missing from {diag}");
    }
    s.player.set_smooth_motion(false);
    s.player.set_display_fps(None);
    let at = s.position();
    s.wait_past(at + 0.3);
    assert!(s.player.diagnostics().contains("video-sync=audio"));
}

#[test]
fn seeks_asked_for_while_the_file_opens_are_applied_once_it_has() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    // The user hits Space and → the moment the movie opens, before mpv has loaded it.
    s.player.load(&fx.long, None).unwrap();
    s.player.set_paused(true);
    s.player.seek_relative(10.0);
    s.player.seek_relative(10.0);
    s.wait("a 20 s jump", LOAD_TIMEOUT, |s| {
        s.loaded()
            && s.player
                .position()
                .is_some_and(|p| (19.0..21.5).contains(&p))
    });
    // An absolute seek replaces anything queued before it.
    s.player.load(&fx.long, None).unwrap();
    s.player.seek_relative(10.0);
    s.player.seek(30.0, true);
    s.wait("the jump to 0:30", LOAD_TIMEOUT, |s| {
        s.player
            .position()
            .is_some_and(|p| (29.0..31.5).contains(&p))
    });
}

#[test]
fn relative_seek_works_when_paused_right_after_loading() {
    let Some(fx) = setup() else { return };
    let mut s = Session::new();
    // The user hits Space as the file opens, then →.
    s.player.load(&fx.long, None).unwrap();
    s.player.set_paused(true);
    s.wait("the file to load", LOAD_TIMEOUT, |s| s.loaded());
    s.idle(Duration::from_millis(200));
    s.player.seek_relative(10.0);
    s.wait("a 10 s jump", LOAD_TIMEOUT, |s| {
        s.player
            .position()
            .is_some_and(|p| (9.0..11.5).contains(&p))
    });
    s.player.seek_relative(-10.0);
    s.wait("a jump back", LOAD_TIMEOUT, |s| {
        s.player.position().is_some_and(|p| p < 1.5)
    });
    assert!(s.player.paused());
}
