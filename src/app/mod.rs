//! The Dropreel window.

mod detail;
mod dialogs;
pub mod events;
mod icons;
mod jobs;
mod library_view;
mod player_view;
mod show_detail;
pub mod state;
mod theme;
mod toasts;
mod watchdog;
mod widgets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;

use crate::encode::{
    EncodeConfig, EncodeEvent, EncodeJob, EncodeOutcome, EncodePreview, EncodeQueue, EncodeStatus,
    EncoderDetector,
};
use crate::library::import::{self, ImportProgress, ImportReport, LibraryEnv};
use crate::library::watch::{FolderWatcher, SETTLE_TIME};
use crate::library::{Db, MovieId, ShowId, TagId, scan};
use crate::metadata::{self, tmdb::TmdbMovie};
use crate::paths::AppPaths;
use crate::player::Player;
use crate::settings::{OpenFiles, Settings, ThemeChoice};
use crate::tools::Tools;
use dialogs::Dialogs;
use events::{AppEvent, EventSink, ToastKind};
use state::{Filter, Library, Prefs};
use toasts::Toasts;

const PREFS_KEY: &str = "dropreel-prefs";

/// Things the UI asked for during a frame, applied after drawing.
pub enum Action {
    Play(MovieId),
    /// Start playback, optionally from a resume position.
    StartPlayback(MovieId, Option<f64>),
    StopPlayback {
        finished: bool,
    },
    /// Go on to the next episode of a show (the Next episode button, N, and the Up next card's
    /// Play now and countdown alike).
    PlayNextEpisode,
    PlaybackFailed(String),
    /// A setting changed in the player's own menu: apply it now, save it once the menu closes.
    PlayerSettingsChanged,
    ShowDetails(MovieId),
    /// Open a show's panel: its seasons and episodes.
    ShowShow(ShowId),
    /// Play a show's next episode (see [`Library::next_episode`]).
    PlayShow(ShowId),
    /// Mark every episode of a show watched or unwatched.
    SetShowWatched(ShowId, bool),
    /// Look a show up on TMDb: poster, plot and episode titles.
    FetchShowMetadata(ShowId),
    /// Asks before removing a show (all its episodes) from the library.
    AskRemoveShow(ShowId),
    RemoveShow(ShowId),
    CloseDetails,
    OpenFolder(MovieId),
    Locate(MovieId),
    AskRemove(MovieId),
    Remove(MovieId),
    SetTag(MovieId, TagId, bool),
    CreateTag(MovieId, String),
    /// Asks for a new name for a tag.
    AskRenameTag(TagId),
    RenameTag(TagId, String),
    /// Asks before deleting a tag.
    AskDeleteTag(TagId),
    DeleteTag(TagId),
    SetWatched(MovieId, bool),
    EditTitle(MovieId),
    SaveTitle(MovieId, String, Option<i32>),
    OpenTmdbSearch(MovieId),
    TmdbSearch,
    TmdbPick(MovieId, TmdbMovie),
    AddFiles,
    AddFolder,
    Import(Vec<PathBuf>),
    OpenSettings,
    SaveSettings(Box<Settings>),
    RescanWatched,
    /// Open the re-encode preview for a movie.
    AskReencode(MovieId),
    StartEncode(Box<EncodePreview>),
    CancelEncode(MovieId),
    /// What to do with the original after a successful encode.
    ResolveOriginal(OriginalChoice),
    /// Quit even though an encode is running.
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginalChoice {
    Keep,
    Trash,
    Delete,
}

/// What the details panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// A movie or an episode.
    Movie(MovieId),
    Show(ShowId),
}

/// The re-encode preview dialog.
pub enum PreviewDialog {
    Loading { movie_id: MovieId, title: String },
    Ready(Box<EncodePreview>),
}

/// A movie's place in the encode queue, for drawing overlays.
#[derive(Clone, Debug)]
pub enum EncodeBadge {
    Queued(usize),
    Running(EncodeStatus),
}

pub struct DropReelApp {
    paths: AppPaths,
    settings: Settings,
    tools: Tools,
    db: Db,
    events: EventSink,
    rx: Receiver<AppEvent>,

    lib: Library,
    prefs: Prefs,
    filter: Filter,
    visible: Vec<state::Entry>,
    filter_dirty: bool,
    selected: Option<Detail>,
    /// What the details panel shows; kept while the panel slides closed.
    detail_shown: Option<Detail>,
    /// When the switch between the library and the player started: the new screen fades in
    /// from black.
    screen_fade: Option<Instant>,

    imports_running: usize,
    import_progress: Option<ImportProgress>,
    reports: VecDeque<ImportReport>,
    toasts: Toasts,
    dialogs: Dialogs,
    watcher: Option<FolderWatcher>,

    /// The built-in player, or why it is unavailable.
    player: Result<Player, String>,
    /// Software OpenGL (e.g. in a VM): always use simple video scaling.
    software_gl: bool,
    now_playing: Option<player_view::NowPlaying>,
    /// "Resume from …?" question: (movie, position).
    resume_prompt: Option<(MovieId, f64)>,

    encode_queue: EncodeQueue,
    watchdog: watchdog::Watchdog,
    detector: Arc<EncoderDetector>,
    encode_preview: Option<PreviewDialog>,
    finished_encodes: VecDeque<EncodeOutcome>,
    /// Refreshed every frame from the queue.
    encode_running: Option<EncodeStatus>,
    encode_queued: Vec<(MovieId, String)>,
    encode_badges: HashMap<MovieId, EncodeBadge>,
    quit_prompt: bool,
    allow_close: bool,
    /// A file opened with "Open with → Dropreel": play it once it's in the library.
    play_after_import: Option<PathBuf>,
    /// The library movie being started for a video opened from outside, when Dropreel was
    /// started just for it: once playing, closing the player closes Dropreel.
    close_after: Option<MovieId>,
    /// Settings changed in the player's menu and not yet saved.
    settings_unsaved: bool,
    /// The window was asked to close (after playing a video Dropreel was opened for).
    closing: bool,
    /// Files a background scan failed to import that the user has already been told about.
    reported_failures: HashSet<PathBuf>,

    reload_requested: bool,
    last_reload: Instant,
    actions: Vec<Action>,
}

impl DropReelApp {
    /// `open_paths` are files passed on the command line ("Open with Dropreel").
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: AppPaths,
        open_paths: Vec<PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::build(cc, paths, open_paths, false)
    }

    /// Like `new`, but when there is no OpenGL context (as in UI tests) the player still works,
    /// just without drawing video.
    pub fn new_headless(
        cc: &eframe::CreationContext<'_>,
        paths: AppPaths,
        open_paths: Vec<PathBuf>,
    ) -> anyhow::Result<Self> {
        Self::build(cc, paths, open_paths, true)
    }

    fn build(
        cc: &eframe::CreationContext<'_>,
        paths: AppPaths,
        open_paths: Vec<PathBuf>,
        headless_player: bool,
    ) -> anyhow::Result<Self> {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        theme::install(&cc.egui_ctx);
        let (settings, settings_warning) = Settings::load(&paths.settings);
        let tools = Tools::locate(&settings);
        let db = Db::open(&paths.db)?;
        let lib = Library::load(&db)?;
        let prefs: Prefs = cc
            .storage
            .and_then(|s| eframe::get_value(s, PREFS_KEY))
            .unwrap_or_default();
        let (events, rx) = EventSink::new(cc.egui_ctx.clone());
        let mut software_gl = false;
        let player = match (&cc.get_proc_address, &cc.gl) {
            (Some(get_proc), Some(gl)) => {
                use eframe::glow::HasContext as _;
                let renderer = unsafe { gl.get_parameter_string(eframe::glow::RENDERER) };
                software_gl = crate::player::is_software_renderer(&renderer);
                tracing::info!("OpenGL renderer: {renderer}");
                Player::new(get_proc.clone(), cc.egui_ctx.clone())
            }
            _ if headless_player => Player::headless(cc.egui_ctx.clone()),
            _ => Err("OpenGL is not available".to_string()),
        };
        if let Err(e) = &player {
            tracing::warn!("built-in player unavailable: {e}");
        }
        let detector = Arc::new(EncoderDetector::default());
        let encode_events = events.clone();
        let repaint_ctx = cc.egui_ctx.clone();
        let encode_queue = EncodeQueue::new(
            EncodeConfig::new(paths.db.clone(), tools.clone(), &settings),
            detector.clone(),
            move |event| encode_events.send(AppEvent::Encode(event)),
            move || repaint_ctx.request_repaint(),
        );

        let mut app = Self {
            filter: Filter {
                sort: prefs.sort,
                descending: prefs.descending,
                ..Filter::default()
            },
            paths,
            settings,
            tools,
            db,
            events,
            rx,
            lib,
            prefs,
            visible: Vec::new(),
            filter_dirty: true,
            selected: None,
            detail_shown: None,
            screen_fade: None,
            imports_running: 0,
            import_progress: None,
            reports: VecDeque::new(),
            toasts: Toasts::default(),
            dialogs: Dialogs::default(),
            watcher: None,
            player,
            software_gl,
            now_playing: None,
            resume_prompt: None,
            encode_queue,
            detector,
            encode_preview: None,
            finished_encodes: VecDeque::new(),
            encode_running: None,
            encode_queued: Vec::new(),
            encode_badges: HashMap::new(),
            quit_prompt: false,
            allow_close: false,
            play_after_import: None,
            close_after: None,
            settings_unsaved: false,
            closing: false,
            watchdog: watchdog::Watchdog::start(),
            reported_failures: HashSet::new(),
            reload_requested: false,
            last_reload: Instant::now(),
            actions: Vec::new(),
        };
        app.apply_theme(&cc.egui_ctx);
        app.apply_video_quality();
        // eframe restores the last window state; never start in fullscreen on the library.
        cc.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        if let Some(w) = settings_warning {
            app.toasts.add(ToastKind::Error, w);
        }
        app.warn_about_missing_tools();
        app.restart_watcher();
        jobs::upgrade_posters(app.env(), app.events.clone());
        // Pick up files added to watched folders while Dropreel was closed, and find moved files.
        if !app.settings.watched_folders.is_empty() {
            app.start_import(app.settings.watched_folders.clone(), false);
        }
        app.open_external(open_paths, true);
        Ok(app)
    }

    /// Opens what was handed to Dropreel from outside: the files it was started with ("Open
    /// with", or Dropreel as the default player), or forwarded by a second launch. One video
    /// plays (see [`OpenFiles`]); anything else is imported. `at_startup`: Dropreel was started
    /// for these, so with "Just play" closing the player closes Dropreel too.
    pub fn open_external(&mut self, paths: Vec<PathBuf>, at_startup: bool) {
        if paths.is_empty() {
            return;
        }
        tracing::info!("opening {paths:?} (at startup: {at_startup})");
        let [single] = paths.as_slice() else {
            self.start_import(paths, true);
            return;
        };
        if !scan::is_video_file(single) {
            self.start_import(paths, true);
            return;
        }
        let path = scan::normalize(single);
        if !path.is_file() {
            self.toasts.add(
                ToastKind::Error,
                format!("{} was not found.", path.display()),
            );
            return;
        }
        // Without the built-in player there is nothing to play it with: handing it to the
        // system's default player could be Dropreel again, and round and round.
        if let Err(reason) = &self.player {
            self.toasts.add(
                ToastKind::Error,
                format!("Could not play {}: {reason}", path.display()),
            );
            return;
        }
        // Whatever was playing gives way, its position saved.
        if self.now_playing.is_some() {
            self.end_playback(false, false);
        }
        self.resume_prompt = None;
        self.close_after = None;
        match (self.settings.open_files, self.find_movie_for_file(&path)) {
            (_, Some(id)) => {
                if at_startup && self.settings.open_files == OpenFiles::JustPlay {
                    self.close_after = Some(id);
                }
                self.actions.push(Action::Play(id));
            }
            (OpenFiles::JustPlay, None) => self.play_file(&path, at_startup),
            (OpenFiles::AddToLibrary, None) => {
                self.play_after_import = Some(path);
                self.start_import(paths, false);
            }
        }
    }

    /// Whether Dropreel asked for its window to close, its job done.
    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// Where later launches of Dropreel hand over their files: they open in this window, which
    /// comes to the front.
    pub fn external_opener(&self) -> impl Fn(Vec<PathBuf>) + Send + 'static {
        let events = self.events.clone();
        move |paths| {
            let ctx = events.context();
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            events.send(AppEvent::OpenPaths(paths));
        }
    }

    /// Plays a video that isn't in the library, without adding it.
    fn play_file(&mut self, path: &std::path::Path, close_when_done: bool) {
        let Ok(player) = self.player.as_mut() else {
            return;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (title, subtitle) = match metadata::filename::parse_episode(path) {
            Some(ep) => {
                let code = format!("S{:02}E{:02}", ep.season, ep.episode);
                let subtitle = match ep.title {
                    Some(t) => format!("{code} · {t}"),
                    None => code,
                };
                (ep.show, Some(subtitle))
            }
            None => (metadata::filename::parse_title_year(&stem).0, None),
        };
        let title = if title.is_empty() { stem } else { title };
        tracing::info!("playing {} (not in the library)", path.display());
        match player.load(path, None) {
            Ok(()) => {
                let mut np = player_view::NowPlaying::new(None, title, None);
                np.subtitle = subtitle;
                np.close_when_done = close_when_done;
                self.now_playing = Some(np);
                self.screen_fade = Some(Instant::now());
            }
            Err(e) => self.toasts.add(
                ToastKind::Error,
                format!("Could not play {}: {e}", path.display()),
            ),
        }
    }

    fn env(&self) -> LibraryEnv {
        LibraryEnv {
            db_path: self.paths.db.clone(),
            posters_dir: self.paths.posters.clone(),
            tools: self.tools.clone(),
            tmdb_key: self.settings.tmdb_api_key.clone(),
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::Dark => egui::Theme::Dark,
            ThemeChoice::Light => egui::Theme::Light,
        });
    }

    fn apply_video_quality(&mut self) {
        let high = self.settings.high_quality_video && !self.software_gl;
        let smooth = self.settings.smooth_motion && !self.software_gl;
        if let Ok(player) = self.player.as_mut() {
            player.set_high_quality_scaling(high);
            player.set_smooth_motion(smooth);
        }
    }

    fn warn_about_missing_tools(&mut self) {
        let missing: Vec<&str> = [
            ("ffmpeg", self.tools.ffmpeg.is_none()),
            ("ffprobe", self.tools.ffprobe.is_none()),
        ]
        .into_iter()
        .filter_map(|(name, missing)| missing.then_some(name))
        .collect();
        if !missing.is_empty() {
            self.toasts.add(
                ToastKind::Error,
                format!(
                    "{} not found — thumbnails and technical details are unavailable. Put the tools next to Dropreel.exe or set their paths in Settings.",
                    missing.join(" and ")
                ),
            );
        }
    }

    fn restart_watcher(&mut self) {
        self.watcher = None;
        let folders = self.settings.watched_folders.clone();
        if folders.is_empty() {
            return;
        }
        let events = self.events.clone();
        match FolderWatcher::start(&folders, SETTLE_TIME, move |files| {
            events.send(AppEvent::WatchedFiles(files))
        }) {
            Ok((watcher, errors)) => {
                self.watcher = Some(watcher);
                for e in errors {
                    self.toasts.add(ToastKind::Error, e);
                }
            }
            Err(e) => self.toasts.add(
                ToastKind::Error,
                format!("Folder watching is unavailable: {e}"),
            ),
        }
    }

    fn start_import(&mut self, paths: Vec<PathBuf>, interactive: bool) {
        self.imports_running += 1;
        jobs::import(self.env(), self.events.clone(), paths, interactive);
    }

    fn reload(&mut self) {
        match Library::load(&self.db) {
            Ok(lib) => {
                self.lib = lib;
                let gone = match self.selected {
                    Some(Detail::Movie(id)) => self.lib.get(id).is_none(),
                    Some(Detail::Show(id)) => self.lib.show(id).is_none(),
                    None => false,
                };
                if gone {
                    self.selected = None;
                }
                if self
                    .filter
                    .tag
                    .is_some_and(|t| !self.lib.tags.iter().any(|tag| tag.id == t))
                {
                    self.filter.tag = None;
                }
            }
            Err(e) => self.toasts.add(
                ToastKind::Error,
                format!("Could not read the library: {e:#}"),
            ),
        }
        self.filter_dirty = true;
        self.reload_requested = false;
        self.last_reload = Instant::now();
    }

    /// Reloads at most a few times a second while an import streams in changes.
    fn maybe_reload(&mut self, ctx: &egui::Context) {
        if !self.reload_requested {
            return;
        }
        let wait = Duration::from_millis(400).saturating_sub(self.last_reload.elapsed());
        if wait.is_zero() {
            self.reload();
        } else {
            ctx.request_repaint_after(wait);
        }
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                AppEvent::LibraryChanged => self.reload_requested = true,
                AppEvent::OpenPaths(paths) => self.open_external(paths, false),
                AppEvent::ImportProgress(p) => self.import_progress = Some(p),
                AppEvent::ImportFinished {
                    mut report,
                    interactive,
                } => {
                    // Scans of watched folders run at every start and on every change; tell
                    // the user about a file that can't be imported only once per session.
                    if let Ok(report) = &mut report {
                        if interactive {
                            for (path, _) in &report.failed {
                                self.reported_failures.insert(path.clone());
                            }
                        } else {
                            report
                                .failed
                                .retain(|(path, _)| self.reported_failures.insert(path.clone()));
                        }
                    }
                    self.imports_running = self.imports_running.saturating_sub(1);
                    if self.imports_running == 0 {
                        self.import_progress = None;
                    }
                    self.reload_requested = true;
                    if let Some(path) = self.play_after_import.take() {
                        self.reload();
                        match self.find_movie_for_file(&path) {
                            Some(id) => self.actions.push(Action::Play(id)),
                            None => self.toasts.add(
                                ToastKind::Error,
                                format!("Could not open {}", path.display()),
                            ),
                        }
                    }
                    match report {
                        Ok(report) if report.is_empty() => {
                            if interactive {
                                self.toasts.add(
                                    ToastKind::Info,
                                    "No video files found in what was dropped.",
                                );
                            }
                        }
                        Ok(report)
                            if report.needs_attention()
                                && (interactive
                                    || !report.relinked.is_empty()
                                    || !report.failed.is_empty()) =>
                        {
                            self.reports.push_back(report);
                        }
                        Ok(report) if !report.added.is_empty() => {
                            self.toasts.add(ToastKind::Success, report.summary());
                        }
                        Ok(_) => {}
                        Err(e) => self
                            .toasts
                            .add(ToastKind::Error, format!("Import failed: {e}")),
                    }
                }
                AppEvent::WatchedFiles(files) => self.start_import(files, false),
                AppEvent::TmdbResults { search_id, result } => {
                    self.dialogs.tmdb_results(search_id, result)
                }
                AppEvent::ShowMetadataFetched { result } => {
                    self.reload_requested = true;
                    match result {
                        Ok(title) => self.toasts.add(
                            ToastKind::Success,
                            format!("Updated \"{title}\" from TMDb."),
                        ),
                        Err(e) => self.toasts.add(ToastKind::Error, e),
                    }
                }
                AppEvent::TmdbApplied { movie_id, result } => {
                    self.reload_requested = true;
                    match result {
                        Ok(()) => {
                            self.dialogs.tmdb = None;
                            let title = self
                                .db
                                .movie(movie_id)
                                .ok()
                                .flatten()
                                .map(|m| m.title)
                                .unwrap_or_default();
                            self.toasts.add(
                                ToastKind::Success,
                                format!("Updated \"{title}\" from TMDb."),
                            );
                        }
                        Err(e) => {
                            self.dialogs.tmdb_failed();
                            self.toasts.add(ToastKind::Error, e);
                        }
                    }
                }
                AppEvent::Toast(kind, text) => self.toasts.add(kind, text),
                AppEvent::EncodePreview(preview) => {
                    let current = match &self.encode_preview {
                        Some(PreviewDialog::Loading { movie_id, .. }) => Some(*movie_id),
                        _ => None,
                    };
                    if current == Some(preview.movie_id) {
                        self.encode_preview = Some(PreviewDialog::Ready(preview));
                    }
                }
                AppEvent::Encode(event) => self.on_encode_event(event),
            }
        }
    }

    /// Forgets shows whose last episode was removed.
    fn drop_empty_shows(&mut self) {
        match self.db.remove_empty_shows() {
            Ok(posters) => {
                for poster in posters {
                    metadata::delete_cached_poster(&self.paths.posters, Some(&poster));
                }
            }
            Err(e) => tracing::warn!("could not tidy up empty shows: {e:#}"),
        }
    }

    fn db_result(&mut self, result: anyhow::Result<()>) {
        match result {
            Ok(()) => self.reload_requested = true,
            Err(e) => self.toasts.add(ToastKind::Error, format!("{e:#}")),
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Play(id) => {
                if self.close_after.is_some_and(|c| c != id) {
                    self.close_after = None;
                }
                self.play(id)
            }
            Action::StartPlayback(id, start) => self.start_playback(id, start),
            Action::StopPlayback { finished } => self.stop_playback(finished, ctx),
            Action::PlayNextEpisode => self.play_next_episode(),
            Action::PlaybackFailed(message) => {
                self.toasts.add(ToastKind::Error, message);
                // Stay open to show why, even if Dropreel was opened just for this video.
                if let Some(np) = self.now_playing.as_mut() {
                    np.close_when_done = false;
                }
                self.stop_playback(false, ctx);
            }
            Action::PlayerSettingsChanged => {
                self.apply_video_quality();
                self.settings_unsaved = true;
            }
            Action::ShowDetails(id) => self.selected = Some(Detail::Movie(id)),
            Action::ShowShow(id) => self.selected = Some(Detail::Show(id)),
            Action::PlayShow(id) => {
                let next = self
                    .lib
                    .show(id)
                    .and_then(|show| self.lib.next_episode(show))
                    .map(|(m, resume)| (m.id, resume));
                if let Some((episode, resume)) = next {
                    match resume {
                        Some(pos) => self.start_playback(episode, Some(pos)),
                        None => self.play(episode),
                    }
                }
            }
            Action::SetShowWatched(id, watched) => {
                let episodes: Vec<MovieId> = self
                    .lib
                    .show(id)
                    .map(|s| self.lib.episodes(s).map(|m| m.id).collect())
                    .unwrap_or_default();
                let result = episodes
                    .into_iter()
                    .try_for_each(|e| self.db.set_watched(e, watched));
                self.db_result(result);
            }
            Action::AskRemoveShow(id) => self.dialogs.confirm_remove_show = Some(id),
            Action::FetchShowMetadata(id) => {
                self.toasts
                    .add(ToastKind::Info, "Looking the show up on TMDb…".to_string());
                jobs::show_metadata(self.env(), self.events.clone(), id);
            }
            Action::RemoveShow(id) => {
                let episodes: Vec<(MovieId, Option<PathBuf>)> = self
                    .lib
                    .show(id)
                    .map(|s| {
                        self.lib
                            .episodes(s)
                            .map(|m| (m.id, m.poster.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut result = Ok(());
                for (episode, poster) in episodes {
                    result = self.db.remove_movie(episode);
                    if result.is_err() {
                        break;
                    }
                    metadata::delete_cached_poster(&self.paths.posters, poster.as_deref());
                }
                if result.is_ok() && self.selected == Some(Detail::Show(id)) {
                    self.selected = None;
                }
                self.db_result(result);
                self.drop_empty_shows();
            }
            Action::CloseDetails => self.selected = None,
            Action::OpenFolder(id) => {
                if let Some(m) = self.lib.get(id) {
                    let result = if m.path.exists() {
                        opener::reveal(&m.path)
                    } else {
                        m.path
                            .parent()
                            .filter(|p| p.exists())
                            .map_or(Ok(()), opener::open)
                    };
                    if let Err(e) = result {
                        self.toasts
                            .add(ToastKind::Error, format!("Could not open the folder: {e}"));
                    }
                }
            }
            Action::Locate(id) => self.locate(id),
            Action::AskRemove(id) => self.dialogs.confirm_remove = Some(id),
            Action::Remove(id) => {
                let poster = self.lib.get(id).and_then(|m| m.poster.clone());
                let result = self.db.remove_movie(id);
                if result.is_ok() {
                    metadata::delete_cached_poster(&self.paths.posters, poster.as_deref());
                    if self.selected == Some(Detail::Movie(id)) {
                        self.selected = None;
                    }
                }
                self.db_result(result);
                self.drop_empty_shows();
            }
            Action::SetTag(movie, tag, on) => {
                let r = self.db.set_movie_tag(movie, tag, on);
                self.db_result(r);
            }
            Action::CreateTag(movie, name) => {
                let r = self
                    .db
                    .create_tag(&name)
                    .and_then(|tag| self.db.set_movie_tag(movie, tag, true));
                self.db_result(r);
            }
            Action::AskRenameTag(tag) => {
                if let Some(t) = self.lib.tags.iter().find(|t| t.id == tag) {
                    self.dialogs.open_rename_tag(t);
                }
            }
            Action::RenameTag(tag, name) => {
                let r = self.db.rename_tag(tag, &name).map(|kept| {
                    // If it merged into another tag, the shelf follows it.
                    if self.filter.tag == Some(tag) {
                        self.filter.tag = Some(kept);
                        self.filter_dirty = true;
                    }
                });
                self.db_result(r);
            }
            Action::AskDeleteTag(tag) => self.dialogs.confirm_delete_tag = Some(tag),
            Action::DeleteTag(tag) => {
                let r = self.db.delete_tag(tag);
                if r.is_ok() && self.filter.tag == Some(tag) {
                    self.filter.tag = None;
                    self.filter_dirty = true;
                }
                self.db_result(r);
            }
            Action::SetWatched(id, on) => {
                let r = self.db.set_watched(id, on);
                self.db_result(r);
            }
            Action::EditTitle(id) => {
                if let Some(m) = self.lib.get(id) {
                    self.dialogs.open_edit(m);
                }
            }
            Action::SaveTitle(id, title, year) => {
                let r = self.db.set_title_year(id, title.trim(), year);
                self.db_result(r);
            }
            Action::OpenTmdbSearch(id) => {
                if let Some(m) = self.lib.get(id) {
                    self.dialogs.open_tmdb(m);
                    self.apply(Action::TmdbSearch, ctx);
                }
            }
            Action::TmdbSearch => {
                if let Some((search_id, query, year)) = self.dialogs.begin_tmdb_search() {
                    jobs::tmdb_search(
                        self.settings.tmdb_api_key.clone(),
                        self.events.clone(),
                        search_id,
                        query,
                        year,
                    );
                }
            }
            Action::TmdbPick(id, choice) => {
                jobs::tmdb_apply(self.env(), self.events.clone(), id, choice)
            }
            Action::AddFiles => {
                if let Some(files) = watchdog::modal(|| {
                    rfd::FileDialog::new()
                        .set_title("Add movies")
                        .add_filter("Video files", scan::VIDEO_EXTENSIONS)
                        .pick_files()
                }) {
                    self.start_import(files, true);
                }
            }
            Action::AddFolder => {
                if let Some(folder) = watchdog::modal(|| {
                    rfd::FileDialog::new()
                        .set_title("Add a folder of movies")
                        .pick_folder()
                }) {
                    self.start_import(vec![folder], true);
                }
            }
            Action::Import(paths) => self.start_import(paths, true),
            Action::OpenSettings => self.dialogs.settings = Some(Box::new(self.settings.clone())),
            Action::SaveSettings(new) => self.save_settings(*new, ctx),
            Action::AskReencode(id) => self.ask_reencode(id),
            Action::StartEncode(preview) => {
                self.encode_preview = None;
                let job = EncodeJob {
                    movie_id: preview.movie_id,
                    title: preview.title.clone(),
                    source: preview.source.clone(),
                };
                if self.encode_queue.enqueue(job) {
                    self.toasts.add(
                        ToastKind::Info,
                        format!("Re-encoding \"{}\" in the background.", preview.title),
                    );
                }
            }
            Action::CancelEncode(id) => self.encode_queue.cancel(id),
            Action::ResolveOriginal(choice) => self.resolve_original(choice),
            Action::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Action::RescanWatched => {
                if self.settings.watched_folders.is_empty() {
                    self.toasts
                        .add(ToastKind::Info, "Add a watched folder in Settings first.");
                } else {
                    self.start_import(self.settings.watched_folders.clone(), false);
                    self.toasts
                        .add(ToastKind::Info, "Scanning watched folders…");
                }
            }
        }
    }

    fn play(&mut self, id: MovieId) {
        let Some(movie) = self.lib.get(id) else {
            return;
        };
        if self.lib.missing.contains(&id) {
            self.selected = Some(Detail::Movie(id));
            self.toasts.add(
                ToastKind::Error,
                format!(
                    "\"{}\" is not where it used to be. Use Locate… to find it.",
                    movie.title
                ),
            );
            return;
        }
        if let Err(reason) = &self.player {
            // Without libmpv, hand the file to the system's default player.
            let message = format!("{reason} Opening in your default player instead.");
            match opener::open(&movie.path) {
                Ok(()) => {
                    self.toasts.add(ToastKind::Info, message);
                    let r = self.db.mark_played(id);
                    self.db_result(r);
                }
                Err(e) => self
                    .toasts
                    .add(ToastKind::Error, format!("Could not open the movie: {e}")),
            }
            return;
        }
        match player_view::should_offer_resume(movie.resume_pos, movie.duration) {
            Some(pos) => self.resume_prompt = Some((id, pos)),
            None => self.start_playback(id, None),
        }
    }

    fn start_playback(&mut self, id: MovieId, start: Option<f64>) {
        let Some(movie) = self.lib.get(id).cloned() else {
            return;
        };
        let Ok(player) = self.player.as_mut() else {
            return;
        };
        tracing::info!("playing \"{}\" from {start:?}", movie.title);
        match player.load(&movie.path, start) {
            Ok(()) => {
                let mut np =
                    player_view::NowPlaying::new(Some(id), movie.title.clone(), movie.duration);
                np.close_when_done = self.close_after.take() == Some(id);
                if let Some(show) = movie.show_id.and_then(|s| self.lib.show(s)) {
                    let code = movie.episode_code().unwrap_or_default();
                    np.subtitle = Some(format!("{code} · {}", movie.title));
                    np.title = show.show.title.clone();
                    np.next = self
                        .lib
                        .episode_after(&movie)
                        .filter(|next| !self.lib.missing.contains(&next.id))
                        .map(|next| {
                            let code = next.episode_code().unwrap_or_default();
                            (next.id, format!("{code} · {}", next.title))
                        });
                }
                self.now_playing = Some(np);
                self.screen_fade = Some(Instant::now());
                let r = self.db.mark_played(id);
                self.db_result(r);
            }
            Err(e) => self.toasts.add(
                ToastKind::Error,
                format!("Could not play \"{}\": {e}", movie.title),
            ),
        }
    }

    /// Saves where playback stopped. Finished movies are marked watched and start over next time.
    fn save_playback_position(&mut self, finished: bool) {
        let (Some(np), Ok(player)) = (self.now_playing.as_ref(), self.player.as_ref()) else {
            return;
        };
        let Some(id) = np.movie_id else {
            return; // Not in the library: nothing to remember.
        };
        let dur = player.duration().or(np.known_duration);
        let resume = if finished {
            None
        } else {
            player_view::resume_position(player.position(), dur)
        };
        let watched = finished
            || resume.is_none()
                && player
                    .position()
                    .zip(dur)
                    .is_some_and(|(p, d)| p >= d * player_view::WATCHED_FRACTION);
        let mut result = self.db.set_resume(id, resume);
        if watched && result.is_ok() {
            result = self.db.set_watched(id, true);
        }
        self.db_result(result);
    }

    /// Ends this episode and starts the next, staying in the player (and in fullscreen).
    /// This episode is saved as if stopped where it is: a resume point, or watched past 95%.
    fn play_next_episode(&mut self) {
        let Some((next, _)) = self.now_playing.as_ref().and_then(|np| np.next.clone()) else {
            return;
        };
        if self
            .now_playing
            .as_ref()
            .is_some_and(|np| np.close_when_done)
        {
            self.close_after = Some(next);
        }
        // The next file replaces this one in mpv directly: no stop in between, which would tear
        // down and rebuild the decoder and video output.
        self.end_playback(false, false);
        let resume = self
            .lib
            .get(next)
            .and_then(|m| m.resume_pos.filter(|_| !m.watched));
        self.start_playback(next, resume);
    }

    fn stop_playback(&mut self, finished: bool, ctx: &egui::Context) {
        let Some(np) = &self.now_playing else {
            return;
        };
        if np.close_when_done {
            // Dropreel was opened just to play this.
            self.end_playback(finished, true);
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if finished && let Some(np) = &self.now_playing {
            let name = match &np.subtitle {
                Some(episode) => format!("{} {episode}", np.title),
                None => np.title.clone(),
            };
            self.toasts
                .add(ToastKind::Success, format!("Finished \"{name}\"."));
        }
        self.end_playback(finished, true);
        self.screen_fade = Some(Instant::now());
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
    }

    /// Saves where playback got to and, with `unload`, stops mpv.
    fn end_playback(&mut self, finished: bool, unload: bool) {
        if let (Some(np), Ok(player)) = (&self.now_playing, &mut self.player) {
            tracing::info!(
                "stopped \"{}\" at {:?} (finished: {finished})",
                np.title,
                player.position()
            );
            player.log_diagnostics();
        }
        self.save_playback_position(finished);
        if unload && let Ok(player) = self.player.as_mut() {
            player.stop();
        }
        self.now_playing = None;
    }

    fn locate(&mut self, id: MovieId) {
        let Some(movie) = self.lib.get(id) else {
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Locate \"{}\"", movie.title))
            .add_filter("Video files", scan::VIDEO_EXTENSIONS);
        if let Some(dir) = movie.path.parent().filter(|d| d.exists()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(file) = watchdog::modal(|| dialog.pick_file()) else {
            return;
        };
        match import::relink_manually(&self.env(), id, &file) {
            Ok(()) => {
                self.toasts.add(
                    ToastKind::Success,
                    "Found it — the movie is linked to the new location.",
                );
                self.reload();
            }
            Err(e) => self.toasts.add(ToastKind::Error, e),
        }
    }

    /// Saves settings changed in the player's menu (already applied).
    fn save_player_settings(&mut self) {
        self.settings_unsaved = false;
        if let Err(e) = self.settings.save(&self.paths.settings) {
            self.toasts
                .add(ToastKind::Error, format!("Could not save settings: {e:#}"));
        }
    }

    fn save_settings(&mut self, new: Settings, ctx: &egui::Context) {
        if let Err(e) = new.save(&self.paths.settings) {
            self.toasts
                .add(ToastKind::Error, format!("Could not save settings: {e:#}"));
            return;
        }
        let old = std::mem::replace(&mut self.settings, new);
        self.dialogs.settings = None;
        self.apply_theme(ctx);
        self.apply_video_quality();
        self.filter_dirty = true;
        let tools = Tools::locate(&self.settings);
        if tools != self.tools {
            self.tools = tools;
            self.warn_about_missing_tools();
        }
        self.encode_queue.set_config(EncodeConfig::new(
            self.paths.db.clone(),
            self.tools.clone(),
            &self.settings,
        ));
        if old.watched_folders != self.settings.watched_folders {
            self.restart_watcher();
            let added: Vec<PathBuf> = self
                .settings
                .watched_folders
                .iter()
                .filter(|f| !old.watched_folders.contains(f))
                .cloned()
                .collect();
            if !added.is_empty() {
                self.start_import(added, true);
            }
        }
        self.toasts.add(ToastKind::Success, "Settings saved.");
    }

    /// The library entry for a file: by path, or by fingerprint if it's a copy of a known movie.
    fn find_movie_for_file(&self, path: &std::path::Path) -> Option<MovieId> {
        if let Ok(Some(id)) = self.db.find_by_path(path) {
            return Some(id);
        }
        let fp = crate::library::fingerprint::fingerprint(path).ok()?;
        let matches = self.db.find_by_fingerprint(&fp).ok()?;
        matches
            .iter()
            .find(|(_, p, _)| p.exists())
            .or(matches.first())
            .map(|(id, _, _)| *id)
    }

    fn ask_reencode(&mut self, id: MovieId) {
        let Some(movie) = self.lib.get(id).cloned() else {
            return;
        };
        if movie.size <= self.settings.threshold_bytes() || self.lib.missing.contains(&id) {
            return;
        }
        self.encode_preview = Some(PreviewDialog::Loading {
            movie_id: id,
            title: movie.title.clone(),
        });
        let config = EncodeConfig::new(self.paths.db.clone(), self.tools.clone(), &self.settings);
        let detector = self.detector.clone();
        let events = self.events.clone();
        let spawned = std::thread::Builder::new()
            .name("encode-preview".into())
            .spawn(move || {
                let preview = crate::encode::queue::preview(
                    &config,
                    &detector,
                    id,
                    &movie.title,
                    &movie.path,
                );
                events.send(AppEvent::EncodePreview(Box::new(preview)));
            });
        if let Err(e) = spawned {
            self.encode_preview = None;
            self.toasts
                .add(ToastKind::Error, format!("Could not start: {e}"));
        }
    }

    fn on_encode_event(&mut self, event: EncodeEvent) {
        self.reload_requested = true;
        match event {
            EncodeEvent::Finished(outcome) => {
                self.toasts.add(
                    ToastKind::Success,
                    format!(
                        "\"{}\" re-encoded to {}.",
                        outcome.title,
                        crate::util::format_bytes(outcome.output_size)
                    ),
                );
                self.finished_encodes.push_back(outcome);
            }
            EncodeEvent::Failed { title, error, .. } => {
                self.toasts.add(
                    ToastKind::Error,
                    format!("Re-encoding \"{title}\" failed: {error}"),
                );
            }
            EncodeEvent::Cancelled { title, .. } => {
                self.toasts.add(
                    ToastKind::Info,
                    format!("Cancelled re-encoding \"{title}\"."),
                );
            }
        }
    }

    /// Keep, recycle or delete the original after a successful encode.
    fn resolve_original(&mut self, choice: OriginalChoice) {
        let Some(outcome) = self.finished_encodes.pop_front() else {
            return;
        };
        let original = &outcome.original;
        let result = match choice {
            OriginalChoice::Keep => return,
            OriginalChoice::Trash => trash::delete(original).map_err(|e| e.to_string()),
            OriginalChoice::Delete => std::fs::remove_file(original).map_err(|e| e.to_string()),
        };
        match result {
            Ok(()) => {
                // An entry still pointing at the original (added back by an older version, or by
                // hand) would only show up as missing now.
                if let Ok(Some(stale)) = self.db.find_by_path(original)
                    && stale != outcome.movie_id
                {
                    self.actions.push(Action::Remove(stale));
                }
                let verb = if choice == OriginalChoice::Trash {
                    "Moved the original to the Recycle Bin"
                } else {
                    "Deleted the original"
                };
                self.toasts.add(
                    ToastKind::Success,
                    format!(
                        "{verb} — freed {}.",
                        crate::util::format_bytes(outcome.original_size)
                    ),
                );
            }
            Err(e) => self.toasts.add(
                ToastKind::Error,
                format!("Could not remove {}: {e}", original.display()),
            ),
        }
    }

    /// Copies the queue state for this frame's drawing.
    fn refresh_encode_state(&mut self) {
        let (running, queued) = self.encode_queue.snapshot();
        self.encode_badges.clear();
        if let Some(r) = &running {
            self.encode_badges
                .insert(r.movie_id, EncodeBadge::Running(r.clone()));
        }
        for (i, (id, _)) in queued.iter().enumerate() {
            self.encode_badges
                .insert(*id, EncodeBadge::Queued(i + usize::from(running.is_some())));
        }
        self.encode_running = running;
        self.encode_queued = queued;
    }

    /// Fades the screen in from black after switching between the library and the player.
    fn paint_screen_fade(&mut self, ctx: &egui::Context) {
        const FADE: f32 = 0.35;
        let Some(start) = self.screen_fade else {
            return;
        };
        let t = start.elapsed().as_secs_f32() / FADE;
        if t >= 1.0 {
            self.screen_fade = None;
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("screen_fade"),
        ));
        // Ease out: quick at first, settling gently.
        let alpha = (1.0 - t).powi(2);
        painter.rect_filled(
            ctx.content_rect(),
            0.0,
            theme::ramp::N950.gamma_multiply(alpha),
        );
        ctx.request_repaint();
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.actions.push(Action::Import(dropped));
        }
    }
}

impl eframe::App for DropReelApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.watchdog.frame_started();
        let ctx = ui.ctx().clone();
        self.process_events();
        self.maybe_reload(&ctx);
        self.refresh_encode_state();
        if ctx.input(|i| i.viewport().close_requested())
            && self.encode_queue.is_busy()
            && !self.allow_close
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_prompt = true;
        }
        if self.filter_dirty {
            self.visible = state::visible_entries(&self.lib, &self.filter);
            self.filter_dirty = false;
        }
        self.handle_dropped_files(&ctx);

        if self.now_playing.is_some() {
            player_view::show(self, ui, frame);
        } else {
            // Nothing is playing, but mpv may still be sending replies and log messages about
            // the file that was.
            if let Ok(player) = self.player.as_mut() {
                player.poll_events();
            }
            library_view::sidebar(self, ui);
            if self.selected.is_some() {
                self.detail_shown = self.selected;
            }
            detail::panel(self, ui);
            library_view::toolbar(self, ui);
            library_view::central(self, ui);
        }
        dialogs::show(self, &ctx);
        let toast_bottom = if self.now_playing.is_some() {
            player_view::BAR_HEIGHT + 16.0
        } else {
            20.0
        };
        self.toasts.show(&ctx, toast_bottom);
        library_view::drop_overlay(&ctx);
        self.paint_screen_fade(&ctx);

        for action in std::mem::take(&mut self.actions) {
            self.apply(action, &ctx);
        }
        if self.imports_running > 0 {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.settings_unsaved
            && !(self.now_playing.is_some()
                && egui::Popup::is_id_open(&ctx, player_view::settings_menu_id()))
        {
            self.save_player_settings();
        }
        self.watchdog.frame_ended();
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.save_playback_position(false);
        if self.settings_unsaved {
            self.save_player_settings();
        }
        self.encode_queue.shutdown();
        if let Ok(player) = self.player.as_mut() {
            player.destroy(gl);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.prefs.sort = self.filter.sort;
        self.prefs.descending = self.filter.descending;
        eframe::set_value(storage, PREFS_KEY, &self.prefs);
    }
}
