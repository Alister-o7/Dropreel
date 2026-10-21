//! End-to-end checks against a real ffmpeg/ffprobe. Skipped when they are not installed.

use std::path::{Path, PathBuf};

use dropreel::encode::args::{self, EncodeSpec};
use dropreel::encode::{Encoder, EncoderDetector, plan, verify};
use dropreel::library::Db;
use dropreel::library::import::{LibraryEnv, run_import};
use dropreel::metadata::{self, probe, thumbnail};
use dropreel::settings::{EncoderChoice, Settings};
use dropreel::tools::{self, Tools};

fn tools_or_skip() -> Option<Tools> {
    let t = Tools::locate(&Settings::default());
    if t.ffmpeg.is_none() || t.ffprobe.is_none() {
        // CI sets this so a missing ffmpeg fails loudly instead of silently skipping.
        assert!(
            std::env::var_os("DROPREEL_REQUIRE_FFMPEG").is_none(),
            "ffmpeg/ffprobe not found"
        );
        eprintln!("ffmpeg/ffprobe not found; skipping");
        return None;
    }
    Some(t)
}

/// A 6-second 320×240 clip with stereo AAC audio and an SRT subtitle track.
fn make_clip(ffmpeg: &Path, dir: &Path) -> PathBuf {
    let srt = dir.join("subs.srt");
    std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:03,000\nHello\n").unwrap();
    let out = dir.join("Test.Movie.2021.1080p.mkv");
    let status = tools::command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=24:d=6",
        ])
        .args(["-f", "lavfi", "-i", "sine=f=440:d=6", "-i"])
        .arg(&srt)
        .args([
            "-map",
            "0",
            "-map",
            "1",
            "-map",
            "2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-ac",
            "2",
            "-c:s",
            "srt",
        ])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    out
}

#[test]
fn import_probe_thumbnail_and_reencode() {
    let Some(tools) = tools_or_skip() else { return };
    let (ffmpeg, ffprobe) = (
        tools.ffmpeg.clone().unwrap(),
        tools.ffprobe.clone().unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let clip = make_clip(&ffmpeg, dir.path());

    // Import fills in technical details and a frame-grab poster.
    let posters = dir.path().join("posters");
    std::fs::create_dir_all(&posters).unwrap();
    let env = LibraryEnv {
        db_path: dir.path().join("library.db"),
        posters_dir: posters,
        tools: tools.clone(),
        tmdb_key: String::new(),
    };
    let report = run_import(&env, std::slice::from_ref(&clip), true, &|_| {}, &|| {}).unwrap();
    assert_eq!(report.added, vec!["Test Movie".to_string()], "{report:?}");
    assert!(report.warnings.is_empty(), "{report:?}");
    let movie = Db::open(&env.db_path)
        .unwrap()
        .all_movies()
        .unwrap()
        .remove(0);
    assert_eq!(movie.year, Some(2021));
    assert_eq!((movie.width, movie.height), (Some(320), Some(240)));
    assert!((movie.duration.unwrap() - 6.0).abs() < 0.2);
    assert_eq!(movie.audio_tracks.len(), 1);
    assert_eq!(movie.subtitle_tracks.len(), 1);
    assert!(
        movie.poster.as_ref().is_some_and(|p| p.is_file()),
        "poster was generated"
    );

    let info = probe::probe(&ffprobe, &clip).unwrap();
    assert!(
        thumbnail::grab_frame(
            &ffmpeg,
            &clip,
            info.duration,
            false,
            &dir.path().join("t.jpg")
        )
        .is_ok()
    );

    // Plan for a tiny target so the bitrate math is exercised, then encode with x265 2-pass.
    let detector = EncoderDetector::default();
    if !detector.is_usable(&ffmpeg, Encoder::X265) {
        eprintln!("libx265 unavailable; skipping the encode step");
        return;
    }
    assert_eq!(
        detector.select(&ffmpeg, EncoderChoice::X265),
        Ok(Encoder::X265)
    );
    let target = 400_000;
    let p = plan::plan(&plan::PlanInput::from_media(&info, target).unwrap()).unwrap();
    let video = info.video.clone().unwrap();
    let subs: Vec<String> = info.subtitles.iter().map(|s| s.codec.clone()).collect();
    let part = dir.path().join("Test.Movie.2021.1080p.hevc.mkv.part");
    let spec = EncodeSpec {
        input: &clip,
        output: &part,
        encoder: Encoder::X265,
        video_kbps: p.video_kbps,
        x265_preset: "ultrafast",
        video: &video,
        hdr: None,
        subtitle_codecs: &subs,
        stats_file: "x265-test.log",
    };
    for &pass in args::passes(Encoder::X265) {
        let out = tools::command(&ffmpeg)
            .current_dir(dir.path())
            .args(args::ffmpeg_args(&spec, pass))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "pass {pass:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let size = std::fs::metadata(&part).unwrap().len();
    let limit = target * 100 / 95;
    verify::check_output(
        size,
        limit,
        info.duration.unwrap(),
        verify::probe_duration(&ffprobe, &part),
    )
    .unwrap();
    let encoded = probe::probe(&ffprobe, &part).unwrap();
    assert_eq!(encoded.video.unwrap().codec, "hevc");
    assert_eq!(encoded.audio.len(), 1, "audio copied");
    assert_eq!(encoded.subtitles.len(), 1, "subtitles copied");
}

#[test]
fn small_frame_posters_are_regrabbed_at_the_movies_width() {
    let Some(tools) = tools_or_skip() else { return };
    let ffmpeg = tools.ffmpeg.clone().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let clip = make_clip(&ffmpeg, dir.path());
    let posters = dir.path().join("posters");
    std::fs::create_dir_all(&posters).unwrap();
    let env = LibraryEnv {
        db_path: dir.path().join("library.db"),
        posters_dir: posters.clone(),
        tools: tools.clone(),
        tmdb_key: String::new(),
    };
    run_import(&env, std::slice::from_ref(&clip), true, &|_| {}, &|| {}).unwrap();
    let db = Db::open(&env.db_path).unwrap();
    let movie = db.all_movies().unwrap().remove(0);
    let grabbed = movie.poster.clone().unwrap();
    assert!(metadata::is_frame_poster(&grabbed));
    // A 320-pixel clip is grabbed at its own width, not blown up.
    assert_eq!(image::image_dimensions(&grabbed).unwrap().0, 320);

    // An older version's smaller frame...
    let old = posters.join(format!("{}-frame-1.jpg", movie.id));
    let status = tools::command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&grabbed)
        .args(["-vf", "scale=160:-2"])
        .arg(&old)
        .status()
        .unwrap();
    assert!(status.success());
    db.set_poster(movie.id, Some(&old)).unwrap();

    // ...is grabbed again at the movie's width, and the old file goes away.
    let refreshed = std::cell::Cell::new(0);
    let upgraded =
        metadata::upgrade_frame_posters(&env, &db, &|| refreshed.set(refreshed.get() + 1));
    assert_eq!((upgraded, refreshed.get()), (1, 1));
    let poster = db.movie(movie.id).unwrap().unwrap().poster.unwrap();
    assert_eq!(image::image_dimensions(&poster).unwrap().0, 320);
    assert!(!old.exists());
    // Nothing left to do the next time.
    assert_eq!(metadata::upgrade_frame_posters(&env, &db, &|| {}), 0);
}
