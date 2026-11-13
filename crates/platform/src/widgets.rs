//! Reusable interface pieces built on the design system.

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::theme::{self, ACCENT, ACCENT_HI, BORDER, RADIUS, RADIUS_SMALL, SURFACE, SURFACE_HI, SURFACE_HOVER, TEXT, TEXT_DIM, TEXT_FAINT};

/// How long hover and press transitions take; zeroed by "reduce motion".
pub fn anim_time(ui: &Ui) -> f32 {
    ui.style().animation_time
}

/// A vertical gradient fill for `rect`.
pub fn gradient(ui: &Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(mesh);
}

/// A horizontal gradient fill for `rect`.
pub fn gradient_h(ui: &Ui, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(mesh);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// A button with an optional leading icon.
pub fn button(ui: &mut Ui, kind: ButtonKind, icon: &str, label: &str) -> Response {
    button_sized(ui, kind, icon, label, 36.0)
}

pub fn button_sized(ui: &mut Ui, kind: ButtonKind, icon: &str, label: &str, height: f32) -> Response {
    let font = theme::medium(if height >= 44.0 { 16.0 } else { 14.0 });
    let icon_font = theme::regular(font.size + 3.0);
    let text_width = if label.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(label.to_string(), font.clone(), TEXT).size().x };
    let icon_width = if icon.is_empty() { 0.0 } else { icon_font.size };
    let gap = if icon.is_empty() || label.is_empty() { 0.0 } else { 8.0 };
    let padding = if label.is_empty() { (height - icon_width) / 2.0 } else { height * 0.45 };
    let size = vec2(padding * 2.0 + icon_width + gap + text_width, height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), anim_time(ui));
    let pressed = response.is_pointer_button_down_on();
    let enabled = ui.is_enabled();
    let (fill, text, stroke) = match kind {
        ButtonKind::Primary => (theme::mix(ACCENT, ACCENT_HI, hover), Color32::WHITE, Stroke::NONE),
        ButtonKind::Secondary => (theme::mix(SURFACE_HI, SURFACE_HOVER, hover), TEXT, Stroke::new(1.0, theme::mix(BORDER, ACCENT, hover * 0.7))),
        ButtonKind::Ghost => (theme::with_alpha(SURFACE_HOVER, hover), theme::mix(TEXT_DIM, TEXT, hover), Stroke::NONE),
        ButtonKind::Danger => (theme::mix(theme::with_alpha(theme::DANGER, 0.14), theme::with_alpha(theme::DANGER, 0.3), hover), theme::DANGER, Stroke::NONE),
    };
    let (fill, text) = if enabled { (fill, text) } else { (theme::with_alpha(fill, 0.4), theme::with_alpha(text, 0.4)) };
    let rect = if pressed { rect.shrink(1.0) } else { rect };
    let radius = CornerRadius::same((height / 2.0).min(RADIUS as f32) as u8);
    if kind == ButtonKind::Primary && enabled {
        // A soft glow that grows on hover.
        ui.painter().rect_filled(rect.expand(2.0 + 2.0 * hover).translate(vec2(0.0, 2.0)), radius, theme::with_alpha(ACCENT, 0.12 + 0.14 * hover));
    }
    ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
    let mut x = rect.center().x - (icon_width + gap + text_width) / 2.0;
    if !icon.is_empty() {
        ui.painter().text(pos2(x + icon_width / 2.0, rect.center().y), Align2::CENTER_CENTER, icon, icon_font, text);
        x += icon_width + gap;
    }
    if !label.is_empty() {
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, font, text);
    }
    response
}

/// A square icon-only button with a tooltip.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    button_sized(ui, ButtonKind::Ghost, icon, "", 34.0).on_hover_text(tooltip)
}

/// A selectable pill, used for category filters and tabs.
pub fn chip(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let font = theme::medium(13.0);
    let width = ui.painter().layout_no_wrap(label.to_string(), font.clone(), TEXT).size().x;
    let (rect, response) = ui.allocate_exact_size(vec2(width + 28.0, 32.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), anim_time(ui));
    let on = ui.ctx().animate_bool_with_time(response.id.with("on"), selected, anim_time(ui));
    let fill = theme::mix(theme::mix(SURFACE_HI, SURFACE_HOVER, hover), ACCENT, on);
    ui.painter().rect_filled(rect, CornerRadius::same(16), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, theme::mix(TEXT_DIM, Color32::WHITE, on.max(hover * 0.6)));
    response
}

/// An on/off switch.
pub fn toggle(ui: &mut Ui, value: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(44.0, 24.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let on = ui.ctx().animate_bool_with_time(response.id, *value, anim_time(ui));
    ui.painter().rect_filled(rect, CornerRadius::same(12), theme::mix(SURFACE_HOVER, ACCENT, on));
    let x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), on);
    ui.painter().circle_filled(pos2(x, rect.center().y), 9.0, Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A rounded search field. Returns the text edit's response.
pub fn search_box(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    let id = Id::new(("search", hint));
    let focused = ui.memory(|m| m.has_focus(id));
    let glow = ui.ctx().animate_bool_with_time(id.with("focus"), focused, anim_time(ui));
    ui.painter().rect(rect, CornerRadius::same(20), SURFACE_HI, Stroke::new(1.0, theme::mix(BORDER, ACCENT, glow)), StrokeKind::Inside);
    ui.painter().text(pos2(rect.left() + 22.0, rect.center().y), Align2::CENTER_CENTER, crate::icons::MAGNIFYING_GLASS, theme::regular(17.0), TEXT_FAINT);
    let inner = Rect::from_min_max(pos2(rect.left() + 40.0, rect.top() + 4.0), pos2(rect.right() - 14.0, rect.bottom() - 4.0));
    let edit = egui::TextEdit::singleline(text).id(id).hint_text(hint).frame(egui::Frame::NONE).font(theme::regular(14.0)).vertical_align(egui::Align::Center);
    ui.put(inner, edit)
}

/// Title with an optional subtitle, used above each content section.
pub fn section_header(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).font(theme::semibold(20.0)).color(TEXT));
    if !subtitle.is_empty() {
        ui.add_space(-6.0);
        ui.label(egui::RichText::new(subtitle).font(theme::regular(13.0)).color(TEXT_DIM));
    }
    ui.add_space(4.0);
}

/// A friendly placeholder for screens with nothing to show yet. `action`
/// draws optional buttons under the message.
pub fn empty_state(ui: &mut Ui, icon: &str, title: &str, body: &str, action: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    egui::Frame::new().fill(SURFACE).corner_radius(CornerRadius::same(RADIUS)).stroke(Stroke::new(1.0, BORDER)).inner_margin(egui::Margin::same(28)).show(ui, |ui| {
        ui.set_width(width - 58.0);
        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 32.0, theme::with_alpha(ACCENT, 0.16));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, theme::regular(30.0), ACCENT_HI);
            ui.add_space(6.0);
            ui.label(egui::RichText::new(title).font(theme::semibold(17.0)));
            ui.label(egui::RichText::new(body).font(theme::regular(13.5)).color(TEXT_DIM));
            ui.add_space(8.0);
            action(ui);
        });
    });
}

/// A small rounded label painted at `pos` (its top-left corner). Returns
/// the rectangle it occupies.
pub fn badge(ui: &Ui, pos: Pos2, text: &str, fill: Color32, text_color: Color32) -> Rect {
    let font = theme::semibold(11.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, text_color);
    let rect = Rect::from_min_size(pos, galley.size() + vec2(16.0, 8.0));
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_SMALL), fill);
    ui.painter().galley(rect.min + vec2(8.0, 4.0), galley, text_color);
    rect
}

/// An indeterminate loading ring centred on `center`.
pub fn spinner(ui: &Ui, center: Pos2, radius: f32) {
    let time = ui.input(|i| i.time) as f32;
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = time * 5.0 + i as f32 / 24.0 * std::f32::consts::PI * 1.4;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(3.0, ACCENT_HI)));
    ui.ctx().request_repaint();
}

/// A surface card container.
pub fn card<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SURFACE)
        .corner_radius(CornerRadius::same(RADIUS))
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(egui::Margin::same(18))
        .show(ui, add_contents)
        .inner
}

/// Truncates text with an ellipsis so it fits `max_width`.
pub fn elide(ui: &Ui, text: &str, font: egui::FontId, max_width: f32) -> String {
    let fits = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), TEXT).size().x <= max_width;
    if fits(text) {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut end = chars.len();
    while end > 1 {
        end -= 1;
        let candidate: String = chars[..end].iter().collect::<String>().trim_end().to_string() + "…";
        if fits(&candidate) {
            return candidate;
        }
    }
    "…".to_string()
}
