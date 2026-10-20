//! Short-lived notifications in the bottom-right corner.

use std::time::{Duration, Instant};

use eframe::egui::{self, RichText};

use super::events::ToastKind;
use super::icons;
use super::theme::{self, palette};

struct Toast {
    kind: ToastKind,
    text: String,
    created: Instant,
    until: Instant,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub fn add(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        match kind {
            ToastKind::Error => tracing::warn!("{text}"),
            _ => tracing::info!("{text}"),
        }
        let secs = if kind == ToastKind::Error { 12 } else { 5 };
        let now = Instant::now();
        self.items.push(Toast {
            kind,
            text,
            created: now,
            until: now + Duration::from_secs(secs),
        });
        if self.items.len() > 4 {
            self.items.remove(0);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.items.retain(|t| t.until > now);
        if self.items.is_empty() {
            return;
        }
        let p = palette(ctx);
        let mut dismissed = None;
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.set_max_width(400.0);
                for (i, toast) in self.items.iter().enumerate() {
                    let (icon, color) = match toast.kind {
                        ToastKind::Info => (icons::INFO, p.accent),
                        ToastKind::Success => (icons::CHECK_CIRCLE, p.success),
                        ToastKind::Error => (icons::WARNING_CIRCLE, p.danger),
                    };
                    // Slide in from the right.
                    let t = (now.duration_since(toast.created).as_secs_f32() / 0.18).min(1.0);
                    let fade = (toast.until.duration_since(now).as_secs_f32() / 0.3).min(1.0);
                    let response = egui::Frame::new()
                        .fill(p.surface)
                        .stroke(egui::Stroke::new(1.0, p.border))
                        .corner_radius(12)
                        .inner_margin(egui::Margin {
                            left: 14,
                            right: 16,
                            top: 12,
                            bottom: 12,
                        })
                        .shadow(ctx.global_style().visuals.popup_shadow)
                        .multiply_with_opacity(t.min(fade))
                        .show(ui, |ui| {
                            ui.set_width(360.0);
                            ui.horizontal(|ui| {
                                ui.label(theme::icon_fill(icon).size(20.0).color(color));
                                ui.add(
                                    egui::Label::new(RichText::new(&toast.text).color(p.text))
                                        .wrap(),
                                );
                            });
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_text("Click to dismiss");
                    if response.clicked() {
                        dismissed = Some(i);
                    }
                    ui.add_space(8.0);
                }
            });
        if let Some(i) = dismissed {
            self.items.remove(i);
        }
        // Animate while toasts appear or fade, otherwise wake up for the next expiry.
        let animating = self.items.iter().any(|t| {
            now.duration_since(t.created) < Duration::from_millis(200)
                || t.until.duration_since(now) < Duration::from_millis(300)
        });
        let next = self
            .items
            .iter()
            .map(|t| {
                t.until
                    .duration_since(now)
                    .saturating_sub(Duration::from_millis(300))
            })
            .min();
        ctx.request_repaint_after(if animating {
            Duration::from_millis(16)
        } else {
            next.unwrap_or(Duration::from_secs(1))
        });
    }
}
