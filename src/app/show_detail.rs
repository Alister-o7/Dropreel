//! The detail panel for a show: its seasons and episodes.

use eframe::egui::{self, Align, Layout, Margin, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2};

use super::detail::{hero, panel_frame, tag_chip};
use super::icons;
use super::library_view::{movie_menu, show_menu, show_poster, show_summary};
use super::theme::{self, media, palette, radius};
use super::widgets::{self, ButtonKind};
use super::{Action, DropReelApp};
use crate::library::{Movie, ShowId};
use crate::util::{format_bytes, format_duration};

const THUMB: Vec2 = vec2(128.0, 72.0);

pub fn panel(app: &mut DropReelApp, ui: &mut egui::Ui, id: ShowId) {
    let Some(show) = app.lib.show(id).cloned() else {
        return;
    };
    panel_frame(app, ui, ("show", id), |app, ui| {
        let p = palette(ui.ctx());
        let lib = &app.lib;
        let episodes: Vec<Movie> = lib.episodes(&show).cloned().collect();
        let all_missing = episodes.iter().all(|m| lib.missing.contains(&m.id));
        let poster = show_poster(lib, &show).map(|p| p.to_path_buf());
        let summary = show_summary(lib, &show);
        // The next episode: its id, code, resume position and whether its file is missing.
        let next = lib.next_episode(&show).map(|(m, pos)| {
            let code = m.episode_code().unwrap_or_default();
            (m.id, m.season, code, pos, lib.missing.contains(&m.id))
        });
        let title = show.show.title.clone();
        hero(app, ui, poster.as_deref(), &title, all_missing);

        egui::Frame::new()
            .inner_margin(Margin {
                left: 22,
                right: 22,
                top: 0,
                bottom: 22,
            })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                ui.add(
                    egui::Label::new(
                        theme::bold(&title)
                            .size(26.0)
                            .color(p.text)
                            .extra_letter_spacing(-0.4),
                    )
                    .wrap(),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    if let Some(y) = show.show.year {
                        widgets::chip(ui, None, &y.to_string());
                    }
                    widgets::chip(ui, Some(icons::TELEVISION), &summary);
                    let size: u64 = episodes.iter().map(|m| m.size).sum();
                    widgets::chip(ui, Some(icons::HARD_DRIVES), &format_bytes(size));
                });

                // Play the next episode.
                ui.horizontal(|ui| {
                    if let Some((_, _, code, resume, missing)) = &next {
                        let label = match resume {
                            Some(pos) => format!("Resume {code} at {}", format_duration(*pos)),
                            None => format!("Play {code}"),
                        };
                        if widgets::button_sized(
                            ui,
                            ButtonKind::Primary,
                            Some(icons::PLAY),
                            &label,
                            120.0,
                            !missing,
                        )
                        .clicked()
                        {
                            app.actions.push(Action::PlayShow(id));
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let more = widgets::icon_button(ui, icons::DOTS_THREE_VERTICAL, "More");
                        egui::Popup::menu(&more).show(|ui| {
                            show_menu(ui, &app.lib, &show, &mut app.actions);
                        });
                        if app.settings.tmdb_enabled()
                            && widgets::icon_button(ui, icons::CLOUD_ARROW_DOWN, "Look up on TMDb")
                                .on_hover_text("Fetch the poster, plot and episode titles")
                                .clicked()
                        {
                            app.actions.push(Action::FetchShowMetadata(id));
                        }
                    });
                });

                if let Some(plot) = &show.show.plot {
                    ui.label(
                        RichText::new(plot)
                            .size(14.0)
                            .color(p.text)
                            .line_height(Some(21.0)),
                    );
                }

                // Seasons: one chip each, starting at the next episode's season.
                let mut seasons: Vec<Option<u32>> = episodes.iter().map(|m| m.season).collect();
                seasons.dedup();
                let season_id = egui::Id::unique(("show-season", id));
                let next_season = next.as_ref().and_then(|(_, season, ..)| *season);
                let mut season = ui
                    .data(|d| d.get_temp::<Option<u32>>(season_id))
                    .filter(|s| seasons.contains(s))
                    .unwrap_or(next_season.or(seasons.first().copied().flatten()));
                if seasons.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for s in &seasons {
                            let name = match s {
                                Some(n) => format!("Season {n}"),
                                None => "Other".to_string(),
                            };
                            if tag_chip(ui, &name, season == *s).clicked() {
                                season = *s;
                            }
                        }
                    });
                    ui.data_mut(|d| d.insert_temp(season_id, season));
                } else {
                    widgets::section_label(ui, "Episodes");
                }

                for movie in episodes.iter().filter(|m| m.season == season) {
                    episode_row(app, ui, movie);
                }

                if !app.lib.tags.is_empty() {
                    widgets::section_label(ui, "Tags");
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for tag in &app.lib.tags {
                            let ids: Vec<_> = episodes.iter().map(|m| m.id).collect();
                            let on = ids.iter().all(|&e| app.lib.has_tag(e, tag.id));
                            if tag_chip(ui, &tag.name, on).clicked() {
                                for e in ids {
                                    app.actions.push(Action::SetTag(e, tag.id, !on));
                                }
                            }
                        }
                    });
                }

                ui.add_space(8.0);
                if widgets::button(
                    ui,
                    ButtonKind::Ghost,
                    Some(icons::TRASH),
                    "Remove show from library",
                )
                .clicked()
                {
                    app.actions.push(Action::AskRemoveShow(id));
                }
            });
    });
}

/// One episode: a frame from it, its number, title and length. Clicking plays it (from where it
/// was left); the menu has the rest.
fn episode_row(app: &mut DropReelApp, ui: &mut egui::Ui, movie: &Movie) {
    let p = palette(ui.ctx());
    let id = movie.id;
    let missing = app.lib.missing.contains(&id);
    let large = movie.size > app.settings.threshold_bytes();
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, THUMB.y + 12.0), Sense::click());
    let code = movie.episode_code().unwrap_or_default();
    let resp = widgets::named(resp, &format!("{code} {}", movie.title));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let menu_rect =
        Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), Vec2::splat(30.0));
    let menu = ui.interact(menu_rect, resp.id.with("menu"), Sense::click());
    let menu = widgets::named(menu, &format!("More: {code}")).on_hover_text("More");
    let hovered = resp.hovered() || menu.hovered();
    if hovered {
        ui.painter().rect_filled(rect, radius::CARD, p.hover);
    }

    let thumb = Rect::from_min_size(rect.min + vec2(6.0, 6.0), THUMB);
    widgets::paint_poster(
        ui,
        thumb,
        movie.poster.as_deref(),
        &movie.title,
        missing,
        radius::BADGE,
    );
    let painter = ui.painter();
    if hovered && !missing {
        widgets::play_disc(painter, thumb.center(), 16.0, 1.0);
    }
    if let (Some(pos), Some(dur)) = (movie.resume_pos, movie.duration)
        && pos > 0.0
        && dur > 0.0
        && !movie.watched
    {
        let bar = Rect::from_min_max(
            thumb.left_bottom() + vec2(6.0, -7.0),
            thumb.right_bottom() + vec2(-6.0, -4.0),
        );
        painter.rect_filled(bar, 1, media::TRACK);
        let mut done = bar;
        done.set_width(bar.width() * (pos / dur).clamp(0.0, 1.0) as f32);
        painter.rect_filled(done, 1, media::PROGRESS);
    }

    let text_left = thumb.right() + 12.0;
    let text_w = (menu_rect.left() - 6.0 - text_left).max(40.0);
    // "3. Title", but just "Episode 3" when the episode has no title of its own.
    let number = movie
        .episode
        .filter(|n| movie.title != format!("Episode {n}"))
        .map(|n| format!("{n}. "))
        .unwrap_or_default();
    let mut job = egui::text::LayoutJob::simple(
        format!("{number}{}", movie.title),
        theme::font(14.0, "semibold"),
        if missing { p.text_weak } else { p.text },
        text_w,
    );
    job.wrap.max_rows = 2;
    let title = ui.fonts_mut(|f| f.layout_job(job));
    let title_h = title.size().y;
    let top = thumb.top() + 4.0;
    ui.painter().galley(pos2(text_left, top), title, p.text);
    let mut sub = Vec::new();
    if missing {
        sub.push(format!("{} File missing", icons::WARNING));
    } else if movie.watched {
        sub.push(format!("{} Watched", icons::CHECK_CIRCLE));
    }
    if let Some(d) = movie.duration {
        sub.push(widgets::short_duration(d));
    }
    if large {
        sub.push(format!("{} {}", icons::LIGHTNING, format_bytes(movie.size)));
    }
    if let Some(badge) = app.encode_badges.get(&id) {
        sub.push(match badge {
            super::EncodeBadge::Running(s) => format!("Re-encoding {:.0}%", s.fraction * 100.0),
            super::EncodeBadge::Queued(_) => "Queued for re-encode".into(),
        });
    }
    let color = if missing { p.danger } else { p.text_weak };
    let mut job = egui::text::LayoutJob::simple_singleline(
        sub.join(" · "),
        theme::font(12.5, "regular"),
        color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter()
        .galley(pos2(text_left, top + title_h + 4.0), galley, color);

    let menu_color = if menu.hovered() { p.text } else { p.text_weak };
    if menu.hovered() {
        ui.painter()
            .circle_filled(menu_rect.center(), 15.0, p.raised);
        ui.painter()
            .circle_stroke(menu_rect.center(), 15.0, Stroke::new(1.0, p.border));
    }
    widgets::icon_centered(
        ui.painter(),
        menu_rect.center(),
        icons::DOTS_THREE_VERTICAL,
        theme::icon_font(18.0),
        menu_color,
    );
    egui::Popup::menu(&menu).show(|ui| {
        movie_menu(ui, movie, missing, large, &mut app.actions);
    });
    if resp.clicked() {
        app.actions.push(if missing {
            Action::ShowDetails(id)
        } else {
            match movie.resume_pos.filter(|_| !movie.watched) {
                Some(pos) => Action::StartPlayback(id, Some(pos)),
                None => Action::Play(id),
            }
        });
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.context_menu(|ui| movie_menu(ui, movie, missing, large, &mut app.actions));
}
