//! Look and feel: Inter typography, Phosphor icons, and the dark/light palettes.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText,
    Shadow, Stroke, TextStyle, Theme, vec2,
};

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");
const PHOSPHOR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");
const PHOSPHOR_FILL: &[u8] = include_bytes!("../../assets/fonts/Phosphor-Fill.ttf");
pub const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");
pub const APP_ICON_URI: &str = "bytes://dropreel-icon.png";

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Main content background.
    pub bg: Color32,
    /// Sidebar, toolbar, detail panel.
    pub surface: Color32,
    /// Cards, dialogs, buttons.
    pub raised: Color32,
    pub hover: Color32,
    pub input: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub accent2: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub dark: bool,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(13, 14, 18),
    surface: Color32::from_rgb(20, 22, 28),
    raised: Color32::from_rgb(30, 33, 41),
    hover: Color32::from_rgb(40, 44, 55),
    input: Color32::from_rgb(10, 11, 14),
    border: Color32::from_rgb(44, 48, 60),
    text: Color32::from_rgb(236, 237, 242),
    text_weak: Color32::from_rgb(140, 146, 162),
    accent: Color32::from_rgb(139, 92, 246),
    accent_soft: Color32::from_rgba_premultiplied(56, 37, 98, 120),
    accent2: Color32::from_rgb(236, 72, 153),
    warn: Color32::from_rgb(245, 158, 11),
    danger: Color32::from_rgb(239, 68, 68),
    success: Color32::from_rgb(34, 197, 94),
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(244, 245, 248),
    surface: Color32::from_rgb(255, 255, 255),
    raised: Color32::from_rgb(238, 239, 244),
    hover: Color32::from_rgb(226, 228, 236),
    input: Color32::from_rgb(246, 247, 250),
    border: Color32::from_rgb(222, 224, 232),
    text: Color32::from_rgb(20, 22, 30),
    text_weak: Color32::from_rgb(104, 110, 128),
    accent: Color32::from_rgb(124, 58, 237),
    accent_soft: Color32::from_rgba_premultiplied(124, 58, 237, 36),
    accent2: Color32::from_rgb(219, 39, 119),
    warn: Color32::from_rgb(217, 119, 6),
    danger: Color32::from_rgb(220, 38, 38),
    success: Color32::from_rgb(22, 163, 74),
    dark: false,
};

pub fn palette(ctx: &egui::Context) -> &'static Palette {
    if ctx.theme() == Theme::Dark {
        &DARK
    } else {
        &LIGHT
    }
}

pub fn family(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

pub fn medium(text: impl Into<String>) -> RichText {
    RichText::new(text).family(family("medium"))
}

pub fn semibold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(family("semibold"))
}

pub fn bold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(family("bold"))
}

/// A filled Phosphor icon.
pub fn icon_fill(icon: &str) -> RichText {
    RichText::new(icon).family(family("icons-fill"))
}

pub fn font(size: f32, weight: &str) -> FontId {
    FontId::new(
        size,
        if weight == "regular" {
            FontFamily::Proportional
        } else {
            family(weight)
        },
    )
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, family("icons"))
}

pub fn icon_fill_font(size: f32) -> FontId {
    FontId::new(size, family("icons-fill"))
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let icon_tweak = egui::FontTweak {
        y_offset_factor: 0.08,
        ..Default::default()
    };
    for (name, bytes, tweak) in [
        ("inter", INTER_REGULAR, None),
        ("inter-medium", INTER_MEDIUM, None),
        ("inter-semibold", INTER_SEMIBOLD, None),
        ("inter-bold", INTER_BOLD, None),
        ("phosphor", PHOSPHOR, Some(icon_tweak.clone())),
        ("phosphor-fill", PHOSPHOR_FILL, Some(icon_tweak.clone())),
    ] {
        let mut data = FontData::from_static(bytes);
        if let Some(t) = tweak {
            data = data.tweak(t);
        }
        fonts.font_data.insert(name.into(), Arc::new(data));
    }
    // egui's bundled fonts stay as fallbacks for symbols Inter lacks.
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with = |primary: &str| {
        let mut list = vec![primary.to_string(), "phosphor".to_string()];
        list.extend(fallbacks.iter().cloned());
        list
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with("inter"));
    fonts
        .families
        .insert(family("medium"), with("inter-medium"));
    fonts
        .families
        .insert(family("semibold"), with("inter-semibold"));
    fonts.families.insert(family("bold"), with("inter-bold"));
    fonts
        .families
        .insert(family("icons"), vec!["phosphor".into(), "inter".into()]);
    fonts.families.insert(
        family("icons-fill"),
        vec!["phosphor-fill".into(), "inter".into()],
    );
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("phosphor".into());
    }
    ctx.set_fonts(fonts);
}

fn style_for(p: &Palette) -> egui::Style {
    let mut style = egui::Style {
        text_styles: [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, family("medium"))),
            (TextStyle::Heading, FontId::new(20.0, family("bold"))),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into(),
        ..Default::default()
    };

    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 8.0);
    s.button_padding = vec2(12.0, 6.0);
    s.interact_size = vec2(36.0, 30.0);
    s.window_margin = Margin::same(18);
    s.menu_margin = Margin::same(6);
    s.combo_width = 120.0;
    s.icon_width = 16.0;
    s.scroll = egui::style::ScrollStyle::floating();

    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.surface;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.input;
    v.faint_bg_color = if p.dark {
        Color32::from_rgb(18, 20, 25)
    } else {
        Color32::from_rgb(248, 249, 251)
    };
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_stroke = Stroke::new(1.0, p.border);
    let shadow_alpha = if p.dark { 140 } else { 40 };
    v.window_shadow = Shadow {
        offset: [0, 14],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.popup_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = p.accent;
    v.slider_trailing_fill = true;
    v.striped = true;

    let radius = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = p.hover;
    w.hovered.weak_bg_fill = p.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = radius;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.hover;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = radius;
    w.active.expansion = 0.0;
    w.open.bg_fill = p.hover;
    w.open.weak_bg_fill = p.hover;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = radius;
    style.visuals = v;
    style
}

/// Installs fonts, both theme styles and the logo image. Call once at startup.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_style_of(Theme::Dark, style_for(&DARK));
    ctx.set_style_of(Theme::Light, style_for(&LIGHT));
    ctx.include_bytes(APP_ICON_URI, APP_ICON_PNG);
}
