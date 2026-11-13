//! Araria's design system: palette, typography and base widget styling.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals};

pub const BG: Color32 = Color32::from_rgb(13, 15, 23);
pub const SIDEBAR: Color32 = Color32::from_rgb(16, 18, 28);
pub const SURFACE: Color32 = Color32::from_rgb(23, 26, 39);
pub const SURFACE_HI: Color32 = Color32::from_rgb(31, 35, 52);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(40, 45, 67);
pub const BORDER: Color32 = Color32::from_rgb(42, 47, 69);
pub const TEXT: Color32 = Color32::from_rgb(238, 240, 247);
pub const TEXT_DIM: Color32 = Color32::from_rgb(166, 172, 192);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(110, 116, 140);
pub const ACCENT: Color32 = Color32::from_rgb(124, 92, 255);
pub const ACCENT_HI: Color32 = Color32::from_rgb(154, 130, 255);
pub const ACCENT_2: Color32 = Color32::from_rgb(34, 211, 238);
pub const SUCCESS: Color32 = Color32::from_rgb(74, 222, 128);
pub const WARNING: Color32 = Color32::from_rgb(251, 191, 36);
pub const DANGER: Color32 = Color32::from_rgb(248, 113, 113);

pub const RADIUS: u8 = 12;
pub const RADIUS_SMALL: u8 = 8;

fn family(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, family("medium"))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, family("semibold"))
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, family("bold"))
}
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
pub fn mono_bold(size: f32) -> FontId {
    FontId::new(size, family("mono-bold"))
}

/// Blends two colors (`t` = 0 gives `a`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

pub fn with_alpha(color: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

/// A stable, pleasant color derived from a string (placeholder covers,
/// category accents).
pub fn color_for(text: &str) -> Color32 {
    let hash = text.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
    let hue = (hash % 360) as f32;
    let c = araria_core::Color::from_hsv(hue, 0.62, 0.86).to_rgb8();
    Color32::from_rgb(c[0], c[1], c[2])
}

pub fn category_color(category: &str) -> Color32 {
    match category {
        "Obby" => Color32::from_rgb(255, 122, 89),
        "Sandbox" => Color32::from_rgb(74, 222, 128),
        "Racing" => Color32::from_rgb(248, 113, 113),
        "Survival" => Color32::from_rgb(251, 191, 36),
        "Platformer" => Color32::from_rgb(96, 165, 250),
        "Arena" => Color32::from_rgb(244, 114, 182),
        "Adventure" => Color32::from_rgb(45, 212, 191),
        _ => ACCENT_HI,
    }
}

/// Installs fonts and the base style. Call once per egui context.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
    };
    add("poppins", include_bytes!("../../../assets/fonts/Poppins-Regular.ttf"));
    add("poppins-medium", include_bytes!("../../../assets/fonts/Poppins-Medium.ttf"));
    add("poppins-semibold", include_bytes!("../../../assets/fonts/Poppins-SemiBold.ttf"));
    add("poppins-bold", include_bytes!("../../../assets/fonts/Poppins-Bold.ttf"));
    add("mono", include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"));
    add("mono-bold", include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf"));
    add("icons", include_bytes!("../../../assets/fonts/Phosphor.ttf"));
    // Icons are a fallback in every family so they can be mixed into text.
    let stack = |primary: &str, fallback: &str| vec![primary.to_string(), "icons".to_string(), fallback.to_string()];
    fonts.families.insert(FontFamily::Proportional, stack("poppins", "mono"));
    fonts.families.insert(FontFamily::Monospace, stack("mono", "poppins"));
    fonts.families.insert(family("medium"), stack("poppins-medium", "mono"));
    fonts.families.insert(family("semibold"), stack("poppins-semibold", "mono"));
    fonts.families.insert(family("bold"), stack("poppins-bold", "mono"));
    fonts.families.insert(family("mono-bold"), stack("mono-bold", "poppins"));
    ctx.set_fonts(fonts);

    let mut visuals = Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = Color32::from_rgb(10, 12, 18);
    visuals.faint_bg_color = SURFACE_HI;
    visuals.override_text_color = Some(TEXT);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS_SMALL);
    visuals.selection.bg_fill = with_alpha(ACCENT, 0.45);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT_HI);
    visuals.hyperlink_color = ACCENT_HI;
    let radius = CornerRadius::same(RADIUS_SMALL);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = SURFACE_HI;
    w.inactive.weak_bg_fill = SURFACE_HI;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = SURFACE_HOVER;
    w.hovered.weak_bg_fill = SURFACE_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    w.hovered.corner_radius = radius;
    w.active.bg_fill = ACCENT;
    w.active.weak_bg_fill = ACCENT;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT_HI);
    w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.active.corner_radius = radius;
    w.open.bg_fill = SURFACE_HOVER;
    w.open.weak_bg_fill = SURFACE_HOVER;
    w.open.corner_radius = radius;
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 7.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = true;
        style.text_styles = [
            (egui::TextStyle::Heading, semibold(22.0)),
            (egui::TextStyle::Body, regular(14.0)),
            (egui::TextStyle::Button, medium(14.0)),
            (egui::TextStyle::Small, regular(12.0)),
            (egui::TextStyle::Monospace, mono(13.0)),
        ]
        .into();
    });
}
