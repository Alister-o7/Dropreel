//! End-to-end tests of the library: importing real files with ffprobe/ffmpeg, duplicates,
//! moved and missing files, removal, folder watching, tags and the browse filters.
//! Skipped when ffmpeg/ffprobe are not installed (CI sets DROPREEL_REQUIRE_FFMPEG).

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dropreel::app::state::{Filter, Library, Shelf, SortKey, visible_entries};
use dropreel::library::import::relink_manually;
use dropreel::library::watch::FolderWatcher;
use dropreel::library::{Db, Movie, scan};

use common::{background_import, copy_into, env, fixtures, import, tools, wait_for};

fn movies(db_path: &Path) -> Vec<Movie> {
    Db::open(db_path).unwrap().all_movies().unwrap()
}

fn by_file<'a>(movies: &'a [Movie], file: &Path) -> &'a Movie {
    let name = file.file_name().unwrap();
    movies
        .iter()
        .find(|m| m.path.file_name() == Some(name))
        .unwrap_or_else(|| panic!("{} is not in the library", file.display()))
}

#[test]
fn imports_every_supported_format_recursively_with_details_and_posters() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let drop = root.path().join("Movies");
    let good = [
        &fx.mkv,
        &fx.mp4,
        &fx.avi,
        &fx.webm,
        &fx.ts,
        &fx.hevc10,
        &fx.long,
        &fx.multitrack,
        &fx.silent,
        &fx.unicode,
    ];
    for (i, file) in good.iter().enumerate() {
        // Spread the files over nested folders.
        copy_into(
            file,
            &drop.join(format!("shelf {}", i % 3)).join("deeper"),
            None,
        );
    }
    // Files that are not videos are ignored.
    for junk in [
        "readme.txt",
        "poster.jpg",
        "movie.nfo",
        "subs.srt",
        "sample.mkv.part",
    ] {
        std::fs::write(drop.join(junk), b"not a video").unwrap();
    }

    let env = env(root.path(), &tools);
    let report = import(&env, &[drop]);
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);
    assert!(report.skipped.is_empty(), "skipped: {:?}", report.skipped);
    assert!(
        report.warnings.is_empty(),
        "warnings: {:?}",
        report.warnings
    );
    assert_eq!(report.added.len(), good.len(), "added: {:?}", report.added);

    let all = movies(&env.db_path);
    assert_eq!(all.len(), good.len());
    for m in &all {
        assert!(m.path.is_absolute(), "{} is not absolute", m.path.display());
        assert!(m.path.exists());
        assert_eq!(m.size, std::fs::metadata(&m.path).unwrap().len());
        assert!(!m.fingerprint.is_empty());
        let d = m
            .duration
            .unwrap_or_else(|| panic!("{}: no duration", m.title));
        assert!(d > 3.5, "{}: duration {d}", m.title);
        assert!(m.video_codec.is_some(), "{}: no codec", m.title);
        assert_eq!(
            m.width,
            Some(if m.title.contains("Long") { 160 } else { 320 })
        );
        let poster = m
            .poster
            .as_ref()
            .unwrap_or_else(|| panic!("{}: no poster", m.title));
        assert!(poster.exists(), "{}: poster file missing", m.title);
        assert!(!m.watched && m.resume_pos.is_none() && m.last_played.is_none());
    }

    let hevc = by_file(&all, &fx.hevc10);
    assert_eq!(hevc.title, "Drive");
    assert_eq!(hevc.year, Some(2011));
    assert_eq!(hevc.video_codec.as_deref(), Some("hevc"));
    assert_eq!(hevc.audio_tracks.len(), 1);
    assert_eq!(hevc.audio_tracks[0].channels, Some(6));

    let main = by_file(&all, &fx.mkv);
    assert_eq!(main.title, "The Test Movie");
    assert_eq!(main.year, Some(2019));
    assert!((main.duration.unwrap() - 6.0).abs() < 0.2);

    let mp4 = by_file(&all, &fx.mp4);
    assert_eq!((mp4.title.as_str(), mp4.year), ("Another Film", Some(2007)));

    let multi = by_file(&all, &fx.multitrack);
    let audio: Vec<_> = multi
        .audio_tracks
        .iter()
        .map(|t| (t.language.as_deref(), t.title.as_deref()))
        .collect();
    assert_eq!(
        audio,
        [
            (Some("eng"), Some("Main")),
            (Some("fre"), Some("Commentary"))
        ]
    );
    let subs: Vec<_> = multi
        .subtitle_tracks
        .iter()
        .map(|t| (t.language.as_deref(), t.title.as_deref()))
        .collect();
    assert_eq!(
        subs,
        [
            (Some("eng"), Some("English SDH")),
            (Some("ger"), Some("Deutsch"))
        ]
    );

    let silent = by_file(&all, &fx.silent);
    assert!(silent.audio_tracks.is_empty());

    let unicode = by_file(&all, &fx.unicode);
    assert!(unicode.title.starts_with("Amélie"), "{}", unicode.title);
    assert_eq!(unicode.year, Some(2001));
    let db = Db::open(&env.db_path).unwrap();
    assert_eq!(db.find_by_path(&unicode.path).unwrap(), Some(unicode.id));
}

#[test]
fn files_that_are_not_playable_videos_are_reported_not_added() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let drop = root.path().join("in");
    for f in [&fx.garbage, &fx.empty, &fx.truncated] {
        copy_into(f, &drop, None);
    }
    let env = env(root.path(), &tools);
    let report = import(&env, &[drop]);

    let failed: Vec<_> = report
        .failed
        .iter()
        .map(|(p, _)| p.file_name().unwrap().to_owned())
        .collect();
    assert!(
        failed.contains(&fx.garbage.file_name().unwrap().to_owned()),
        "garbage file should fail: {report:?}"
    );
    assert!(
        failed.contains(&fx.empty.file_name().unwrap().to_owned()),
        "empty file should fail: {report:?}"
    );
    for (_, reason) in &report.failed {
        assert!(!reason.is_empty());
    }
    // A partial download still plays as far as it goes, so it is kept.
    assert_eq!(report.added, ["Truncated Download"], "{report:?}");
    assert_eq!(movies(&env.db_path).len(), 1);
    assert!(
        report.summary().contains("2 failed"),
        "{}",
        report.summary()
    );
}

#[test]
fn duplicates_are_skipped_within_one_drop_and_across_drops() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let a = copy_into(&fx.mkv, &root.path().join("a"), None);
    let b = copy_into(&fx.mkv, &root.path().join("b"), Some("copy of movie.mkv"));
    let env = env(root.path(), &tools);

    let report = import(&env, &[a.clone(), b.clone()]);
    assert_eq!(report.added.len(), 1, "{report:?}");
    assert_eq!(report.skipped.len(), 1, "{report:?}");
    assert!(
        report.skipped[0].1.starts_with("duplicate of"),
        "{report:?}"
    );

    // Dropping the same file again says it's already there, not that it's a duplicate.
    let again = import(&env, std::slice::from_ref(&a));
    assert!(again.added.is_empty());
    assert_eq!(again.skipped.len(), 1, "{again:?}");
    assert_eq!(again.skipped[0].1, "already in the library");
    assert_eq!(again.summary(), "Skipped 1 file");

    // The same file through a different spelling of its path is still the same file.
    let detour = root
        .path()
        .join("a")
        .join("..")
        .join("a")
        .join(a.file_name().unwrap());
    let detour_report = import(&env, &[detour]);
    assert!(detour_report.added.is_empty(), "{detour_report:?}");

    // Background scans of watched folders stay quiet about known files, and add nothing.
    let scan = background_import(&env, &[root.path().to_path_buf()]);
    assert!(scan.added.is_empty() && scan.failed.is_empty(), "{scan:?}");
    assert_eq!(movies(&env.db_path).len(), 1);
}

#[test]
fn concurrent_imports_of_the_same_files_add_each_movie_once() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let drop = root.path().join("in");
    for f in [&fx.mkv, &fx.mp4, &fx.avi, &fx.ts, &fx.silent] {
        copy_into(f, &drop, None);
    }
    let env = env(root.path(), &tools);
    Db::open(&env.db_path).unwrap();

    // E.g. a drag & drop while the startup scan of a watched folder is running.
    let reports: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..3)
            .map(|i| {
                let env = env.clone();
                let drop = drop.clone();
                s.spawn(move || common::import_with(&env, &[drop], i != 0))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    assert_eq!(movies(&env.db_path).len(), 5);
    let added: usize = reports.iter().map(|r| r.added.len()).sum();
    assert_eq!(added, 5, "{reports:#?}");
    for r in &reports {
        assert!(r.failed.is_empty(), "{r:?}");
        for (path, reason) in &r.skipped {
            assert_eq!(
                reason,
                "already in the library",
                "{} was reported as a duplicate of itself",
                path.display()
            );
        }
    }
}

#[test]
fn moved_files_are_found_again_and_keep_their_history() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let old = copy_into(&fx.long, &root.path().join("old"), None);
    let env = env(root.path(), &tools);
    import(&env, std::slice::from_ref(&old));

    let db = Db::open(&env.db_path).unwrap();
    let id = db.all_movies().unwrap()[0].id;
    db.set_resume(id, Some(12.5)).unwrap();
    db.mark_played(id).unwrap();
    let tag = db.create_tag("Favourites").unwrap();
    db.set_movie_tag(id, tag, true).unwrap();

    // The user moves (and renames) the file outside Dropreel.
    let new_dir = root.path().join("new");
    std::fs::create_dir_all(&new_dir).unwrap();
    let new = new_dir.join("renamed.mkv");
    std::fs::rename(&old, &new).unwrap();

    let lib = Library::load(&db).unwrap();
    assert!(lib.missing.contains(&id));
    assert_eq!(lib.shelf_count(Shelf::Missing), 1);

    // A scan of the folder it moved to re-links it instead of adding a second movie.
    let report = background_import(&env, &[new_dir]);
    assert_eq!(report.relinked.len(), 1, "{report:?}");
    assert!(report.added.is_empty());
    let all = db.all_movies().unwrap();
    assert_eq!(all.len(), 1);
    let m = &all[0];
    assert_eq!(m.id, id);
    assert_eq!(m.path, scan::normalize(&new));
    assert_eq!(m.resume_pos, Some(12.5));
    assert!(m.last_played.is_some());
    let lib = Library::load(&db).unwrap();
    assert!(lib.missing.is_empty());
    assert_eq!(lib.tag_names(id), ["Favourites"]);
    // The original title is kept, not re-parsed from the new name.
    assert_eq!(m.title, "Long Take");
}

#[test]
fn removed_movies_stay_removed_until_added_by_hand() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("watched");
    let file = copy_into(&fx.mp4, &folder, None);
    let env = env(root.path(), &tools);
    background_import(&env, std::slice::from_ref(&folder));
    let db = Db::open(&env.db_path).unwrap();
    let id = db.all_movies().unwrap()[0].id;

    db.remove_movie(id).unwrap();
    assert!(
        file.exists(),
        "removing from the library must never delete the file"
    );
    assert!(db.all_movies().unwrap().is_empty());

    // The watched-folder scan (e.g. at the next start) doesn't bring it back...
    assert!(background_import(&env, std::slice::from_ref(&folder)).is_empty());
    assert!(db.all_movies().unwrap().is_empty());

    // ...but dropping it onto the window does, and then scans see it as known again.
    let report = import(&env, std::slice::from_ref(&file));
    assert_eq!(report.added.len(), 1, "{report:?}");
    assert!(db.ignored_paths().unwrap().is_empty());
    assert!(background_import(&env, &[folder]).is_empty());
}

#[test]
fn locate_relinks_by_hand_and_rejects_bad_choices() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let a = copy_into(&fx.mkv, &root.path().join("lib"), None);
    let b = copy_into(&fx.mp4, &root.path().join("lib"), None);
    let env = env(root.path(), &tools);
    import(&env, &[a.clone(), b.clone()]);
    let db = Db::open(&env.db_path).unwrap();
    let all = db.all_movies().unwrap();
    let id_a = by_file(&all, &a).id;

    std::fs::remove_file(&a).unwrap();
    let text = root.path().join("notes.txt");
    std::fs::write(&text, "hi").unwrap();
    assert!(relink_manually(&env, id_a, &text).is_err());
    assert!(
        relink_manually(&env, id_a, &b)
            .unwrap_err()
            .contains("already in the library")
    );

    let replacement = copy_into(&fx.avi, &root.path().join("elsewhere"), None);
    relink_manually(&env, id_a, &replacement).unwrap();
    let m = db.movie(id_a).unwrap().unwrap();
    assert_eq!(m.path, scan::normalize(&replacement));
    assert_eq!(m.size, std::fs::metadata(&replacement).unwrap().len());
    assert!(Library::load(&db).unwrap().missing.is_empty());
}

#[test]
fn library_survives_reopening_the_database() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let files = [
        copy_into(&fx.mkv, &root.path().join("m"), None),
        copy_into(&fx.unicode, &root.path().join("m"), None),
    ];
    let env = env(root.path(), &tools);
    import(&env, &files);
    let before = movies(&env.db_path);
    {
        let db = Db::open(&env.db_path).unwrap();
        db.set_watched(before[0].id, true).unwrap();
    }
    let after = movies(&env.db_path);
    assert_eq!(after.len(), 2);
    assert!(after.iter().any(|m| m.watched));
    assert_eq!(
        before.iter().map(|m| &m.path).collect::<Vec<_>>(),
        after.iter().map(|m| &m.path).collect::<Vec<_>>()
    );
}

#[test]
fn watched_folder_picks_up_new_files_once_they_finish_copying() {
    let Some(fx) = fixtures() else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let watched = root.path().join("watched");
    std::fs::create_dir_all(&watched).unwrap();
    let seen: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let sink = seen.clone();
    let (_watcher, errors) = FolderWatcher::start(
        std::slice::from_ref(&watched),
        Duration::from_millis(500),
        move |files| sink.lock().unwrap().extend(files),
    )
    .unwrap();
    assert!(errors.is_empty(), "{errors:?}");

    // A file copied in slowly, like a big download: reported once, only when complete.
    let slow = watched.join("Slow.Copy.2018.mkv");
    let bytes = std::fs::read(&fx.mkv).unwrap();
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&slow).unwrap();
        for chunk in bytes.chunks(bytes.len() / 6 + 1) {
            f.write_all(chunk).unwrap();
            f.flush().unwrap();
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                seen.lock().unwrap().is_empty(),
                "reported while still copying"
            );
        }
    }
    // A folder created after watching started, with a video inside, and a non-video file.
    let nested = watched.join("New Folder").join("CD1");
    std::fs::create_dir_all(&nested).unwrap();
    let inner = copy_into(&fx.mp4, &nested, None);
    std::fs::write(watched.join("info.nfo"), "x").unwrap();

    assert!(
        wait_for(Duration::from_secs(15), || {
            let s = seen.lock().unwrap();
            s.contains(&slow) && s.iter().any(|p| p.file_name() == inner.file_name())
        }),
        "watcher reported {:?}",
        seen.lock().unwrap()
    );
    std::thread::sleep(Duration::from_millis(1500));
    let s = seen.lock().unwrap();
    assert_eq!(s.iter().filter(|p| **p == slow).count(), 1, "{s:?}");
    assert_eq!(std::fs::metadata(&slow).unwrap().len(), bytes.len() as u64);
    assert!(s.iter().all(|p| scan::is_video_file(p)), "{s:?}");
}

#[test]
fn episodes_are_grouped_into_shows() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let tv = root.path().join("TV");
    copy_into(
        &fx.mkv,
        &tv.join("Night Shift/Season 1"),
        Some("Night.Shift.S01E01.Pilot.mkv"),
    );
    copy_into(
        &fx.mp4,
        &tv.join("Night Shift/Season 2"),
        Some("S02E01.mp4"),
    );
    copy_into(&fx.webm, &tv, Some("Other Show - 1x05 - Five.webm"));
    copy_into(&fx.avi, &root.path().join("Movies"), Some("Heat.1995.avi"));
    let env = env(root.path(), &tools);
    let report = import(&env, &[tv.clone(), root.path().join("Movies")]);
    assert_eq!(report.summary(), "Added 1 movie and 3 episodes");

    let db = Db::open(&env.db_path).unwrap();
    let mut shows = db.all_shows().unwrap();
    shows.sort_by(|a, b| a.title.cmp(&b.title));
    let names: Vec<&str> = shows.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(names, ["Night Shift", "Other Show"]);
    let lib = Library::load(&db).unwrap();
    let night = lib.show(shows[0].id).unwrap();
    let episodes: Vec<_> = lib
        .episodes(night)
        .map(|m| (m.episode_code().unwrap(), m.title.clone()))
        .collect();
    assert_eq!(
        episodes,
        [
            ("S01E01".to_string(), "Pilot".to_string()),
            ("S02E01".to_string(), "Episode 1".to_string()),
        ]
    );
    let heat = movies(&env.db_path)
        .into_iter()
        .find(|m| m.show_id.is_none())
        .unwrap();
    assert_eq!((heat.title.as_str(), heat.year), ("Heat", Some(1995)));

    // A new episode joins its show; removing a show's episodes removes the show.
    copy_into(&fx.ts, &tv.join("Night Shift/Season 2"), Some("S02E02.ts"));
    background_import(&env, std::slice::from_ref(&tv));
    let lib = Library::load(&db).unwrap();
    assert_eq!(lib.show(shows[0].id).unwrap().episodes.len(), 3);
    for m in lib.episodes(lib.show(shows[1].id).unwrap()) {
        db.remove_movie(m.id).unwrap();
    }
    db.remove_empty_shows().unwrap();
    assert_eq!(db.all_shows().unwrap().len(), 1);
}

#[test]
fn search_sort_shelves_and_tags_over_a_real_library() {
    let (Some(tools), Some(fx)) = (tools(), fixtures()) else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("m");
    for f in [&fx.mkv, &fx.mp4, &fx.hevc10, &fx.unicode, &fx.silent] {
        copy_into(f, &dir, None);
    }
    let env = env(root.path(), &tools);
    import(&env, &[dir]);
    let db = Db::open(&env.db_path).unwrap();
    let all = db.all_movies().unwrap();
    let id = |file: &Path| by_file(&all, file).id;

    db.set_watched(id(&fx.mp4), true).unwrap();
    db.set_resume(id(&fx.hevc10), Some(1.5)).unwrap();
    let noir = db.create_tag("Neo-noir").unwrap();
    db.set_movie_tag(id(&fx.hevc10), noir, true).unwrap();
    std::fs::remove_file(by_file(&all, &fx.silent).path.clone()).unwrap();

    let lib = Library::load(&db).unwrap();
    let titles = |filter: &Filter| -> Vec<String> {
        visible_entries(&lib, filter)
            .into_iter()
            .map(|e| lib.entry_title(e).to_string())
            .collect()
    };
    let search = |s: &str| {
        titles(&Filter {
            search: s.into(),
            ..Filter::default()
        })
    };

    assert_eq!(search("drive"), ["Drive"]);
    assert_eq!(search("  DRIVE "), ["Drive"]);
    assert_eq!(search("2011"), ["Drive"]);
    assert_eq!(search("nothing like this"), Vec::<String>::new());
    // Searching without accents still finds accented titles.
    assert_eq!(search("amelie").len(), 1, "{:?}", search("amelie"));

    let by_title = titles(&Filter::default());
    assert_eq!(by_title.len(), 5);
    // "The Test Movie" sorts under T without "The": after Silent, before nothing else.
    let pos = |t: &str| by_title.iter().position(|x| x.starts_with(t)).unwrap();
    assert!(pos("Another") < pos("Drive") && pos("Drive") < pos("Silent"));
    assert!(pos("Silent") < pos("The Test Movie"));

    let by_year = titles(&Filter {
        sort: SortKey::Year,
        descending: true,
        ..Filter::default()
    });
    assert_eq!(by_year.first().map(String::as_str), Some("The Test Movie"));

    let shelf = |shelf| {
        titles(&Filter {
            shelf,
            ..Filter::default()
        })
    };
    assert_eq!(shelf(Shelf::Watched), ["Another Film"]);
    assert_eq!(shelf(Shelf::ContinueWatching), ["Drive"]);
    assert_eq!(shelf(Shelf::Unwatched).len(), 4);
    assert_eq!(shelf(Shelf::Missing).len(), 1);
    assert_eq!(
        titles(&Filter {
            tag: Some(noir),
            ..Filter::default()
        }),
        ["Drive"]
    );
    assert_eq!(lib.tag_count(noir), 1);

    // Deleting a tag removes it from movies too.
    db.delete_tag(noir).unwrap();
    let lib = Library::load(&db).unwrap();
    assert!(lib.tag_names(id(&fx.hevc10)).is_empty());
    assert_eq!(lib.tag_count(noir), 0);
}
