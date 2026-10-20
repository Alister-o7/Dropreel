//! Top bar, status bar, poster grid and list view.

use eframe::egui::{
    self, Align, Color32, Id, Layout as UiLayout, Rect, RichText, Sense, Stroke, Vec2, vec2,
};
use egui_extras::{Column, TableBuilder};

use super::state::{Layout, Library, SortKey};
use super::widgets::{self, ACCENT, DANGER, WARN};
use super::{Action, DropReelApp};
use crate::library::Movie;
use crate::util::{format_bytes, format_date, format_duration};

const CARD_W: f32 = 154.0;
const POSTER_H: f32 = 231.0;
const TEXT_H: f32 = 44.0;
const GAP: f32 = 16.0;

pub fn top_bar(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let frame =
        egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(12, 8));
    egui::Panel::top("top_bar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("🎬 DropReel").size(20.0).strong());
            ui.add_space(8.0);
            if ui.button("+ Add files…").clicked() {
                app.actions.push(Action::AddFiles);
            }
            if ui.button("📁 Add folder…").clicked() {
                app.actions.push(Action::AddFolder);
            }
            ui.separator();

            let search = ui.add(
                egui::TextEdit::singleline(&mut app.filter.search)
                    .id(Id::unique("search"))
                    .hint_text("🔍 Search titles  (Ctrl+F)")
                    .desired_width(220.0),
            );
            if search.changed() {
                app.filter_dirty = true;
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                search.request_focus();
            }
            if !app.filter.search.is_empty()
                && ui.small_button("✖").on_hover_text("Clear search").clicked()
            {
                app.filter.search.clear();
                app.filter_dirty = true;
            }

            let before = (app.filter.sort, app.filter.descending, app.filter.tag);
            egui::ComboBox::from_id_salt("sort")
                .selected_text(format!("Sort: {}", app.filter.sort.label()))
                .show_ui(ui, |ui| {
                    for key in SortKey::ALL {
                        if ui
                            .selectable_value(&mut app.filter.sort, key, key.label())
                            .clicked()
                        {
                            app.filter.descending = key.default_descending();
                        }
                    }
                });
            let arrow = if app.filter.descending { "⬇" } else { "⬆" };
            if ui.button(arrow).on_hover_text("Reverse order").clicked() {
                app.filter.descending = !app.filter.descending;
            }

            let tag_label = app
                .filter
                .tag
                .and_then(|t| app.lib.tags.iter().find(|tag| tag.id == t))
                .map_or("All tags".to_string(), |t| format!("🏷 {}", t.name));
            egui::ComboBox::from_id_salt("tag_filter")
                .selected_text(tag_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.filter.tag, None, "All tags");
                    for tag in &app.lib.tags {
                        ui.selectable_value(&mut app.filter.tag, Some(tag.id), &tag.name);
                    }
                });
            if before != (app.filter.sort, app.filter.descending, app.filter.tag) {
                app.filter_dirty = true;
            }

            ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                if ui.button("⚙ Settings").clicked() {
                    app.actions.push(Action::OpenSettings);
                }
                ui.separator();
                ui.selectable_value(&mut app.prefs.layout, Layout::List, "☰ List");
                ui.selectable_value(&mut app.prefs.layout, Layout::Grid, "⊞ Grid");
            });
        });
    });
}

pub fn status_bar(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let frame =
        egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(12, 4));
    egui::Panel::bottom("status_bar").frame(frame).show(ui, |ui| {
        ui.horizontal(|ui| {
            let total = app.lib.movies.len();
            let mut text = format!("{total} movies · {}", format_bytes(app.lib.total_size()));
            if app.visible.len() != total {
                text.push_str(&format!(" · showing {}", app.visible.len()));
            }
            ui.weak(text);
            if !app.lib.missing.is_empty() {
                ui.colored_label(DANGER, format!("⚠ {} missing", app.lib.missing.len()))
                    .on_hover_text("These files are not where they used to be. Open a movie and use Locate…, or add the folder they moved to as a watched folder.");
            }
            ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                if app.imports_running > 0 {
                    let text = match app.import_progress {
                        Some(super::ImportProgress::Reading { done, total }) => format!("Reading files {done}/{total}"),
                        Some(super::ImportProgress::Metadata { done, total }) => format!("Fetching posters {done}/{total}"),
                        _ => "Scanning…".to_string(),
                    };
                    ui.label(text);
                    ui.spinner();
                }
            });
        });
    });
}

pub fn central(app: &mut DropReelApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        if app.lib.movies.is_empty() {
            empty_state(app, ui);
        } else if app.visible.is_empty() {
            ui.centered_and_justified(|ui| ui.weak("No movies match the search or tag filter."));
        } else {
            match app.prefs.layout {
                Layout::Grid => grid(app, ui),
                Layout::List => list(app, ui),
            }
        }
    });
}

fn empty_state(app: &mut DropReelApp, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 110.0).max(20.0));
        ui.label(RichText::new("🎬").size(64.0));
        ui.label(RichText::new("Your library is empty").size(24.0).strong());
        ui.add_space(4.0);
        ui.weak("Drag movie files or folders anywhere onto this window.");
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 250.0).max(0.0) / 2.0);
            if ui.button("+ Add files…").clicked() {
                app.actions.push(Action::AddFiles);
            }
            if ui.button("📁 Add folder…").clicked() {
                app.actions.push(Action::AddFolder);
            }
        });
    });
}

fn grid(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let DropReelApp {
        lib,
        visible,
        actions,
        settings,
        selected,
        ..
    } = app;
    let threshold = settings.threshold_bytes();
    let cols = ((ui.available_width() + GAP) / (CARD_W + GAP))
        .floor()
        .max(1.0) as usize;
    let rows = visible.len().div_ceil(cols);
    ui.spacing_mut().item_spacing = vec2(GAP, GAP);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, POSTER_H + TEXT_H, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    for &idx in visible.iter().skip(row * cols).take(cols) {
                        card(ui, lib, &lib.movies[idx], threshold, *selected, actions);
                    }
                });
            }
        });
}

fn card(
    ui: &mut egui::Ui,
    lib: &Library,
    movie: &Movie,
    threshold: u64,
    selected: Option<i64>,
    actions: &mut Vec<Action>,
) {
    let (rect, resp) = ui.allocate_exact_size(vec2(CARD_W, POSTER_H + TEXT_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let id = movie.id;
    let missing = lib.missing.contains(&id);
    let poster_rect = Rect::from_min_size(rect.min, vec2(CARD_W, POSTER_H));
    widgets::paint_poster(
        ui,
        poster_rect,
        movie.poster.as_deref(),
        &movie.title,
        missing,
    );

    let info_rect = Rect::from_center_size(
        poster_rect.right_top() + vec2(-18.0, 18.0),
        Vec2::splat(26.0),
    );
    let info = ui
        .interact(info_rect, resp.id.with("info"), Sense::click())
        .on_hover_text("Details");
    let hovered = resp.hovered() || info.hovered();

    let painter = ui.painter();
    if hovered || selected == Some(id) {
        painter.rect_stroke(
            poster_rect,
            6.0,
            Stroke::new(2.0, ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    let mut badge_pos = poster_rect.min + vec2(6.0, 6.0);
    if missing {
        let r = widgets::badge(painter, badge_pos, "Missing", DANGER);
        badge_pos.y = r.max.y + 4.0;
    }
    if movie.size > threshold {
        widgets::badge(painter, badge_pos, &format_bytes(movie.size), WARN);
    }
    if movie.watched {
        widgets::badge(
            painter,
            poster_rect.right_bottom() - vec2(30.0, 26.0),
            "✔",
            Color32::from_rgb(60, 150, 90),
        );
    }
    if let (Some(pos), Some(dur)) = (movie.resume_pos, movie.duration)
        && pos > 0.0
        && dur > 0.0
    {
        let bar = Rect::from_min_max(
            poster_rect.left_bottom() - vec2(0.0, 4.0),
            poster_rect.right_bottom(),
        );
        painter.rect_filled(bar, 0.0, Color32::from_black_alpha(160));
        let done = Rect::from_min_size(
            bar.min,
            vec2(
                bar.width() * (pos / dur).clamp(0.0, 1.0) as f32,
                bar.height(),
            ),
        );
        painter.rect_filled(done, 0.0, Color32::from_rgb(220, 60, 50));
    }
    if hovered {
        painter.circle_filled(info_rect.center(), 13.0, Color32::from_black_alpha(190));
        painter.text(
            info_rect.center(),
            egui::Align2::CENTER_CENTER,
            "ℹ",
            egui::FontId::proportional(15.0),
            Color32::WHITE,
        );
    }

    let text_top = poster_rect.left_bottom() + vec2(2.0, 6.0);
    let title = painter.layout(
        movie.title.clone(),
        egui::FontId::proportional(14.0),
        ui.visuals().strong_text_color(),
        CARD_W - 4.0,
    );
    let title = truncate_to_one_line(ui, title, &movie.title, CARD_W - 4.0);
    let title_h = title.size().y;
    painter.galley(text_top, title, Color32::WHITE);
    let sub = match movie.year {
        Some(y) => format!("{y} · {}", format_bytes(movie.size)),
        None => format_bytes(movie.size),
    };
    painter.text(
        text_top + vec2(0.0, title_h + 2.0),
        egui::Align2::LEFT_TOP,
        sub,
        egui::FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );

    if info.clicked() {
        actions.push(Action::ShowDetails(id));
    } else if resp.clicked() {
        actions.push(if missing {
            Action::ShowDetails(id)
        } else {
            Action::Play(id)
        });
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.on_hover_text(&movie.title)
        .context_menu(|ui| movie_menu(ui, movie, missing, actions));
}

fn truncate_to_one_line(
    ui: &egui::Ui,
    galley: std::sync::Arc<egui::Galley>,
    text: &str,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    if galley.rows.len() <= 1 {
        return galley;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(
        text.to_string(),
        egui::FontId::proportional(14.0),
        ui.visuals().strong_text_color(),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Right-click menu shared by the grid and the list.
pub fn movie_menu(ui: &mut egui::Ui, movie: &Movie, missing: bool, actions: &mut Vec<Action>) {
    let id = movie.id;
    if ui
        .add_enabled(!missing, egui::Button::new("▶ Play"))
        .clicked()
    {
        actions.push(Action::Play(id));
    }
    if ui.button("ℹ Details").clicked() {
        actions.push(Action::ShowDetails(id));
    }
    if ui.button("📂 Show in folder").clicked() {
        actions.push(Action::OpenFolder(id));
    }
    if missing && ui.button("🔍 Locate…").clicked() {
        actions.push(Action::Locate(id));
    }
    if ui.button("✏ Edit title…").clicked() {
        actions.push(Action::EditTitle(id));
    }
    let label = if movie.watched {
        "Mark as unwatched"
    } else {
        "✔ Mark as watched"
    };
    if ui.button(label).clicked() {
        actions.push(Action::SetWatched(id, !movie.watched));
    }
    ui.separator();
    if ui.button("🗑 Remove from library…").clicked() {
        actions.push(Action::AskRemove(id));
    }
}

fn list(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let DropReelApp {
        lib,
        visible,
        actions,
        filter,
        filter_dirty,
        settings,
        ..
    } = app;
    let threshold = settings.threshold_bytes();
    let mut header_cell =
        |row: &mut egui_extras::TableRow<'_, '_>, label: &str, key: Option<SortKey>| {
            row.col(|ui| {
                let Some(key) = key else {
                    ui.strong(label);
                    return;
                };
                let arrow = if filter.sort == key {
                    if filter.descending { " ⬇" } else { " ⬆" }
                } else {
                    ""
                };
                if ui
                    .add(
                        egui::Button::new(RichText::new(format!("{label}{arrow}")).strong())
                            .frame(false),
                    )
                    .clicked()
                {
                    if filter.sort == key {
                        filter.descending = !filter.descending;
                    } else {
                        filter.sort = key;
                        filter.descending = key.default_descending();
                    }
                    *filter_dirty = true;
                }
            });
        };
    TableBuilder::new(ui)
        .striped(true)
        .sense(Sense::click())
        .resizable(true)
        .cell_layout(UiLayout::left_to_right(Align::Center))
        .column(Column::remainder().at_least(220.0).clip(true))
        .column(Column::exact(56.0))
        .column(Column::exact(84.0))
        .column(Column::exact(70.0))
        .column(Column::initial(120.0).clip(true))
        .column(Column::exact(92.0))
        .column(Column::exact(92.0))
        .column(Column::initial(150.0).clip(true))
        .header(26.0, |mut row| {
            header_cell(&mut row, "Title", Some(SortKey::Title));
            header_cell(&mut row, "Year", Some(SortKey::Year));
            header_cell(&mut row, "Size", Some(SortKey::Size));
            header_cell(&mut row, "Length", None);
            header_cell(&mut row, "Video", None);
            header_cell(&mut row, "Added", Some(SortKey::DateAdded));
            header_cell(&mut row, "Last played", Some(SortKey::LastPlayed));
            header_cell(&mut row, "Tags", None);
        })
        .body(|body| {
            body.rows(24.0, visible.len(), |mut row| {
                let movie = &lib.movies[visible[row.index()]];
                let missing = lib.missing.contains(&movie.id);
                row.col(|ui| {
                    if ui.small_button("ℹ").on_hover_text("Details").clicked() {
                        actions.push(Action::ShowDetails(movie.id));
                    }
                    if missing {
                        ui.colored_label(DANGER, "⚠").on_hover_text("File missing");
                    }
                    let mut text = RichText::new(&movie.title);
                    if missing {
                        text = text.weak();
                    }
                    let resp = ui
                        .add(egui::Label::new(text).sense(Sense::click()).truncate())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resp.clicked() {
                        actions.push(if missing {
                            Action::ShowDetails(movie.id)
                        } else {
                            Action::Play(movie.id)
                        });
                    }
                });
                row.col(|ui| {
                    ui.label(movie.year.map(|y| y.to_string()).unwrap_or_default());
                });
                row.col(|ui| {
                    let size = format_bytes(movie.size);
                    if movie.size > threshold {
                        ui.colored_label(WARN, size)
                    } else {
                        ui.label(size)
                    };
                });
                row.col(|ui| {
                    ui.label(movie.duration.map(format_duration).unwrap_or_default());
                });
                row.col(|ui| {
                    ui.label(video_summary(movie));
                });
                row.col(|ui| {
                    ui.label(format_date(movie.date_added));
                });
                row.col(|ui| {
                    ui.label(
                        movie
                            .last_played
                            .map(format_date)
                            .unwrap_or_else(|| "—".into()),
                    );
                });
                row.col(|ui| {
                    ui.weak(lib.tag_names(movie.id).join(", "));
                });
                let resp = row.response();
                if resp.double_clicked() && !missing {
                    actions.push(Action::Play(movie.id));
                }
                resp.container_context_menu(|ui| movie_menu(ui, movie, missing, actions));
            });
        });
}

/// e.g. "hevc 3840×2160"
pub fn video_summary(movie: &Movie) -> String {
    match (&movie.video_codec, movie.width, movie.height) {
        (Some(c), Some(w), Some(h)) => format!("{c} {w}×{h}"),
        (Some(c), _, _) => c.clone(),
        _ => "—".into(),
    }
}

pub fn drop_overlay(ctx: &egui::Context) {
    if ctx.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::unique("drop_overlay"),
    ));
    let rect = ctx.content_rect();
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(190));
    painter.rect_stroke(
        rect.shrink(24.0),
        16.0,
        Stroke::new(3.0, ACCENT),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop to add to your library",
        egui::FontId::proportional(30.0),
        Color32::WHITE,
    );
}
