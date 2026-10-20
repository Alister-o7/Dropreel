//! The movie detail panel on the right.

use eframe::egui::{self, Align, Layout, RichText, vec2};

use super::library_view::{movie_menu, video_summary};
use super::widgets::{self, DANGER};
use super::{Action, DropReelApp};
use crate::util::{format_bytes, format_date, format_duration};

pub fn panel(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let Some(movie) = app.selected.and_then(|id| app.lib.get(id)).cloned() else {
        return;
    };
    let id = movie.id;
    let missing = app.lib.missing.contains(&id);
    egui::Panel::right("details")
        .default_size(340.0)
        .size_range(280.0..=560.0)
        .resizable(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.strong("Details");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("✖").on_hover_text("Close").clicked() {
                                app.actions.push(Action::CloseDetails);
                            }
                            ui.menu_button("…", |ui| {
                                movie_menu(ui, &movie, missing, &mut app.actions)
                            });
                        });
                    });
                    ui.add_space(6.0);

                    let width = ui.available_width();
                    let height = (width * 1.5).min(440.0);
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
                    let poster_rect =
                        egui::Rect::from_center_size(rect.center(), vec2(height / 1.5, height));
                    widgets::paint_poster(
                        ui,
                        poster_rect,
                        movie.poster.as_deref(),
                        &movie.title,
                        missing,
                    );
                    ui.add_space(8.0);

                    ui.label(RichText::new(&movie.title).size(22.0).strong());
                    if let Some(year) = movie.year {
                        ui.weak(year.to_string());
                    }
                    ui.add_space(6.0);

                    if missing {
                        egui::Frame::new()
                            .fill(DANGER.gamma_multiply(0.25))
                            .corner_radius(6.0)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.colored_label(
                                    DANGER,
                                    RichText::new("⚠ File not found").strong(),
                                );
                                ui.label(format!("It used to be at {}", movie.path.display()));
                                ui.horizontal(|ui| {
                                    if ui.button("🔍 Locate…").clicked() {
                                        app.actions.push(Action::Locate(id));
                                    }
                                    if ui.button("Rescan watched folders").clicked() {
                                        app.actions.push(Action::RescanWatched);
                                    }
                                });
                            });
                        ui.add_space(6.0);
                    }

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !missing,
                                egui::Button::new(RichText::new("▶ Play").size(16.0)),
                            )
                            .clicked()
                        {
                            app.actions.push(Action::Play(id));
                        }
                        if ui.button("📂 Show in folder").clicked() {
                            app.actions.push(Action::OpenFolder(id));
                        }
                    });
                    ui.add_space(8.0);

                    if let Some(plot) = &movie.plot {
                        ui.label(plot);
                        ui.add_space(8.0);
                    }

                    egui::Grid::new("detail_info")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.weak("Size");
                            ui.label(format_bytes(movie.size));
                            ui.end_row();
                            ui.weak("Length");
                            ui.label(
                                movie
                                    .duration
                                    .map(format_duration)
                                    .unwrap_or_else(|| "unknown".into()),
                            );
                            ui.end_row();
                            ui.weak("Video");
                            ui.label(video_summary(&movie));
                            ui.end_row();
                            ui.weak("Added");
                            ui.label(format_date(movie.date_added));
                            ui.end_row();
                            ui.weak("Last played");
                            ui.label(
                                movie
                                    .last_played
                                    .map(format_date)
                                    .unwrap_or_else(|| "never".into()),
                            );
                            ui.end_row();
                            ui.weak("Watched");
                            let mut watched = movie.watched;
                            if ui.checkbox(&mut watched, "").changed() {
                                app.actions.push(Action::SetWatched(id, watched));
                            }
                            ui.end_row();
                        });
                    ui.add_space(6.0);

                    let tracks =
                        |ui: &mut egui::Ui, title: &str, list: &[crate::library::TrackInfo]| {
                            egui::CollapsingHeader::new(format!("{title} ({})", list.len()))
                                .id_salt(title)
                                .show(ui, |ui| {
                                    if list.is_empty() {
                                        ui.weak("none");
                                    }
                                    for (i, t) in list.iter().enumerate() {
                                        ui.label(format!("{}. {}", i + 1, t.describe()));
                                    }
                                });
                        };
                    tracks(ui, "Audio tracks", &movie.audio_tracks);
                    tracks(ui, "Subtitles", &movie.subtitle_tracks);
                    egui::CollapsingHeader::new("File").show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(movie.path.display().to_string())
                                    .monospace()
                                    .small(),
                            )
                            .wrap(),
                        );
                    });

                    ui.add_space(8.0);
                    ui.strong("Tags");
                    ui.horizontal_wrapped(|ui| {
                        for tag in &app.lib.tags {
                            let mut on = app.lib.has_tag(id, tag.id);
                            if ui.toggle_value(&mut on, &tag.name).changed() {
                                app.actions.push(Action::SetTag(id, tag.id, on));
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut app.dialogs.new_tag)
                                .hint_text("New tag")
                                .desired_width(150.0),
                        );
                        let enter =
                            resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let name = app.dialogs.new_tag.trim().to_string();
                        if (ui
                            .add_enabled(!name.is_empty(), egui::Button::new("+ Add"))
                            .clicked()
                            || enter)
                            && !name.is_empty()
                        {
                            app.actions.push(Action::CreateTag(id, name));
                            app.dialogs.new_tag.clear();
                        }
                    });

                    ui.add_space(8.0);
                    ui.strong("Metadata");
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("✏ Edit title…").clicked() {
                            app.actions.push(Action::EditTitle(id));
                        }
                        let tmdb = ui
                            .add_enabled(
                                app.settings.tmdb_enabled(),
                                egui::Button::new("🔍 Search TMDb…"),
                            )
                            .on_disabled_hover_text(
                                "Add a TMDb API key in Settings to look up posters and plots.",
                            );
                        if tmdb.clicked() {
                            app.actions.push(Action::OpenTmdbSearch(id));
                        }
                    });
                    ui.add_space(12.0);
                    if ui.button("🗑 Remove from library…").clicked() {
                        app.actions.push(Action::AskRemove(id));
                    }
                    ui.add_space(12.0);
                });
        });
}
