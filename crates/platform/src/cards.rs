//! Game covers and cards.

use std::collections::HashMap;

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};

use crate::library::{GameEntry, GameKey, Library, Origin};
use crate::theme::{self, ACCENT, BORDER, RADIUS, SURFACE, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::widgets::{self, ButtonKind};
use crate::icons;

/// Decoded cover images, cached per game.
#[derive(Default)]
pub struct Thumbnails {
    cache: HashMap<GameKey, Option<TextureHandle>>,
}

impl Thumbnails {
    pub fn get(&mut self, ctx: &egui::Context, library: &Library, key: &GameKey) -> Option<TextureHandle> {
        self.cache
            .entry(key.clone())
            .or_insert_with(|| {
                let bytes = library.thumbnail(key)?;
                let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
                let size = [decoded.width() as usize, decoded.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
                Some(ctx.load_texture(key.label(), pixels, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    /// Forgets cached covers (after a project's thumbnail changes).
    pub fn invalidate(&mut self, key: &GameKey) {
        self.cache.remove(key);
    }
}

pub fn category_icon(category: &str) -> &'static str {
    match category {
        "Obby" => icons::FLAG,
        "Sandbox" => icons::CUBE,
        "Racing" => icons::CAR,
        "Survival" => icons::CAMPFIRE,
        "Platformer" => icons::LIGHTNING,
        "Arena" => icons::SWORD,
        "Adventure" => icons::MOUNTAINS,
        _ => icons::GAME_CONTROLLER,
    }
}

/// Paints a game's cover into `rect`: its thumbnail cropped to fill, or a
/// generated placeholder when it has none.
pub fn cover(ui: &Ui, thumbs: &mut Thumbnails, library: &Library, entry: &GameEntry, rect: Rect, radius: CornerRadius) {
    match thumbs.get(ui.ctx(), library, &entry.key) {
        Some(texture) => {
            // "Cover" fit: crop the image so it fills the rectangle.
            let image = texture.size_vec2();
            let (image_aspect, rect_aspect) = (image.x / image.y, rect.width() / rect.height());
            let uv = if image_aspect > rect_aspect {
                let w = rect_aspect / image_aspect;
                Rect::from_min_max(pos2(0.5 - w / 2.0, 0.0), pos2(0.5 + w / 2.0, 1.0))
            } else {
                let h = image_aspect / rect_aspect;
                Rect::from_min_max(pos2(0.0, 0.5 - h / 2.0), pos2(1.0, 0.5 + h / 2.0))
            };
            egui::Image::from_texture((texture.id(), rect.size())).uv(uv).corner_radius(radius).paint_at(ui, rect);
        }
        None => {
            let base = theme::category_color(&entry.manifest.category);
            let tint = theme::color_for(&entry.manifest.id);
            ui.painter().rect_filled(rect, radius, theme::mix(theme::mix(base, tint, 0.35), Color32::BLACK, 0.45));
            let painter = ui.painter().with_clip_rect(rect);
            // Soft overlapping discs give each placeholder its own look.
            let seed = entry.manifest.id.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
            for i in 0..4u32 {
                let f = |shift: u32| ((seed >> (shift + i * 5)) & 0xff) as f32 / 255.0;
                let center = rect.min + vec2(f(0) * rect.width(), f(3) * rect.height());
                painter.circle_filled(center, rect.height() * (0.35 + 0.4 * f(6)), theme::with_alpha(theme::mix(base, tint, f(1)), 0.2));
            }
            painter.text(rect.center(), Align2::CENTER_CENTER, category_icon(&entry.manifest.category), theme::regular(rect.height() * 0.38), theme::with_alpha(Color32::WHITE, 0.85));
        }
    }
}

/// What the player did with a card this frame.
#[derive(Default, Clone, Copy)]
pub struct CardResponse {
    pub opened: bool,
    pub play: bool,
}

/// A game card: cover, title, category and a play button on hover.
pub fn game_card(ui: &mut Ui, thumbs: &mut Thumbnails, library: &Library, entry: &GameEntry, width: f32) -> CardResponse {
    let cover_height = (width * 9.0 / 16.0).round();
    let (rect, response) = ui.allocate_exact_size(vec2(width, cover_height + 68.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut out = CardResponse { opened: response.clicked(), play: false };
    if !ui.is_rect_visible(rect) {
        return out;
    }
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered() || response.has_focus(), widgets::anim_time(ui));
    let rect = rect.translate(vec2(0.0, -4.0 * hover));
    let radius = CornerRadius::same(RADIUS);
    if hover > 0.0 {
        ui.painter().rect_filled(rect.expand(3.0 * hover).translate(vec2(0.0, 6.0 * hover)), radius, theme::with_alpha(Color32::BLACK, 0.35 * hover));
    }
    ui.painter().rect(rect, radius, theme::mix(SURFACE, theme::SURFACE_HI, hover), Stroke::new(1.0, theme::mix(BORDER, ACCENT, hover)), StrokeKind::Inside);

    let cover_rect = Rect::from_min_size(rect.min, vec2(width, cover_height)).shrink(1.0);
    cover(ui, thumbs, library, entry, cover_rect, CornerRadius { nw: RADIUS, ne: RADIUS, sw: 0, se: 0 });

    let category = entry.manifest.category.as_str();
    widgets::badge(ui, cover_rect.min + vec2(10.0, 10.0), category, theme::with_alpha(Color32::BLACK, 0.55), theme::category_color(category));
    if entry.key.origin == Origin::Project {
        widgets::badge(ui, pos2(cover_rect.right() - 74.0, cover_rect.top() + 10.0), "PROJECT", theme::with_alpha(ACCENT, 0.85), Color32::WHITE);
    } else if entry.installed {
        widgets::badge(ui, pos2(cover_rect.right() - 86.0, cover_rect.top() + 10.0), "INSTALLED", theme::with_alpha(Color32::BLACK, 0.55), theme::SUCCESS);
    }

    let text_left = rect.left() + 14.0;
    let title_font = theme::semibold(15.0);
    let title = widgets::elide(ui, &entry.manifest.name, title_font.clone(), width - 28.0);
    ui.painter().text(pos2(text_left, cover_rect.bottom() + 12.0), Align2::LEFT_TOP, title, title_font, TEXT);
    let plays = library.play_count(&entry.key);
    let meta = if plays > 0 { format!("{}  ·  played {plays}×", players_label(entry.manifest.max_players)) } else { players_label(entry.manifest.max_players) };
    ui.painter().text(pos2(text_left, cover_rect.bottom() + 36.0), Align2::LEFT_TOP, format!("{}  {meta}", icons::USERS), theme::regular(12.5), TEXT_DIM);

    // The play button fades in over the cover.
    if hover > 0.01 {
        let center = cover_rect.center();
        let play_rect = Rect::from_center_size(center, vec2(52.0, 52.0));
        let play = ui.interact(play_rect, response.id.with("play"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(cover_rect, CornerRadius { nw: RADIUS, ne: RADIUS, sw: 0, se: 0 }, theme::with_alpha(Color32::BLACK, 0.35 * hover));
        let scale = 0.8 + 0.2 * hover + if play.hovered() { 0.08 } else { 0.0 };
        ui.painter().circle_filled(center, 26.0 * scale, theme::with_alpha(ACCENT, hover));
        ui.painter().text(center + vec2(1.5, 0.0), Align2::CENTER_CENTER, icons::PLAY, theme::regular(22.0 * scale), theme::with_alpha(Color32::WHITE, hover));
        if play.clicked() {
            out.play = true;
            out.opened = false;
        }
    }
    out
}

pub fn players_label(max_players: u32) -> String {
    if max_players <= 1 { "Single player".to_string() } else { format!("1–{max_players} players") }
}

/// The wide banner for a featured game. Returns (play clicked, details clicked).
pub fn hero(ui: &mut Ui, thumbs: &mut Thumbnails, library: &Library, entry: &GameEntry, height: f32) -> (bool, bool) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let radius = CornerRadius::same(18);
    cover(ui, thumbs, library, entry, rect, radius);
    // Darken toward the left and bottom so the text stays readable.
    let shade = Color32::from_rgb(10, 11, 18);
    ui.painter().rect_filled(rect, radius, theme::with_alpha(shade, 0.25));
    let left = Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * 0.62, rect.bottom()));
    widgets::gradient_h(ui, left.shrink2(vec2(0.0, 0.5)), theme::with_alpha(shade, 0.92), theme::with_alpha(shade, 0.0));
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, BORDER), StrokeKind::Inside);

    let x = rect.left() + 36.0;
    let mut y = rect.top() + 34.0;
    let badge = widgets::badge(ui, pos2(x, y), "FEATURED", ACCENT, Color32::WHITE);
    widgets::badge(ui, pos2(badge.right() + 8.0, y), &entry.manifest.category.to_uppercase(), theme::with_alpha(Color32::BLACK, 0.5), theme::category_color(&entry.manifest.category));
    y += 40.0;
    ui.painter().text(pos2(x, y), Align2::LEFT_TOP, &entry.manifest.name, theme::bold(40.0), Color32::WHITE);
    y += 58.0;
    let description = ui.painter().layout(entry.manifest.description.clone(), theme::regular(15.0), TEXT, (rect.width() * 0.46).max(240.0));
    // Never run into the buttons below.
    let description_height = description.size().y.min(66.0).min((rect.bottom() - 92.0 - y).max(0.0));
    ui.painter().with_clip_rect(Rect::from_min_size(pos2(x, y), vec2(rect.width(), description_height))).galley(pos2(x, y), description, TEXT);

    let buttons = Rect::from_min_size(pos2(x, rect.bottom() - 78.0), vec2(420.0, 48.0));
    let mut clicked = (false, false);
    ui.scope_builder(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        clicked.0 = widgets::button_sized(ui, ButtonKind::Primary, icons::PLAY, "Play now", 48.0).clicked();
        clicked.1 = widgets::button_sized(ui, ButtonKind::Secondary, icons::INFO, "Details", 48.0).clicked();
    });
    let _ = TEXT_FAINT;
    clicked
}
