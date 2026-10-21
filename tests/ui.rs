//! Drives the whole app headlessly with egui_kittest: the real library, import pipeline and
//! (headless) mpv player behind the real UI. Interactions go through the accessibility tree,
//! the same way a screen reader sees the app.
//! Skipped when ffmpeg is unavailable; playback tests also need libmpv.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dropreel::app::DropReelApp;
use dropreel::library::Db;
use dropreel::paths::AppPaths;
use eframe::egui::{self, Key, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use common::{copy_into, fixtures, mpv_available, tools};

type App = Harness<'static, DropReelApp>;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Starts DropReel with its data in `root`, as if launched with `open` on the command line.
fn launch(root: &Path, open: Vec<PathBuf>) -> App {
    launch_sized(root, open, egui::vec2(1280.0, 800.0))
}

fn launch_sized(root: &Path, open: Vec<PathBuf>, size: egui::Vec2) -> App {
    // The app's log goes to the test output (shown for failing tests).
    let _ = tracing_subscriber::fmt()
        .with_env_filter("dropreel=debug")
        .with_test_writer()
        .try_init();
    let paths = AppPaths::at(&root.join("data")).unwrap();
    Harness::builder()
        .with_size(size)
        .build_eframe(move |cc| DropReelApp::new_headless(cc, paths, open).unwrap())
}

fn db(root: &Path) -> Db {
    Db::open(&root.join("data").join("library.db")).unwrap()
}

/// Runs frames until `cond` holds (background imports and mpv run on their own threads).
fn until(app: &mut App, what: &str, mut cond: impl FnMut(&App) -> bool) {
    let start = Instant::now();
    loop {
        app.step();
        if cond(app) {
            return;
        }
        assert!(
            start.elapsed() < TIMEOUT,
            "timed out waiting for {what}; screen shows: {}",
            labels(app)
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Runs frames for a while.
fn run_for(app: &mut App, time: Duration) {
    let end = Instant::now() + time;
    until(app, "time to pass", |_| Instant::now() >= end);
}

/// The first node whose name (buttons) or value (text) satisfies `pred`. Names win, so
/// `click` finds the button rather than a text that happens to read the same.
fn find<'a>(app: &'a App, pred: impl Fn(&str) -> bool) -> Option<egui_kittest::Node<'a>> {
    let nodes: Vec<_> = app.root().children_recursive().collect();
    let by_name = nodes
        .iter()
        .find(|n| n.accesskit_node().label().is_some_and(|l| pred(l.trim())));
    by_name
        .or_else(|| {
            nodes
                .iter()
                .find(|n| n.accesskit_node().value().is_some_and(|v| pred(v.trim())))
        })
        .cloned()
}

fn has(app: &App, text: &str) -> bool {
    find(app, |t| t == text).is_some()
}

fn has_text(app: &App, part: &str) -> bool {
    find(app, |t| t.contains(part)).is_some()
}

/// Every labelled node, for failure messages.
fn labels(app: &App) -> String {
    let mut out = Vec::new();
    collect_labels(app.root(), &mut out);
    out.join(" | ")
}

fn collect_labels(node: egui_kittest::Node<'_>, out: &mut Vec<String>) {
    let n = node.accesskit_node();
    match (n.label().filter(|l| !l.trim().is_empty()), n.value()) {
        (Some(label), Some(value)) => out.push(format!("{label}={value}")),
        (Some(label), None) => out.push(label.to_string()),
        (None, Some(value)) if !value.trim().is_empty() => {
            out.push(format!("{:?}={value}", n.role()))
        }
        _ => {}
    }
    for child in node.children() {
        collect_labels(child, out);
    }
}

/// Clicks the widget named `label` with the mouse, like a user would: once it is on screen
/// (scrolling to it if needed) and has stopped moving.
fn click(app: &mut App, label: &str) {
    click_matching(app, label, |t| t == label);
}

fn click_containing(app: &mut App, part: &str) {
    click_matching(app, part, |t| t.contains(part));
}

fn click_matching(app: &mut App, what: &str, pred: impl Fn(&str) -> bool) {
    until(app, &format!("\"{what}\""), |a| find(a, &pred).is_some());
    let screen = app.ctx.content_rect();
    let mut last = None;
    until(app, &format!("\"{what}\" to settle"), |a| {
        let Some(node) = find(a, &pred) else {
            return false;
        };
        let rect = node.rect();
        if !screen.contains_rect(rect) {
            node.scroll_to_me();
        }
        let settled = last == Some(rect) && screen.contains_rect(rect);
        last = Some(rect);
        settled
    });
    find(app, &pred).unwrap().click();
    app.step();
}

/// Clicks into the search box and types.
fn search(app: &mut App, text: &str) {
    let field = app.get_by_role(Role::TextInput);
    field.focus();
    app.step();
    app.get_by_role(Role::TextInput).type_text(text);
    app.step();
}

/// A file dropped onto the window.
#[derive(Debug)]
struct Dropped(PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

fn drop_files(app: &mut App, files: &[PathBuf]) {
    for f in files {
        app.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(Dropped(f.clone())));
    }
    app.step();
}

/// Current playback position in seconds, from the player bar's time label ("0:07").
fn shown_position(app: &App) -> Option<u32> {
    // The first time label in the bar is the position, the second the duration.
    app.root().children_recursive().find_map(|n| {
        if n.accesskit_node().role() != Role::Label {
            return None;
        }
        let l = n.accesskit_node().value()?;
        let (m, s) = l.split_once(':')?;
        if m.len() > 2 || s.len() != 2 {
            return None;
        }
        Some(m.parse::<u32>().ok()? * 60 + s.parse::<u32>().ok()?)
    })
}

fn in_player(app: &App) -> bool {
    has(app, "Back to the library (Esc)")
}

#[test]
fn first_start_shows_an_empty_library() {
    let Some(_) = tools() else { return };
    let root = tempfile::tempdir().unwrap();
    let mut app = launch(root.path(), vec![]);
    run_for(&mut app, Duration::from_millis(200));
    assert!(has(&app, "Your library is empty"), "{}", labels(&app));
    assert!(has(&app, "Add movies"));
    assert!(has(&app, "Settings"));
}

#[test]
fn dropped_files_become_poster_cards_and_survive_a_restart() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let movies = root.path().join("movies");
    let files = vec![
        copy_into(&fx.mkv, &movies, None),
        copy_into(&fx.hevc10, &movies, None),
        copy_into(&fx.unicode, &movies, None),
    ];
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &files);
    until(&mut app, "three cards", |a| {
        has(a, "The Test Movie") && has(a, "Drive") && has_text(a, "Amélie")
    });
    assert!(!has(&app, "Your library is empty"));
    until(&mut app, "the import toast", |a| {
        has_text(a, "Added 3 movies")
    });
    assert_eq!(db(root.path()).all_movies().unwrap().len(), 3);
    drop(app);

    let mut app = launch(root.path(), vec![]);
    until(&mut app, "the cards after a restart", |a| {
        has(a, "The Test Movie") && has(a, "Drive")
    });
}

#[test]
fn dropping_a_folder_with_junk_reports_what_failed() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("downloads");
    copy_into(&fx.mp4, &folder, None);
    copy_into(&fx.garbage, &folder, None);
    copy_into(&fx.empty, &folder, None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[folder]);
    until(&mut app, "the import report", |a| has(a, "Import finished"));
    assert!(has_text(&app, "2 failed"), "{}", labels(&app));
    assert!(has_text(&app, "the file is empty"), "{}", labels(&app));
    click(&mut app, "Done");
    until(&mut app, "the report to close", |a| {
        !has(a, "Import finished")
    });
    until(&mut app, "the imported movie", |a| has(a, "Another Film"));
    assert!(!has(&app, "Not Really A Movie"));
}

#[test]
fn dropping_nothing_playable_says_so() {
    let Some(_) = tools() else { return };
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("docs");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.txt"), "x").unwrap();
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[folder]);
    until(&mut app, "a toast", |a| has_text(a, "No video files found"));
}

#[test]
fn search_filters_the_grid_and_clears() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let movies = root.path().join("movies");
    let files = vec![
        copy_into(&fx.mkv, &movies, None),
        copy_into(&fx.hevc10, &movies, None),
        copy_into(&fx.unicode, &movies, None),
    ];
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &files);
    until(&mut app, "cards", |a| {
        has(a, "Drive") && has(a, "The Test Movie")
    });

    search(&mut app, "amelie");
    until(&mut app, "the search to apply", |a| !has(a, "Drive"));
    assert!(has_text(&app, "Amélie"));
    assert!(!has(&app, "The Test Movie"));

    click(&mut app, "Clear search");
    until(&mut app, "all cards again", |a| {
        has(a, "Drive") && has(a, "The Test Movie")
    });

    search(&mut app, "zzz no such film");
    until(&mut app, "the empty search state", |a| {
        has(a, "No movies here")
    });
}

#[test]
fn removing_a_movie_keeps_the_file() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.mp4, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, std::slice::from_ref(&file));
    click(&mut app, "Details: Another Film");
    click(&mut app, "Remove from library");
    until(&mut app, "the confirmation", |a| {
        has(a, "Remove from library?")
    });
    click(&mut app, "Remove");
    until(&mut app, "the card to go", |a| {
        has(a, "Your library is empty")
    });
    assert!(file.exists());
    assert!(db(root.path()).all_movies().unwrap().is_empty());
}

#[test]
fn missing_files_are_flagged_after_a_restart() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.mp4, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, std::slice::from_ref(&file));
    until(&mut app, "the card", |a| has(a, "Another Film"));
    drop(app);

    std::fs::remove_file(&file).unwrap();
    let mut app = launch(root.path(), vec![]);
    until(&mut app, "the card", |a| has(a, "Another Film"));
    // Clicking a missing movie opens its details (with Locate…) instead of trying to play it.
    click(&mut app, "Another Film");
    until(&mut app, "the Locate button", |a| has(a, "Locate…"));
    assert!(!in_player(&app));
    click(&mut app, "Missing files");
    assert!(has(&app, "Another Film"));
}

#[test]
fn settings_are_saved_to_disk() {
    let Some(_) = tools() else { return };
    let root = tempfile::tempdir().unwrap();
    let mut app = launch(root.path(), vec![]);
    click(&mut app, "Settings");
    until(&mut app, "the settings dialog", |a| has(a, "Smooth motion"));
    click(&mut app, "Smooth motion");
    click(&mut app, "Save");
    until(&mut app, "the dialog to close", |a| {
        !has(a, "Smooth motion")
    });
    let text = std::fs::read_to_string(root.path().join("data").join("settings.toml")).unwrap();
    assert!(text.contains("smooth_motion = false"), "{text}");

    // Cancel throws changes away.
    click(&mut app, "Settings");
    until(&mut app, "the settings dialog", |a| has(a, "Smooth motion"));
    click(&mut app, "High-quality video scaling");
    click(&mut app, "Cancel");
    let again = std::fs::read_to_string(root.path().join("data").join("settings.toml")).unwrap();
    assert_eq!(text, again);
}

#[test]
fn small_window_renders_every_screen() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.multitrack, &root.path().join("movies"), None);
    let mut app = launch_sized(root.path(), vec![], egui::vec2(720.0, 480.0));
    drop_files(&mut app, &[file]);
    click(&mut app, "Details: Multi Track");
    until(&mut app, "details", |a| has(a, "Remove from library"));
    click(&mut app, "Close");
    click(&mut app, "Settings");
    until(&mut app, "settings", |a| has(a, "Smooth motion"));
    click(&mut app, "Cancel");
    app.set_size(egui::vec2(400.0, 300.0));
    run_for(&mut app, Duration::from_millis(300));
}

// ----- playback through the UI -----

fn playback_setup() -> Option<&'static common::Fixtures> {
    let _ = tools()?;
    let fx = fixtures()?;
    mpv_available().then_some(fx)
}

#[test]
fn clicking_a_poster_plays_and_back_saves_the_position() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Long Take");
    until(&mut app, "the player", in_player);
    until(&mut app, "playback to reach 0:02", |a| {
        has(a, "Pause") && shown_position(a).is_some_and(|p| p >= 2)
    });
    // Skip ahead 40 s with the forward button (positions under 30 s aren't offered to resume).
    for _ in 0..4 {
        click(&mut app, "Forward 10 seconds (→)");
    }
    until(&mut app, "playback past 0:40", |a| {
        shown_position(a).is_some_and(|p| p >= 40)
    });

    // Pause with the button; the clock stops.
    click(&mut app, "Pause");
    until(&mut app, "paused", |a| has(a, "Play"));
    let paused_at = shown_position(&app).unwrap();
    run_for(&mut app, Duration::from_millis(1500));
    assert_eq!(shown_position(&app), Some(paused_at));

    // Back to the library saves where we stopped.
    click(&mut app, "Back to the library (Esc)");
    until(&mut app, "the library", |a| {
        !in_player(a) && has(a, "Long Take")
    });
    let m = db(root.path()).all_movies().unwrap().remove(0);
    let saved = m.resume_pos.expect("resume position saved");
    assert!(
        (saved - f64::from(paused_at)).abs() < 1.5,
        "saved {saved}, paused at {paused_at}"
    );
    assert!(m.last_played.is_some());
    assert!(!m.watched);
    assert!(has(&app, "Continue watching"));

    // Playing again offers to resume.
    click(&mut app, "Long Take");
    until(&mut app, "the resume prompt", |a| {
        has_text(a, "Resume from")
    });
    click_containing(&mut app, "Resume from");
    until(&mut app, "the player at the saved point", |a| {
        in_player(a) && shown_position(a).is_some_and(|p| p >= paused_at && p <= paused_at + 3)
    });

    // Esc also leaves the player.
    app.key_press(Key::Escape);
    until(&mut app, "the library", |a| !in_player(a));
}

#[test]
fn start_over_from_the_resume_prompt_starts_at_zero() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    until(&mut app, "the card", |a| has(a, "Long Take"));
    let d = db(root.path());
    let id = d.all_movies().unwrap()[0].id;
    d.set_resume(id, Some(60.0)).unwrap();
    drop(app);

    let mut app = launch(root.path(), vec![]);
    click(&mut app, "Long Take");
    until(&mut app, "the resume prompt", |a| {
        has(a, "Resume from 1:00")
    });
    click(&mut app, "Start over");
    until(&mut app, "playback from the start", |a| {
        in_player(a) && has(a, "Pause") && shown_position(a).is_some_and(|p| p <= 2)
    });
}

#[test]
fn keyboard_controls_pause_and_seek() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Long Take");
    until(&mut app, "playing", |a| in_player(a) && has(a, "Pause"));

    app.key_press(Key::Space);
    until(&mut app, "paused", |a| has(a, "Play"));
    let at = shown_position(&app).unwrap();
    app.key_press(Key::ArrowRight);
    until(&mut app, "a 10 s jump", |a| {
        shown_position(a).is_some_and(|p| p >= at + 9 && p <= at + 11)
    });
    app.key_press(Key::ArrowLeft);
    until(&mut app, "a jump back", |a| {
        shown_position(a).is_some_and(|p| p + 1 >= at && p <= at + 1)
    });
    app.key_press(Key::Space);
    until(&mut app, "playing", |a| has(a, "Pause"));
}

#[test]
fn watching_to_the_end_marks_watched_and_returns_to_the_library() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.mkv, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "The Test Movie");
    until(&mut app, "the player", in_player);
    until(&mut app, "the end", |a| !in_player(a));
    until(&mut app, "the toast", |a| has_text(a, "Finished"));
    let m = db(root.path()).all_movies().unwrap().remove(0);
    assert!(m.watched);
    assert_eq!(m.resume_pos, None);
    // Watched movies play from the start without asking.
    click(&mut app, "The Test Movie");
    until(&mut app, "the player", in_player);
    assert!(!has_text(&app, "Resume from"));
}

#[test]
fn unplayable_file_shows_an_error_and_returns_to_the_library() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.mp4, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, std::slice::from_ref(&file));
    until(&mut app, "the card", |a| has(a, "Another Film"));
    // The file is replaced by junk behind DropReel's back.
    std::fs::copy(&fx.garbage, &file).unwrap();
    click(&mut app, "Another Film");
    until(&mut app, "an error", |a| has_text(a, "Could not play"));
    until(&mut app, "the library", |a| !in_player(a));
}

#[test]
fn open_with_plays_the_file_right_away() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![file.clone()]);
    until(&mut app, "the player", |a| in_player(a) && has(a, "Pause"));
    assert_eq!(db(root.path()).all_movies().unwrap().len(), 1);
    click(&mut app, "Back to the library (Esc)");
    drop(app);

    // Opening a file that is already in the library plays it too, without importing it again.
    let mut app = launch(root.path(), vec![file]);
    until(&mut app, "the player or resume prompt", |a| {
        in_player(a) || has_text(a, "Resume from")
    });
    assert_eq!(db(root.path()).all_movies().unwrap().len(), 1);
}

#[test]
fn tracks_can_be_switched_from_the_player_bar() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.multitrack, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Multi Track");
    until(&mut app, "the audio menu", |a| {
        in_player(a) && has(a, "Audio: eng · aac mono")
    });
    // The file marks its English subtitles as default, so mpv shows them.
    assert!(has(&app, "Subtitles: eng · subrip"), "{}", labels(&app));
    click(&mut app, "Audio: eng · aac mono");
    click_containing(&mut app, "Commentary");
    until(&mut app, "the French track", |a| {
        has(a, "Audio: fre · aac mono")
    });

    click(&mut app, "Subtitles: eng · subrip");
    click_containing(&mut app, "Deutsch");
    until(&mut app, "German subtitles", |a| {
        has(a, "Subtitles: ger · subrip")
    });
    click(&mut app, "Subtitles: ger · subrip");
    click(&mut app, "Off");
    until(&mut app, "subtitles off", |a| has(a, "Subtitles: Off"));
    // Playback carries on.
    until(&mut app, "playback", |a| {
        shown_position(a).is_some_and(|p| p >= 2)
    });
}

#[test]
fn shortcuts_keep_working_after_clicking_player_buttons() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Long Take");
    until(&mut app, "playing", |a| in_player(a) && has(a, "Pause"));

    // Pausing with the mouse leaves the button focused...
    click(&mut app, "Pause");
    until(&mut app, "paused", |a| has(a, "Play"));
    let at = shown_position(&app).unwrap();
    // ...but the keyboard shortcuts still work.
    app.key_press(Key::ArrowRight);
    until(&mut app, "a 10 s jump", |a| {
        shown_position(a).is_some_and(|p| p >= at + 9 && p <= at + 11)
    });
    // Space toggles exactly once (not once for the shortcut and once for the focused button).
    app.key_press(Key::Space);
    run_for(&mut app, Duration::from_millis(300));
    assert!(has(&app, "Pause"), "{}", labels(&app));
    // Same after using a track menu.
    click(&mut app, "Subtitles: Off");
    app.key_press(Key::Escape);
    run_for(&mut app, Duration::from_millis(200));
    app.key_press(Key::Space);
    until(&mut app, "paused by keyboard", |a| has(a, "Play"));
}
