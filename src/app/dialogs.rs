//! Modal dialogs: settings, import report, title editing, TMDb search, removal, resume,
//! re-encode preview/result, and quitting during an encode.

use std::path::PathBuf;

use eframe::egui::{self, Align, Align2, Color32, Id, Layout, Margin, RichText, Vec2, vec2};

use super::icons;
use super::theme::{self, palette};
use super::widgets::{self, ButtonKind};
use super::{Action, DropReelApp, OriginalChoice, PreviewDialog};
use crate::encode::{EncodeOutcome, EncodePreview, Encoder};
use crate::library::import::ImportReport;
use crate::library::{Movie, MovieId};
use crate::metadata::tmdb::TmdbMovie;
use crate::settings::{EncoderChoice, Settings, ThemeChoice, X265_PRESETS};
use crate::tools::Tools;
use crate::util::{format_bytes, format_duration};

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
    /// Second step of "Delete permanently" after an encode.
    pub confirm_delete_original: bool,
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
    settings_dialog(app, ctx);
    import_report_dialog(app, ctx);
    edit_dialog(app, ctx);
    tmdb_dialog(app, ctx);
    remove_dialog(app, ctx);
    resume_dialog(app, ctx);
    encode_preview_dialog(app, ctx);
    encode_finished_dialog(app, ctx);
    quit_dialog(app, ctx);
}

/// A modal with DropReel's styling. Returns (inner result, should_close).
fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let p = palette(ctx);
    let frame = egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(20)
        .inner_margin(Margin::same(26))
        .shadow(ctx.global_style().visuals.window_shadow);
    let resp = egui::Modal::new(Id::unique(id))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 10.0;
            add(ui)
        });
    let close = resp.should_close();
    (resp.inner, close)
}

/// Icon disc + title (+ optional subtitle) at the top of a dialog.
fn header(ui: &mut egui::Ui, icon: &str, color: Color32, title: &str, subtitle: Option<&str>) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::hover());
        ui.painter().circle_filled(
            rect.center(),
            22.0,
            color.gamma_multiply(if p.dark { 0.16 } else { 0.10 }),
        );
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            icon,
            theme::icon_fill_font(23.0),
            color,
        );
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add_space(if subtitle.is_some() { 2.0 } else { 10.0 });
            ui.add(egui::Label::new(theme::bold(title).size(19.0).color(p.text)).wrap());
            if let Some(s) = subtitle {
                ui.add(egui::Label::new(RichText::new(s).color(p.text_weak)).wrap());
            }
        });
    });
    ui.add_space(4.0);
}

/// Right-aligned button row at the bottom of a dialog.
fn footer(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(6.0);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 36.0),
        Layout::right_to_left(Align::Center),
        add,
    );
}

fn path_field(ui: &mut egui::Ui, id: &str, value: &mut Option<PathBuf>, found: Option<&PathBuf>) {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        let mut text = value
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let edit = egui::TextEdit::singleline(&mut text)
            .id_salt(id)
            .hint_text("Auto-detect")
            .desired_width(ui.available_width() - 120.0)
            .margin(Margin::symmetric(10, 6));
        if ui.add(edit).changed() {
            *value = (!text.is_empty()).then(|| PathBuf::from(&text));
        }
        if widgets::button(ui, ButtonKind::Secondary, None, "Browse…").clicked()
            && let Some(file) = rfd::FileDialog::new().pick_file()
        {
            *value = Some(file);
        }
        if value.is_some() && widgets::icon_button(ui, icons::X, "Auto-detect").clicked() {
            *value = None;
        }
    });
    match found {
        Some(path) => ui.label(
            RichText::new(format!("{} {}", icons::CHECK_CIRCLE, path.display()))
                .size(12.0)
                .color(p.success),
        ),
        None => ui.label(
            RichText::new(format!("{} Not found", icons::X_CIRCLE))
                .size(12.0)
                .color(p.danger),
        ),
    };
}

fn settings_section(ui: &mut egui::Ui, icon: &str, title: &str) {
    let p = palette(ui.ctx());
    ui.add_space(10.0);
    ui.label(
        theme::semibold(format!("{icon}  {title}"))
            .size(15.0)
            .color(p.text),
    );
}

fn settings_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(draft) = app.dialogs.settings.as_mut() else {
        return;
    };
    let p = palette(ctx);
    let mut save = false;
    let mut cancel = false;
    let ((), close) = modal(ctx, "settings", 600.0, |ui| {
        header(ui, icons::GEAR, p.accent, "Settings", None);
        egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 240.0).show(ui, |ui| {
            ui.set_width(ui.available_width() - 12.0);
            settings_section(ui, icons::FOLDER_OPEN, "Watched folders");
            ui.label(RichText::new("New movies in these folders are added automatically, and moved files are found again.").color(p.text_weak));
            let mut remove = None;
            for (i, folder) in draft.watched_folders.iter().enumerate() {
                egui::Frame::new().fill(p.raised).corner_radius(8).inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icons::FOLDER_OPEN).color(p.text_weak));
                        ui.add(egui::Label::new(folder.display().to_string()).truncate());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::icon_button(ui, icons::TRASH, "Stop watching").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                });
            }
            if let Some(i) = remove {
                draft.watched_folders.remove(i);
            }
            ui.horizontal(|ui| {
                if widgets::button(ui, ButtonKind::Secondary, Some(icons::FOLDER_PLUS), "Add folder…").clicked()
                    && let Some(folder) = rfd::FileDialog::new().pick_folder()
                    && !draft.watched_folders.contains(&folder)
                {
                    draft.watched_folders.push(folder);
                }
                if widgets::button(ui, ButtonKind::Ghost, Some(icons::ARROW_CLOCKWISE), "Rescan now").on_hover_text("Scans the saved watched folders").clicked() {
                    app.actions.push(Action::RescanWatched);
                }
            });

            settings_section(ui, icons::CLOUD_ARROW_DOWN, "TMDb (optional)");
            ui.add(
                egui::TextEdit::singleline(&mut draft.tmdb_api_key)
                    .password(true)
                    .desired_width(ui.available_width())
                    .hint_text("API key — leave empty to stay offline")
                    .margin(Margin::symmetric(10, 6)),
            );
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new("With a key, new movies get posters, years and plots from").color(p.text_weak));
                ui.hyperlink_to("themoviedb.org", "https://www.themoviedb.org/settings/api");
                ui.label(RichText::new("— otherwise DropReel uses the file name and a frame from the movie.").color(p.text_weak));
            });

            settings_section(ui, icons::LIGHTNING, "Re-encoding");
            egui::Grid::new("encode_settings").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                ui.label("Suggest re-encoding above");
                ui.add(egui::DragValue::new(&mut draft.threshold_gb).range(1.0..=200.0).speed(0.1).suffix(" GB"));
                ui.end_row();
                ui.label("Encoder");
                let combo = egui::ComboBox::from_id_salt("encoder").width(300.0).selected_text(draft.encoder.label()).show_ui(ui, |ui| {
                    for choice in EncoderChoice::ALL {
                        ui.selectable_value(&mut draft.encoder, choice, choice.label());
                    }
                });
                widgets::name_combo(&combo.response, "Encoder");
                ui.end_row();
                ui.label("x265 preset");
                let combo = egui::ComboBox::from_id_salt("x265_preset").selected_text(draft.x265_preset.clone()).show_ui(ui, |ui| {
                    for preset in X265_PRESETS {
                        ui.selectable_value(&mut draft.x265_preset, preset.to_string(), preset);
                    }
                });
                widgets::name_combo(&combo.response, "x265 preset");
                ui.end_row();
            });
            let found = Tools::locate(draft);
            ui.label("ffmpeg");
            path_field(ui, "ffmpeg_path", &mut draft.ffmpeg_path, found.ffmpeg.as_ref());
            ui.label("ffprobe");
            path_field(ui, "ffprobe_path", &mut draft.ffprobe_path, found.ffprobe.as_ref());

            settings_section(ui, icons::MONITOR_PLAY, "Player");
            ui.checkbox(&mut draft.high_quality_video, "High-quality video scaling")
                .on_hover_text("mpv's sharper scalers. Turn this off if videos show only a black picture.");
            ui.checkbox(&mut draft.smooth_motion, "Smooth motion")
                .on_hover_text("Syncs video to your monitor's refresh rate and blends frames, so pans don't judder. Turn this off if playback looks worse.");
            if app.software_gl {
                ui.label(RichText::new("Your PC is using software OpenGL, so simple scaling is always used and smooth motion is off.").size(12.5).color(p.text_weak));
            }
            if let Err(e) = &app.player {
                ui.label(RichText::new(format!("{} Built-in player unavailable: {e}", icons::WARNING)).color(p.danger));
            }

            settings_section(ui, icons::SPARKLE, "Appearance");
            ui.horizontal(|ui| {
                ui.radio_value(&mut draft.theme, ThemeChoice::Dark, "Dark");
                ui.radio_value(&mut draft.theme, ThemeChoice::Light, "Light");
            });

            if !app.lib.tags.is_empty() {
                settings_section(ui, icons::TAG, "Tags");
                ui.horizontal_wrapped(|ui| {
                    for tag in &app.lib.tags {
                        if widgets::button(ui, ButtonKind::Secondary, Some(icons::TRASH), &tag.name).on_hover_text("Delete this tag").clicked() {
                            app.actions.push(Action::DeleteTag(tag.id));
                        }
                    }
                });
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} {}", icons::DATABASE, app.paths.root.display())).size(12.0).color(p.text_weak));
                if ui.small_button("Open").clicked() {
                    let _ = opener::open(&app.paths.root);
                }
            });
        });
        footer(ui, |ui| {
            save = widgets::button(ui, ButtonKind::Primary, Some(icons::CHECK), "Save").clicked();
            cancel = widgets::button(ui, ButtonKind::Secondary, None, "Cancel").clicked();
        });
    });
    if save {
        let mut settings = (**draft).clone();
        settings.tmdb_api_key = settings.tmdb_api_key.trim().to_string();
        app.actions.push(Action::SaveSettings(Box::new(settings)));
    } else if cancel || close {
        app.dialogs.settings = None;
    }
}

fn path_list(ui: &mut egui::Ui, title: &str, items: &[(PathBuf, String)], color: Color32) {
    if items.is_empty() {
        return;
    }
    let p = palette(ui.ctx());
    egui::CollapsingHeader::new(theme::semibold(format!("{title} ({})", items.len())))
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
                        ui.label(theme::medium(name).color(p.text))
                            .on_hover_text(path.display().to_string());
                        ui.add(
                            egui::Label::new(RichText::new(reason).size(12.5).color(color)).wrap(),
                        );
                        ui.add_space(2.0);
                    }
                });
        });
}

fn import_report_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(report) = app.reports.front() else {
        return;
    };
    let p = palette(ctx);
    let mut ok = false;
    let ((), close) = modal(ctx, "import_report", 540.0, |ui| {
        let (icon, color) = if report.failed.is_empty() {
            (icons::CHECK_CIRCLE, p.success)
        } else {
            (icons::WARNING_CIRCLE, p.warn)
        };
        header(ui, icon, color, "Import finished", Some(&report.summary()));
        show_report(ui, report);
        footer(ui, |ui| {
            ok = widgets::button(ui, ButtonKind::Primary, None, "Done").clicked()
        });
    });
    if ok || close {
        app.reports.pop_front();
    }
}

fn show_report(ui: &mut egui::Ui, report: &ImportReport) {
    let p = palette(ui.ctx());
    for note in &report.notes {
        ui.label(RichText::new(format!("{} {note}", icons::INFO)).color(p.warn));
    }
    let relinked: Vec<(PathBuf, String)> = report
        .relinked
        .iter()
        .map(|(title, path)| {
            (
                path.clone(),
                format!("is \"{title}\", which had gone missing"),
            )
        })
        .collect();
    path_list(ui, "Found moved movies", &relinked, p.success);
    path_list(ui, "Skipped", &report.skipped, p.text_weak);
    path_list(ui, "Failed", &report.failed, p.danger);
    path_list(ui, "Added with problems", &report.warnings, p.warn);
}

fn edit_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(d) = app.dialogs.edit.as_mut() else {
        return;
    };
    let p = palette(ctx);
    let mut done = false;
    let ((), close) = modal(ctx, "edit_title", 420.0, |ui| {
        header(ui, icons::PENCIL, p.accent, "Edit title", None);
        ui.label(RichText::new("Title").color(p.text_weak));
        let title = ui.add(
            egui::TextEdit::singleline(&mut d.title)
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if !d.focused {
            title.request_focus();
            d.focused = true;
        }
        ui.label(RichText::new("Year").color(p.text_weak));
        ui.add(
            egui::TextEdit::singleline(&mut d.year)
                .desired_width(90.0)
                .margin(Margin::symmetric(10, 7)),
        );
        let valid = !d.title.trim().is_empty()
            && (d.year.trim().is_empty() || parse_year(&d.year).is_some());
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        footer(ui, |ui| {
            if widgets::button_sized(ui, ButtonKind::Primary, None, "Save", 90.0, valid).clicked()
                || (enter && valid)
            {
                app.actions.push(Action::SaveTitle(
                    d.movie_id,
                    d.title.clone(),
                    parse_year(&d.year),
                ));
                done = true;
            }
            done |= widgets::button(ui, ButtonKind::Secondary, None, "Cancel").clicked();
        });
    });
    if done || close {
        app.dialogs.edit = None;
    }
}

fn tmdb_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(d) = app.dialogs.tmdb.as_mut() else {
        return;
    };
    let p = palette(ctx);
    let mut done = false;
    let ((), close) = modal(ctx, "tmdb", 560.0, |ui| {
        header(
            ui,
            icons::CLOUD_ARROW_DOWN,
            p.accent,
            "Find on TMDb",
            Some(&d.movie_title),
        );
        ui.horizontal(|ui| {
            let q = ui.add(
                egui::TextEdit::singleline(&mut d.query)
                    .desired_width(300.0)
                    .hint_text("Title")
                    .margin(Margin::symmetric(10, 7)),
            );
            ui.add(
                egui::TextEdit::singleline(&mut d.year)
                    .desired_width(60.0)
                    .hint_text("Year")
                    .margin(Margin::symmetric(10, 7)),
            );
            let enter = q.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if widgets::button_sized(
                ui,
                ButtonKind::Primary,
                Some(icons::MAGNIFYING_GLASS),
                "Search",
                0.0,
                !d.busy,
            )
            .clicked()
                || enter
            {
                app.actions.push(Action::TmdbSearch);
            }
            if d.busy {
                ui.add(egui::Spinner::new().color(p.accent));
            }
        });
        match &d.results {
            None => {}
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(p.danger));
            }
            Some(Ok(results)) if results.is_empty() => {
                ui.label(
                    RichText::new("No matches. Try a shorter title or remove the year.")
                        .color(p.text_weak),
                );
            }
            Some(Ok(results)) => {
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        for m in results {
                            egui::Frame::new()
                                .fill(p.raised)
                                .corner_radius(10)
                                .inner_margin(12)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        let year =
                                            m.year().map(|y| format!("  {y}")).unwrap_or_default();
                                        ui.label(theme::semibold(&m.title).color(p.text));
                                        ui.label(RichText::new(year).color(p.text_weak));
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if widgets::button_sized(
                                                    ui,
                                                    ButtonKind::Primary,
                                                    None,
                                                    "Use this",
                                                    0.0,
                                                    !d.busy,
                                                )
                                                .clicked()
                                                {
                                                    d.busy = true;
                                                    app.actions.push(Action::TmdbPick(
                                                        d.movie_id,
                                                        m.clone(),
                                                    ));
                                                }
                                            },
                                        );
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
                                        ui.label(
                                            RichText::new(format!("{short}{ellipsis}"))
                                                .size(12.5)
                                                .color(p.text_weak),
                                        );
                                    }
                                });
                        }
                    });
            }
        }
        footer(ui, |ui| {
            done = widgets::button(ui, ButtonKind::Secondary, None, "Close").clicked()
        });
    });
    if done || close {
        app.dialogs.tmdb = None;
    }
}

fn remove_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(id) = app.dialogs.confirm_remove else {
        return;
    };
    let p = palette(ctx);
    let title = app.lib.get(id).map(|m| m.title.clone()).unwrap_or_default();
    let mut done = false;
    let ((), close) = modal(ctx, "confirm_remove", 400.0, |ui| {
        header(ui, icons::TRASH, p.danger, "Remove from library?", None);
        ui.label(
            RichText::new(format!(
                "\"{title}\" will be removed from DropReel. The file on disk is not deleted."
            ))
            .color(p.text_weak),
        );
        footer(ui, |ui| {
            if widgets::button(ui, ButtonKind::Danger, Some(icons::TRASH), "Remove").clicked() {
                app.actions.push(Action::Remove(id));
                done = true;
            }
            done |= widgets::button(ui, ButtonKind::Secondary, None, "Cancel").clicked();
        });
    });
    if done || close {
        app.dialogs.confirm_remove = None;
    }
}

fn resume_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some((id, pos)) = app.resume_prompt else {
        return;
    };
    let p = palette(ctx);
    let title = app.lib.get(id).map(|m| m.title.clone()).unwrap_or_default();
    let mut done = false;
    let ((), close) = modal(ctx, "resume_prompt", 400.0, |ui| {
        header(
            ui,
            icons::PLAY,
            p.accent,
            &title,
            Some(&format!("You stopped at {}.", format_duration(pos))),
        );
        footer(ui, |ui| {
            if widgets::button(
                ui,
                ButtonKind::Primary,
                Some(icons::PLAY),
                &format!("Resume from {}", format_duration(pos)),
            )
            .clicked()
            {
                app.actions.push(Action::StartPlayback(id, Some(pos)));
                done = true;
            }
            if widgets::button(
                ui,
                ButtonKind::Secondary,
                Some(icons::ARROW_COUNTER_CLOCKWISE),
                "Start over",
            )
            .clicked()
            {
                app.actions.push(Action::StartPlayback(id, None));
                done = true;
            }
        });
    });
    if done || close {
        app.resume_prompt = None;
    }
}

fn stat_row(ui: &mut egui::Ui, icon: &str, label: &str, value: &str) {
    let p = palette(ui.ctx());
    ui.label(RichText::new(format!("{icon}  {label}")).color(p.text_weak));
    ui.add(egui::Label::new(RichText::new(value).color(p.text)).wrap());
    ui.end_row();
}

/// "7.80 GB → ≈ 4.75 GB" with a two-bar comparison.
fn size_comparison(ui: &mut egui::Ui, before: u64, after: u64, after_is_estimate: bool) {
    let p = palette(ui.ctx());
    let approx = if after_is_estimate { "≈ " } else { "" };
    ui.horizontal(|ui| {
        ui.label(
            theme::bold(format_bytes(before))
                .size(26.0)
                .color(p.text_weak),
        );
        ui.label(
            RichText::new(icons::ARROW_RIGHT)
                .size(20.0)
                .color(p.text_weak),
        );
        ui.label(
            theme::bold(format!("{approx}{}", format_bytes(after)))
                .size(26.0)
                .color(p.success),
        );
    });
    let width = ui.available_width();
    for (size, color) in [
        (before, p.text_weak.gamma_multiply(0.6)),
        (after, p.success),
    ] {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 8.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 255, p.raised);
        let mut bar = rect;
        bar.set_width(rect.width() * (size as f32 / before.max(1) as f32).min(1.0));
        ui.painter().rect_filled(bar, 255, color);
    }
    let saved = before.saturating_sub(after);
    ui.label(
        RichText::new(format!(
            "Saves {approx}{} ({:.0}%)",
            format_bytes(saved),
            saved as f64 * 100.0 / before.max(1) as f64
        ))
        .color(p.text_weak),
    );
}

fn encoder_label(e: Encoder) -> String {
    let icon = if e == Encoder::X265 {
        icons::CPU
    } else {
        icons::GRAPHICS_CARD
    };
    format!("{icon} {}", e.label())
}

fn encode_preview_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(state) = app.encode_preview.as_ref() else {
        return;
    };
    let p = palette(ctx);
    let mut done = false;
    let mut start: Option<EncodePreview> = None;
    let ((), close) = match state {
        PreviewDialog::Loading { title, .. } => modal(ctx, "encode_preview", 460.0, |ui| {
            header(
                ui,
                icons::LIGHTNING,
                p.warn,
                "Re-encode to HEVC",
                Some(title),
            );
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(18.0).color(p.accent));
                ui.label(
                    RichText::new("Checking the file and testing your graphics card…")
                        .color(p.text_weak),
                );
            });
            footer(ui, |ui| {
                done = widgets::button(ui, ButtonKind::Secondary, None, "Cancel").clicked()
            });
        }),
        PreviewDialog::Ready(preview) => modal(ctx, "encode_preview", 500.0, |ui| {
            header(
                ui,
                icons::LIGHTNING,
                p.warn,
                "Re-encode to HEVC",
                Some(&preview.title),
            );
            let blocker = preview.blocker();
            if let Ok(plan) = &preview.plan {
                size_comparison(ui, preview.source_size, plan.target_bytes, true);
            }
            ui.add_space(4.0);
            egui::Grid::new("preview_stats")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    if let Ok(e) = &preview.encoder {
                        let speed = if *e == Encoder::X265 {
                            " — slower, best quality per GB"
                        } else {
                            " — fast"
                        };
                        stat_row(
                            ui,
                            icons::SLIDERS,
                            "Encoder",
                            &format!("{}{speed}", encoder_label(*e)),
                        );
                    }
                    if let Ok(plan) = &preview.plan {
                        stat_row(
                            ui,
                            icons::MONITOR_PLAY,
                            "Video",
                            &format!(
                                "{} → HEVC at {:.1} Mbit/s",
                                preview.video.clone().unwrap_or_default(),
                                f64::from(plan.video_kbps) / 1000.0
                            ),
                        );
                        stat_row(
                            ui,
                            icons::MUSIC_NOTES,
                            "Audio & subtitles",
                            &format!(
                                "{} audio, {} subtitle tracks copied as-is ({})",
                                preview.audio_tracks,
                                preview.subtitle_tracks,
                                format_bytes(plan.passthrough_bytes)
                            ),
                        );
                    }
                    if let Some(d) = preview.duration {
                        stat_row(ui, icons::TIMER, "Length", &format_duration(d));
                    }
                    let name = preview
                        .output
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    stat_row(ui, icons::FILE_VIDEO, "New file", &name);
                    if let Some(free) = preview.free_space {
                        stat_row(ui, icons::HARD_DRIVES, "Free space", &format_bytes(free));
                    }
                });
            if let Some(blocker) = &blocker {
                widgets::callout(
                    ui,
                    p.danger,
                    icons::X_CIRCLE,
                    "Can't re-encode this file",
                    |ui| {
                        ui.label(RichText::new(blocker).color(p.text_weak));
                    },
                );
            } else if let Ok(plan) = &preview.plan
                && let Some(w) = &plan.warning
            {
                widgets::callout(ui, p.warn, icons::WARNING, "Quality may suffer", |ui| {
                    ui.label(RichText::new(w).color(p.text_weak));
                });
            }
            ui.label(
                RichText::new("The original stays untouched until the new file has been checked. You'll decide what happens to it afterwards.")
                    .size(12.5)
                    .color(p.text_weak),
            );
            footer(ui, |ui| {
                if blocker.is_none() {
                    let warned = preview.plan.as_ref().is_ok_and(|p| p.warning.is_some());
                    let label = if warned {
                        "Encode anyway"
                    } else {
                        "Start re-encode"
                    };
                    if widgets::button(ui, ButtonKind::Primary, Some(icons::LIGHTNING), label)
                        .clicked()
                    {
                        start = Some((**preview).clone());
                    }
                }
                done = widgets::button(ui, ButtonKind::Secondary, None, "Cancel").clicked();
            });
        }),
    };
    if let Some(preview) = start {
        app.actions.push(Action::StartEncode(Box::new(preview)));
    } else if done || close {
        app.encode_preview = None;
    }
}

fn encode_finished_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    let Some(outcome) = app.finished_encodes.front().cloned() else {
        return;
    };
    let p = palette(ctx);
    let EncodeOutcome {
        title,
        original,
        original_size,
        output,
        output_size,
        encoder,
        elapsed,
        ..
    } = &outcome;
    let mut choice = None;
    let confirming = app.dialogs.confirm_delete_original;
    let mins = (elapsed.as_secs_f64() / 60.0).round().max(1.0);
    let subtitle = format!("Finished in {mins:.0} min with {}", encoder.label());
    let ((), _close) = modal(ctx, "encode_finished", 520.0, |ui| {
        header(
            ui,
            icons::CHECK_CIRCLE,
            p.success,
            &format!("\"{title}\" is re-encoded"),
            Some(&subtitle),
        );
        size_comparison(ui, *original_size, *output_size, false);
        ui.label(
            RichText::new(format!(
                "The library now plays {}. What should happen to the original file?",
                output
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ))
            .color(p.text),
        );
        egui::Frame::new()
            .fill(p.raised)
            .corner_radius(8)
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{} {}", icons::FILE_VIDEO, original.display()))
                            .size(12.5)
                            .color(p.text_weak),
                    )
                    .wrap(),
                );
            });
        if confirming {
            widgets::callout(ui, p.danger, icons::WARNING, "Delete permanently?", |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} ({}) will be deleted. This can't be undone.",
                        original.display(),
                        format_bytes(*original_size)
                    ))
                    .color(p.text_weak),
                );
                ui.horizontal(|ui| {
                    if widgets::button(
                        ui,
                        ButtonKind::Danger,
                        Some(icons::TRASH),
                        "Delete permanently",
                    )
                    .clicked()
                    {
                        choice = Some(OriginalChoice::Delete);
                    }
                    if widgets::button(ui, ButtonKind::Secondary, None, "Back").clicked() {
                        app.dialogs.confirm_delete_original = false;
                    }
                });
            });
        } else {
            footer(ui, |ui| {
                if widgets::button(
                    ui,
                    ButtonKind::Primary,
                    Some(icons::RECYCLE),
                    "Move to Recycle Bin",
                )
                .clicked()
                {
                    choice = Some(OriginalChoice::Trash);
                }
                if widgets::button(ui, ButtonKind::Secondary, None, "Keep original").clicked() {
                    choice = Some(OriginalChoice::Keep);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if widgets::button(ui, ButtonKind::Ghost, Some(icons::TRASH), "Delete…")
                        .clicked()
                    {
                        app.dialogs.confirm_delete_original = true;
                    }
                });
            });
        }
    });
    if let Some(choice) = choice {
        app.dialogs.confirm_delete_original = false;
        app.actions.push(Action::ResolveOriginal(choice));
    }
}

fn quit_dialog(app: &mut DropReelApp, ctx: &egui::Context) {
    if !app.quit_prompt {
        return;
    }
    let p = palette(ctx);
    let title = app
        .encode_running
        .as_ref()
        .map(|s| s.title.clone())
        .unwrap_or_default();
    let mut done = false;
    let ((), close) = modal(ctx, "quit_prompt", 420.0, |ui| {
        header(
            ui,
            icons::LIGHTNING,
            p.warn,
            "A re-encode is still running",
            Some(&title),
        );
        ui.label(
            RichText::new(
                "Quitting cancels it and deletes the unfinished file. The original isn't affected.",
            )
            .color(p.text_weak),
        );
        footer(ui, |ui| {
            if widgets::button(ui, ButtonKind::Danger, None, "Quit anyway").clicked() {
                app.actions.push(Action::Quit);
                done = true;
            }
            done |= widgets::button(ui, ButtonKind::Primary, None, "Keep encoding").clicked();
        });
    });
    if done || close {
        app.quit_prompt = false;
    }
}
