//! Windows and confirmations: settings, import report, title editing, TMDb search, removal.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use super::events::ToastKind;
use super::widgets::{DANGER, WARN};
use super::{Action, DropReelApp};
use crate::library::import::ImportReport;
use crate::library::{Movie, MovieId};
use crate::metadata::tmdb::TmdbMovie;
use crate::settings::{EncoderChoice, Settings, ThemeChoice, X265_PRESETS};
use crate::tools::Tools;

pub struct EditDialog {
    movie_id: MovieId,
    title: String,
    year: String,
    focused: bool,
}

pub struct TmdbDialog {
    movie_id: MovieId,
    movie_title: String,
    query: String,
    year: String,
    search_id: u64,
    busy: bool,
    results: Option<Result<Vec<TmdbMovie>, String>>,
}

#[derive(Default)]
pub struct Dialogs {
    pub settings: Option<Box<Settings>>,
    pub edit: Option<EditDialog>,
    pub tmdb: Option<TmdbDialog>,
    pub confirm_remove: Option<MovieId>,
    /// Text of the "New tag" field in the detail panel.
    pub new_tag: String,
    next_search_id: u64,
}

fn parse_year(s: &str) -> Option<i32> {
    s.trim().parse().ok().filter(|y| (1870..=2100).contains(y))
}

impl Dialogs {
    pub fn open_edit(&mut self, movie: &Movie) {
        self.edit = Some(EditDialog {
            movie_id: movie.id,
            title: movie.title.clone(),
            year: movie.year.map(|y| y.to_string()).unwrap_or_default(),
            focused: false,
        });
    }

    pub fn open_tmdb(&mut self, movie: &Movie) {
        self.tmdb = Some(TmdbDialog {
            movie_id: movie.id,
            movie_title: movie.title.clone(),
            query: movie.title.clone(),
            year: movie.year.map(|y| y.to_string()).unwrap_or_default(),
            search_id: 0,
            busy: false,
            results: None,
        });
    }

    /// Marks the TMDb dialog as searching and returns what to search for.
    pub fn begin_tmdb_search(&mut self) -> Option<(u64, String, Option<i32>)> {
        let d = self.tmdb.as_mut()?;
        if d.query.trim().is_empty() {
            return None;
        }
        self.next_search_id += 1;
        d.search_id = self.next_search_id;
        d.busy = true;
        Some((d.search_id, d.query.trim().to_string(), parse_year(&d.year)))
    }

    pub fn tmdb_results(&mut self, search_id: u64, result: Result<Vec<TmdbMovie>, String>) {
        if let Some(d) = self.tmdb.as_mut().filter(|d| d.search_id == search_id) {
            d.busy = false;
            d.results = Some(result);
        }
    }

    pub fn tmdb_failed(&mut self) {
        if let Some(d) = self.tmdb.as_mut() {
            d.busy = false;
        }
    }
}

pub fn show(app: &mut DropReelApp, ctx: &egui::Context) {
    settings_window(app, ctx);
    import_report_window(app, ctx);
    edit_window(app, ctx);
    tmdb_window(app, ctx);
    remove_modal(app, ctx);
    resume_modal(app, ctx);
}

fn resume_modal(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some((id, pos)) = app.resume_prompt else {
        return;
    };
    let title = app.lib.get(id).map(|m| m.title.clone()).unwrap_or_default();
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("resume_prompt")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading(title);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .button(
                    RichText::new(format!(
                        "▶ Resume from {}",
                        crate::util::format_duration(pos)
                    ))
                    .strong(),
                )
                .clicked()
            {
                app.actions.push(Action::StartPlayback(id, Some(pos)));
                close = true;
            }
            if ui.button("Start over").clicked() {
                app.actions.push(Action::StartPlayback(id, None));
                close = true;
            }
            close |= ui.button("Cancel").clicked();
        });
    });
    if close || modal.should_close() {
        app.resume_prompt = None;
    }
}

fn path_field(ui: &mut egui::Ui, id: &str, value: &mut Option<PathBuf>, found: Option<&PathBuf>) {
    ui.horizontal(|ui| {
        let mut text = value
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let edit = egui::TextEdit::singleline(&mut text)
            .id_salt(id)
            .hint_text("Auto-detect")
            .desired_width(300.0);
        if ui.add(edit).changed() {
            *value = (!text.is_empty()).then(|| PathBuf::from(&text));
        }
        if ui.button("Browse…").clicked()
            && let Some(file) = rfd::FileDialog::new().pick_file()
        {
            *value = Some(file);
        }
        if value.is_some() && ui.small_button("✖").on_hover_text("Auto-detect").clicked() {
            *value = None;
        }
    });
    match found {
        Some(path) => ui.weak(format!("✔ Using {}", path.display())),
        None => ui.colored_label(DANGER, "✖ Not found"),
    };
}

fn settings_window(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(draft) = app.dialogs.settings.as_mut() else {
        return;
    };
    let mut open = true;
    let mut save = false;
    let mut cancel = false;
    egui::Window::new("Settings")
        .open(&mut open)
        .collapsible(false)
        .default_width(560.0)
        .max_height(ctx.content_rect().height() - 80.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Watched folders");
                ui.weak("New movies in these folders are added automatically, and moved files are found again.");
                let mut remove = None;
                for (i, folder) in draft.watched_folders.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("🗑").on_hover_text("Stop watching").clicked() {
                            remove = Some(i);
                        }
                        ui.label(folder.display().to_string());
                    });
                }
                if let Some(i) = remove {
                    draft.watched_folders.remove(i);
                }
                ui.horizontal(|ui| {
                    if ui.button("+ Add folder…").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                        && !draft.watched_folders.contains(&folder)
                    {
                        draft.watched_folders.push(folder);
                    }
                    if ui.button("Rescan now").on_hover_text("Scans the saved watched folders").clicked() {
                        app.actions.push(Action::RescanWatched);
                    }
                });

                ui.separator();
                ui.heading("TMDb (optional)");
                ui.horizontal(|ui| {
                    ui.label("API key");
                    ui.add(egui::TextEdit::singleline(&mut draft.tmdb_api_key).password(true).desired_width(320.0).hint_text("Leave empty to stay offline"));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.weak("With a key, new movies get posters, years and plots from");
                    ui.hyperlink_to("themoviedb.org", "https://www.themoviedb.org/settings/api");
                    ui.weak(". Without one, DropReel uses the file name and a frame from the movie.");
                });

                ui.separator();
                ui.heading("Tools");
                let found = Tools::locate(draft);
                ui.label("ffmpeg");
                path_field(ui, "ffmpeg_path", &mut draft.ffmpeg_path, found.ffmpeg.as_ref());
                ui.label("ffprobe");
                path_field(ui, "ffprobe_path", &mut draft.ffprobe_path, found.ffprobe.as_ref());

                ui.separator();
                ui.heading("Re-encoding");
                egui::Grid::new("encode_settings").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Suggest re-encoding above");
                    ui.add(egui::DragValue::new(&mut draft.threshold_gb).range(1.0..=200.0).speed(0.1).suffix(" GB"));
                    ui.end_row();
                    ui.label("Encoder");
                    egui::ComboBox::from_id_salt("encoder").selected_text(draft.encoder.label()).show_ui(ui, |ui| {
                        for choice in EncoderChoice::ALL {
                            ui.selectable_value(&mut draft.encoder, choice, choice.label());
                        }
                    });
                    ui.end_row();
                    ui.label("x265 preset");
                    egui::ComboBox::from_id_salt("x265_preset").selected_text(draft.x265_preset.clone()).show_ui(ui, |ui| {
                        for preset in X265_PRESETS {
                            ui.selectable_value(&mut draft.x265_preset, preset.to_string(), preset);
                        }
                    });
                    ui.end_row();
                });

                ui.separator();
                ui.heading("Player");
                ui.checkbox(&mut draft.high_quality_video, "High-quality video scaling")
                    .on_hover_text("mpv's sharper scalers. Turn this off if videos show only a black picture.");
                if app.software_gl {
                    ui.weak("Your PC is using software OpenGL, so simple scaling is always used.");
                }
                if let Err(e) = &app.player {
                    ui.colored_label(DANGER, format!("Built-in player unavailable: {e}"));
                }

                ui.separator();
                ui.heading("Appearance");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut draft.theme, ThemeChoice::Dark, "Dark");
                    ui.radio_value(&mut draft.theme, ThemeChoice::Light, "Light");
                });

                if !app.lib.tags.is_empty() {
                    ui.separator();
                    ui.heading("Tags");
                    ui.horizontal_wrapped(|ui| {
                        for tag in &app.lib.tags {
                            if ui.button(format!("🗑 {}", tag.name)).on_hover_text("Delete this tag").clicked() {
                                app.actions.push(Action::DeleteTag(tag.id));
                            }
                        }
                    });
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.weak(format!("Data folder: {}", app.paths.root.display()));
                    if ui.small_button("Open").clicked() {
                        let _ = opener::open(&app.paths.root);
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                save = ui.button(RichText::new("Save").strong()).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
    if save {
        let mut settings = (**draft).clone();
        settings.tmdb_api_key = settings.tmdb_api_key.trim().to_string();
        app.actions.push(Action::SaveSettings(Box::new(settings)));
    } else if cancel || !open {
        app.dialogs.settings = None;
    }
}

fn path_list(
    ui: &mut egui::Ui,
    title: &str,
    items: &[(PathBuf, String)],
    color: Option<egui::Color32>,
) {
    if items.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(format!("{title} ({})", items.len()))
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(title)
                .max_height(220.0)
                .show(ui, |ui| {
                    for (path, reason) in items {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.label(RichText::new(name).strong())
                            .on_hover_text(path.display().to_string());
                        let reason = match color {
                            Some(c) => RichText::new(reason).color(c),
                            None => RichText::new(reason).weak(),
                        };
                        ui.add(egui::Label::new(reason.small()).wrap());
                        ui.add_space(4.0);
                    }
                });
        });
}

fn import_report_window(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(report) = app.reports.front() else {
        return;
    };
    let mut close = false;
    let mut open = true;
    egui::Window::new("Import finished")
        .open(&mut open)
        .collapsible(false)
        .default_width(560.0)
        .show(ctx, |ui| {
            show_report(ui, report);
            ui.separator();
            close = ui.button("OK").clicked();
        });
    if close || !open {
        app.reports.pop_front();
    }
}

fn show_report(ui: &mut egui::Ui, report: &ImportReport) {
    ui.label(RichText::new(report.summary()).size(16.0).strong());
    for note in &report.notes {
        ui.colored_label(WARN, note);
    }
    if !report.relinked.is_empty() {
        let items: Vec<(PathBuf, String)> = report
            .relinked
            .iter()
            .map(|(title, path)| {
                (
                    path.clone(),
                    format!("is \"{title}\", which had gone missing"),
                )
            })
            .collect();
        path_list(ui, "Found moved movies", &items, None);
    }
    path_list(ui, "Skipped", &report.skipped, None);
    path_list(ui, "Failed", &report.failed, Some(DANGER));
    path_list(ui, "Added with problems", &report.warnings, Some(WARN));
}

fn edit_window(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(d) = app.dialogs.edit.as_mut() else {
        return;
    };
    let mut open = true;
    let mut done = false;
    egui::Window::new("Edit title")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("edit_title").num_columns(2).show(ui, |ui| {
                ui.label("Title");
                let title = ui.add(egui::TextEdit::singleline(&mut d.title).desired_width(280.0));
                if !d.focused {
                    title.request_focus();
                    d.focused = true;
                }
                ui.end_row();
                ui.label("Year");
                ui.add(egui::TextEdit::singleline(&mut d.year).desired_width(60.0));
                ui.end_row();
            });
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let valid = !d.title.trim().is_empty()
                    && (d.year.trim().is_empty() || parse_year(&d.year).is_some());
                if ui.add_enabled(valid, egui::Button::new("Save")).clicked() || (enter && valid) {
                    app.actions.push(Action::SaveTitle(
                        d.movie_id,
                        d.title.clone(),
                        parse_year(&d.year),
                    ));
                    done = true;
                }
                done |= ui.button("Cancel").clicked();
            });
        });
    if done || !open {
        app.dialogs.edit = None;
    }
}

fn tmdb_window(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(d) = app.dialogs.tmdb.as_mut() else {
        return;
    };
    let mut open = true;
    egui::Window::new(format!("Find \"{}\" on TMDb", d.movie_title))
        .id(egui::Id::unique("tmdb_dialog"))
        .open(&mut open)
        .collapsible(false)
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let q = ui.add(
                    egui::TextEdit::singleline(&mut d.query)
                        .desired_width(260.0)
                        .hint_text("Title"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut d.year)
                        .desired_width(50.0)
                        .hint_text("Year"),
                );
                let enter = q.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui
                    .add_enabled(!d.busy, egui::Button::new("Search"))
                    .clicked()
                    || enter
                {
                    app.actions.push(Action::TmdbSearch);
                }
                if d.busy {
                    ui.spinner();
                }
            });
            ui.separator();
            match &d.results {
                None => {}
                Some(Err(e)) => {
                    ui.colored_label(DANGER, e);
                }
                Some(Ok(results)) if results.is_empty() => {
                    ui.weak("No matches. Try a shorter title or remove the year.");
                }
                Some(Ok(results)) => {
                    egui::ScrollArea::vertical()
                        .max_height(400.0)
                        .show(ui, |ui| {
                            for m in results {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(!d.busy, egui::Button::new("Use this"))
                                        .clicked()
                                    {
                                        d.busy = true;
                                        app.actions.push(Action::TmdbPick(d.movie_id, m.clone()));
                                    }
                                    let year =
                                        m.year().map(|y| format!(" ({y})")).unwrap_or_default();
                                    ui.strong(format!("{}{year}", m.title));
                                });
                                if let Some(overview) =
                                    m.overview.as_deref().filter(|o| !o.is_empty())
                                {
                                    let short: String = overview.chars().take(220).collect();
                                    let ellipsis = if overview.chars().count() > 220 {
                                        "…"
                                    } else {
                                        ""
                                    };
                                    ui.weak(format!("{short}{ellipsis}"));
                                }
                                ui.add_space(6.0);
                            }
                        });
                }
            }
        });
    if !open {
        app.dialogs.tmdb = None;
    }
}

fn remove_modal(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(id) = app.dialogs.confirm_remove else {
        return;
    };
    let title = app.lib.get(id).map(|m| m.title.clone()).unwrap_or_default();
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("confirm_remove")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading("Remove from library?");
        ui.label(format!(
            "\"{title}\" will be removed from DropReel. The file on disk is not deleted."
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Remove").color(DANGER)).clicked() {
                app.actions.push(Action::Remove(id));
                close = true;
            }
            close |= ui.button("Cancel").clicked();
        });
    });
    if close || modal.should_close() {
        app.dialogs.confirm_remove = None;
    }
    let _ = ToastKind::Info;
}
