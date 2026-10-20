//! The DropReel window.

mod detail;
mod dialogs;
pub mod events;
mod jobs;
mod library_view;
pub mod state;
mod toasts;
mod widgets;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui;

use crate::library::import::{self, ImportProgress, ImportReport, LibraryEnv};
use crate::library::watch::{FolderWatcher, SETTLE_TIME};
use crate::library::{Db, MovieId, TagId, scan};
use crate::metadata::{self, tmdb::TmdbMovie};
use crate::paths::AppPaths;
use crate::settings::{Settings, ThemeChoice};
use crate::tools::Tools;
use dialogs::Dialogs;
use events::{AppEvent, EventSink, ToastKind};
use state::{Filter, Library, Prefs};
use toasts::Toasts;

const PREFS_KEY: &str = "dropreel-prefs";

/// Things the UI asked for during a frame, applied after drawing.
pub enum Action {
    Play(MovieId),
    ShowDetails(MovieId),
    CloseDetails,
    OpenFolder(MovieId),
    Locate(MovieId),
    AskRemove(MovieId),
    Remove(MovieId),
    SetTag(MovieId, TagId, bool),
    CreateTag(MovieId, String),
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
    visible: Vec<usize>,
    filter_dirty: bool,
    selected: Option<MovieId>,

    imports_running: usize,
    import_progress: Option<ImportProgress>,
    reports: VecDeque<ImportReport>,
    toasts: Toasts,
    dialogs: Dialogs,
    watcher: Option<FolderWatcher>,

    reload_requested: bool,
    last_reload: Instant,
    actions: Vec<Action>,
}

impl DropReelApp {
    /// `open_paths` are files passed on the command line ("Open with DropReel").
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: AppPaths,
        open_paths: Vec<PathBuf>,
    ) -> anyhow::Result<Self> {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let (settings, settings_warning) = Settings::load(&paths.settings);
        let tools = Tools::locate(&settings);
        let db = Db::open(&paths.db)?;
        let lib = Library::load(&db)?;
        let prefs: Prefs = cc
            .storage
            .and_then(|s| eframe::get_value(s, PREFS_KEY))
            .unwrap_or_default();
        let (events, rx) = EventSink::new(cc.egui_ctx.clone());

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
            imports_running: 0,
            import_progress: None,
            reports: VecDeque::new(),
            toasts: Toasts::default(),
            dialogs: Dialogs::default(),
            watcher: None,
            reload_requested: false,
            last_reload: Instant::now(),
            actions: Vec::new(),
        };
        app.apply_theme(&cc.egui_ctx);
        if let Some(w) = settings_warning {
            app.toasts.add(ToastKind::Error, w);
        }
        app.warn_about_missing_tools();
        app.restart_watcher();
        // Pick up files added to watched folders while DropReel was closed, and find moved files.
        if !app.settings.watched_folders.is_empty() {
            app.start_import(app.settings.watched_folders.clone(), false);
        }
        if !open_paths.is_empty() {
            app.start_import(open_paths, true);
        }
        Ok(app)
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
                    "{} not found — thumbnails and technical details are unavailable. Put the tools next to DropReel.exe or set their paths in Settings.",
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
                if self.selected.is_some_and(|id| self.lib.get(id).is_none()) {
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
                AppEvent::ImportProgress(p) => self.import_progress = Some(p),
                AppEvent::ImportFinished {
                    report,
                    interactive,
                } => {
                    self.imports_running = self.imports_running.saturating_sub(1);
                    if self.imports_running == 0 {
                        self.import_progress = None;
                    }
                    self.reload_requested = true;
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
            }
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
            Action::Play(id) => self.play(id),
            Action::ShowDetails(id) => self.selected = Some(id),
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
                    if self.selected == Some(id) {
                        self.selected = None;
                    }
                }
                self.db_result(result);
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
            Action::DeleteTag(tag) => {
                let r = self.db.delete_tag(tag);
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
                if let Some(files) = rfd::FileDialog::new()
                    .set_title("Add movies")
                    .add_filter("Video files", scan::VIDEO_EXTENSIONS)
                    .pick_files()
                {
                    self.start_import(files, true);
                }
            }
            Action::AddFolder => {
                if let Some(folder) = rfd::FileDialog::new()
                    .set_title("Add a folder of movies")
                    .pick_folder()
                {
                    self.start_import(vec![folder], true);
                }
            }
            Action::Import(paths) => self.start_import(paths, true),
            Action::OpenSettings => self.dialogs.settings = Some(Box::new(self.settings.clone())),
            Action::SaveSettings(new) => self.save_settings(*new, ctx),
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
            self.selected = Some(id);
            self.toasts.add(
                ToastKind::Error,
                format!(
                    "\"{}\" is not where it used to be. Use Locate… to find it.",
                    movie.title
                ),
            );
            return;
        }
        // Until the built-in player lands, hand the file to the system's default player.
        match opener::open(&movie.path) {
            Ok(()) => {
                let r = self.db.mark_played(id);
                self.db_result(r);
            }
            Err(e) => self
                .toasts
                .add(ToastKind::Error, format!("Could not open the movie: {e}")),
        }
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
        let Some(file) = dialog.pick_file() else {
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

    fn save_settings(&mut self, new: Settings, ctx: &egui::Context) {
        if let Err(e) = new.save(&self.paths.settings) {
            self.toasts
                .add(ToastKind::Error, format!("Could not save settings: {e:#}"));
            return;
        }
        let old = std::mem::replace(&mut self.settings, new);
        self.dialogs.settings = None;
        self.apply_theme(ctx);
        let tools = Tools::locate(&self.settings);
        if tools != self.tools {
            self.tools = tools;
            self.warn_about_missing_tools();
        }
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_events();
        self.maybe_reload(&ctx);
        if self.filter_dirty {
            self.visible = state::visible_movies(&self.lib, &self.filter);
            self.filter_dirty = false;
        }
        self.handle_dropped_files(&ctx);

        library_view::top_bar(self, ui);
        library_view::status_bar(self, ui);
        if self.selected.is_some() {
            detail::panel(self, ui);
        }
        library_view::central(self, ui);
        dialogs::show(self, &ctx);
        self.toasts.show(&ctx);
        library_view::drop_overlay(&ctx);

        for action in std::mem::take(&mut self.actions) {
            self.apply(action, &ctx);
        }
        if self.imports_running > 0 {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.prefs.sort = self.filter.sort;
        self.prefs.descending = self.filter.descending;
        eframe::set_value(storage, PREFS_KEY, &self.prefs);
    }
}
