//! Look and feel: Inter typography, Phosphor icons, the dark/light palettes and the bundled
//! artwork. The interface itself is monochrome and high-contrast; colour comes from the posters
//! and the artwork.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText,
    Shadow, Stroke, TextStyle, TextureOptions, Theme, vec2,
};

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");
const PHOSPHOR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");
const PHOSPHOR_FILL: &[u8] = include_bytes!("../../assets/fonts/Phosphor-Fill.ttf");
/// The white droplet-and-play mark from the logo, on transparent.
pub const LOGO_MARK_URI: &str = "bytes://dropreel-mark.png";
/// Illustration for the empty library.
pub const EMPTY_ART_URI: &str = "bytes://dropreel-empty.jpg";
/// Soft indigo-to-pink light ribbons, behind the drop overlay and the sidebar header.
pub const AURORA_URI: &str = "bytes://dropreel-aurora.jpg";

/// Downscaled images (logo, artwork) stay smooth with mipmaps.
pub const SMOOTH: TextureOptions =
    TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));

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
    pub warn: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub dark: bool,
}

/// The "Midnight" scheme: a cool neutral ramp from near-black to white. The interface is built
/// from these greys alone; white (or, on the light theme, near-black) is the accent, and colour is
/// left to the posters, the artwork and the three status hues below.
pub mod ramp {
    use super::Color32;

    pub const N950: Color32 = Color32::from_rgb(4, 4, 6);
    pub const N900: Color32 = Color32::from_rgb(10, 10, 13);
    pub const N850: Color32 = Color32::from_rgb(17, 17, 21);
    pub const N800: Color32 = Color32::from_rgb(25, 25, 31);
    pub const N750: Color32 = Color32::from_rgb(35, 35, 43);
    pub const N700: Color32 = Color32::from_rgb(48, 48, 58);
    pub const N600: Color32 = Color32::from_rgb(82, 82, 94);
    pub const N400: Color32 = Color32::from_rgb(170, 170, 182);
    pub const N300: Color32 = Color32::from_rgb(208, 208, 218);
    pub const N200: Color32 = Color32::from_rgb(226, 226, 234);
    pub const N100: Color32 = Color32::from_rgb(240, 240, 245);
    pub const N50: Color32 = Color32::from_rgb(248, 248, 251);
    pub const WHITE: Color32 = Color32::WHITE;
}

pub const DARK: Palette = Palette {
    bg: ramp::N950,
    surface: ramp::N900,
    raised: ramp::N800,
    hover: ramp::N750,
    input: ramp::N850,
    border: ramp::N700,
    text: ramp::WHITE,
    text_weak: ramp::N400,
    accent: ramp::WHITE,
    // White at 10%, premultiplied.
    accent_soft: Color32::from_rgba_premultiplied(26, 26, 26, 26),
    warn: Color32::from_rgb(255, 196, 64),
    danger: Color32::from_rgb(255, 99, 99),
    success: Color32::from_rgb(74, 222, 128),
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: ramp::N50,
    surface: ramp::WHITE,
    raised: ramp::N100,
    hover: ramp::N200,
    input: ramp::N100,
    border: ramp::N300,
    text: ramp::N950,
    text_weak: ramp::N600,
    accent: ramp::N950,
    // Black at 7%, premultiplied.
    accent_soft: Color32::from_rgba_premultiplied(0, 0, 0, 18),
    warn: Color32::from_rgb(180, 83, 9),
    danger: Color32::from_rgb(200, 30, 45),
    success: Color32::from_rgb(21, 128, 61),
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
    v.faint_bg_color = if p.dark { ramp::N850 } else { ramp::N50 };
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(12);
    v.window_stroke = Stroke::new(1.0, p.border);
    let shadow_alpha = if p.dark { 160 } else { 36 };
    v.window_shadow = Shadow {
        offset: [0, 18],
        blur: 48,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    v.popup_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(shadow_alpha),
    };
    // The accent is white (dark theme) or black (light theme): select with a translucent wash
    // so selected text stays readable.
    v.selection.bg_fill = p.accent.gamma_multiply(0.3);
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.hyperlink_color = p.accent;
    v.slider_trailing_fill = true;
    v.striped = false;

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
    w.active.bg_fill = p.hover;
    w.active.weak_bg_fill = p.hover;
    w.active.bg_stroke = Stroke::new(1.0, p.text_weak);
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

/// Installs fonts, both theme styles, the logo and the artwork. Call once at startup.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_style_of(Theme::Dark, style_for(&DARK));
    ctx.set_style_of(Theme::Light, style_for(&LIGHT));
    ctx.include_bytes(
        LOGO_MARK_URI,
        include_bytes!("../../assets/logo-mark.png").as_slice(),
    );
    ctx.include_bytes(
        EMPTY_ART_URI,
        include_bytes!("../../assets/art/empty-library.jpg").as_slice(),
    );
    ctx.include_bytes(
        AURORA_URI,
        include_bytes!("../../assets/art/aurora.jpg").as_slice(),
    );
}
