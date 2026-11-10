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

/// Starts Dropreel with its data in `root`, as if launched with `open` on the command line.
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

/// Presses on `label`, holds the button for a few frames, then lets go, like a person does (a
/// plain `click` presses and releases within one frame).
fn slow_click(app: &mut App, label: &str) {
    let at = settle_on(app, label, |t| t == label).rect().center();
    app.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        for _ in 0..3 {
            app.step();
        }
    }
}

/// Clicks a spot that has no widget of its own.
fn click_at(app: &mut App, at: egui::Pos2) {
    app.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    app.step();
}

/// Opens a movie's details from its right-click menu (the middle of a poster plays it).
fn open_details(app: &mut App, title: &str) {
    // The play button is small enough to be fully on screen even in a tiny window.
    let play = format!("Play movie: {title}");
    settle_on(app, &play, |t| t == play).click_secondary();
    app.step();
    // Menu items read "<icon>   Details".
    click_matching(app, "Details", |t| t.ends_with("   Details"));
}

fn click_matching(app: &mut App, what: &str, pred: impl Fn(&str) -> bool) {
    settle_on(app, what, &pred).click();
    app.step();
}

/// Waits for the node matching `pred` to be on screen and stay put, and returns it.
fn settle_on<'a>(
    app: &'a mut App,
    what: &str,
    pred: impl Fn(&str) -> bool,
) -> egui_kittest::Node<'a> {
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
    find(app, &pred).unwrap()
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
/// The playback position, in whole seconds, from the seek bar.
fn shown_position(app: &App) -> Option<u32> {
    let seek = find(app, |t| t == "Seek")?;
    Some(seek.accesskit_node().numeric_value()?.round() as u32)
}

/// The first time shown next to the seek bar, in seconds.
fn shown_time(app: &App) -> Option<u32> {
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
    // "Close" when Dropreel was opened just to play a video.
    has(app, "Back to the library (Esc)") || has(app, "Close (Esc)")
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
    open_details(&mut app, "Another Film");
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
    // Wait for the whole import (poster included): Windows can't delete a file ffmpeg has open.
    until(&mut app, "the import to finish", |a| {
        has_text(a, "Added 1 movie")
    });
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
    // The encoder and x265 preset are only offered for the Custom quality preset.
    assert!(has(&app, "Fast") && !has(&app, "Encoder"));
    click(&mut app, "Custom");
    until(&mut app, "the encoder choice", |a| has(a, "Encoder"));
    click(&mut app, "Best quality");
    until(&mut app, "the encoder choice to go", |a| !has(a, "Encoder"));
    // Off on a fresh install; turn it on.
    click(&mut app, "Smooth motion");
    click(&mut app, "Time played and length");
    click(&mut app, "Save");
    until(&mut app, "the dialog to close", |a| {
        !has(a, "Smooth motion")
    });
    let text = std::fs::read_to_string(root.path().join("data").join("settings.toml")).unwrap();
    assert!(text.contains("smooth_motion = true"), "{text}");
    assert!(text.contains("high_quality_video = true"), "{text}");
    assert!(text.contains("hide_controls_after = 2.5"), "{text}");
    assert!(text.contains("quality = \"best\""), "{text}");
    assert!(text.contains("time_display = \"normal\""), "{text}");

    // The TMDb key can be typed (or pasted), shown, and is saved trimmed.
    click(&mut app, "Settings");
    until(&mut app, "the settings dialog", |a| has(a, "Smooth motion"));
    let key_node = |a: &App| {
        a.root()
            .children_recursive()
            .find(|n| n.accesskit_node().label().as_deref() == Some("TMDb API key"))
            .map(|n| n.accesskit_node().value())
    };
    until(&mut app, "the key field", |a| key_node(a).is_some());
    let field = || {
        |n: &egui_kittest::Node<'_>| n.accesskit_node().label().as_deref() == Some("TMDb API key")
    };
    app.root()
        .children_recursive()
        .find(field())
        .unwrap()
        .focus();
    app.step();
    app.root()
        .children_recursive()
        .find(field())
        .unwrap()
        .type_text(" abc123 ");
    app.step();
    click(&mut app, "Show key");
    until(&mut app, "the key in plain text", |a| {
        key_node(a).flatten().as_deref() == Some(" abc123 ")
    });
    assert!(has(&app, "Hide key"));
    click(&mut app, "Save");
    until(&mut app, "the dialog to close", |a| {
        !has(a, "Smooth motion")
    });
    let text = std::fs::read_to_string(root.path().join("data").join("settings.toml")).unwrap();
    assert!(text.contains("tmdb_api_key = \"abc123\""), "{text}");

    // Cancel throws changes away.
    click(&mut app, "Settings");
    until(&mut app, "the settings dialog", |a| has(a, "Smooth motion"));
    // The version is at the bottom.
    assert!(has(
        &app,
        &format!("Dropreel {}", env!("CARGO_PKG_VERSION").replace('+', "."))
    ));
    // On screen, but below the dialog's scrolled-into-view part: bring it into view first.
    find(&app, |t| t == "High-quality video scaling")
        .unwrap()
        .scroll_to_me();
    run_for(&mut app, Duration::from_millis(200));
    click(&mut app, "High-quality video scaling");
    click(&mut app, "Cancel");
    let again = std::fs::read_to_string(root.path().join("data").join("settings.toml")).unwrap();
    assert_eq!(text, again);
}

/// Types into the text field that currently holds `value`.
fn type_into_field(app: &mut App, value: &str, text: &str) {
    let holds = |n: &egui_kittest::Node<'_>| n.accesskit_node().value().as_deref() == Some(value);
    until(app, &format!("a field holding \"{value}\""), |a| {
        a.get_all_by_role(Role::TextInput).any(|n| holds(&n))
    });
    app.get_all_by_role(Role::TextInput)
        .find(|n| holds(n))
        .unwrap()
        .focus();
    app.step();
    app.get_all_by_role(Role::TextInput)
        .find(|n| holds(n))
        .unwrap()
        .type_text(text);
    app.step();
}

#[test]
fn tags_can_be_renamed_and_deleted() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.mkv, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);

    // Tag the movie from its details panel.
    open_details(&mut app, "The Test Movie");
    until(&mut app, "details", |a| has(a, "Remove from library"));
    let field = app.get_by_label("New tag");
    field.focus();
    app.step();
    app.get_by_label("New tag").type_text("Noir");
    app.step();
    click(&mut app, "Add");
    until(&mut app, "the tag", |a| {
        has(a, "Rename tag Noir") || has(a, "Noir")
    });

    // Rename it from Settings.
    click(&mut app, "Settings");
    click(&mut app, "Rename tag Noir");
    until(&mut app, "the rename dialog", |a| has(a, "Rename tag"));
    type_into_field(&mut app, "Noir", " classics");
    click(&mut app, "Rename");
    until(&mut app, "the new name", |a| {
        has(a, "Rename tag Noir classics")
    });
    let tags = db(root.path()).all_tags().unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "Noir classics");

    // Delete it: the movie stays.
    click(&mut app, "Delete tag Noir classics");
    until(&mut app, "the confirmation", |a| has(a, "Delete tag?"));
    click(&mut app, "Delete tag");
    until(&mut app, "the tag to go", |a| {
        !has(a, "Delete tag Noir classics")
    });
    assert!(db(root.path()).all_tags().unwrap().is_empty());
    assert_eq!(db(root.path()).all_movies().unwrap().len(), 1);
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
    // In a window this small the import toast sits over the poster's details button.
    until(&mut app, "the import toast", |a| {
        has_text(a, "Added 1 movie")
    });
    click(&mut app, "Click to dismiss");
    until(&mut app, "the toast to go", |a| !has(a, "Click to dismiss"));
    open_details(&mut app, "Multi Track");
    until(&mut app, "details", |a| has(a, "Remove from library"));
    click(&mut app, "Close");
    click(&mut app, "Settings");
    until(&mut app, "settings", |a| has(a, "Smooth motion"));
    click(&mut app, "Cancel");
    app.set_size(egui::vec2(400.0, 300.0));
    run_for(&mut app, Duration::from_millis(300));
}

#[test]
fn toolbar_items_never_overlap() {
    let (Some(_), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.multitrack, &root.path().join("movies"), None);
    let mut app = launch_sized(root.path(), vec![], egui::vec2(1280.0, 800.0));
    drop_files(&mut app, &[file]);
    open_details(&mut app, "Multi Track");
    until(&mut app, "details", |a| has(a, "Remove from library"));
    // From roomy to the narrowest window, with the details panel taking its share.
    for width in [1280.0, 1100.0, 960.0, 860.0, 760.0] {
        app.set_size(egui::vec2(width, 800.0));
        run_for(&mut app, Duration::from_millis(200));
        let names = [
            "All movies",
            "Search movies",
            "Sort: Title",
            "Reverse order",
            "Poster grid",
            "List view",
            "Settings",
        ];
        // The page title is the last "All movies" (the first is the sidebar's).
        let rects: Vec<(&str, egui::Rect)> = names
            .iter()
            .filter_map(|&name| {
                let rect = app
                    .root()
                    .children_recursive()
                    .filter(|n| {
                        n.accesskit_node().label().as_deref() == Some(name)
                            || n.accesskit_node().value().as_deref() == Some(name)
                    })
                    .last()?
                    .rect();
                Some((name, rect))
            })
            .collect();
        assert_eq!(rects.len(), names.len(), "at {width}: {rects:?}");
        for (i, (a, ra)) in rects.iter().enumerate() {
            for (b, rb) in &rects[i + 1..] {
                assert!(
                    !ra.shrink(1.0).intersects(rb.shrink(1.0)),
                    "at {width} wide, {a} {ra:?} overlaps {b} {rb:?}"
                );
            }
        }
    }
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
    // Clicking the poster away from its play button opens the details; it doesn't play.
    let corner = settle_on(&mut app, "Long Take", |t| t == "Long Take")
        .rect()
        .left_top()
        + egui::vec2(20.0, 20.0);
    click_at(&mut app, corner);
    until(&mut app, "the details", |a| has(a, "Remove from library"));
    assert!(!in_player(&app));
    click(&mut app, "Play movie: Long Take");
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
    // mpv can report one last position just after the pause itself; let that land.
    run_for(&mut app, Duration::from_millis(300));
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
    click(&mut app, "Play movie: Long Take");
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
    click(&mut app, "Play movie: Long Take");
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
    click(&mut app, "Play movie: Long Take");
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
fn time_left_display_counts_down() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    // The default on a fresh install.
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Play movie: Long Take");
    until(&mut app, "playing", |a| in_player(a) && has(a, "Pause"));
    app.key_press(Key::Space);
    until(&mut app, "paused", |a| has(a, "Play"));
    run_for(&mut app, Duration::from_millis(300));

    // Only one time is shown, and it is the time left: it shrinks as playback moves forward.
    let times = |a: &App| {
        a.root()
            .children_recursive()
            .filter(|n| n.accesskit_node().role() == Role::Label)
            .filter_map(|n| n.accesskit_node().value())
            .filter(|v| v.contains(':') && v.chars().all(|c| c.is_ascii_digit() || c == ':'))
            .count()
    };
    assert_eq!(times(&app), 1);
    let left = shown_time(&app).unwrap();
    let at = shown_position(&app).unwrap();
    assert!(
        (88..=92).contains(&(left + at)),
        "{left} s left at {at} s of 90"
    );
    app.key_press(Key::ArrowRight);
    until(&mut app, "10 s less to go", |a| {
        shown_time(a).is_some_and(|p| p + 9 <= left && p + 11 >= left)
    });
}

#[test]
fn clicking_the_picture_pauses_and_plays() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Play movie: Long Take");
    // Clicks on the picture count once the file has opened and is playing.
    until(&mut app, "playback under way", |a| {
        in_player(a) && has(a, "Pause") && shown_position(a).is_some_and(|p| p >= 1)
    });
    click(&mut app, "Video");
    until(&mut app, "paused", |a| has(a, "Play"));
    // Far enough apart not to count as a double-click (which is fullscreen).
    run_for(&mut app, Duration::from_millis(600));
    click(&mut app, "Video");
    until(&mut app, "playing again", |a| has(a, "Pause"));
    // A double-click is for fullscreen: it neither pauses nor plays. (The harness runs a frame
    // per event, a quarter of a second apart, so allow a longer double-click.)
    run_for(&mut app, Duration::from_millis(600));
    app.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 1.0);
    let picture = settle_on(&mut app, "Video", |t| t == "Video")
        .rect()
        .center();
    app.event(egui::Event::PointerMoved(picture));
    app.step();
    for pressed in [true, false, true, false] {
        app.event(egui::Event::PointerButton {
            pos: picture,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        app.step();
        assert!(
            has(&app, "Pause"),
            "the first click of a double-click paused"
        );
    }
    let end = Instant::now() + Duration::from_millis(800);
    until(&mut app, "the double-click time to pass", |a| {
        assert!(has(a, "Pause"), "a double-click paused playback");
        Instant::now() >= end
    });
    app.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 0.3);
    // The control bar is not the picture: a click on its empty middle changes nothing.
    run_for(&mut app, Duration::from_millis(600));
    let bar = app.ctx.content_rect().center_bottom() - egui::vec2(0.0, 30.0);
    click_at(&mut app, bar);
    run_for(&mut app, Duration::from_millis(300));
    assert!(has(&app, "Pause"), "still playing");
}

#[test]
fn watching_to_the_end_marks_watched_and_returns_to_the_library() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.mkv, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Play movie: The Test Movie");
    until(&mut app, "the player", in_player);
    until(&mut app, "the end", |a| !in_player(a));
    until(&mut app, "the toast", |a| has_text(a, "Finished"));
    let m = db(root.path()).all_movies().unwrap().remove(0);
    assert!(m.watched);
    assert_eq!(m.resume_pos, None);
    // Watched movies play from the start without asking.
    click(&mut app, "Play movie: The Test Movie");
    until(&mut app, "the player", in_player);
    assert!(!has_text(&app, "Resume from"));
}

#[test]
fn shows_group_their_episodes_and_play_them_in_order() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let tv = root.path().join("TV");
    let season = tv.join("Test Show").join("Season 1");
    copy_into(&fx.long, &season, Some("Test.Show.S01E01.Pilot.mkv"));
    copy_into(&fx.webm, &season, Some("S01E02.webm"));
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, std::slice::from_ref(&tv));
    until(&mut app, "the show", |a| {
        has(a, "Test Show") && has(a, "Shows")
    });
    // One card for the show, none for its episodes. (Its middle is a play button.)
    assert!(!has(&app, "Pilot"));
    click(&mut app, "More info: Test Show");
    until(&mut app, "the episodes", |a| {
        has(a, "S01E01 Pilot") && has(a, "S01E02 Episode 2")
    });

    click(&mut app, "Play S01E01");
    until(&mut app, "playback under way", |a| {
        in_player(a) && shown_position(a).is_some_and(|p| p >= 1)
    });
    // Into the last tenth of the 90-second episode, short of the 95% that counts as watched:
    // the next episode is offered rather than going back to the library.
    for _ in 0..8 {
        app.key_press(Key::ArrowRight);
    }
    until(&mut app, "the Up next card", |a| has(a, "Play now"));
    assert!(in_player(&app));
    slow_click(&mut app, "Play now");
    let episode = |n: u32| {
        db(root.path())
            .all_movies()
            .unwrap()
            .into_iter()
            .find(|m| m.episode == Some(n))
            .unwrap()
    };
    // Right away, not after the countdown.
    run_for(&mut app, Duration::from_secs(2));
    assert!(in_player(&app) && episode(2).last_played.is_some());
    // Like the Next episode button: the episode left keeps its place, not yet watched.
    let first = episode(1);
    assert!(!first.watched);
    assert!(
        first.resume_pos.is_some_and(|p| (80.0..86.0).contains(&p)),
        "{:?}",
        first.resume_pos
    );
    // The last episode ends like a movie.
    until(&mut app, "the end of the show", |a| !in_player(a));
    until(&mut app, "the toast", |a| has_text(a, "Finished"));
    assert!(episode(2).watched);
}

#[test]
fn unplayable_file_shows_an_error_and_returns_to_the_library() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.mp4, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, std::slice::from_ref(&file));
    until(&mut app, "the import to finish", |a| {
        has_text(a, "Added 1 movie")
    });
    // The file is replaced by junk behind Dropreel's back.
    std::fs::copy(&fx.garbage, &file).unwrap();
    click(&mut app, "Play movie: Another Film");
    until(&mut app, "an error", |a| has_text(a, "Could not play"));
    until(&mut app, "the library", |a| !in_player(a));
}

/// Whether the app asked for its window to close (the harness itself stays open).
fn asked_to_close(app: &App) -> bool {
    app.state().is_closing()
}

#[test]
fn opening_a_video_from_outside_just_plays_it_and_closing_the_player_closes_dropreel() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![file.clone()]);
    until(&mut app, "the player", |a| in_player(a) && has(a, "Pause"));
    // Not added to the library.
    assert!(db(root.path()).all_movies().unwrap().is_empty());
    click(&mut app, "Close (Esc)");
    until(&mut app, "the window to close", asked_to_close);
    drop(app);

    // Esc closes it too (leaving fullscreen first, when in it).
    let mut app = launch(root.path(), vec![file]);
    until(&mut app, "the player", |a| in_player(a) && has(a, "Pause"));
    app.key_press(Key::Escape);
    until(&mut app, "the window to close", asked_to_close);
}

#[test]
fn opening_a_video_from_outside_can_add_it_to_the_library() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    std::fs::write(
        root.path().join("data").join("settings.toml"),
        "open_files = \"add_to_library\"\n",
    )
    .unwrap();
    let mut app = launch(root.path(), vec![file.clone()]);
    until(&mut app, "the player", |a| in_player(a) && has(a, "Pause"));
    assert_eq!(db(root.path()).all_movies().unwrap().len(), 1);
    click(&mut app, "Back to the library (Esc)");
    until(&mut app, "the library", |a| !in_player(a));
    assert!(!asked_to_close(&app));
    drop(app);

    // Back to "just play": a video already in the library plays from there, keeping its place,
    // without being imported again.
    std::fs::write(root.path().join("data").join("settings.toml"), "").unwrap();
    let mut app = launch(root.path(), vec![file]);
    until(&mut app, "the player or resume prompt", |a| {
        in_player(a) || has_text(a, "Resume from")
    });
    assert_eq!(db(root.path()).all_movies().unwrap().len(), 1);
}

#[test]
fn a_video_handed_over_by_a_later_launch_replaces_what_plays() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let first = copy_into(&fx.long, &root.path().join("movies"), None);
    let second = copy_into(&fx.mkv, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, std::slice::from_ref(&first));
    until(&mut app, "the import", |a| has_text(a, "Added 1 movie"));
    click(&mut app, "Play movie: Long Take");
    until(&mut app, "the player", |a| {
        in_player(a) && has(a, "Pause") && shown_time(a).is_some_and(|t| t > 60)
    });
    // Opened from Explorer while Dropreel runs: it plays here, and closing it goes back to the
    // library, since Dropreel wasn't started for it.
    app.state_mut().open_external(vec![second], false);
    // The 6-second video, with the time left shown.
    until(&mut app, "the second video", |a| {
        in_player(a) && shown_time(a).is_some_and(|t| t <= 6)
    });
    assert!(has(&app, "Back to the library (Esc)"));
    click(&mut app, "Back to the library (Esc)");
    until(&mut app, "the library", |a| !in_player(a));
    assert!(!asked_to_close(&app));
    assert_eq!(db(root.path()).all_movies().unwrap().len(), 1);
}

#[test]
fn the_player_settings_menu_changes_settings_on_the_spot() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.long, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![file]);
    until(&mut app, "the player", |a| in_player(a) && has(a, "Pause"));
    // Fresh settings show the time left.
    until(&mut app, "the time left", |a| shown_time(a).is_some());
    click(&mut app, "Player settings");
    until(&mut app, "the menu", |a| has(a, "Smooth motion"));
    click(&mut app, "Time played and length");
    run_for(&mut app, Duration::from_millis(300));
    // It applies at once; the menu stays open, and so do the controls.
    assert!(has(&app, "Smooth motion"));
    click(&mut app, "Add them to the library");
    // Esc closes the menu, not the player; that saves the changes.
    app.key_press(Key::Escape);
    until(&mut app, "the menu to close", |a| !has(a, "Smooth motion"));
    assert!(in_player(&app) && !asked_to_close(&app));
    let settings = root.path().join("data").join("settings.toml");
    until(&mut app, "the settings to be saved", |_| {
        std::fs::read_to_string(&settings).is_ok_and(|t| t.contains("time_display = \"normal\""))
    });
    let text = std::fs::read_to_string(&settings).unwrap();
    assert!(text.contains("open_files = \"add_to_library\""), "{text}");
    // Still playing: neither the clicks in the menu nor Esc paused or stopped it.
    assert!(has(&app, "Pause"));
}

#[test]
fn tracks_can_be_switched_from_the_player_bar() {
    let Some(fx) = playback_setup() else { return };
    let root = tempfile::tempdir().unwrap();
    let file = copy_into(&fx.multitrack, &root.path().join("movies"), None);
    let mut app = launch(root.path(), vec![]);
    drop_files(&mut app, &[file]);
    click(&mut app, "Play movie: Multi Track");
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
    click(&mut app, "Play movie: Long Take");
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
