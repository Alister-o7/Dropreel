//! The full-window player screen: video, auto-hiding controls, keyboard shortcuts.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Key, Modifiers, Rect, RichText, Sense, Vec2, pos2, vec2};

use super::icons;
use super::theme::{self, media};
use super::widgets;
use super::{Action, DropReelApp};
use crate::library::MovieId;
use crate::player::{Player, PlayerEvent, RefreshRate, Track, TrackKind};
use crate::util::format_duration;

/// How often the playback position is saved while playing.
const SAVE_INTERVAL: Duration = Duration::from_secs(10);
/// Controls hide after this long without mouse movement (fullscreen only).
const HIDE_CONTROLS_AFTER: Duration = Duration::from_millis(2500);
/// How long after a file loads its playback stats are first logged.
const DIAGNOSTICS_AFTER: Duration = Duration::from_secs(20);
/// Height of the control bar. In a window the video sits above it; in fullscreen it overlays.
pub const BAR_HEIGHT: f32 = 104.0;
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
    refresh: RefreshRate,
    /// When to write the first playback stats to the log (once decoding has settled).
    diagnostics_at: Option<Instant>,
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
            refresh: RefreshRate::default(),
            diagnostics_at: None,
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
                np.diagnostics_at = Some(Instant::now() + DIAGNOSTICS_AFTER);
            }
            PlayerEvent::TracksChanged => np.tracks = player.tracks(),
            PlayerEvent::Error(e) => {
                actions.push(Action::PlaybackFailed(format!(
                    "Could not play \"{}\": {e}",
                    np.title
                )));
                return;
            }
        }
    }

    player.set_display_fps(np.refresh.poll(frame));
    if np.diagnostics_at.is_some_and(|t| Instant::now() >= t) {
        np.diagnostics_at = None;
        tracing::info!("playback stats: {}", player.diagnostics());
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
    // The controls sit over the video and fade out when the mouse rests, windowed or not.
    // They stay laid out while faded, so a click right after the mouse moves still lands.
    let controls_opacity =
        ctx.animate_bool_with_time(egui::Id::unique("player_controls"), recently_active, 0.25);

    let panel_frame = egui::Frame::NONE.fill(theme::ramp::N950);
    egui::CentralPanel::no_frame()
        .frame(panel_frame)
        .show(ui, |ui| {
            // The video fills the window; the controls float over it.
            let rect = ui.max_rect();
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

            controls(
                ui,
                rect,
                player,
                np,
                actions,
                pos,
                dur,
                paused,
                fullscreen,
                controls_opacity,
            );
            if !recently_active && ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p)) {
                ctx.set_cursor_icon(egui::CursorIcon::None);
            }
        });

    // Keep the clock and auto-hide timer ticking even when no new video frame arrives.
    if np.loaded && !paused {
        // Draw every display refresh while playing: mpv gets a render call (and a swap report)
        // each vsync, which keeps frame pacing even.
        ctx.request_repaint();
    } else {
        // Keep the clock and auto-hide timer ticking while paused.
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn handle_keys(ctx: &egui::Context, player: &mut Player, actions: &mut Vec<Action>) {
    if ctx.text_edit_focused() {
        return;
    }
    // A player button or menu keeps keyboard focus after a mouse click. Drop it, so Space
    // doesn't also "press" the focused button and the arrow keys don't move focus around.
    if let Some(id) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(id));
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
    // With a track menu open, Esc just closes the menu (egui handles that).
    if !egui::Popup::is_any_open(ctx)
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
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
    opacity: f32,
) {
    ui.multiply_opacity(opacity);
    let painter = ui.painter().clone();
    let top = Rect::from_min_max(rect.left_top(), pos2(rect.right(), rect.top() + 56.0));
    let bar = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() - BAR_HEIGHT),
        rect.right_bottom(),
    );
    // Shade the top and bottom of the picture so the title and controls read on any frame.
    widgets::vertical_gradient(
        &painter,
        Rect::from_min_max(top.left_top(), pos2(top.right(), top.bottom() + 40.0)),
        media::SCRIM_STRONG,
        Color32::TRANSPARENT,
    );
    widgets::vertical_gradient(
        &painter,
        Rect::from_min_max(pos2(bar.left(), bar.top() - 90.0), bar.right_bottom()),
        Color32::TRANSPARENT,
        media::SCRIM_STRONG,
    );
    painter.text(
        top.left_center() + vec2(24.0, 0.0),
        egui::Align2::LEFT_CENTER,
        &np.title,
        theme::font(20.0, "bold"),
        media::TEXT,
    );
    np.controls_hovered = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|p| bar.contains(p) || top.contains(p));

    ui.scope_builder(
        egui::UiBuilder::new().max_rect(bar.shrink2(vec2(16.0, 10.0))),
        |ui| {
            // The bar is always dark, whatever the theme.
            *ui.visuals_mut() = theme::visuals(&theme::DARK);
            ui.visuals_mut().selection.bg_fill = media::PROGRESS;
            ui.visuals_mut().widgets.inactive.bg_fill = media::TRACK;
            ui.spacing_mut().interact_size.y = 26.0;

            // Seek bar
            let duration = dur.unwrap_or(0.0);
            let shown = np.seek_drag.or(pos).unwrap_or(0.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format_duration(shown))
                        .monospace()
                        .color(media::TEXT),
                );
                let time_w = 70.0;
                let width = (ui.available_width() - time_w).max(50.0);
                let (value, r) = line_slider(ui, width, shown, duration, "Seek");
                // Fast keyframe seeks while the button is held, one exact seek on release (or
                // on a click that pressed and released within one frame).
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
                }
                ui.label(
                    RichText::new(format_duration(duration))
                        .monospace()
                        .color(media::TEXT),
                );
            });

            // Buttons, in a row as tall as the play button from the start, so every button is
            // centred on the same line (a plain horizontal row centres early items on its height
            // so far).
            let row = vec2(ui.available_width(), 46.0);
            ui.allocate_ui_with_layout(
                row,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if player_button(ui, icons::ARROW_LEFT, 22.0, "Back to the library (Esc)")
                        .clicked()
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
                    if player_button(ui, icons::FAST_FORWARD, 22.0, "Forward 10 seconds (→)")
                        .clicked()
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
                        let (volume, r) = line_slider(ui, 96.0, player.volume(), 130.0, "Volume");
                        if r.on_hover_text(format!("Volume {volume:.0}%")).changed() {
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
                },
            );
        },
    );
}

/// Netflix's slider, for seeking and volume: a thin red line on a grey track with a knob,
/// thicker while hovered. Returns the value the pointer picks (or `shown`) on `0..=max`, and the
/// response, marked changed when the pointer sets a value.
fn line_slider(
    ui: &mut egui::Ui,
    width: f32,
    shown: f64,
    max: f64,
    name: &str,
) -> (f64, egui::Response) {
    let enabled = max > 0.0;
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, mut r) = ui.allocate_exact_size(vec2(width, 26.0), sense);
    let mut value = shown;
    // A press and release within one frame may only report the pointer on the context.
    if enabled
        && (r.is_pointer_button_down_on() || r.clicked())
        && let Some(p) = r
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_interact_pos())
    {
        let t = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        value = f64::from(t) * max;
        // Holding still sends nothing new; a click always counts.
        if (value - shown).abs() > f64::EPSILON || r.clicked() {
            r.mark_changed();
        }
    }
    r.widget_info(|| egui::WidgetInfo::slider(enabled, value, name));

    let hot = ui.ctx().animate_bool_with_time(
        r.id.with("hot"),
        enabled && (r.hovered() || r.dragged()),
        0.12,
    );
    let fraction = if enabled {
        (value / max).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0 + 3.0 * hot));
    let painter = ui.painter();
    painter.rect_filled(track, 2, media::TRACK);
    let mut done = track;
    done.set_width(track.width() * fraction);
    painter.rect_filled(done, 2, media::PROGRESS);
    if enabled {
        painter.circle_filled(
            pos2(done.right(), track.center().y),
            6.0 + 2.5 * hot,
            media::PROGRESS,
        );
    }
    if enabled {
        r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    (value, r)
}

/// A round, white icon button for the dark player bar.
fn player_button(ui: &mut egui::Ui, icon: &str, size: f32, tooltip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    ui.painter()
        .circle_filled(rect.center(), 19.0, media::GLASS.gamma_multiply(t));
    let color = media::TEXT_WEAK.lerp_to_gamma(media::TEXT, t);
    widgets::icon_centered(
        ui.painter(),
        rect.center(),
        icon,
        theme::icon_font(size),
        color,
    );
    widgets::named(resp, tooltip)
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn play_button(ui: &mut egui::Ui, paused: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    ui.painter().circle_filled(
        rect.center(),
        22.0 + 1.5 * t,
        media::TEXT.lerp_to_gamma(theme::ramp::N300, t),
    );
    if paused {
        widgets::play_icon(ui.painter(), rect.center(), 22.0, theme::ramp::N950);
    } else {
        widgets::icon_centered(
            ui.painter(),
            rect.center(),
            icons::PAUSE,
            theme::icon_fill_font(22.0),
            theme::ramp::N950,
        );
    }
    widgets::named(resp, if paused { "Play" } else { "Pause" })
        .on_hover_text("Play/pause (Space)")
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
        .map_or("Off".to_string(), |t| t.short_label());
    let mut choice: Option<Option<i64>> = None;
    let combo = egui::ComboBox::from_id_salt(name)
        .selected_text(format!("{icon}  {selected}"))
        .width(160.0)
        .truncate()
        .show_ui(ui, |ui| {
            if allow_off
                && ui
                    .selectable_label(tracks.iter().all(|t| !t.selected), "Off")
                    .clicked()
            {
                choice = Some(None);
            }
            ui.set_max_width(420.0);
            for t in &tracks {
                let item = egui::Button::selectable(t.selected, t.label()).truncate();
                if ui.add(item).on_hover_text(t.label()).clicked() {
                    choice = Some(Some(t.id));
                }
            }
            if tracks.is_empty() {
                ui.weak("No subtitles in this file");
            }
        });
    widgets::name_combo(&combo.response, &format!("{name}: {selected}"));
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
