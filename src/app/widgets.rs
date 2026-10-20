//! Drawing helpers: posters, placeholders and badges.

use std::path::Path;

use eframe::egui::{self, Color32, Rect, Vec2, pos2};

use crate::util::file_uri;

/// A stable, muted colour per title for poster placeholders.
fn placeholder_color(title: &str) -> Color32 {
    let hash = title
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    let hue = (hash % 360) as f32 / 360.0;
    egui::ecolor::Hsva::new(hue, 0.35, 0.32, 1.0).into()
}

/// Paints the poster into `rect`, cropping it to fill ("cover"). Falls back to a placeholder
/// with the title while loading or when there is no poster.
pub fn paint_poster(ui: &egui::Ui, rect: Rect, poster: Option<&Path>, title: &str, dim: bool) {
    let painter = ui.painter_at(rect);
    let radius = 6.0;
    let tint = if dim {
        Color32::from_gray(90)
    } else {
        Color32::WHITE
    };
    if let Some(path) = poster {
        let uri = file_uri(path);
        if let Ok(egui::load::TexturePoll::Ready { texture }) = ui.ctx().try_load_texture(
            &uri,
            egui::TextureOptions::LINEAR,
            egui::SizeHint::default(),
        ) {
            let uv = cover_uv(texture.size, rect.size());
            egui::Image::new(texture)
                .uv(uv)
                .tint(tint)
                .corner_radius(radius)
                .paint_at(ui, rect);
            return;
        }
    }
    let base = placeholder_color(title);
    let fill = if dim { base.gamma_multiply(0.4) } else { base };
    painter.rect_filled(rect, radius, fill);
    painter.text(
        rect.center() - Vec2::new(0.0, 18.0),
        egui::Align2::CENTER_CENTER,
        "🎬",
        egui::FontId::proportional(34.0),
        Color32::from_white_alpha(150),
    );
    let galley = painter.layout(
        title.to_string(),
        egui::FontId::proportional(14.0),
        Color32::from_white_alpha(220),
        rect.width() - 16.0,
    );
    let pos = pos2(
        rect.center().x - galley.size().x / 2.0,
        rect.center().y + 12.0,
    );
    painter.galley(pos, galley, Color32::WHITE);
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

/// A small rounded label painted at `pos` (top-left corner).
pub fn badge(painter: &egui::Painter, pos: egui::Pos2, text: &str, fill: Color32) -> Rect {
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(11.5),
        Color32::WHITE,
    );
    let rect = Rect::from_min_size(pos, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(rect, 4.0, fill);
    painter.galley(rect.min + Vec2::new(5.0, 2.0), galley, Color32::WHITE);
    rect
}

pub const WARN: Color32 = Color32::from_rgb(214, 128, 30);
pub const DANGER: Color32 = Color32::from_rgb(200, 60, 55);
pub const ACCENT: Color32 = Color32::from_rgb(90, 150, 230);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crop_keeps_aspect_ratio() {
        // 16:9 frame into a 2:3 poster: crop the sides.
        let uv = cover_uv(Vec2::new(1600.0, 900.0), Vec2::new(200.0, 300.0));
        assert!((uv.width() - (2.0 / 3.0) / (16.0 / 9.0)).abs() < 1e-5);
        assert_eq!(uv.height(), 1.0);
        assert!((uv.center().x - 0.5).abs() < 1e-6);
        // Same aspect: whole image.
        assert_eq!(
            cover_uv(Vec2::new(2.0, 3.0), Vec2::new(200.0, 300.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
        );
    }
}
