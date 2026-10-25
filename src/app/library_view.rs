//! Sidebar, toolbar, poster grid and list view.

use eframe::egui::{
    self, Align, Align2, Color32, Id, Layout as UiLayout, Margin, Rect, RichText, Sense, Stroke,
    Vec2, pos2, vec2,
};
use egui_extras::{Column, TableBuilder};

use super::icons;
use super::state::{Layout, Library, Shelf, SortKey};
use super::theme::{self, media, palette, radius};
use super::widgets::{self, ButtonKind};
use super::{Action, DropReelApp, EncodeBadge};
use crate::library::{Movie, MovieId};
use crate::metadata;
use crate::util::format_bytes;

const MIN_CARD_W: f32 = 158.0;
const TEXT_H: f32 = 48.0;
const GAP: f32 = 20.0;
/// Poster corner radius in the grid.
const RADIUS: u8 = radius::CARD;

pub fn sidebar(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    let frame = egui::Frame::new().fill(p.surface).inner_margin(Margin {
        left: 14,
        right: 14,
        top: 16,
        bottom: 14,
    });
    egui::Panel::left("sidebar")
        .exact_size(236.0)
        .frame(frame)
        .show(ui, |ui| {
            sidebar_glow(ui);
            ui.horizontal(|ui| {
                logo(ui, 32.0);
                ui.add_space(2.0);
                ui.label(
                    theme::bold("DropReel")
                        .size(20.0)
                        .color(p.text)
                        .extra_letter_spacing(-0.3),
                );
            });
            ui.add_space(18.0);
            let width = ui.available_width();
            if widgets::button_sized(
                ui,
                ButtonKind::Primary,
                Some(icons::PLUS),
                "Add movies",
                width,
                true,
            )
            .on_hover_text("Pick video files — or just drag them onto the window")
            .clicked()
            {
                app.actions.push(Action::AddFiles);
            }
            ui.add_space(2.0);
            if widgets::button_sized(
                ui,
                ButtonKind::Secondary,
                Some(icons::FOLDER_PLUS),
                "Add folder",
                width,
                true,
            )
            .clicked()
            {
                app.actions.push(Action::AddFolder);
            }
            ui.add_space(10.0);

            widgets::section_label(ui, "Library");
            let shelves = [
                (Shelf::All, icons::FILM_SLATE),
                (Shelf::ContinueWatching, icons::CLOCK_COUNTER_CLOCKWISE),
                (Shelf::Unwatched, icons::EYE),
                (Shelf::Watched, icons::CHECK_CIRCLE),
                (Shelf::Missing, icons::WARNING_CIRCLE),
            ];
            for (shelf, icon) in shelves {
                let count = app.lib.shelf_count(shelf);
                if matches!(shelf, Shelf::Missing | Shelf::ContinueWatching)
                    && count == 0
                    && app.filter.shelf != shelf
                {
                    continue;
                }
                let label = shelf.label();
                let selected = app.filter.shelf == shelf && app.filter.tag.is_none();
                if widgets::nav_item(ui, icon, label, Some(count), selected).clicked() {
                    app.filter.shelf = shelf;
                    app.filter.tag = None;
                    app.filter_dirty = true;
                }
            }

            if app.encode_running.is_some() || !app.encode_queued.is_empty() {
                ui.add_space(8.0);
                widgets::section_label(ui, "Re-encoding");
                queue_card(app, ui);
            }

            ui.add_space(8.0);
            widgets::section_label(ui, "Tags");
            if app.lib.tags.is_empty() {
                ui.label(
                    RichText::new("Tag movies from their details panel.")
                        .size(12.5)
                        .color(p.text_weak),
                );
            }
            egui::ScrollArea::vertical()
                .id_salt("tag_scroll")
                .max_height((ui.available_height() - 40.0).max(60.0))
                .show(ui, |ui| {
                    for tag in &app.lib.tags {
                        let selected = app.filter.tag == Some(tag.id);
                        let item = widgets::nav_item(
                            ui,
                            icons::TAG,
                            &tag.name,
                            Some(app.lib.tag_count(tag.id)),
                            selected,
                        )
                        .on_hover_text("Right-click to rename or delete");
                        if item.clicked() {
                            app.filter.tag = if selected { None } else { Some(tag.id) };
                            app.filter.shelf = Shelf::All;
                            app.filter_dirty = true;
                        }
                        item.context_menu(|ui| {
                            if menu_item(ui, icons::PENCIL, "Rename…") {
                                app.actions.push(Action::AskRenameTag(tag.id));
                            }
                            if menu_item(ui, icons::TRASH, "Delete tag…") {
                                app.actions.push(Action::AskDeleteTag(tag.id));
                            }
                        });
                    }
                });

            ui.with_layout(UiLayout::bottom_up(Align::Min), |ui| {
                let total = app.lib.movies.len();
                ui.label(
                    RichText::new(format!(
                        "{}  {total} movies · {}",
                        icons::HARD_DRIVES,
                        format_bytes(app.lib.total_size())
                    ))
                    .size(12.0)
                    .color(p.text_weak),
                );
            });
        });
}

/// The logo's white droplet on a frosted disc (a solid dark disc on the light theme).
fn logo(ui: &mut egui::Ui, size: f32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    let r = size / 2.0;
    if p.dark {
        painter.circle_filled(rect.center(), r, media::GLASS_FAINT);
        painter.circle_filled(rect.center(), r * 0.8, media::GLASS);
    } else {
        painter.circle_filled(rect.center(), r, p.text);
    }
    let h = size * 0.56;
    let mark = Rect::from_center_size(rect.center(), vec2(h * 0.74, h));
    egui::Image::new(theme::LOGO_MARK_URI)
        .texture_options(theme::SMOOTH)
        .paint_at(ui, mark);
}

/// A faint wash of the brand colours behind the logo at the top of the sidebar.
fn sidebar_glow(ui: &egui::Ui) {
    let p = palette(ui.ctx());
    if !p.dark {
        return;
    }
    let panel = ui.max_rect().expand2(vec2(14.0, 16.0));
    let glow = Rect::from_min_size(panel.min, vec2(panel.width(), 190.0));
    let painter = ui.painter();
    egui::Image::new(theme::AURORA_URI)
        .texture_options(theme::SMOOTH)
        .uv(Rect::from_min_max(pos2(0.0, 0.18), pos2(0.4, 0.62)))
        .tint(Color32::WHITE.gamma_multiply(0.22))
        .paint_at(ui, glow);
    widgets::vertical_gradient(
        painter,
        Rect::from_min_max(pos2(glow.left(), glow.top() + 40.0), glow.right_bottom()),
        Color32::TRANSPARENT,
        p.surface,
    );
}

/// The encode queue status card at the bottom of the sidebar.
fn queue_card(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    let Some(status) = app.encode_running.clone() else {
        if !app.encode_queued.is_empty() {
            ui.label(
                RichText::new(format!(
                    "{} {} waiting to encode",
                    icons::HOURGLASS,
                    app.encode_queued.len()
                ))
                .color(p.text_weak),
            );
        }
        return;
    };
    let resp = egui::Frame::new()
        .fill(p.raised)
        .corner_radius(radius::CARD)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::icon_fill(icons::LIGHTNING).size(16.0).color(p.warn));
                ui.label(
                    theme::semibold(status.stage.label())
                        .size(13.0)
                        .color(p.text),
                );
                if !app.encode_queued.is_empty() {
                    ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("+{}", app.encode_queued.len()))
                                .size(12.0)
                                .color(p.text_weak),
                        )
                        .on_hover_text(
                            app.encode_queued
                                .iter()
                                .map(|(_, t)| t.as_str())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                    });
                }
            });
            ui.add(
                egui::Label::new(RichText::new(&status.title).size(12.5).color(p.text_weak))
                    .truncate(),
            );
            widgets::progress_bar(ui, status.fraction, 6.0);
            ui.label(
                RichText::new(encode_progress_text(&status))
                    .size(12.0)
                    .color(p.text_weak),
            );
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Show details");
    if resp.clicked() {
        app.actions.push(Action::ShowDetails(status.movie_id));
    }
}

/// "42% · 12 min left · 2.1×"
pub fn encode_progress_text(s: &crate::encode::EncodeStatus) -> String {
    let mut parts = vec![format!("{:.0}%", s.fraction * 100.0)];
    if let Some(eta) = s.eta {
        let mins = (eta.as_secs_f64() / 60.0).ceil() as u64;
        parts.push(if mins >= 60 {
            format!("{}h {:02}m left", mins / 60, mins % 60)
        } else {
            format!("{mins} min left")
        });
    }
    if let Some(speed) = s.speed {
        parts.push(format!("{speed:.1}×"));
    }
    parts.join(" · ")
}

/// Narrowest search box worth putting next to the title on one line.
const MIN_ONE_ROW_SEARCH: f32 = 200.0;

pub fn toolbar(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    let frame = egui::Frame::new().fill(p.bg).inner_margin(Margin {
        left: 28,
        right: 20,
        top: 18,
        bottom: 10,
    });
    egui::Panel::top("toolbar").frame(frame).show(ui, |ui| {
        let title = match app
            .filter
            .tag
            .and_then(|t| app.lib.tags.iter().find(|tag| tag.id == t))
        {
            Some(tag) => format!("{} {}", icons::TAG, tag.name),
            None => app.filter.shelf.label().to_string(),
        };
        let title = theme::bold(title)
            .size(26.0)
            .color(p.text)
            .extra_letter_spacing(-0.4);
        let count = app.visible.len().to_string();
        let title_w = egui::WidgetText::from(title.clone())
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            )
            .size()
            .x;
        // The controls' width from the last frame (the search box aside).
        let controls_id = Id::unique("toolbar-controls-width");
        let controls_w = ui.data(|d| d.get_temp::<f32>(controls_id)).unwrap_or(330.0);
        // One line when the title, its count, the controls and a usable search box all fit;
        // otherwise the title gets a line of its own, so nothing is drawn over anything else.
        let one_row = ui.available_width() >= title_w + 60.0 + controls_w + MIN_ONE_ROW_SEARCH;
        if one_row {
            ui.horizontal(|ui| {
                ui.label(title);
                widgets::chip(ui, None, &count);
                ui.with_layout(UiLayout::right_to_left(Align::Center), |ui| {
                    let used = ui
                        .scope(|ui| controls(app, ui, true, false))
                        .response
                        .rect
                        .width();
                    ui.data_mut(|d| d.insert_temp(controls_id, used));
                    ui.add_space(6.0);
                    // Takes what the title leaves over, up to a comfortable width.
                    // (The frame's margins take another 28 points.)
                    let search_w = (ui.available_width() - 32.0).clamp(64.0, 250.0);
                    search_box(app, ui, search_w);
                    import_status(app, ui);
                });
            });
        } else {
            ui.horizontal(|ui| {
                ui.scope(|ui| {
                    ui.set_max_width((ui.available_width() - 60.0).max(40.0));
                    ui.add(egui::Label::new(title).truncate());
                });
                widgets::chip(ui, None, &count);
            });
            ui.add_space(6.0);
            // Left to right, wrapping onto another line rather than overlapping. When the search
            // box and the controls can't share a line, the search box gets a line of its own.
            let compact = ui.available_width() < 480.0;
            let shared = ui.available_width() >= 160.0 + controls_w;
            if !shared {
                let search_w = (ui.available_width() - 32.0).clamp(64.0, 400.0);
                ui.horizontal(|ui| search_box(app, ui, search_w));
                ui.add_space(6.0);
            }
            ui.horizontal_wrapped(|ui| {
                if shared {
                    let search_w = (ui.available_width() - controls_w - 40.0).clamp(120.0, 250.0);
                    search_box(app, ui, search_w);
                    ui.add_space(6.0);
                }
                controls(app, ui, false, compact);
                import_status(app, ui);
            });
        }
    });
}

/// Sort, reverse, grid/list and settings, added in reading order unless `right_to_left`.
/// `compact` drops the sort box's "Sort:" prefix to save room.
fn controls(app: &mut DropReelApp, ui: &mut egui::Ui, right_to_left: bool, compact: bool) {
    if right_to_left {
        settings_button(app, ui);
        ui.add_space(4.0);
        view_toggles(app, ui);
        ui.add_space(4.0);
        reverse_button(app, ui);
        sort_combo(app, ui, compact);
    } else {
        sort_combo(app, ui, compact);
        reverse_button(app, ui);
        ui.add_space(4.0);
        view_toggles(app, ui);
        ui.add_space(4.0);
        settings_button(app, ui);
    }
}

fn settings_button(app: &mut DropReelApp, ui: &mut egui::Ui) {
    if widgets::icon_button(ui, icons::GEAR, "Settings").clicked() {
        app.actions.push(Action::OpenSettings);
    }
}

fn view_toggles(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    egui::Frame::new()
        .fill(p.raised)
        .corner_radius(radius::CONTROL + 2)
        .inner_margin(2)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // Grid then list, reading left to right, whichever way the toolbar lays out.
            let mut order = [Layout::Grid, Layout::List];
            if ui.layout().prefer_right_to_left() {
                order.reverse();
            }
            for layout in order {
                let (icon, name) = match layout {
                    Layout::Grid => (icons::SQUARES_FOUR, "Poster grid"),
                    Layout::List => (icons::LIST, "List view"),
                };
                if widgets::icon_toggle(ui, icon, app.prefs.layout == layout, name).clicked() {
                    app.prefs.layout = layout;
                }
            }
        });
}

fn reverse_button(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let dir_icon = if app.filter.descending {
        icons::SORT_DESCENDING
    } else {
        icons::SORT_ASCENDING
    };
    if widgets::icon_button(ui, dir_icon, "Reverse order").clicked() {
        app.filter.descending = !app.filter.descending;
        app.filter_dirty = true;
    }
}

fn sort_combo(app: &mut DropReelApp, ui: &mut egui::Ui, compact: bool) {
    let before = app.filter.sort;
    let sort_label = format!("Sort: {}", app.filter.sort.label());
    let (shown, width) = if compact {
        (app.filter.sort.label().to_string(), 110.0)
    } else {
        (sort_label.clone(), 150.0)
    };
    let sort = egui::ComboBox::from_id_salt("sort")
        .selected_text(shown)
        .width(width)
        .show_ui(ui, |ui| {
            for key in SortKey::ALL {
                ui.selectable_value(&mut app.filter.sort, key, key.label());
            }
        });
    widgets::name_combo(&sort.response, &sort_label);
    if before != app.filter.sort {
        app.filter.descending = app.filter.sort.default_descending();
        app.filter_dirty = true;
    }
}

fn import_status(app: &DropReelApp, ui: &mut egui::Ui) {
    if app.imports_running == 0 {
        return;
    }
    let p = palette(ui.ctx());
    ui.add_space(8.0);
    let text = match app.import_progress {
        Some(super::ImportProgress::Reading { done, total }) => {
            format!("Reading {done}/{total}")
        }
        Some(super::ImportProgress::Metadata { done, total }) => {
            format!("Posters {done}/{total}")
        }
        _ => "Scanning…".to_string(),
    };
    ui.label(RichText::new(text).color(p.text_weak));
    ui.add(egui::Spinner::new().size(16.0).color(p.accent));
}

fn search_box(app: &mut DropReelApp, ui: &mut egui::Ui, width: f32) {
    let p = palette(ui.ctx());
    let search_id = Id::unique("search");
    let focused = ui.memory(|m| m.has_focus(search_id));
    let stroke = if focused {
        Stroke::new(1.5, p.accent)
    } else {
        Stroke::new(1.0, p.border)
    };
    egui::Frame::new()
        .fill(if focused { p.input } else { p.raised })
        .stroke(stroke)
        .corner_radius(radius::CONTROL)
        .inner_margin(Margin::symmetric(14, 5))
        .show(ui, |ui| {
            // The toolbar lays out right-to-left; the search box itself reads left-to-right.
            ui.allocate_ui_with_layout(
                vec2(width, 26.0),
                UiLayout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (glass, _) = ui.allocate_exact_size(vec2(16.0, 26.0), Sense::hover());
                    widgets::icon_centered(
                        ui.painter(),
                        glass.center(),
                        icons::MAGNIFYING_GLASS,
                        theme::icon_font(16.0),
                        p.text_weak,
                    );
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut app.filter.search)
                            .id(search_id)
                            .frame(egui::Frame::NONE)
                            .hint_text(if width > 200.0 {
                                "Search movies  (Ctrl+F)"
                            } else if width > 110.0 {
                                "Search"
                            } else {
                                ""
                            })
                            .desired_width(width - 54.0),
                    );
                    // The hint is hidden when the box is narrow, so name it explicitly.
                    ui.ctx().accesskit_node_builder(edit.id, |node| {
                        node.set_label("Search movies");
                    });
                    if edit.changed() {
                        app.filter_dirty = true;
                    }
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                        edit.request_focus();
                    }
                    // The clear button's slot is always there, so the field's layout stays steady.
                    let sense = if app.filter.search.is_empty() {
                        Sense::hover()
                    } else {
                        Sense::click()
                    };
                    let (slot, clear) = ui.allocate_exact_size(vec2(18.0, 26.0), sense);
                    if !app.filter.search.is_empty() {
                        let color = if clear.hovered() { p.text } else { p.text_weak };
                        widgets::icon_centered(
                            ui.painter(),
                            slot.center(),
                            icons::X_CIRCLE,
                            theme::icon_font(16.0),
                            color,
                        );
                        if widgets::named(clear, "Clear search")
                            .on_hover_text("Clear search")
                            .clicked()
                        {
                            app.filter.search.clear();
                            app.filter_dirty = true;
                        }
                    }
                },
            );
        });
}

pub fn central(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    let frame = egui::Frame::new().fill(p.bg).inner_margin(Margin {
        left: 28,
        right: 12,
        top: 6,
        bottom: 0,
    });
    egui::CentralPanel::default().frame(frame).show(ui, |ui| {
        if app.lib.movies.is_empty() {
            empty_state(app, ui);
        } else if app.visible.is_empty() {
            no_results(ui);
        } else {
            match app.prefs.layout {
                Layout::Grid => grid(app, ui),
                Layout::List => list(app, ui),
            }
        }
    });
}

/// The empty-library illustration. Its edges fade to the dark theme's background; on the light
/// theme it sits in a rounded card.
fn hero_art(ui: &mut egui::Ui, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, width * 0.743), Sense::hover());
    egui::Image::new(theme::EMPTY_ART_URI)
        .texture_options(theme::SMOOTH)
        .corner_radius(radius::PANEL)
        .paint_at(ui, rect);
}

/// A large gradient disc with an icon and a soft glow, for empty screens.
fn hero_icon(ui: &mut egui::Ui, icon: &str, size: f32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size * 2.6), Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size * 1.3, p.text.gamma_multiply(0.05));
    painter.circle_filled(rect.center(), size, p.text.gamma_multiply(0.09));
    widgets::icon_centered(
        painter,
        rect.center(),
        icon,
        theme::icon_font(size * 0.9),
        p.text,
    );
}

fn empty_state(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        let art_w = (ui.available_height() * 0.62).clamp(220.0, 420.0);
        ui.add_space((ui.available_height() / 2.0 - art_w * 0.375 - 150.0).max(10.0));
        hero_art(ui, art_w);
        ui.add_space(10.0);
        ui.label(
            theme::bold("Your library is empty")
                .size(30.0)
                .color(p.text)
                .extra_letter_spacing(-0.5),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new("Drag movie files or whole folders anywhere onto this window.")
                .size(15.0)
                .color(p.text_weak),
        );
        ui.add_space(20.0);
        let total_w = 290.0;
        ui.allocate_ui_with_layout(
            vec2(total_w, 36.0),
            UiLayout::left_to_right(Align::Center),
            |ui| {
                if widgets::button(ui, ButtonKind::Primary, Some(icons::PLUS), "Add movies")
                    .clicked()
                {
                    app.actions.push(Action::AddFiles);
                }
                if widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    Some(icons::FOLDER_PLUS),
                    "Add folder",
                )
                .clicked()
                {
                    app.actions.push(Action::AddFolder);
                }
            },
        );
        ui.add_space(18.0);
        ui.label(
            RichText::new(format!(
                "{} Tip: add a watched folder in Settings and new movies show up automatically.",
                icons::SPARKLE
            ))
            .size(12.5)
            .color(p.text_weak),
        );
    });
}

fn no_results(ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - 120.0).max(30.0));
        hero_icon(ui, icons::MAGNIFYING_GLASS, 34.0);
        ui.add_space(6.0);
        ui.label(theme::bold("No movies here").size(20.0).color(p.text));
        ui.label(RichText::new("Try another search, shelf or tag.").color(p.text_weak));
    });
}

fn grid(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let DropReelApp {
        lib,
        visible,
        actions,
        settings,
        selected,
        encode_badges,
        filter,
        ..
    } = app;
    let threshold = settings.threshold_bytes();
    let avail = ui.available_width() - 16.0;
    let cols = ((avail + GAP) / (MIN_CARD_W + GAP)).floor().max(1.0) as usize;
    let card_w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).min(230.0);
    let card_h = card_w * 1.5 + TEXT_H;
    let row_h = card_h + GAP;
    let rows = visible.len().div_ceil(cols);
    // The banner sits above the whole library only, not above a shelf, tag or search.
    let unfiltered =
        filter.shelf == Shelf::All && filter.tag.is_none() && filter.search.trim().is_empty();
    let banner = featured(lib).filter(|_| unfiltered);
    let banner_h = if banner.is_some() {
        (avail * 0.36).clamp(220.0, 380.0) + GAP
    } else {
        0.0
    };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            ui.set_height(banner_h + rows as f32 * row_h);
            let origin = ui.max_rect().min;
            if let Some((movie, resume)) = banner
                && viewport.min.y < banner_h
            {
                let rect =
                    Rect::from_min_size(origin + vec2(0.0, 6.0), vec2(avail, banner_h - GAP));
                billboard(ui, rect, movie, resume, actions);
            }
            // Only the rows in view are laid out.
            let first = ((viewport.min.y - banner_h) / row_h).floor().max(0.0) as usize;
            let last = (((viewport.max.y - banner_h) / row_h).ceil().max(0.0) as usize).min(rows);
            for row in first..last {
                let top = origin.y + banner_h + row as f32 * row_h + 6.0;
                let row_rect = Rect::from_min_size(pos2(origin.x, top), vec2(avail + 16.0, card_h));
                let builder = egui::UiBuilder::new()
                    .max_rect(row_rect)
                    .layout(UiLayout::left_to_right(Align::Min));
                ui.scope_builder(builder, |ui| {
                    ui.spacing_mut().item_spacing = vec2(GAP, 0.0);
                    for &idx in visible.iter().skip(row * cols).take(cols) {
                        let movie = &lib.movies[idx];
                        card(
                            ui,
                            lib,
                            movie,
                            card_w,
                            threshold,
                            *selected,
                            encode_badges.get(&movie.id),
                            actions,
                        );
                    }
                });
            }
        });
}

/// The movie for the banner: the one most recently left part-way, or else the newest.
/// Returns it with the position to resume from, if any.
fn featured(lib: &Library) -> Option<(&Movie, Option<f64>)> {
    let present = |m: &&Movie| !lib.missing.contains(&m.id);
    lib.movies
        .iter()
        .filter(present)
        .filter(|m| !m.watched && m.resume_pos.is_some_and(|p| p > 0.0))
        .max_by_key(|m| m.last_played.unwrap_or(0))
        .map(|m| (m, m.resume_pos))
        .or_else(|| {
            lib.movies
                .iter()
                .filter(present)
                .max_by_key(|m| m.date_added)
                .map(|m| (m, None))
        })
}

/// A wide banner for one movie, like Netflix's billboard: its picture fading into the page, with
/// the title and Play / More info.
fn billboard(
    ui: &mut egui::Ui,
    rect: Rect,
    movie: &Movie,
    resume: Option<f64>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    widgets::paint_poster(ui, rect, movie.poster.as_deref(), "", false, radius::PANEL);
    let painter = ui.painter();
    // Darken the left side for the text, and melt the bottom into the page.
    widgets::horizontal_gradient(
        painter,
        Rect::from_min_max(
            rect.min,
            pos2(rect.left() + rect.width() * 0.7, rect.bottom()),
        ),
        media::SCRIM_STRONG,
        Color32::TRANSPARENT,
    );
    widgets::vertical_gradient(
        painter,
        Rect::from_min_max(
            pos2(rect.left(), rect.top() + rect.height() * 0.55),
            rect.right_bottom(),
        ),
        Color32::TRANSPARENT,
        p.bg,
    );

    let left = rect.left() + 32.0;
    let text_w = (rect.width() * 0.5).max(240.0);
    let buttons_y = rect.bottom() - 52.0;
    let mut meta = Vec::new();
    if let Some(y) = movie.year {
        meta.push(y.to_string());
    }
    if let Some(d) = movie.duration {
        meta.push(widgets::short_duration(d));
    }
    if let Some(r) = widgets::resolution_label(movie.width, movie.height) {
        meta.push(r.to_string());
    }
    let meta = painter.layout_no_wrap(
        meta.join("  ·  "),
        theme::font(14.0, "medium"),
        media::TEXT_WEAK,
    );
    let meta_y = buttons_y - 14.0 - meta.size().y;
    let title_size = (rect.height() * 0.12).clamp(26.0, 44.0);
    let mut job = egui::text::LayoutJob::simple(
        movie.title.clone(),
        theme::font(title_size, "bold"),
        media::TEXT,
        text_w,
    );
    job.wrap.max_rows = 2;
    let title = ui.fonts_mut(|f| f.layout_job(job));
    let title_y = meta_y - 8.0 - title.size().y;
    let kicker = if resume.is_some() {
        "CONTINUE WATCHING"
    } else {
        "RECENTLY ADDED"
    };
    let kicker = painter.layout_no_wrap(kicker.into(), theme::font(12.0, "bold"), media::TEXT);
    let kicker_y = title_y - 6.0 - kicker.size().y;
    painter.rect_filled(
        Rect::from_min_size(pos2(left, kicker_y + 1.0), vec2(3.0, kicker.size().y - 2.0)),
        1,
        p.primary,
    );
    painter.galley(pos2(left + 10.0, kicker_y), kicker, media::TEXT);
    painter.galley(pos2(left, title_y), title, media::TEXT);
    painter.galley(pos2(left, meta_y), meta, media::TEXT_WEAK);

    let row = Rect::from_min_size(pos2(left, buttons_y), vec2(text_w, 36.0));
    let builder = egui::UiBuilder::new()
        .max_rect(row)
        .layout(UiLayout::left_to_right(Align::Center));
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let label = match resume {
            Some(pos) => format!("Resume {}", crate::util::format_duration(pos)),
            None => "Play".to_string(),
        };
        let play = widgets::button_sized(
            ui,
            ButtonKind::Primary,
            Some(icons::PLAY),
            &label,
            110.0,
            true,
        );
        if widgets::named(play, &format!("{label}: {}", movie.title)).clicked() {
            actions.push(match resume {
                Some(pos) => Action::StartPlayback(movie.id, Some(pos)),
                None => Action::Play(movie.id),
            });
        }
        let info = widgets::button(ui, ButtonKind::Secondary, Some(icons::INFO), "More info");
        if widgets::named(info, &format!("More info: {}", movie.title)).clicked() {
            actions.push(Action::ShowDetails(movie.id));
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    lib: &Library,
    movie: &Movie,
    width: f32,
    threshold: u64,
    selected: Option<MovieId>,
    encode: Option<&EncodeBadge>,
    actions: &mut Vec<Action>,
) {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, width * 1.5 + TEXT_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let id = movie.id;
    let missing = lib.missing.contains(&id);
    let poster_base = Rect::from_min_size(rect.min, vec2(width, width * 1.5));
    let info_rect = Rect::from_center_size(
        poster_base.right_top() + vec2(-20.0, 20.0),
        Vec2::splat(30.0),
    );
    let resp = widgets::named(resp, &movie.title);
    let info = ui
        .interact(info_rect, resp.id.with("info"), Sense::click())
        .on_hover_text("Details");
    let info = widgets::named(info, &format!("Details: {}", movie.title));
    let hovered = resp.hovered() || info.hovered();
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("hover"), hovered, 0.16);
    // Like Netflix, a hovered poster grows a little rather than lifting.
    let poster = poster_base.expand2(poster_base.size() * 0.03 * t);
    let painter = ui.painter();

    if t > 0.0 {
        let shadow = egui::Shadow {
            offset: [0, 14],
            blur: 32,
            spread: 0,
            color: media::SCRIM.gamma_multiply(t),
        };
        painter.add(shadow.as_shape(poster, RADIUS));
    }
    let poster_shown = widgets::paint_poster(
        ui,
        poster,
        movie.poster.as_deref(),
        &movie.title,
        missing,
        RADIUS,
    );
    // A frame from the movie has no title of its own: put one on it, like box art.
    let title_on_art = poster_shown > 0.0
        && movie
            .poster
            .as_deref()
            .is_some_and(metadata::is_frame_poster);
    let painter = ui.painter();

    if t > 0.01 && encode.is_none() {
        let r = f32::from(RADIUS);
        let lower = Rect::from_min_max(
            pos2(poster.left(), poster.top() + poster.height() * 0.4),
            pos2(poster.right(), poster.bottom() - r),
        );
        widgets::vertical_gradient(
            painter,
            lower,
            Color32::TRANSPARENT,
            media::SCRIM_STRONG.gamma_multiply(t),
        );
        painter.rect_filled(
            Rect::from_min_max(
                pos2(poster.left(), poster.bottom() - r),
                poster.right_bottom(),
            ),
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: RADIUS,
                se: RADIUS,
            },
            media::SCRIM_STRONG.gamma_multiply(t),
        );
        if !missing {
            widgets::play_disc(painter, poster.center(), 22.0 + 2.0 * t, t);
        }
        let info_center = poster.right_top() + vec2(-20.0, 20.0);
        painter.circle_filled(info_center, 15.0, media::BUTTON.gamma_multiply(t));
        painter.circle_stroke(
            info_center,
            15.0,
            Stroke::new(1.5, media::BUTTON_RIM.gamma_multiply(t)),
        );
        widgets::icon_centered(
            painter,
            info_center,
            icons::INFO,
            theme::icon_font(18.0),
            media::TEXT.gamma_multiply(t),
        );
    }
    // After the hover shading, so the title stays bright.
    if title_on_art {
        title_on_poster(
            ui,
            poster,
            &movie.title,
            poster_shown,
            movie.resume_pos.is_some(),
        );
    }
    let painter = ui.painter();
    if selected == Some(id) || t > 0.01 {
        let color = if selected == Some(id) {
            p.accent
        } else {
            media::HAIRLINE.gamma_multiply(t)
        };
        painter.rect_stroke(
            poster,
            RADIUS,
            Stroke::new(if selected == Some(id) { 2.0 } else { 1.0 }, color),
            egui::StrokeKind::Outside,
        );
    }

    // Badges
    let mut y = poster.top() + 8.0;
    if missing {
        let r = widgets::pill(
            painter,
            pos2(poster.left() + 8.0, y),
            Align2::LEFT_TOP,
            Some(icons::WARNING),
            "Missing",
            media::DANGER,
            media::TEXT,
        );
        y = r.bottom() + 4.0;
    }
    if movie.size > threshold && encode.is_none() {
        widgets::pill(
            painter,
            pos2(poster.left() + 8.0, y),
            Align2::LEFT_TOP,
            Some(icons::LIGHTNING),
            &format_bytes(movie.size),
            media::PILL,
            media::ON_PILL,
        );
    }
    if movie.watched {
        widgets::pill(
            painter,
            poster.right_bottom() - vec2(8.0, 10.0),
            Align2::RIGHT_BOTTOM,
            Some(icons::CHECK),
            "",
            media::PILL,
            media::ON_PILL,
        );
    }
    if let (Some(pos), Some(dur)) = (movie.resume_pos, movie.duration)
        && pos > 0.0
        && dur > 0.0
        && !movie.watched
    {
        let bar = Rect::from_min_max(
            poster.left_bottom() + vec2(10.0, -10.0),
            poster.right_bottom() + vec2(-10.0, -6.0),
        );
        painter.rect_filled(bar, 1, media::TRACK);
        let mut done = bar;
        done.set_width(bar.width() * (pos / dur).clamp(0.0, 1.0) as f32);
        painter.rect_filled(done, 1, media::PROGRESS);
    }

    // Encode overlay
    if let Some(badge) = encode {
        painter.rect_filled(poster, RADIUS, media::SCRIM_STRONG);
        let (fraction, label) = match badge {
            EncodeBadge::Running(s) => (s.fraction, format!("{:.0}%", s.fraction * 100.0)),
            EncodeBadge::Queued(n) => (0.0, format!("#{}", n + 1)),
        };
        let center = poster.center() - vec2(0.0, 8.0);
        widgets::progress_ring(
            painter,
            center,
            30.0,
            fraction,
            media::PROGRESS,
            media::TRACK,
            5.0,
        );
        let label = painter.layout_no_wrap(label, theme::font(16.0, "bold"), media::TEXT);
        widgets::galley_centered(painter, center, label, media::TEXT);
        let caption = if matches!(badge, EncodeBadge::Running(_)) {
            "Re-encoding"
        } else {
            "Queued"
        };
        painter.text(
            center + vec2(0.0, 46.0),
            Align2::CENTER_CENTER,
            caption,
            theme::font(12.5, "semibold"),
            media::TEXT_WEAK,
        );
    }

    // Caption: the title (unless it is already on the art), then year and length.
    let text_top = poster_base.left_bottom() + vec2(2.0, 10.0);
    let mut title_h = -3.0;
    if !title_on_art {
        let mut job = egui::text::LayoutJob::simple_singleline(
            movie.title.clone(),
            theme::font(14.0, "semibold"),
            p.text,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(width - 4.0);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        title_h = galley.size().y;
        ui.painter().galley(text_top, galley, p.text);
    }
    let mut sub = Vec::new();
    if let Some(y) = movie.year {
        sub.push(y.to_string());
    }
    if let Some(d) = movie.duration {
        sub.push(widgets::short_duration(d));
    }
    if sub.is_empty() {
        sub.push(format_bytes(movie.size));
    }
    ui.painter().text(
        text_top + vec2(0.0, title_h + 3.0),
        Align2::LEFT_TOP,
        sub.join(" · "),
        theme::font(12.5, "regular"),
        p.text_weak,
    );

    if info.clicked() {
        actions.push(Action::ShowDetails(id));
    } else if resp.clicked() {
        actions.push(if missing || encode.is_some() {
            Action::ShowDetails(id)
        } else {
            Action::Play(id)
        });
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.context_menu(|ui| movie_menu(ui, movie, missing, movie.size > threshold, actions));
}

/// The title across the bottom of a poster, over a dark fade; `opacity` follows the poster's own
/// fade-in. Leaves room for the resume bar when there is one.
fn title_on_poster(ui: &egui::Ui, poster: Rect, title: &str, opacity: f32, resume_bar: bool) {
    let painter = ui.painter();
    let fade = Rect::from_min_max(
        pos2(poster.left(), poster.top() + poster.height() * 0.55),
        poster.right_bottom() - vec2(0.0, f32::from(RADIUS)),
    );
    widgets::vertical_gradient(
        painter,
        fade,
        Color32::TRANSPARENT,
        media::SCRIM_STRONG.gamma_multiply(opacity),
    );
    painter.rect_filled(
        Rect::from_min_max(
            pos2(poster.left(), poster.bottom() - f32::from(RADIUS)),
            poster.right_bottom(),
        ),
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: RADIUS,
            se: RADIUS,
        },
        media::SCRIM_STRONG.gamma_multiply(opacity),
    );
    let mut job = egui::text::LayoutJob::simple(
        title.to_string(),
        theme::font(16.0, "bold"),
        media::TEXT,
        poster.width() - 24.0,
    );
    job.wrap.max_rows = 2;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let bottom = poster.bottom() - if resume_bar { 18.0 } else { 12.0 };
    painter.galley_with_override_text_color(
        pos2(poster.left() + 12.0, bottom - galley.size().y),
        galley,
        media::TEXT.gamma_multiply(opacity),
    );
}

fn menu_item(ui: &mut egui::Ui, icon: &str, label: &str) -> bool {
    ui.add(
        egui::Button::new(format!("{icon}   {label}"))
            .frame_when_inactive(false)
            .min_size(vec2(200.0, 28.0)),
    )
    .clicked()
}

/// Right-click menu shared by the grid, the list and the detail panel.
pub fn movie_menu(
    ui: &mut egui::Ui,
    movie: &Movie,
    missing: bool,
    large: bool,
    actions: &mut Vec<Action>,
) {
    let id = movie.id;
    if !missing && menu_item(ui, icons::PLAY, "Play") {
        actions.push(Action::Play(id));
    }
    if menu_item(ui, icons::INFO, "Details") {
        actions.push(Action::ShowDetails(id));
    }
    if menu_item(ui, icons::FOLDER_OPEN, "Show in folder") {
        actions.push(Action::OpenFolder(id));
    }
    if missing && menu_item(ui, icons::CROSSHAIR, "Locate…") {
        actions.push(Action::Locate(id));
    }
    if large && !missing && menu_item(ui, icons::LIGHTNING, "Re-encode to HEVC…") {
        actions.push(Action::AskReencode(id));
    }
    if menu_item(ui, icons::PENCIL, "Edit title…") {
        actions.push(Action::EditTitle(id));
    }
    let (icon, label) = if movie.watched {
        (icons::EYE, "Mark as unwatched")
    } else {
        (icons::CHECK_CIRCLE, "Mark as watched")
    };
    if menu_item(ui, icon, label) {
        actions.push(Action::SetWatched(id, !movie.watched));
    }
    ui.separator();
    if menu_item(ui, icons::TRASH, "Remove from library…") {
        actions.push(Action::AskRemove(id));
    }
}

fn list(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let p = palette(ui.ctx());
    let DropReelApp {
        lib,
        visible,
        actions,
        filter,
        filter_dirty,
        settings,
        encode_badges,
        ..
    } = app;
    let threshold = settings.threshold_bytes();
    let mut header_cell =
        |row: &mut egui_extras::TableRow<'_, '_>, label: &str, key: Option<SortKey>| {
            row.col(|ui| {
                let text = RichText::new(label.to_uppercase())
                    .size(11.0)
                    .color(p.text_weak)
                    .extra_letter_spacing(0.6);
                let Some(key) = key else {
                    ui.label(text);
                    return;
                };
                let arrow = if filter.sort == key {
                    if filter.descending { " ↓" } else { " ↑" }
                } else {
                    ""
                };
                let label = RichText::new(format!("{}{arrow}", label.to_uppercase()))
                    .size(11.0)
                    .color(if filter.sort == key {
                        p.text
                    } else {
                        p.text_weak
                    });
                if ui
                    .add(egui::Label::new(label).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
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
    ui.add_space(4.0);
    TableBuilder::new(ui)
        .striped(true)
        .sense(Sense::click())
        .cell_layout(UiLayout::left_to_right(Align::Center))
        .column(Column::exact(44.0))
        .column(Column::remainder().at_least(220.0).clip(true))
        .column(Column::exact(64.0))
        .column(Column::exact(80.0))
        .column(Column::exact(96.0))
        .column(Column::exact(100.0))
        .column(Column::initial(160.0).clip(true))
        .header(28.0, |mut row| {
            header_cell(&mut row, "", None);
            header_cell(&mut row, "Title", Some(SortKey::Title));
            header_cell(&mut row, "Year", Some(SortKey::Year));
            header_cell(&mut row, "Length", None);
            header_cell(&mut row, "Size", Some(SortKey::Size));
            header_cell(&mut row, "Added", Some(SortKey::DateAdded));
            header_cell(&mut row, "Tags", None);
        })
        .body(|body| {
            body.rows(52.0, visible.len(), |mut row| {
                let movie = &lib.movies[visible[row.index()]];
                let missing = lib.missing.contains(&movie.id);
                let encode = encode_badges.get(&movie.id);
                row.col(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 45.0), Sense::hover());
                    widgets::paint_poster(
                        ui,
                        rect,
                        movie.poster.as_deref(),
                        &movie.title,
                        missing,
                        5,
                    );
                });
                row.col(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let has_status = encode.is_some()
                            || missing
                            || movie.watched
                            || movie.resume_pos.is_some();
                        ui.add_space(if has_status { 8.0 } else { 16.0 });
                        let mut title = theme::semibold(&movie.title).color(if missing {
                            p.text_weak
                        } else {
                            p.text
                        });
                        if missing {
                            title = title.strikethrough();
                        }
                        ui.add(egui::Label::new(title).truncate());
                        let status = match encode {
                            Some(EncodeBadge::Running(s)) => Some((
                                icons::LIGHTNING,
                                format!("Re-encoding {:.0}%", s.fraction * 100.0),
                                p.accent,
                            )),
                            Some(EncodeBadge::Queued(_)) => Some((
                                icons::HOURGLASS,
                                "Queued for re-encode".to_string(),
                                p.text_weak,
                            )),
                            None if missing => {
                                Some((icons::WARNING, "File missing".to_string(), p.danger))
                            }
                            None if movie.watched => {
                                Some((icons::CHECK_CIRCLE, "Watched".to_string(), p.success))
                            }
                            None => movie.resume_pos.map(|r| {
                                (
                                    icons::CLOCK_COUNTER_CLOCKWISE,
                                    format!("Stopped at {}", crate::util::format_duration(r)),
                                    p.text_weak,
                                )
                            }),
                        };
                        if let Some((icon, text, color)) = status {
                            ui.label(
                                RichText::new(format!("{icon} {text}"))
                                    .size(12.0)
                                    .color(color),
                            );
                        }
                    });
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(movie.year.map(|y| y.to_string()).unwrap_or_default())
                            .color(p.text_weak),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(
                            movie
                                .duration
                                .map(widgets::short_duration)
                                .unwrap_or_default(),
                        )
                        .color(p.text_weak),
                    );
                });
                row.col(|ui| {
                    if movie.size > threshold {
                        ui.label(
                            RichText::new(format!(
                                "{} {}",
                                icons::LIGHTNING,
                                format_bytes(movie.size)
                            ))
                            .color(p.warn),
                        )
                        .on_hover_text("Over the size limit — can be re-encoded to HEVC");
                    } else {
                        ui.label(RichText::new(format_bytes(movie.size)).color(p.text_weak));
                    }
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(crate::util::format_date(movie.date_added))
                            .color(p.text_weak),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(lib.tag_names(movie.id).join(", "))
                            .size(12.5)
                            .color(p.text_weak),
                    );
                });
                let resp = row.response();
                if resp.double_clicked() && !missing {
                    actions.push(Action::Play(movie.id));
                } else if resp.clicked() {
                    actions.push(Action::ShowDetails(movie.id));
                }
                resp.container_context_menu(|ui| {
                    movie_menu(ui, movie, missing, movie.size > threshold, actions)
                });
            });
        });
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
    // Aurora backdrop, cropped to cover the window, darkened for contrast.
    let aurora = ctx.try_load_texture(theme::AURORA_URI, theme::SMOOTH, egui::SizeHint::default());
    painter.rect_filled(rect, 0.0, theme::ramp::N950);
    if let Ok(egui::load::TexturePoll::Ready { texture }) = aurora {
        painter.image(
            texture.id,
            rect,
            widgets::cover_uv(texture.size, rect.size()),
            Color32::WHITE.gamma_multiply(0.85),
        );
    }
    painter.rect_filled(rect, 0.0, media::SCRIM_SOFT);
    let inner = rect.shrink(28.0);
    painter.rect_stroke(
        inner,
        radius::PANEL,
        Stroke::new(2.0, media::EDGE),
        egui::StrokeKind::Inside,
    );
    let c = rect.center();
    let disc = c - vec2(0.0, 70.0);
    painter.circle_filled(disc, 74.0, media::GLASS_FAINT);
    painter.circle_filled(disc, 60.0, media::GLASS);
    if let Ok(egui::load::TexturePoll::Ready { texture }) = ctx.try_load_texture(
        theme::LOGO_MARK_URI,
        theme::SMOOTH,
        egui::SizeHint::default(),
    ) {
        let h = 70.0;
        let size = vec2(h * texture.size.x / texture.size.y.max(1.0), h);
        painter.image(
            texture.id,
            Rect::from_center_size(disc, size),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    painter.text(
        c + vec2(0.0, 36.0),
        Align2::CENTER_CENTER,
        "Drop to add to your library",
        theme::font(32.0, "bold"),
        media::TEXT,
    );
    painter.text(
        c + vec2(0.0, 74.0),
        Align2::CENTER_CENTER,
        "Files and whole folders are both fine",
        theme::font(15.0, "regular"),
        media::TEXT_WEAK,
    );
}
