//! The full-window player screen: video, auto-hiding controls, keyboard shortcuts.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, Modifiers, Rect, RichText, Sense, Vec2, pos2, vec2};

use super::icons;
use super::theme;
use super::{Action, DropReelApp};
use crate::library::MovieId;
use crate::player::{Player, PlayerEvent, Track, TrackKind};
use crate::util::format_duration;

/// How often the playback position is saved while playing.
const SAVE_INTERVAL: Duration = Duration::from_secs(10);
/// Controls hide after this long without mouse movement (fullscreen only).
const HIDE_CONTROLS_AFTER: Duration = Duration::from_millis(2500);
/// Height of the control bar. In a window the video sits above it; in fullscreen it overlays.
const BAR_HEIGHT: f32 = 104.0;
/// Fraction of the movie after which it counts as watched.
pub const WATCHED_FRACTION: f64 = 0.95;

pub struct NowPlaying {
    pub movie_id: MovieId,
    pub title: String,
    /// Duration from the library, used until mpv knows it.
    pub known_duration: Option<f64>,
    last_save: Instant,
    last_activity: Instant,
    controls_hovered: bool,
    seek_drag: Option<f64>,
    tracks: Vec<Track>,
    loaded: bool,
    pub watched_marked: bool,
}

impl NowPlaying {
    pub fn new(movie_id: MovieId, title: String, known_duration: Option<f64>) -> Self {
        Self {
            movie_id,
            title,
            known_duration,
            last_save: Instant::now(),
            last_activity: Instant::now(),
            controls_hovered: false,
            seek_drag: None,
            tracks: Vec::new(),
            loaded: false,
            watched_marked: false,
        }
    }
}

/// Where to save the resume position when playback stops at `pos` of `dur`:
/// `None` clears it (finished, or barely started).
pub fn resume_position(pos: Option<f64>, dur: Option<f64>) -> Option<f64> {
    let pos = pos?;
    if pos < 5.0 {
        return None;
    }
    match dur {
        Some(d) if pos >= d * WATCHED_FRACTION => None,
        _ => Some(pos),
    }
}

/// Whether to offer "Resume from …" for a saved position.
pub fn should_offer_resume(resume: Option<f64>, dur: Option<f64>) -> Option<f64> {
    let pos = resume.filter(|p| *p >= 30.0)?;
    match dur {
        Some(d) if pos >= d * WATCHED_FRACTION => None,
        _ => Some(pos),
    }
}

fn is_fullscreen(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
}

pub fn toggle_fullscreen(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fullscreen(ctx)));
}

pub fn show(app: &mut DropReelApp, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
    let ctx = ui.ctx().clone();
    let DropReelApp {
        player,
        now_playing,
        actions,
        db,
        ..
    } = app;
    let (Ok(player), Some(np)) = (player.as_mut(), now_playing.as_mut()) else {
        actions.push(Action::StopPlayback { finished: false });
        return;
    };

    for event in player.poll_events() {
        match event {
            PlayerEvent::FileLoaded => {
                np.loaded = true;
                np.tracks = player.tracks();
            }
            PlayerEvent::Error(e) => {
                actions.push(Action::PlaybackFailed(format!(
                    "Could not play \"{}\": {e}",
                    np.title
                )));
                return;
            }
        }
    }

    let pos = player.position();
    let dur = player.duration().or(np.known_duration);
    if np.loaded {
        if np.last_save.elapsed() >= SAVE_INTERVAL {
            np.last_save = Instant::now();
            if let Some(p) = resume_position(pos, dur)
                && let Err(e) = db.set_resume(np.movie_id, Some(p))
            {
                tracing::warn!("could not save position: {e}");
            }
        }
        if !np.watched_marked
            && let (Some(p), Some(d)) = (pos, dur)
            && p >= d * WATCHED_FRACTION
        {
            np.watched_marked = true;
            let _ = db.set_watched(np.movie_id, true);
        }
        // Wait for the mouse to be released so dragging to the end doesn't exit mid-drag.
        if player.eof_reached() && np.seek_drag.is_none() && !ctx.input(|i| i.pointer.any_down()) {
            actions.push(Action::StopPlayback { finished: true });
        }
    }

    handle_keys(&ctx, player, actions);

    let fullscreen = is_fullscreen(&ctx);
    if ctx.input(|i| {
        i.pointer.delta() != Vec2::ZERO || i.pointer.any_down() || !i.keys_down.is_empty()
    }) {
        np.last_activity = Instant::now();
    }
    let paused = player.paused();
    let recently_active =
        paused || np.controls_hovered || np.last_activity.elapsed() < HIDE_CONTROLS_AFTER;
    // In a window the bar has its own space below the video and stays visible.
    let show_controls = !fullscreen || recently_active;

    let panel_frame = egui::Frame::NONE.fill(Color32::BLACK);
    egui::CentralPanel::no_frame()
        .frame(panel_frame)
        .show(ui, |ui| {
            let full = ui.max_rect();
            let rect = if fullscreen {
                full
            } else {
                Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - BAR_HEIGHT))
            };
            let ppp = ctx.pixels_per_point();
            let size = [
                (rect.width() * ppp).round() as i32,
                (rect.height() * ppp).round() as i32,
            ];
            if let Some(gl) = frame.gl().cloned()
                && let Some(texture) = player.render(&gl, frame, size)
            {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(texture, rect, uv, Color32::WHITE);
            }
            let video = ui.interact(rect, ui.scope_id().with("video"), Sense::click());
            if video.double_clicked() {
                toggle_fullscreen(&ctx);
            }
            if !np.loaded {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Loading…",
                    egui::FontId::proportional(20.0),
                    Color32::GRAY,
                );
            }

            if show_controls {
                controls(
                    ui,
                    full,
                    player,
                    np,
                    actions,
                    pos,
                    dur,
                    paused,
                    fullscreen,
                    recently_active,
                );
            } else {
                np.controls_hovered = false;
            }
            if !recently_active && ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p)) {
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }
        });

    // Keep the clock and auto-hide timer ticking even when no new video frame arrives.
    ctx.request_repaint_after(Duration::from_millis(250));
}

fn handle_keys(ctx: &egui::Context, player: &mut Player, actions: &mut Vec<Action>) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Space) {
            let paused = player.paused();
            player.set_paused(!paused);
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            player.seek_relative(-10.0);
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
            player.seek_relative(10.0);
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            let v = player.volume();
            player.set_volume(v + 5.0);
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            let v = player.volume();
            player.set_volume(v - 5.0);
        }
        if i.consume_key(Modifiers::NONE, Key::M) {
            let m = player.muted();
            player.set_muted(!m);
        }
    });
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F)) {
        toggle_fullscreen(ctx);
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        if is_fullscreen(ctx) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        } else {
            actions.push(Action::StopPlayback { finished: false });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn controls(
    ui: &mut egui::Ui,
    rect: Rect,
    player: &mut Player,
    np: &mut NowPlaying,
    actions: &mut Vec<Action>,
    pos: Option<f64>,
    dur: Option<f64>,
    paused: bool,
    fullscreen: bool,
    show_title: bool,
) {
    let painter = ui.painter().clone();
    let top = Rect::from_min_max(rect.left_top(), pos2(rect.right(), rect.top() + 44.0));
    let bar = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() - BAR_HEIGHT),
        rect.right_bottom(),
    );
    let bar_alpha = if fullscreen { 180 } else { 255 };
    painter.rect_filled(
        bar,
        0.0,
        Color32::from_rgba_unmultiplied(12, 12, 12, bar_alpha),
    );
    if show_title {
        painter.rect_filled(top, 0.0, Color32::from_black_alpha(150));
        painter.text(
            top.left_center() + vec2(20.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &np.title,
            theme::font(18.0, "semibold"),
            Color32::WHITE,
        );
    }
    np.controls_hovered = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|p| bar.contains(p) || (show_title && top.contains(p)));

    ui.scope_builder(
        egui::UiBuilder::new().max_rect(bar.shrink2(vec2(16.0, 10.0))),
        |ui| {
            *ui.visuals_mut() = egui::Visuals::dark();
            ui.spacing_mut().interact_size.y = 26.0;

            // Seek bar
            let duration = dur.unwrap_or(0.0);
            let shown = np.seek_drag.or(pos).unwrap_or(0.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format_duration(shown))
                        .monospace()
                        .color(Color32::WHITE),
                );
                let time_w = 70.0;
                ui.spacing_mut().slider_width = (ui.available_width() - time_w).max(50.0);
                let mut value = shown;
                let slider = egui::Slider::new(&mut value, 0.0..=duration.max(0.1))
                    .show_value(false)
                    .trailing_fill(true);
                let r = ui
                    .add_enabled(duration > 0.0, slider)
                    .interact(Sense::click());
                // Fast keyframe seeks while the button is held, one exact seek on release. A quick
                // click may only report its new value on the release frame.
                if r.is_pointer_button_down_on() {
                    if r.changed() {
                        np.seek_drag = Some(value);
                        player.seek(value, false);
                    }
                } else if r.changed() {
                    np.seek_drag = None;
                    player.seek(value, true);
                } else if let Some(target) = np.seek_drag.take() {
                    player.seek(target, true);
                } else if r.clicked()
                    && let Some(p) = r
                        .interact_pointer_pos()
                        .or_else(|| ui.ctx().pointer_interact_pos())
                {
                    // Press and release in the same frame: egui never saw the button down, so
                    // map the click position ourselves (same geometry as egui's slider).
                    let track = r.rect.x_range().shrink(r.rect.height() / 2.5);
                    let t = ((p.x - track.min) / track.span()).clamp(0.0, 1.0) as f64;
                    player.seek(t * duration, true);
                }
                ui.label(
                    RichText::new(format_duration(duration))
                        .monospace()
                        .color(Color32::WHITE),
                );
            });

            // Buttons
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if player_button(ui, icons::ARROW_LEFT, 22.0, "Back to the library (Esc)").clicked()
                {
                    actions.push(Action::StopPlayback { finished: false });
                }
                ui.add_space(10.0);
                if player_button(ui, icons::REWIND, 22.0, "Back 10 seconds (←)").clicked() {
                    player.seek_relative(-10.0);
                }
                if play_button(ui, paused).clicked() {
                    player.set_paused(!paused);
                }
                if player_button(ui, icons::FAST_FORWARD, 22.0, "Forward 10 seconds (→)").clicked()
                {
                    player.seek_relative(10.0);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (fs_icon, fs_tip) = if fullscreen {
                        (icons::CORNERS_IN, "Exit fullscreen (F)")
                    } else {
                        (icons::CORNERS_OUT, "Fullscreen (F)")
                    };
                    if player_button(ui, fs_icon, 22.0, fs_tip).clicked() {
                        toggle_fullscreen(ui.ctx());
                    }
                    ui.add_space(4.0);
                    track_menu(ui, player, np, TrackKind::Subtitle);
                    track_menu(ui, player, np, TrackKind::Audio);
                    ui.add_space(8.0);
                    let mut volume = player.volume();
                    ui.spacing_mut().slider_width = 96.0;
                    if ui
                        .add(egui::Slider::new(&mut volume, 0.0..=130.0).show_value(false))
                        .on_hover_text(format!("Volume {volume:.0}%"))
                        .changed()
                    {
                        player.set_volume(volume);
                    }
                    let muted = player.muted();
                    let icon = if muted || volume < 1.0 {
                        icons::SPEAKER_SLASH
                    } else {
                        icons::SPEAKER_HIGH
                    };
                    if player_button(ui, icon, 21.0, "Mute (M)").clicked() {
                        player.set_muted(!muted);
                    }
                });
            });
        },
    );
}

/// A round, white icon button for the dark player bar.
fn player_button(ui: &mut egui::Ui, icon: &str, size: f32, tooltip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    ui.painter().circle_filled(
        rect.center(),
        19.0,
        Color32::from_white_alpha((28.0 * t) as u8),
    );
    let color = Color32::from_white_alpha((200.0 + 55.0 * t) as u8);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        theme::icon_font(size),
        color,
    );
    resp.on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn play_button(ui: &mut egui::Ui, paused: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let accent = theme::palette(ui.ctx()).accent;
    ui.painter().circle_filled(
        rect.center(),
        23.0,
        accent.lerp_to_gamma(Color32::WHITE, 0.15 * t),
    );
    let (icon, nudge) = if paused {
        (icons::PLAY, 2.0)
    } else {
        (icons::PAUSE, 0.0)
    };
    ui.painter().text(
        rect.center() + vec2(nudge, 0.0),
        egui::Align2::CENTER_CENTER,
        icon,
        theme::icon_fill_font(22.0),
        Color32::WHITE,
    );
    resp.on_hover_text("Play/pause (Space)")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn track_menu(ui: &mut egui::Ui, player: &mut Player, np: &mut NowPlaying, kind: TrackKind) {
    let tracks: Vec<&Track> = np.tracks.iter().filter(|t| t.kind == kind).collect();
    let (name, icon, allow_off) = match kind {
        TrackKind::Audio => ("Audio", icons::MUSIC_NOTES, false),
        _ => ("Subtitles", icons::SUBTITLES, true),
    };
    if tracks.is_empty() && kind == TrackKind::Audio {
        return;
    }
    let selected = tracks
        .iter()
        .find(|t| t.selected)
        .map_or("Off".to_string(), |t| t.label());
    let mut choice: Option<Option<i64>> = None;
    egui::ComboBox::from_id_salt(name)
        .selected_text(format!("{icon}  {selected}"))
        .width(190.0)
        .show_ui(ui, |ui| {
            if allow_off
                && ui
                    .selectable_label(tracks.iter().all(|t| !t.selected), "Off")
                    .clicked()
            {
                choice = Some(None);
            }
            for t in &tracks {
                if ui.selectable_label(t.selected, t.label()).clicked() {
                    choice = Some(Some(t.id));
                }
            }
            if tracks.is_empty() {
                ui.weak("No subtitles in this file");
            }
        });
    if let Some(id) = choice {
        player.select_track(kind, id);
        np.tracks = player.tracks();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_position_rules() {
        assert_eq!(resume_position(Some(600.0), Some(7200.0)), Some(600.0));
        assert_eq!(
            resume_position(Some(3.0), Some(7200.0)),
            None,
            "barely started"
        );
        assert_eq!(
            resume_position(Some(7000.0), Some(7200.0)),
            None,
            "finished (≥95%)"
        );
        assert_eq!(resume_position(Some(600.0), None), Some(600.0));
        assert_eq!(resume_position(None, Some(7200.0)), None);
    }

    #[test]
    fn resume_is_offered_only_in_the_middle() {
        assert_eq!(
            should_offer_resume(Some(4354.0), Some(7200.0)),
            Some(4354.0)
        );
        assert_eq!(should_offer_resume(Some(10.0), Some(7200.0)), None);
        assert_eq!(should_offer_resume(Some(7100.0), Some(7200.0)), None);
        assert_eq!(should_offer_resume(None, None), None);
    }
}
