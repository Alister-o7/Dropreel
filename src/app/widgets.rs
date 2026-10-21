//! Custom widgets and drawing helpers: posters, pills, buttons, progress ring, gradients.

use std::path::Path;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, Ui,
    Vec2, pos2, vec2,
};

use super::icons;
use super::theme::{self, Palette, media, palette, radius};
use crate::util::file_uri;

/// Paints the poster into `rect`, cropped to fill. Without a poster, or while it loads, draws a
/// grey gradient placeholder with the logo's droplet and the title; a loading poster shimmers,
/// then fades in over it. Returns how far the poster has faded in (0–1).
pub fn paint_poster(
    ui: &Ui,
    rect: Rect,
    poster: Option<&Path>,
    title: &str,
    dim: bool,
    radius: u8,
) -> f32 {
    let tint = if dim {
        Color32::from_gray(80)
    } else {
        Color32::WHITE
    };
    let poll = poster.map(|path| {
        ui.ctx().try_load_texture(
            &file_uri(path),
            egui::TextureOptions::LINEAR,
            egui::SizeHint::default(),
        )
    });
    let texture = match &poll {
        Some(Ok(egui::load::TexturePoll::Ready { texture })) => Some(*texture),
        _ => None,
    };
    let loading = matches!(poll, Some(Ok(egui::load::TexturePoll::Pending { .. })));
    let fade_id = egui::Id::unique("poster-fade").with(poster).with(radius);
    let shown = ui
        .ctx()
        .animate_bool_with_time(fade_id, texture.is_some(), 0.3);
    if shown < 1.0 {
        paint_placeholder(ui, rect, title, dim, radius);
        if loading {
            shimmer(ui, rect);
        }
    }
    if let Some(texture) = texture {
        egui::Image::new(texture)
            .uv(cover_uv(texture.size, rect.size()))
            .tint(tint.gamma_multiply(shown))
            .corner_radius(radius)
            .paint_at(ui, rect);
    }
    shown
}

fn paint_placeholder(ui: &Ui, rect: Rect, title: &str, dim: bool, radius: u8) {
    let (top, bottom) = (theme::ramp::N700, theme::ramp::N850);
    let (top, bottom) = if dim {
        (top.gamma_multiply(0.4), bottom.gamma_multiply(0.4))
    } else {
        (top, bottom)
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, radius, bottom);
    vertical_gradient(
        &painter,
        rect.shrink2(vec2(0.0, f32::from(radius))),
        top,
        bottom,
    );
    painter.rect_filled(
        Rect::from_min_max(
            rect.min,
            pos2(rect.max.x, rect.min.y + f32::from(radius) + 1.0),
        ),
        CornerRadius {
            nw: radius,
            ne: radius,
            sw: 0,
            se: 0,
        },
        top,
    );
    // A soft highlight in the top corner, then the logo's droplet as a watermark.
    painter.circle_filled(
        rect.left_top() + rect.size() * vec2(0.2, 0.1),
        rect.width() * 0.75,
        media::GLASS_FAINT,
    );
    let icon_size = (rect.width() * 0.26).clamp(14.0, 56.0);
    let mark = Rect::from_center_size(
        rect.center() - vec2(0.0, icon_size * 0.45),
        vec2(icon_size * 0.74, icon_size),
    );
    egui::Image::new(theme::LOGO_MARK_URI)
        .texture_options(theme::SMOOTH)
        .tint(Color32::from_white_alpha(80))
        .paint_at(ui, mark);
    if rect.width() > 60.0 {
        let galley = painter.layout(
            title.to_string(),
            theme::font(13.0, "semibold"),
            media::TEXT_WEAK,
            rect.width() - 20.0,
        );
        let pos = pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y + icon_size * 0.3,
        );
        painter.galley(pos, galley, media::TEXT);
    }
}

/// A soft band of light sweeping across `rect`, for things that are loading.
fn shimmer(ui: &Ui, rect: Rect) {
    let phase = (ui.input(|i| i.time) / 1.3).fract() as f32;
    let band = rect.width() * 0.45;
    let x = rect.left() - band + phase * (rect.width() + 2.0 * band);
    let painter = ui.painter_at(rect);
    let peak = media::GLASS;
    horizontal_gradient(
        &painter,
        Rect::from_min_max(pos2(x - band / 2.0, rect.top()), pos2(x, rect.bottom())),
        Color32::TRANSPARENT,
        peak,
    );
    horizontal_gradient(
        &painter,
        Rect::from_min_max(pos2(x, rect.top()), pos2(x + band / 2.0, rect.bottom())),
        peak,
        Color32::TRANSPARENT,
    );
    ui.ctx().request_repaint();
}

/// UV rectangle that crops an image of `image` size to fill `target` without stretching.
pub fn cover_uv(image: Vec2, target: Vec2) -> Rect {
    if image.x <= 0.0 || image.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    }
    let (ia, ta) = (image.x / image.y, target.x / target.y);
    if ia > ta {
        let w = ta / ia;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ia / ta;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// Netflix's play button over a poster: a white disc with a black icon.
pub fn play_disc(painter: &egui::Painter, center: Pos2, radius: f32, opacity: f32) {
    painter.circle_filled(
        center,
        radius + 3.0,
        media::SCRIM_SOFT.gamma_multiply(opacity),
    );
    painter.circle_filled(center, radius, media::TEXT.gamma_multiply(opacity));
    // The play triangle looks centred a little right of the true centre.
    painter.text(
        center + vec2(radius * 0.08, 0.0),
        Align2::CENTER_CENTER,
        icons::PLAY,
        theme::icon_fill_font(radius * 0.9),
        media::ON_PILL.gamma_multiply(opacity),
    );
}

/// A rectangle filled with a left-to-right gradient.
pub fn horizontal_gradient(painter: &egui::Painter, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    painter.add(Shape::mesh(mesh));
}

/// A rectangle filled with a top-to-bottom gradient.
pub fn vertical_gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    let i = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i + 1, i + 3, i + 2);
    painter.add(Shape::mesh(mesh));
}

/// A small rounded label with an optional icon, painted at `pos` (anchored by `anchor`).
pub fn pill(
    painter: &egui::Painter,
    pos: Pos2,
    anchor: Align2,
    icon: Option<&str>,
    text: &str,
    fill: Color32,
    fg: Color32,
) -> Rect {
    let label = match icon {
        Some(i) if text.is_empty() => i.to_string(),
        Some(i) => format!("{i} {text}"),
        None => text.to_string(),
    };
    let galley = painter.layout_no_wrap(label, theme::font(11.5, "semibold"), fg);
    let size = galley.size() + vec2(14.0, 6.0);
    let rect = anchor.anchor_size(pos, size);
    painter.rect_filled(rect, CornerRadius::same(radius::BADGE), fill);
    painter.galley(rect.min + vec2(7.0, 3.0), galley, fg);
    rect
}

/// A circular progress indicator.
pub fn progress_ring(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    fraction: f32,
    color: Color32,
    track: Color32,
    width: f32,
) {
    painter.circle_stroke(center, radius, Stroke::new(width, track));
    let f = fraction.clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    let steps = (64.0 * f).ceil().max(2.0) as usize;
    let start = -std::f32::consts::FRAC_PI_2;
    let points: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let a = start + std::f32::consts::TAU * f * i as f32 / steps as f32;
            center + radius * vec2(a.cos(), a.sin())
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(width, color)));
}

/// A thin progress bar filling the available width: red over a grey track.
pub fn progress_bar(ui: &mut Ui, fraction: f32, height: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = palette(ui.ctx());
    let r = CornerRadius::same(2);
    ui.painter().rect_filled(rect, r, p.hover);
    let mut done = rect;
    done.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    if done.width() > 0.5 {
        ui.painter().rect_filled(done, r, p.primary);
    }
    resp
}

/// Gives a hand-painted widget an accessible name, read by screen readers and UI tests.
pub fn named(resp: Response, name: &str) -> Response {
    let enabled = resp.enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
    resp
}

/// Names a combo box, which egui leaves unnamed when it has no visible label.
pub fn name_combo(resp: &Response, name: &str) {
    let enabled = resp.enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, name));
}

/// Like `named`, for a widget with an on/off or selected state.
pub fn named_selectable(resp: Response, name: &str, selected: bool) -> Response {
    let enabled = resp.enabled();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, name));
    resp
}

/// A square icon-only button that highlights on hover.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    icon_toggle(ui, icon, false, tooltip)
}

/// An icon button with a selected state (used for segmented controls).
pub fn icon_toggle(ui: &mut Ui, icon: &str, selected: bool, tooltip: &str) -> Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let bg = if selected {
        p.accent_soft
    } else {
        Color32::TRANSPARENT.lerp_to_gamma(p.hover, t)
    };
    ui.painter().rect_filled(rect, radius::CONTROL, bg);
    let fg = if selected {
        p.accent
    } else {
        p.text_weak.lerp_to_gamma(p.text, t)
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        theme::icon_font(19.0),
        fg,
    );
    named_selectable(resp, tooltip, selected)
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Warn,
    Danger,
    Ghost,
}

fn kind_colors(p: &Palette, kind: ButtonKind) -> (Color32, Color32) {
    match kind {
        ButtonKind::Primary => (p.primary, p.on_primary),
        ButtonKind::Secondary => (p.secondary, p.text),
        ButtonKind::Warn => (p.warn, p.bg),
        // Outlined, so a destructive button never looks like the primary one.
        ButtonKind::Danger => (Color32::TRANSPARENT, p.danger),
        ButtonKind::Ghost => (Color32::TRANSPARENT, p.text_weak),
    }
}

/// A filled button with an optional leading icon.
pub fn button(ui: &mut Ui, kind: ButtonKind, icon: Option<&str>, label: &str) -> Response {
    button_sized(ui, kind, icon, label, 0.0, true)
}

pub fn button_sized(
    ui: &mut Ui,
    kind: ButtonKind,
    icon: Option<&str>,
    label: &str,
    min_width: f32,
    enabled: bool,
) -> Response {
    let p = palette(ui.ctx());
    let (fill, fg) = kind_colors(p, kind);
    let text = match icon {
        Some(i) if label.is_empty() => i.to_string(),
        Some(i) => format!("{i}  {label}"),
        None => label.to_string(),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text, theme::font(14.0, "semibold"), fg);
    let size = vec2((galley.size().x + 28.0).max(min_width), 34.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, enabled && resp.hovered(), 0.12);
    let bg = match kind {
        ButtonKind::Ghost => Color32::TRANSPARENT.lerp_to_gamma(p.hover, t),
        // Netflix's grey button fades a little on hover.
        ButtonKind::Secondary if p.dark => fill.gamma_multiply(1.0 - 0.4 * t),
        ButtonKind::Secondary => fill.lerp_to_gamma(p.hover, t),
        ButtonKind::Primary => fill.lerp_to_gamma(theme::ramp::WHITE, 0.1 * t),
        ButtonKind::Danger => Color32::TRANSPARENT.lerp_to_gamma(p.danger.gamma_multiply(0.14), t),
        ButtonKind::Warn => fill.lerp_to_gamma(theme::ramp::WHITE, 0.14 * t),
    };
    let (bg, fg_alpha) = if enabled {
        (bg, 1.0)
    } else {
        (bg.gamma_multiply(0.45), 0.5)
    };
    let radius = CornerRadius::same(radius::CONTROL);
    ui.painter().rect_filled(rect, radius, bg);
    let outline = match kind {
        ButtonKind::Secondary if !p.dark => {
            Some(Stroke::new(1.0, p.border.gamma_multiply(1.0 - t)))
        }
        ButtonKind::Danger => Some(Stroke::new(1.5, p.danger.gamma_multiply(fg_alpha))),
        _ => None,
    };
    if let Some(stroke) = outline {
        ui.painter()
            .rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
    }
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter()
        .galley_with_override_text_color(pos, galley, fg.gamma_multiply(fg_alpha));
    let resp = if label.is_empty() {
        resp
    } else {
        named(resp, label)
    };
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// A full-width sidebar navigation row.
pub fn nav_item(
    ui: &mut Ui,
    icon: &str,
    label: &str,
    count: Option<usize>,
    selected: bool,
) -> Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let t = ui
        .ctx()
        .animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let bg = if selected {
        p.accent_soft
    } else {
        Color32::TRANSPARENT.lerp_to_gamma(p.hover, t)
    };
    ui.painter().rect_filled(rect, radius::CONTROL, bg);
    if selected {
        // A short bar marks the current shelf.
        let bar = Rect::from_center_size(rect.left_center() + vec2(1.5, 0.0), vec2(3.0, 16.0));
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), p.primary);
    }
    let fg = if selected {
        p.text
    } else {
        p.text_weak.lerp_to_gamma(p.text, t)
    };
    let icon_color = if selected { p.accent } else { fg };
    ui.painter().text(
        rect.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        icon,
        theme::icon_font(18.0),
        icon_color,
    );
    let weight = if selected { "semibold" } else { "medium" };
    ui.painter().text(
        rect.left_center() + vec2(40.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        theme::font(14.0, weight),
        fg,
    );
    if let Some(n) = count {
        let galley = ui.painter().layout_no_wrap(
            n.to_string(),
            theme::font(11.5, "semibold"),
            if selected { p.text } else { p.text_weak },
        );
        let badge = Align2::RIGHT_CENTER.anchor_size(
            rect.right_center() - vec2(8.0, 0.0),
            vec2((galley.size().x + 12.0).max(22.0), 20.0),
        );
        let badge_fill = if selected {
            p.accent.gamma_multiply(0.16)
        } else {
            p.raised
        };
        ui.painter()
            .rect_filled(badge, CornerRadius::same(radius::CONTROL), badge_fill);
        ui.painter()
            .galley(badge.center() - galley.size() / 2.0, galley, p.text_weak);
    }
    named_selectable(resp, label, selected).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Small uppercase section heading.
pub fn section_label(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.add_space(6.0);
    ui.label(
        theme::semibold(text.to_uppercase())
            .size(11.0)
            .color(p.text_weak)
            .extra_letter_spacing(0.8),
    );
}

/// A tinted box with an icon and title, used for warnings and status cards.
pub fn callout<R>(
    ui: &mut Ui,
    color: Color32,
    icon: &str,
    title: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let p = palette(ui.ctx());
    let fill = if p.dark {
        color.gamma_multiply(0.14)
    } else {
        color.gamma_multiply(0.10)
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.45)))
        .corner_radius(radius::CARD)
        .inner_margin(14)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(icon)
                        .family(theme::family("icons-fill"))
                        .size(18.0)
                        .color(color),
                );
                ui.label(theme::semibold(title).size(15.0).color(p.text));
            });
            add(ui)
        })
        .inner
}

/// A rounded "chip" label for metadata (year, resolution, codec…).
pub fn chip(ui: &mut Ui, icon: Option<&str>, text: &str) -> Response {
    let p = palette(ui.ctx());
    let label = match icon {
        Some(i) => format!("{i} {text}"),
        None => text.to_string(),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label, theme::font(12.5, "medium"), p.text_weak);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(18.0, 10.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL), p.raised);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius::CONTROL),
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.min + vec2(9.0, 5.0), galley, p.text_weak);
    resp
}

/// Human label for a video's resolution.
pub fn resolution_label(width: Option<u32>, height: Option<u32>) -> Option<&'static str> {
    let (w, h) = (width?, height?);
    Some(match w.max(h * 16 / 9) {
        x if x >= 3200 => "4K",
        x if x >= 2400 => "1440p",
        x if x >= 1700 => "1080p",
        x if x >= 1100 => "720p",
        _ => "SD",
    })
}

/// "2h 16m" / "48m".
pub fn short_duration(seconds: f64) -> String {
    if seconds < 60.0 {
        return format!("{:.0}s", seconds.max(0.0));
    }
    let m = (seconds / 60.0).round() as u64;
    if m >= 60 {
        format!("{}h {:02}m", m / 60, m % 60)
    } else {
        format!("{m}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crop_keeps_aspect_ratio() {
        let uv = cover_uv(Vec2::new(1600.0, 900.0), Vec2::new(200.0, 300.0));
        assert!((uv.width() - (2.0 / 3.0) / (16.0 / 9.0)).abs() < 1e-5);
        assert_eq!(uv.height(), 1.0);
        assert!((uv.center().x - 0.5).abs() < 1e-6);
        assert_eq!(
            cover_uv(Vec2::new(2.0, 3.0), Vec2::new(200.0, 300.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
        );
    }

    #[test]
    fn resolution_and_duration_labels() {
        assert_eq!(resolution_label(Some(3840), Some(2160)), Some("4K"));
        assert_eq!(
            resolution_label(Some(1920), Some(800)),
            Some("1080p"),
            "scope films are 1080p"
        );
        assert_eq!(resolution_label(Some(1280), Some(720)), Some("720p"));
        assert_eq!(resolution_label(Some(720), Some(480)), Some("SD"));
        assert_eq!(resolution_label(None, Some(480)), None);
        assert_eq!(short_duration(8160.0), "2h 16m");
        assert_eq!(short_duration(2880.0), "48m");
        assert_eq!(short_duration(9.4), "9s");
    }
}
