//! Look and feel: Inter typography, Phosphor icons, the Netflix-style palettes and the bundled
//! artwork.

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
/// The logo mark: a white droplet with a brick-red play triangle, on transparent.
pub const LOGO_MARK_URI: &str = "bytes://dropreel-mark.png";
/// Illustration for the empty library.
pub const EMPTY_ART_URI: &str = "bytes://dropreel-empty.jpg";
/// Soft ribbons of light in greyscale, behind the drop overlay and the sidebar header.
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
    /// Focus rings, selected outlines and icons: soft white on dark, near-black on light.
    pub accent: Color32,
    pub accent_soft: Color32,
    /// The red of primary buttons, progress and the current-shelf marker.
    pub primary: Color32,
    /// Text and icons on `primary`.
    pub on_primary: Color32,
    /// Secondary buttons: Netflix's translucent grey.
    pub secondary: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub dark: bool,
}

/// The colour scheme, after Netflix: a near-black page, slightly lighter panels, soft white text with
/// a soft grey for secondary text, and one brick red for primary actions, progress and the current
/// selection. Warnings and success use the text colour; red also marks errors and destructive
/// actions.
pub mod ramp {
    use super::Color32;

    pub const N950: Color32 = Color32::from_gray(0);
    pub const N900: Color32 = Color32::from_gray(20);
    pub const N850: Color32 = Color32::from_gray(24);
    pub const N800: Color32 = Color32::from_gray(35);
    pub const N750: Color32 = Color32::from_gray(51);
    pub const N700: Color32 = Color32::from_gray(64);
    pub const N600: Color32 = Color32::from_gray(89);
    pub const N400: Color32 = Color32::from_gray(179);
    pub const N300: Color32 = Color32::from_gray(210);
    pub const N200: Color32 = Color32::from_gray(229);
    pub const N100: Color32 = Color32::from_gray(238);
    pub const N50: Color32 = Color32::from_gray(245);
    pub const WHITE: Color32 = Color32::WHITE;
    /// The soft white of text and white controls on dark: easier on the eyes than pure white.
    pub const ASH: Color32 = Color32::from_gray(217);

    /// A calm brick red, and a deeper one for red text on white.
    pub const RED: Color32 = Color32::from_rgb(179, 55, 58);
    pub const RED_DEEP: Color32 = Color32::from_rgb(143, 42, 45);
}

/// Corner radii: tight, like Netflix's controls and cards.
pub mod radius {
    /// Buttons, inputs, menus items, chips.
    pub const CONTROL: u8 = 4;
    /// Small badges on posters.
    pub const BADGE: u8 = 3;
    /// Poster cards and in-panel cards.
    pub const CARD: u8 = 6;
    /// Dialogs, menus and toasts.
    pub const PANEL: u8 = 8;
}

pub const DARK: Palette = Palette {
    bg: ramp::N900,
    surface: ramp::N850,
    raised: ramp::N800,
    hover: ramp::N750,
    input: ramp::N950,
    border: ramp::N700,
    text: ramp::ASH,
    text_weak: ramp::N400,
    accent: ramp::ASH,
    // White at 10%, premultiplied.
    accent_soft: Color32::from_rgba_premultiplied(26, 26, 26, 26),
    primary: ramp::RED,
    on_primary: ramp::ASH,
    // rgba(109, 109, 110, 0.7), premultiplied.
    secondary: Color32::from_rgba_premultiplied(76, 76, 77, 179),
    warn: ramp::ASH,
    danger: ramp::RED,
    success: ramp::ASH,
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: ramp::N50,
    surface: ramp::WHITE,
    raised: ramp::N100,
    hover: ramp::N200,
    input: ramp::WHITE,
    border: ramp::N300,
    text: ramp::N900,
    text_weak: ramp::N600,
    accent: ramp::N900,
    // Black at 7%, premultiplied.
    accent_soft: Color32::from_rgba_premultiplied(0, 0, 0, 18),
    primary: ramp::RED,
    on_primary: ramp::WHITE,
    secondary: ramp::N100,
    warn: ramp::N900,
    danger: ramp::RED_DEEP,
    success: ramp::N900,
    dark: false,
};

/// Colours for anything painted over posters, artwork or video. Those stay dark on
/// either theme, so these don't change with it: white content on black scrims and white "glass".
pub mod media {
    use super::Color32;

    /// Text and icons: the soft white, and the same at 75%.
    pub const TEXT: Color32 = super::ramp::ASH;
    pub const TEXT_WEAK: Color32 = Color32::from_rgba_premultiplied(163, 163, 163, 191);
    /// Frosted fills, from barely there to clearly visible.
    pub const GLASS_FAINT: Color32 = Color32::from_rgba_premultiplied(14, 14, 14, 14);
    pub const GLASS: Color32 = Color32::from_rgba_premultiplied(24, 24, 24, 24);
    /// Tracks, rails and quiet outlines.
    pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(40, 40, 40, 40);
    /// The rim of glass controls and frames.
    pub const EDGE: Color32 = Color32::from_rgba_premultiplied(64, 64, 64, 64);
    /// Darkening layers, so white content reads on any image.
    pub const SCRIM_SOFT: Color32 = Color32::from_black_alpha(100);
    pub const SCRIM: Color32 = Color32::from_black_alpha(140);
    pub const SCRIM_STRONG: Color32 = Color32::from_black_alpha(180);
    /// Badges: soft white with black content, or red with soft white.
    pub const PILL: Color32 = super::ramp::ASH;
    pub const ON_PILL: Color32 = super::ramp::N950;
    pub const DANGER: Color32 = super::ramp::RED;
    /// Progress on posters and in the player: red over a grey track.
    pub const PROGRESS: Color32 = super::ramp::RED;
    pub const TRACK: Color32 = Color32::from_rgba_premultiplied(77, 77, 77, 200);
}

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

    // Long enough that panels visibly slide and things fade, short enough to stay snappy.
    style.animation_time = 0.18;

    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 8.0);
    s.button_padding = vec2(12.0, 6.0);
    s.interact_size = vec2(36.0, 30.0);
    s.window_margin = Margin::same(18);
    s.menu_margin = Margin::same(6);
    s.combo_width = 120.0;
    s.icon_width = 16.0;
    s.scroll = egui::style::ScrollStyle::floating();

    style.visuals = visuals(p);
    style
}

/// egui's visuals in DropReel's colours. The player bar uses the dark ones on either theme.
pub fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.surface;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.input;
    v.faint_bg_color = if p.dark { ramp::N850 } else { ramp::N50 };
    v.window_corner_radius = CornerRadius::same(radius::PANEL);
    v.menu_corner_radius = CornerRadius::same(radius::PANEL);
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
    // A translucent red wash keeps selected text readable; sliders fill in red.
    v.selection.bg_fill = p.primary.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.hyperlink_color = p.accent;
    v.slider_trailing_fill = true;
    v.striped = false;

    let radius = CornerRadius::same(radius::CONTROL);
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
    v
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
