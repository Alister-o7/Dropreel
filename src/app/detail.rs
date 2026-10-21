//! The movie detail panel on the right.

use eframe::egui::{
    self, Align, Align2, Color32, Layout, Margin, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2,
};

use super::icons;
use super::library_view::{encode_progress_text, movie_menu};
use super::theme::{self, palette};
use super::widgets::{self, ButtonKind};
use super::{Action, DropReelApp, EncodeBadge};
use crate::util::{format_bytes, format_date, format_duration};

const HERO_H: f32 = 230.0;

pub fn panel(app: &mut DropReelApp, ui: &mut egui::Ui) {
    let Some(movie) = app.selected.and_then(|id| app.lib.get(id)).cloned() else {
        return;
    };
    let p = palette(ui.ctx());
    let id = movie.id;
    let missing = app.lib.missing.contains(&id);
    let large = movie.size > app.settings.threshold_bytes();
    let encode = app.encode_badges.get(&id).cloned();
    let frame = egui::Frame::new().fill(p.surface).inner_margin(0);
    egui::Panel::right("details").default_size(380.0).size_range(320.0..=560.0).resizable(true).frame(frame).show(ui, |ui| {
        // One scroll position per movie, so switching movies starts at the top.
        egui::ScrollArea::vertical().id_salt(("details", id)).auto_shrink([false, false]).show(ui, |ui| {
            // Hero image fading into the panel.
            let (hero, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_H), Sense::hover());
            widgets::paint_poster(ui, hero, movie.poster.as_deref(), &movie.title, missing, 0);
            let fade = Rect::from_min_max(pos2(hero.left(), hero.top() + HERO_H * 0.35), hero.right_bottom());
            widgets::vertical_gradient(ui.painter(), fade, Color32::TRANSPARENT, p.surface);
            let close = Rect::from_center_size(hero.right_top() + vec2(-26.0, 26.0), Vec2::splat(34.0));
            let close_resp = widgets::named(ui.interact(close, ui.scope_id().with("close"), Sense::click()), "Close").on_hover_text("Close");
            let bg = if close_resp.hovered() { 210 } else { 150 };
            ui.painter().circle_filled(close.center(), 17.0, Color32::from_black_alpha(bg));
            ui.painter().text(close.center(), Align2::CENTER_CENTER, icons::X, theme::icon_font(18.0), Color32::WHITE);
            if close_resp.clicked() {
                app.actions.push(Action::CloseDetails);
            }

            egui::Frame::new().inner_margin(Margin { left: 22, right: 22, top: 0, bottom: 22 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.add(egui::Label::new(theme::bold(&movie.title).size(25.0).color(p.text)).wrap());

                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    if let Some(y) = movie.year {
                        widgets::chip(ui, None, &y.to_string());
                    }
                    if let Some(d) = movie.duration {
                        widgets::chip(ui, Some(icons::TIMER), &widgets::short_duration(d));
                    }
                    if let Some(r) = widgets::resolution_label(movie.width, movie.height) {
                        widgets::chip(ui, Some(icons::MONITOR_PLAY), r);
                    }
                    if let Some(c) = &movie.video_codec {
                        widgets::chip(ui, None, &c.to_uppercase());
                    }
                    widgets::chip(ui, Some(icons::HARD_DRIVES), &format_bytes(movie.size));
                });

                // Primary actions
                ui.horizontal(|ui| {
                    let resume = movie.resume_pos.filter(|_| !movie.watched && !missing);
                    match resume {
                        Some(pos) => {
                            if widgets::button(ui, ButtonKind::Primary, Some(icons::PLAY), &format!("Resume {}", format_duration(pos))).clicked() {
                                app.actions.push(Action::StartPlayback(id, Some(pos)));
                            }
                            if widgets::named(widgets::button(ui, ButtonKind::Secondary, Some(icons::ARROW_COUNTER_CLOCKWISE), ""), "Start over").on_hover_text("Start over").clicked() {
                                app.actions.push(Action::StartPlayback(id, None));
                            }
                        }
                        None => {
                            if widgets::button_sized(ui, ButtonKind::Primary, Some(icons::PLAY), "Play", 120.0, !missing).clicked() {
                                app.actions.push(Action::Play(id));
                            }
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.menu_button(RichText::new(icons::DOTS_THREE_VERTICAL).size(19.0), |ui| {
                            movie_menu(ui, &movie, missing, large, &mut app.actions);
                        });
                        if widgets::icon_button(ui, icons::PENCIL, "Edit title").clicked() {
                            app.actions.push(Action::EditTitle(id));
                        }
                        if app.settings.tmdb_enabled() && widgets::icon_button(ui, icons::CLOUD_ARROW_DOWN, "Find on TMDb").clicked() {
                            app.actions.push(Action::OpenTmdbSearch(id));
                        }
                        if widgets::icon_button(ui, icons::FOLDER_OPEN, "Show in folder").clicked() {
                            app.actions.push(Action::OpenFolder(id));
                        }
                    });
                });

                if missing {
                    widgets::callout(ui, p.danger, icons::WARNING_CIRCLE, "File not found", |ui| {
                        ui.label(RichText::new(format!("It used to be at {}", movie.path.display())).size(12.5).color(p.text_weak));
                        ui.horizontal(|ui| {
                            if widgets::button(ui, ButtonKind::Secondary, Some(icons::CROSSHAIR), "Locate…").clicked() {
                                app.actions.push(Action::Locate(id));
                            }
                            if widgets::button(ui, ButtonKind::Ghost, Some(icons::ARROW_CLOCKWISE), "Rescan folders").clicked() {
                                app.actions.push(Action::RescanWatched);
                            }
                        });
                    });
                }

                match &encode {
                    Some(EncodeBadge::Running(status)) => {
                        widgets::callout(ui, p.accent, icons::LIGHTNING, status.stage.label(), |ui| {
                            widgets::progress_bar(ui, status.fraction, p.accent, 8.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(encode_progress_text(status)).color(p.text_weak));
                                if let Some(enc) = status.encoder {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let icon = if enc == crate::encode::Encoder::X265 { icons::CPU } else { icons::GRAPHICS_CARD };
                                        ui.label(RichText::new(format!("{icon} {}", enc.label())).size(12.0).color(p.text_weak));
                                    });
                                }
                            });
                            if widgets::button(ui, ButtonKind::Secondary, Some(icons::X), "Cancel").clicked() {
                                app.actions.push(Action::CancelEncode(id));
                            }
                        });
                    }
                    Some(EncodeBadge::Queued(n)) => {
                        widgets::callout(ui, p.accent, icons::HOURGLASS, "Queued for re-encoding", |ui| {
                            ui.label(RichText::new(format!("{} ahead of it in the queue.", n)).color(p.text_weak));
                            if widgets::button(ui, ButtonKind::Secondary, Some(icons::X), "Remove from queue").clicked() {
                                app.actions.push(Action::CancelEncode(id));
                            }
                        });
                    }
                    None if large && !missing => {
                        let title = format!("This file is {}", format_bytes(movie.size));
                        widgets::callout(ui, p.warn, icons::LIGHTNING, &title, |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "Re-encode it to HEVC to bring it under {} GB. Audio and subtitles are kept as they are, and the original is only touched after the new file checks out.",
                                    app.settings.threshold_gb
                                ))
                                .color(p.text_weak),
                            );
                            let can = app.tools.ffmpeg.is_some() && app.tools.ffprobe.is_some();
                            let resp = widgets::button_sized(ui, ButtonKind::Warn, Some(icons::LIGHTNING), "Re-encode to HEVC…", 0.0, can);
                            if !can {
                                resp.on_hover_text("ffmpeg and ffprobe are needed — see Settings");
                            } else if resp.clicked() {
                                app.actions.push(Action::AskReencode(id));
                            }
                        });
                    }
                    None => {}
                }

                if let Some(plot) = &movie.plot {
                    ui.label(RichText::new(plot).size(14.0).color(p.text).line_height(Some(21.0)));
                }

                widgets::section_label(ui, "Details");
                egui::Grid::new("detail_info").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                    let row = |ui: &mut egui::Ui, icon: &str, label: &str, value: String| {
                        ui.label(RichText::new(format!("{icon}  {label}")).color(p.text_weak));
                        ui.add(egui::Label::new(RichText::new(value).color(p.text)).wrap());
                        ui.end_row();
                    };
                    let video = match (&movie.video_codec, movie.width, movie.height) {
                        (Some(c), Some(w), Some(h)) => format!("{} · {w}×{h}", c.to_uppercase()),
                        (Some(c), _, _) => c.to_uppercase(),
                        _ => "unknown".into(),
                    };
                    row(ui, icons::MONITOR_PLAY, "Video", video);
                    row(ui, icons::MUSIC_NOTES, "Audio", tracks_text(&movie.audio_tracks));
                    row(ui, icons::SUBTITLES, "Subtitles", tracks_text(&movie.subtitle_tracks));
                    row(ui, icons::PLUS, "Added", format_date(movie.date_added));
                    row(ui, icons::CLOCK_COUNTER_CLOCKWISE, "Last played", movie.last_played.map(format_date).unwrap_or_else(|| "never".into()));
                    ui.label(RichText::new(format!("{}  Watched", icons::CHECK_CIRCLE)).color(p.text_weak));
                    let mut watched = movie.watched;
                    if ui.checkbox(&mut watched, "").changed() {
                        app.actions.push(Action::SetWatched(id, watched));
                    }
                    ui.end_row();
                });
                ui.add(egui::Label::new(RichText::new(movie.path.display().to_string()).size(11.5).color(p.text_weak).monospace()).wrap())
                    .on_hover_text("File location");

                widgets::section_label(ui, "Tags");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for tag in &app.lib.tags {
                        let on = app.lib.has_tag(id, tag.id);
                        if tag_chip(ui, &tag.name, on).clicked() {
                            app.actions.push(Action::SetTag(id, tag.id, !on));
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut app.dialogs.new_tag)
                            .hint_text(format!("{} New tag", icons::TAG))
                            .desired_width(ui.available_width() - 90.0)
                            .margin(Margin::symmetric(10, 6)),
                    );
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let name = app.dialogs.new_tag.trim().to_string();
                    let add = widgets::button_sized(ui, ButtonKind::Secondary, Some(icons::PLUS), "Add", 0.0, !name.is_empty()).clicked();
                    if (add || enter) && !name.is_empty() {
                        app.actions.push(Action::CreateTag(id, name));
                        app.dialogs.new_tag.clear();
                    }
                });

                ui.add_space(8.0);
                if widgets::button(ui, ButtonKind::Ghost, Some(icons::TRASH), "Remove from library").clicked() {
                    app.actions.push(Action::AskRemove(id));
                }
            });
        });
    });
}

fn tracks_text(tracks: &[crate::library::TrackInfo]) -> String {
    if tracks.is_empty() {
        return "none".into();
    }
    tracks
        .iter()
        .map(|t| t.describe())
        .collect::<Vec<_>>()
        .join("\n")
}

fn tag_chip(ui: &mut egui::Ui, name: &str, on: bool) -> egui::Response {
    let p = palette(ui.ctx());
    let label = if on {
        format!("{} {name}", icons::CHECK)
    } else {
        name.to_string()
    };
    let fg = if on { Color32::WHITE } else { p.text };
    let galley = ui
        .painter()
        .layout_no_wrap(label, theme::font(13.0, "medium"), fg);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(20.0, 10.0), Sense::click());
    let fill = if on {
        p.accent
    } else if resp.hovered() {
        p.hover
    } else {
        p.raised
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(255), fill);
    if !on {
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(255),
            Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().galley(rect.min + vec2(10.0, 5.0), galley, fg);
    widgets::named_selectable(resp, name, on).on_hover_cursor(egui::CursorIcon::PointingHand)
}
