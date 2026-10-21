//! Shared helpers for the integration tests: locating ffmpeg/libmpv and generating small test
//! clips. Clips are generated once and cached in Cargo's per-target temp dir.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use dropreel::library::import::{ImportReport, LibraryEnv, run_import};
use dropreel::settings::Settings;
use dropreel::tools::{self, Tools};

/// Bump to regenerate cached clips after changing them.
const FIXTURE_VERSION: u32 = 3;

/// The ffmpeg/ffprobe pair, or `None` (test skipped) when they are not installed. CI sets
/// `DROPREEL_REQUIRE_FFMPEG` so a missing tool fails instead.
pub fn tools() -> Option<Tools> {
    let tools = Tools::locate(&Settings::default());
    if tools.ffmpeg.is_none() || tools.ffprobe.is_none() {
        assert!(
            std::env::var_os("DROPREEL_REQUIRE_FFMPEG").is_none(),
            "ffmpeg/ffprobe not found"
        );
        eprintln!("ffmpeg/ffprobe not found; skipping");
        return None;
    }
    Some(tools)
}

/// Test clips. Every clip has different content, so no two are duplicates of each other.
pub struct Fixtures {
    pub dir: PathBuf,
    /// 6 s, 320x240, H.264 + AAC stereo, Matroska. Named like a scene release.
    pub mkv: PathBuf,
    /// 4 s, H.264 + AAC, MP4.
    pub mp4: PathBuf,
    /// 4 s, MPEG-4 part 2 + AC-3, AVI.
    pub avi: PathBuf,
    /// 4 s, VP9 + Opus, WebM.
    pub webm: PathBuf,
    /// 4 s, MPEG-2 + MP2, MPEG-TS.
    pub ts: PathBuf,
    /// 4 s, 10-bit HEVC + AC-3 5.1, like most re-encoded films.
    pub hevc10: PathBuf,
    /// 90 s, for seeking and resume tests.
    pub long: PathBuf,
    /// 30 s, two audio tracks (eng "Main", fre "Commentary") and two subtitle tracks
    /// (eng "English SDH", ger "Deutsch").
    pub multitrack: PathBuf,
    /// 4 s, no audio track.
    pub silent: PathBuf,
    /// Non-ASCII file name.
    pub unicode: PathBuf,
    /// The first half of `mkv`'s bytes: plays, then ends early.
    pub truncated: PathBuf,
    /// Random bytes with a .mkv extension.
    pub garbage: PathBuf,
    /// Zero bytes with a .mp4 extension.
    pub empty: PathBuf,
}

/// Generates (once per test binary, cached across runs) and returns the test clips.
pub fn fixtures() -> Option<&'static Fixtures> {
    static FIXTURES: OnceLock<Option<Fixtures>> = OnceLock::new();
    FIXTURES
        .get_or_init(|| {
            let tools = tools()?;
            Some(generate(tools.ffmpeg.as_deref()?))
        })
        .as_ref()
}

fn generate(ffmpeg: &Path) -> Fixtures {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("fixtures-v{FIXTURE_VERSION}"));
    std::fs::create_dir_all(&dir).unwrap();
    let srt_eng = dir.join("eng.srt");
    let srt_ger = dir.join("ger.srt");
    std::fs::write(
        &srt_eng,
        "1\n00:00:00,500 --> 00:00:03,000\n[ENGINE STARTS]\n\n2\n00:00:04,000 --> 00:00:07,000\nYou give me a time and a place.\n",
    )
    .unwrap();
    std::fs::write(
        &srt_ger,
        "1\n00:00:00,500 --> 00:00:03,000\n[MOTOR STARTET]\n\n2\n00:00:04,000 --> 00:00:07,000\nDu nennst mir Zeit und Ort.\n",
    )
    .unwrap();

    let video = |secs: u32, size: &str| format!("testsrc2=s={size}:r=24:d={secs}");
    let tone = |secs: u32, freq: u32| format!("sine=f={freq}:d={secs}");
    let make = |name: &str, args: Vec<String>| -> PathBuf {
        let out = dir.join(name);
        if !out.exists() {
            let part = dir.join(format!("part-{name}"));
            let output = tools::command(ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(&args)
                .arg(&part)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "making {name} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::rename(&part, &out).unwrap();
        }
        out
    };
    let lavfi = |v: String, a: Option<String>| {
        let mut args = vec!["-f".into(), "lavfi".into(), "-i".into(), v];
        if let Some(a) = a {
            args.extend(["-f".into(), "lavfi".into(), "-i".into(), a]);
        }
        args
    };
    let with = |mut base: Vec<String>, extra: &[&str]| {
        base.extend(extra.iter().map(|s| s.to_string()));
        base
    };
    let h264 = [
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
    ];

    let mkv = make(
        "The.Test.Movie.2019.1080p.BluRay.x264-GRP.mkv",
        with(
            lavfi(video(6, "320x240"), Some(tone(6, 440))),
            &[&h264[..], &["-c:a", "aac"]].concat(),
        ),
    );
    let mp4 = make(
        "Another Film (2007).mp4",
        with(
            lavfi(video(4, "320x240"), Some(tone(4, 500))),
            &[&h264[..], &["-c:a", "aac", "-movflags", "+faststart"]].concat(),
        ),
    );
    let avi = make(
        "old_movie_1999.avi",
        with(
            lavfi(video(4, "320x240"), Some(tone(4, 550))),
            &["-c:v", "mpeg4", "-q:v", "5", "-c:a", "ac3"],
        ),
    );
    let webm = make(
        "web clip.webm",
        with(
            lavfi(video(4, "320x240"), Some(tone(4, 600))),
            &[
                "-c:v",
                "libvpx-vp9",
                "-deadline",
                "realtime",
                "-cpu-used",
                "8",
                "-b:v",
                "300k",
                "-c:a",
                "libopus",
            ],
        ),
    );
    let ts = make(
        "broadcast.ts",
        with(
            lavfi(video(4, "320x240"), Some(tone(4, 650))),
            &["-c:v", "mpeg2video", "-q:v", "5", "-c:a", "mp2"],
        ),
    );
    let hevc10 = make(
        "Drive.2011.1080p.BluRay.DDP5.1.x265.10bit-GalaxyRG265.mkv",
        with(
            lavfi(
                video(4, "320x240"),
                Some(format!("{},aformat=channel_layouts=5.1", tone(4, 700))),
            ),
            &[
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p10le",
                "-x265-params",
                "log-level=error",
                "-c:a",
                "ac3",
            ],
        ),
    );
    let long = make(
        "Long.Take.2020.mkv",
        with(
            lavfi(video(90, "160x120"), Some(tone(90, 750))),
            &[&h264[..], &["-g", "24", "-c:a", "aac"]].concat(),
        ),
    );
    let multitrack = make(
        "Multi.Track.2015.mkv",
        vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            video(30, "320x240"),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            tone(30, 300),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            tone(30, 900),
            "-i".into(),
            srt_eng.to_string_lossy().into_owned(),
            "-i".into(),
            srt_ger.to_string_lossy().into_owned(),
        ]
        .into_iter()
        .chain(
            [
                "-map",
                "0",
                "-map",
                "1",
                "-map",
                "2",
                "-map",
                "3",
                "-map",
                "4",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-c:s",
                "srt",
                "-metadata:s:a:0",
                "language=eng",
                "-metadata:s:a:0",
                "title=Main",
                "-metadata:s:a:1",
                "language=fre",
                "-metadata:s:a:1",
                "title=Commentary",
                "-metadata:s:s:0",
                "language=eng",
                "-metadata:s:s:0",
                "title=English SDH",
                "-metadata:s:s:1",
                "language=ger",
                "-metadata:s:s:1",
                "title=Deutsch",
            ]
            .map(String::from),
        )
        .collect(),
    );
    let silent = make(
        "Silent Film 1927.mkv",
        with(lavfi(video(4, "320x240"), None), &h264),
    );
    let unicode = make(
        "Amélie – Le Fabuleux Destin (2001) 日本語.mkv",
        with(
            lavfi(video(4, "320x240"), Some(tone(4, 800))),
            &[&h264[..], &["-c:a", "aac"]].concat(),
        ),
    );

    let truncated = dir.join("Truncated.Download.2021.mkv");
    if !truncated.exists() {
        let bytes = std::fs::read(&mkv).unwrap();
        std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    }
    let garbage = dir.join("Not.Really.A.Movie.2010.mkv");
    if !garbage.exists() {
        // Deterministic pseudo-random bytes (xorshift).
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let bytes: Vec<u8> = (0..300_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        std::fs::write(&garbage, bytes).unwrap();
    }
    let empty = dir.join("Empty.2012.mp4");
    if !empty.exists() {
        std::fs::write(&empty, b"").unwrap();
    }

    Fixtures {
        dir,
        mkv,
        mp4,
        avi,
        webm,
        ts,
        hevc10,
        long,
        multitrack,
        silent,
        unicode,
        truncated,
        garbage,
        empty,
    }
}

/// Copies `src` into `dir` (created if needed), keeping its file name unless `name` is given.
pub fn copy_into(src: &Path, dir: &Path, name: Option<&str>) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let dest = dir.join(name.map_or_else(|| src.file_name().unwrap().to_owned(), Into::into));
    std::fs::copy(src, &dest).unwrap();
    dest
}

/// A library environment with its own database and poster folder inside `root`.
pub fn env(root: &Path, tools: &Tools) -> LibraryEnv {
    LibraryEnv {
        db_path: root.join("library.db"),
        posters_dir: root.join("posters"),
        tools: tools.clone(),
        tmdb_key: String::new(),
    }
}

/// Runs an import the way the app does for a drag & drop.
pub fn import(env: &LibraryEnv, paths: &[PathBuf]) -> ImportReport {
    import_with(env, paths, true)
}

/// Runs an import the way the app does for a background scan of watched folders.
pub fn background_import(env: &LibraryEnv, paths: &[PathBuf]) -> ImportReport {
    import_with(env, paths, false)
}

pub fn import_with(env: &LibraryEnv, paths: &[PathBuf], report_known: bool) -> ImportReport {
    run_import(env, paths, report_known, &|_| {}, &|| {}).unwrap()
}

/// Polls `cond` until it is true or `timeout` passes. Returns whether it became true.
pub fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// True when libmpv can be loaded. CI sets `DROPREEL_REQUIRE_MPV` so a missing library fails
/// instead of skipping the player tests.
pub fn mpv_available() -> bool {
    match dropreel::player::Player::headless(eframe::egui::Context::default()) {
        Ok(_) => true,
        Err(e) => {
            assert!(
                std::env::var_os("DROPREEL_REQUIRE_MPV").is_none(),
                "libmpv not available: {e}"
            );
            eprintln!("libmpv not available ({e}); skipping");
            false
        }
    }
}
