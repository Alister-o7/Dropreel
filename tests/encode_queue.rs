//! Runs the real encode queue against ffmpeg. Skipped when ffmpeg/libx265 are unavailable.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use dropreel::encode::{
    EncodeConfig, EncodeEvent, EncodeJob, EncodeQueue, Encoder, EncoderDetector,
};
use dropreel::library::Db;
use dropreel::library::import::{LibraryEnv, run_import};
use dropreel::settings::{EncoderChoice, Settings};
use dropreel::tools::{self, Tools};

fn setup() -> Option<(Tools, Arc<EncoderDetector>)> {
    let tools = Tools::locate(&Settings::default());
    let ffmpeg = tools.ffmpeg.clone()?;
    tools.ffprobe.as_ref()?;
    let detector = Arc::new(EncoderDetector::default());
    if !detector.is_usable(&ffmpeg, Encoder::X265) {
        eprintln!("libx265 unavailable; skipping");
        return None;
    }
    Some((tools, detector))
}

fn make_clip(ffmpeg: &Path, out: &Path, seconds: u32) {
    let status = tools::command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("testsrc2=s=640x360:r=24:d={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=d={seconds}"))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-qp",
            "10",
            "-c:a",
            "aac",
        ])
        .arg(out)
        .status()
        .unwrap();
    assert!(status.success());
}

fn import_one(dir: &Path, clip: &Path) -> (PathBuf, i64) {
    let posters = dir.join("posters");
    std::fs::create_dir_all(&posters).unwrap();
    let env = LibraryEnv {
        db_path: dir.join("library.db"),
        posters_dir: posters,
        tools: Tools::default(),
        tmdb_key: String::new(),
    };
    run_import(&env, &[clip.to_path_buf()], true, &|_| {}, &|| {}).unwrap();
    let id = Db::open(&env.db_path).unwrap().all_movies().unwrap()[0].id;
    (env.db_path, id)
}

fn config(db_path: PathBuf, tools: &Tools, clip: &Path) -> EncodeConfig {
    // Pretend the limit is 60% of the clip so it "needs" re-encoding.
    let threshold = std::fs::metadata(clip).unwrap().len() * 6 / 10;
    EncodeConfig {
        db_path,
        tools: tools.clone(),
        encoder: EncoderChoice::X265,
        x265_preset: "ultrafast".into(),
        threshold_bytes: threshold,
        target_bytes: threshold * 95 / 100,
    }
}

#[test]
fn encodes_verifies_and_relinks_the_movie() {
    let Some((tools, detector)) = setup() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("Big.Movie.2020.mkv");
    make_clip(tools.ffmpeg.as_ref().unwrap(), &clip, 8);
    let (db_path, id) = import_one(dir.path(), &clip);
    let cfg = config(db_path.clone(), &tools, &clip);
    let threshold = cfg.threshold_bytes;

    let (tx, rx) = mpsc::channel();
    let queue = EncodeQueue::new(cfg, detector, move |e| tx.send(e).unwrap(), || {});
    assert!(queue.enqueue(EncodeJob {
        movie_id: id,
        title: "Big Movie".into(),
        source: clip.clone()
    }));
    assert!(
        !queue.enqueue(EncodeJob {
            movie_id: id,
            title: "Big Movie".into(),
            source: clip.clone()
        }),
        "no duplicates"
    );

    let outcome = match rx.recv_timeout(Duration::from_secs(120)).unwrap() {
        EncodeEvent::Finished(o) => o,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(outcome.output, dir.path().join("Big.Movie.2020.hevc.mkv"));
    assert!(outcome.output_size < threshold);
    assert!(clip.exists(), "the original is left alone");
    assert!(!dir.path().join("Big.Movie.2020.hevc.mkv.part").exists());
    let movie = Db::open(&db_path).unwrap().movie(id).unwrap().unwrap();
    assert_eq!(movie.path, outcome.output);
    assert_eq!(movie.video_codec.as_deref(), Some("hevc"));
    assert!(!queue.is_busy());
}

#[test]
fn cancelling_stops_ffmpeg_and_cleans_up() {
    let Some((tools, detector)) = setup() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("Long.Movie.mkv");
    make_clip(tools.ffmpeg.as_ref().unwrap(), &clip, 40);
    let (db_path, id) = import_one(dir.path(), &clip);
    let mut cfg = config(db_path, &tools, &clip);
    cfg.x265_preset = "slower".into();

    let (tx, rx) = mpsc::channel();
    let queue = EncodeQueue::new(cfg, detector, move |e| tx.send(e).unwrap(), || {});
    queue.enqueue(EncodeJob {
        movie_id: id,
        title: "Long Movie".into(),
        source: clip.clone(),
    });
    // Wait until ffmpeg is actually running, then cancel.
    for _ in 0..200 {
        if let Some(dropreel::encode::JobState::Running(s)) = queue.job_state(id)
            && s.fraction > 0.0
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    queue.cancel(id);
    match rx.recv_timeout(Duration::from_secs(30)).unwrap() {
        EncodeEvent::Cancelled { movie_id, .. } => assert_eq!(movie_id, id),
        other => panic!("unexpected {other:?}"),
    }
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        !leftovers
            .iter()
            .any(|n| n.to_string_lossy().contains(".part")),
        "{leftovers:?}"
    );
    assert!(!dir.path().join("Long.Movie.hevc.mkv").exists());
}
