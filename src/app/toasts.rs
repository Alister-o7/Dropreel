//! Short-lived notifications in the bottom-right corner.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};

use super::events::ToastKind;

struct Toast {
    kind: ToastKind,
    text: String,
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
        let secs = if kind == ToastKind::Error { 10 } else { 5 };
        self.items.push(Toast {
            kind,
            text,
            until: Instant::now() + Duration::from_secs(secs),
        });
        if self.items.len() > 5 {
            self.items.remove(0);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.items.retain(|t| t.until > now);
        if self.items.is_empty() {
            return;
        }
        let mut dismissed = None;
        egui::Area::new(egui::Id::unique("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -36.0])
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for (i, toast) in self.items.iter().enumerate() {
                    let accent = match toast.kind {
                        ToastKind::Info => Color32::from_rgb(90, 150, 230),
                        ToastKind::Success => Color32::from_rgb(80, 180, 110),
                        ToastKind::Error => Color32::from_rgb(220, 80, 70),
                    };
                    let response = egui::Frame::popup(ui.style())
                        .stroke(egui::Stroke::new(1.0, accent))
                        .show(ui, |ui| {
                            ui.set_width(360.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(accent, "⏺");
                                ui.label(&toast.text);
                            });
                        })
                        .response
                        .interact(egui::Sense::click())
                        .on_hover_text("Click to dismiss");
                    if response.clicked() {
                        dismissed = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismissed {
            self.items.remove(i);
        }
        if let Some(next) = self.items.iter().map(|t| t.until).min() {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }
}
