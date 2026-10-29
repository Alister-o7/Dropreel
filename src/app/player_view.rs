//! The full-window player screen: video, auto-hiding controls, keyboard shortcuts.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, Key, Modifiers, Rect, RichText, Sense, Stroke, Vec2, pos2, vec2,
};

use super::icons;
use super::theme::{self, media};
use super::widgets;
use super::{Action, DropReelApp};
use crate::library::MovieId;
use crate::player::{Player, PlayerEvent, RefreshRate, Track, TrackKind};
use crate::settings::TimeDisplay;
use crate::util::format_duration;

/// How often the playback position is saved while playing.
const SAVE_INTERVAL: Duration = Duration::from_secs(10);
/// Controls hide after this long without mouse movement (fullscreen only).
/// How long after a file loads its playback stats are first logged.
const DIAGNOSTICS_AFTER: Duration = Duration::from_secs(20);
/// Height of the control bar. In a window the video sits above it; in fullscreen it overlays.
pub const BAR_HEIGHT: f32 = 104.0;
/// Fraction of the movie after which it counts as watched.
pub const WATCHED_FRACTION: f64 = 0.95;

/// How long the play/pause icon shows after a click on the picture.
const CLICK_FLASH: Duration = Duration::from_millis(600);

/// How long the volume level stays up after the volume last changed.
const VOLUME_LEVEL_FOR: Duration = Duration::from_millis(1500);

/// How long before the end of an episode the "Up next" card appears, in seconds (at most a tenth
/// of the episode).
const UP_NEXT_BEFORE_END: f64 = 60.0;

/// How long the "Up next" card counts down before the next episode starts.
pub const UP_NEXT_AFTER: Duration = Duration::from_secs(8);

pub struct NowPlaying {
    pub movie_id: MovieId,
    pub title: String,
    /// For an episode: "S01E02 · Episode title", under the show's title.
    pub subtitle: Option<String>,
    /// The episode after this one: its id and "S01E03 · Title".
    pub next: Option<(MovieId, String)>,
    /// When the "Up next" card appeared and its countdown began.
    up_next_since: Option<Instant>,
    /// The card was cancelled: the episode plays on to its end, then back to the library.
    up_next_cancelled: bool,
    /// A click on the picture played or paused it: when, and whether it is now paused.
    click_flash: Option<(Instant, bool)>,
    /// The volume last frame, and when it last changed (by slider, keys or anything else).
    last_volume: Option<f64>,
    volume_changed: Option<Instant>,
    /// Whether the volume slider was being dragged last frame.
    volume_dragging: bool,
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
            subtitle: None,
            next: None,
            up_next_since: None,
            up_next_cancelled: false,
            click_flash: None,
            last_volume: None,
            volume_changed: None,
            volume_dragging: false,
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
        settings,
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
        let at_end = player.eof_reached();
        // An episode with another after it offers that one in its last minute (the last tenth
        // of a short one), counting down.
        let last_minute = matches!((pos, dur), (Some(p), Some(d))
            if d > 0.0 && d - p <= UP_NEXT_BEFORE_END.min(d * 0.1));
        // Wait for the mouse to be released so dragging to the end doesn't act mid-drag.
        let settled = np.seek_drag.is_none() && !ctx.input(|i| i.pointer.any_down());
        if np.next.is_some() && !np.up_next_cancelled && (last_minute || at_end) {
            let since = np.up_next_since.get_or_insert_with(Instant::now);
            if player.paused() && !at_end {
                // Paused, the countdown holds.
                *since = Instant::now();
            }
            if since.elapsed() >= UP_NEXT_AFTER && settled {
                actions.push(Action::PlayNextEpisode);
            }
        } else {
            np.up_next_since = None;
            if !last_minute && !at_end {
                // Seeking back before the last minute offers the next episode again later.
                np.up_next_cancelled = false;
            }
            if at_end && settled {
                actions.push(Action::StopPlayback { finished: true });
            }
        }
    }

    handle_keys(&ctx, player, actions, np.next.is_some());

    let fullscreen = is_fullscreen(&ctx);
    if ctx.input(|i| {
        i.pointer.delta() != Vec2::ZERO || i.pointer.any_down() || !i.keys_down.is_empty()
    }) {
        np.last_activity = Instant::now();
    }
    let paused = player.paused();
    // Paused or not, the controls fade once the mouse has rested for the chosen time.
    let hide_after = Duration::from_secs_f64(settings.hide_controls_after);
    let recently_active = np.controls_hovered || np.last_activity.elapsed() < hide_after;
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
                // Until the new file's first frame, the texture still shows the last one.
                && player.has_picture()
            {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(texture, rect, uv, Color32::WHITE);
            }
            // The picture, without the control bar: clicks there are for the controls.
            let picture =
                Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - BAR_HEIGHT));
            let video = ui.interact(picture, ui.scope_id().with("video"), Sense::click());
            let video = widgets::named(video, "Video");
            // Like Netflix, a click on the picture plays or pauses. A double-click (two clicks:
            // pause, then play again) toggles fullscreen.
            if video.clicked() && np.loaded {
                player.set_paused(!paused);
                np.click_flash = Some((Instant::now(), !paused));
            }
            if video.double_clicked() {
                toggle_fullscreen(&ctx);
            }
            if let Some((at, now_paused)) = np.click_flash {
                if at.elapsed() < CLICK_FLASH {
                    click_flash(ui, rect.center(), at.elapsed(), now_paused);
                    ctx.request_repaint();
                } else {
                    np.click_flash = None;
                }
            }
            if !np.loaded || (frame.gl().is_some() && !player.has_picture()) {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Loading…",
                    egui::FontId::proportional(20.0),
                    Color32::GRAY,
                );
            }

            // In a scope of their own: the controls fade, the Up next card must not.
            ui.scope(|ui| {
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
                    settings.time_display,
                );
            });
            if let (Some(since), Some((_, next))) = (np.up_next_since, &np.next) {
                up_next(ui, rect, next, since, actions, &mut np.up_next_cancelled);
            }
            if !recently_active
                && np.up_next_since.is_none()
                && ctx.pointer_hover_pos().is_some_and(|p| rect.contains(p))
            {
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

fn handle_keys(
    ctx: &egui::Context,
    player: &mut Player,
    actions: &mut Vec<Action>,
    has_next: bool,
) {
    if ctx.text_edit_focused() {
        return;
    }
    // A player button or menu keeps keyboard focus after a mouse click. Drop it, so Space
    // doesn't also "press" the focused button and the arrow keys don't move focus around.
    if let Some(id) = ctx.memory(|m| m.focused()) {
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
    // Read the keys first and talk to mpv afterwards, so egui's input lock isn't held while mpv
    // works.
    let pressed = |key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
    if pressed(Key::Space) {
        let paused = player.paused();
        player.set_paused(!paused);
    }
    if pressed(Key::ArrowLeft) {
        player.seek_relative(-10.0);
    }
    if pressed(Key::ArrowRight) {
        player.seek_relative(10.0);
    }
    if pressed(Key::ArrowUp) {
        let v = player.volume();
        player.set_volume(v + 5.0);
    }
    if pressed(Key::ArrowDown) {
        let v = player.volume();
        player.set_volume(v - 5.0);
    }
    if pressed(Key::M) {
        let m = player.muted();
        player.set_muted(!m);
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F)) {
        toggle_fullscreen(ctx);
    }
    if has_next && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::N)) {
        actions.push(Action::PlayNextEpisode);
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

/// The icon that answers a click on the picture: pause or play, growing and fading away.
fn click_flash(ui: &egui::Ui, center: egui::Pos2, elapsed: Duration, paused: bool) {
    let t = elapsed.as_secs_f32() / CLICK_FLASH.as_secs_f32();
    let opacity = 1.0 - t;
    let radius = 34.0 + 14.0 * t;
    let painter = ui.painter();
    painter.circle_filled(center, radius, media::SCRIM_STRONG.gamma_multiply(opacity));
    let color = media::TEXT.gamma_multiply(opacity);
    if paused {
        let font = theme::icon_fill_font(radius * 0.9);
        widgets::icon_centered(painter, center, icons::PAUSE, font, color);
    } else {
        widgets::play_icon(painter, center, radius * 0.9, color);
    }
}

/// The card at the end of an episode: what's next, a countdown, and Play now / Cancel.
fn up_next(
    ui: &mut egui::Ui,
    rect: Rect,
    next: &str,
    since: Instant,
    actions: &mut Vec<Action>,
    cancelled: &mut bool,
) {
    let left = UP_NEXT_AFTER.saturating_sub(since.elapsed()).as_secs_f32();
    let size = vec2(340.0, 132.0);
    let card = Rect::from_min_size(
        rect.right_bottom() - size - vec2(32.0, BAR_HEIGHT + 24.0),
        size,
    );
    let builder = egui::UiBuilder::new().max_rect(card);
    ui.scope_builder(builder, |ui| {
        *ui.visuals_mut() = theme::visuals(&theme::DARK);
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(20, 20, 20, 235))
            .stroke(Stroke::new(1.0, media::HAIRLINE))
            .corner_radius(theme::radius::PANEL)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_width(size.x - 32.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.label(
                    RichText::new("UP NEXT")
                        .size(11.5)
                        .color(media::TEXT_WEAK)
                        .extra_letter_spacing(0.8),
                );
                ui.add(
                    egui::Label::new(theme::bold(next).size(16.0).color(media::TEXT)).truncate(),
                );
                ui.label(
                    RichText::new(format!("Plays in {:.0} s", left.ceil()))
                        .size(12.5)
                        .color(media::TEXT_WEAK),
                );
                ui.horizontal(|ui| {
                    let play = widgets::button(
                        ui,
                        widgets::ButtonKind::Primary,
                        Some(icons::PLAY),
                        "Play now",
                    );
                    if play.clicked() {
                        actions.push(Action::PlayNextEpisode);
                    }
                    if widgets::button(ui, widgets::ButtonKind::Secondary, None, "Cancel").clicked()
                    {
                        // Plays on to the end of this episode, then back to the library.
                        *cancelled = true;
                    }
                });
            });
    });
    // Tick the countdown.
    ui.ctx().request_repaint_after(Duration::from_millis(200));
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
    time_display: TimeDisplay,
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
    let title = painter.layout_no_wrap(np.title.clone(), theme::font(20.0, "bold"), media::TEXT);
    let title_pos = top.left_center() + vec2(24.0, -title.size().y / 2.0);
    if let Some(sub) = &np.subtitle {
        // The show's name, then the episode beside it, lighter.
        let sub =
            painter.layout_no_wrap(sub.clone(), theme::font(16.0, "medium"), media::TEXT_WEAK);
        let sub_pos = pos2(
            title_pos.x + title.size().x + 12.0,
            title_pos.y + title.size().y - sub.size().y - 1.0,
        );
        painter.galley(sub_pos, sub, media::TEXT_WEAK);
    }
    painter.galley(title_pos, title, media::TEXT);
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
                let time = |text: String| RichText::new(text).monospace().color(media::TEXT);
                if time_display == TimeDisplay::Normal {
                    ui.label(time(format_duration(shown)));
                }
                let time_w = 70.0;
                let width = (ui.available_width() - time_w).max(50.0);
                let (value, r) = line_slider(ui, width, shown, duration, "Seek");
                if duration > 0.0
                    && let Some(pointer) = r.hover_pos()
                {
                    seek_hint(ui, r.rect, pointer.x, duration);
                }
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
                ui.label(time(format_duration(match time_display {
                    TimeDisplay::Normal => duration,
                    TimeDisplay::Remaining => (duration - shown).max(0.0),
                })));
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
                    if let Some((_, next)) = &np.next {
                        ui.add_space(4.0);
                        let tip = format!("Next episode (N): {next}");
                        if player_button(ui, icons::SKIP_FORWARD, 22.0, &tip).clicked() {
                            actions.push(Action::PlayNextEpisode);
                        }
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
                        // The level shows while the volume changes, then fades. Its slot is always
                        // there, so the slider doesn't shift.
                        let level = player.volume();
                        if np.last_volume.is_some_and(|v| (v - level).abs() > 0.01) {
                            np.volume_changed = Some(Instant::now());
                        }
                        np.last_volume = Some(level);
                        let recent = np
                            .volume_changed
                            .is_some_and(|t| t.elapsed() < VOLUME_LEVEL_FOR);
                        let shown = ui.ctx().animate_bool_with_time(
                            egui::Id::unique("volume-level"),
                            recent || np.volume_dragging,
                            0.2,
                        );
                        if recent {
                            ui.ctx().request_repaint_after(Duration::from_millis(100));
                        }
                        let (slot, _) = ui.allocate_exact_size(vec2(40.0, 20.0), Sense::hover());
                        if shown > 0.0 {
                            ui.painter().text(
                                slot.right_center(),
                                egui::Align2::RIGHT_CENTER,
                                format!("{level:.0}%"),
                                theme::font(13.0, "medium"),
                                media::TEXT.gamma_multiply(shown),
                            );
                        }
                        let (volume, r) = line_slider(ui, 96.0, level, 130.0, "Volume");
                        np.volume_dragging = r.dragged();
                        if r.changed() {
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

/// The time under the pointer, in a small label above the seek bar.
fn seek_hint(ui: &egui::Ui, bar: Rect, x: f32, duration: f64) {
    let t = ((x - bar.left()) / bar.width()).clamp(0.0, 1.0);
    let painter = ui.painter();
    let text = painter.layout_no_wrap(
        format_duration(f64::from(t) * duration),
        theme::font(13.0, "semibold"),
        media::TEXT,
    );
    let size = text.size() + vec2(16.0, 8.0);
    let center_x = x.clamp(bar.left() + size.x / 2.0, bar.right() - size.x / 2.0);
    let label = Rect::from_center_size(pos2(center_x, bar.top() - 4.0 - size.y / 2.0), size);
    painter.rect_filled(
        label,
        theme::radius::BADGE,
        Color32::from_rgba_unmultiplied(20, 20, 20, 235),
    );
    painter.galley(label.min + vec2(8.0, 4.0), text, media::TEXT);
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
