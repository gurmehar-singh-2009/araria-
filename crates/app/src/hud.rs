//! Everything a game draws over the 3D view: the interface scripts build
//! under `Gui`, labels and name tags floating in the world, interaction
//! prompts, notifications, the leaderboard and the health bar.

use std::collections::HashSet;

use araria_core::{DataModel, Instance, InstanceId, Variant, Vec3};
use araria_platform::theme;
use araria_runtime::{Command, Runtime};
use bevy_egui::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind};

/// A message shown at the top of the screen for a few seconds.
struct Notice {
    text: String,
    age: f32,
    duration: f32,
}

/// Overlay state that lives across frames.
#[derive(Default)]
pub struct Hud {
    notices: Vec<Notice>,
    /// Interface elements the pointer was over last frame.
    hovered: HashSet<InstanceId>,
    /// Seconds the local character has been dead.
    dead_for: f32,
}

/// What the overlay needs from the frame it is drawn in.
pub struct HudFrame<'a> {
    pub runtime: &'a Runtime,
    /// Requests scripts made this frame.
    pub commands: &'a [Command],
    /// The part of the window showing the game.
    pub area: Rect,
    pub dt: f32,
    /// False when something else (a menu) owns the pointer.
    pub interactive: bool,
    /// Resolves a texture asset id for `ImageLabel`.
    pub texture: &'a mut dyn FnMut(&str) -> Option<egui::TextureId>,
}

fn rgba(color: araria_core::Color, alpha: f32) -> Color32 {
    let [r, g, b] = color.to_rgb8();
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn plate() -> Color32 {
    theme::with_alpha(theme::BG, 0.78)
}

impl Hud {
    pub fn clear(&mut self) {
        *self = Hud::default();
    }

    /// Draws the overlay. Returns true when the pointer is over an
    /// interface element, so the game should not also react to the click.
    pub fn draw(&mut self, ui: &egui::Ui, frame: &mut HudFrame) -> bool {
        let painter = ui.painter().with_clip_rect(frame.area);
        let mut over_interface = false;
        {
            let dm = frame.runtime.dm.borrow();
            let project = |world: Vec3| -> Option<Pos2> {
                let (screen, visible) = frame.runtime.input.borrow().world_to_screen(world);
                visible.then(|| frame.area.min + egui::vec2(screen.x, screen.y))
            };
            let eye = {
                let camera = dm.get(dm.service("Camera"));
                camera.map_or(Vec3::ZERO, |c| c.vec3("Position"))
            };
            world_labels(&painter, &dm, &project, eye, frame.area);
            if let Some(active) = frame.runtime.active_prompt() {
                prompt(&painter, &dm, &project, active.prompt, active.progress);
            }
        }

        // The game's own interface.
        let mut events: Vec<(InstanceId, &'static str)> = Vec::new();
        let mut hovered = HashSet::new();
        {
            let dm = frame.runtime.dm.borrow();
            let mut children: Vec<&Instance> = dm.children(dm.service("Gui")).iter().filter_map(|c| dm.get(*c)).collect();
            children.sort_by(|a, b| a.f32("ZIndex").total_cmp(&b.f32("ZIndex")));
            for child in children {
                over_interface |= self.element(ui, &painter, &dm, child, frame.area, frame, &mut events, &mut hovered);
            }
        }
        for id in hovered.difference(&self.hovered) {
            events.push((*id, "MouseEnter"));
        }
        for id in self.hovered.difference(&hovered) {
            events.push((*id, "MouseLeave"));
        }
        self.hovered = hovered;
        if !events.is_empty() {
            let mut dm = frame.runtime.dm.borrow_mut();
            for (id, name) in events {
                if dm.contains(id) {
                    dm.fire(id, name, Vec::new());
                }
            }
        }

        let dm = frame.runtime.dm.borrow();
        leaderboard(&painter, &dm, frame.area);
        self.vitals(&painter, &dm, frame);
        self.notifications(&painter, frame);
        over_interface
    }

    /// Draws one interface element and its children. Returns whether the
    /// pointer is over something solid.
    #[allow(clippy::too_many_arguments)]
    fn element(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        dm: &DataModel,
        instance: &Instance,
        parent: Rect,
        frame: &mut HudFrame,
        events: &mut Vec<(InstanceId, &'static str)>,
        hovered: &mut HashSet<InstanceId>,
    ) -> bool {
        if !instance.is_a("GuiObject") || !instance.bool("Visible") {
            return false;
        }
        let parent_size = araria_core::Vec2::new(parent.width(), parent.height());
        let size = instance.udim2("Size").resolve(parent_size).max(araria_core::Vec2::ZERO);
        let anchor = instance.vec2("AnchorPoint");
        let position = instance.udim2("Position").resolve(parent_size) - anchor * size;
        let rect = Rect::from_min_size(parent.min + egui::vec2(position.x, position.y), egui::vec2(size.x, size.y));
        let radius = CornerRadius::same(instance.f32("CornerRadius").min(size.min_element() * 0.5).clamp(0.0, 255.0) as u8);
        let opacity = 1.0 - instance.f32("BackgroundTransparency");
        let pointer_inside = frame.interactive && ui.ctx().pointer_latest_pos().is_some_and(|p| rect.contains(p) && frame.area.contains(p));
        let is_button = instance.is_a("TextButton");
        let mut solid = pointer_inside && (opacity > 0.05 || is_button);
        if pointer_inside {
            hovered.insert(instance.id);
        }

        let mut fill = instance.color("BackgroundColor");
        if is_button {
            let response = ui.interact(rect, egui::Id::new(("gui", instance.id.0)), if frame.interactive { Sense::click() } else { Sense::hover() });
            if response.hovered() {
                fill = instance.color("HoverColor");
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                events.push((instance.id, "Activated"));
            }
        }
        if opacity > 0.0 {
            painter.rect_filled(rect, radius, rgba(fill, opacity));
        }
        let border = instance.f32("BorderSize");
        if border > 0.0 {
            painter.rect_stroke(rect, radius, Stroke::new(border, rgba(instance.color("BorderColor"), 1.0)), StrokeKind::Inside);
        }

        if instance.is_a("TextLabel") {
            let size = instance.f32("TextSize");
            let font = if instance.bool("Bold") { theme::bold(size) } else { theme::medium(size) };
            let galley = painter.layout(instance.str("Text").to_string(), font, rgba(instance.color("TextColor"), 1.0), (rect.width() - 16.0).max(8.0));
            let x = match instance.str("TextAlign") {
                "Left" => rect.left() + 8.0,
                "Right" => rect.right() - 8.0 - galley.size().x,
                _ => rect.center().x - galley.size().x * 0.5,
            };
            painter.galley(Pos2::new(x, rect.center().y - galley.size().y * 0.5), galley, Color32::WHITE);
        } else if instance.is_a("ProgressBar") {
            let value = instance.f32("Value").clamp(0.0, 1.0);
            if value > 0.0 {
                let filled = Rect::from_min_size(rect.min, egui::vec2((rect.width() * value).max(rect.height().min(rect.width())), rect.height()));
                painter.with_clip_rect(Rect::from_min_size(rect.min, egui::vec2(rect.width() * value, rect.height())).intersect(frame.area)).rect_filled(filled, radius, rgba(instance.color("FillColor"), 1.0));
            }
        } else if instance.is_a("ImageLabel") {
            if let Some(texture) = (frame.texture)(instance.str("Image")) {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture, rect, uv, rgba(instance.color("ImageColor"), 1.0));
            }
        }

        let mut children: Vec<&Instance> = instance.children.iter().filter_map(|c| dm.get(*c)).collect();
        children.sort_by(|a, b| a.f32("ZIndex").total_cmp(&b.f32("ZIndex")));
        for child in children {
            solid |= self.element(ui, painter, dm, child, rect, frame, events, hovered);
        }
        solid
    }

    /// Health bar and the "respawning" banner for the local character.
    fn vitals(&mut self, painter: &egui::Painter, dm: &DataModel, frame: &HudFrame) {
        let Some(character) = frame.runtime.local_character().and_then(|c| dm.get(c)) else {
            self.dead_for = 0.0;
            return;
        };
        let (health, max) = (character.f32("Health"), character.f32("MaxHealth").max(1.0));
        let area = frame.area;
        if health <= 0.0 {
            self.dead_for += frame.dt;
            let fade = (self.dead_for * 2.5).min(1.0);
            painter.rect_filled(area, 0, theme::with_alpha(Color32::from_rgb(40, 6, 12), 0.35 * fade));
            painter.text(area.center() - egui::vec2(0.0, 30.0), Align2::CENTER_CENTER, "You were knocked out", theme::bold(30.0), theme::with_alpha(Color32::WHITE, fade));
            painter.text(area.center() + egui::vec2(0.0, 6.0), Align2::CENTER_CENTER, "Respawning…", theme::regular(15.0), theme::with_alpha(theme::TEXT_DIM, fade));
            return;
        }
        self.dead_for = 0.0;
        if health >= max {
            return;
        }
        let bar = Rect::from_center_size(Pos2::new(area.center().x, area.bottom() - 34.0), egui::vec2(260.0, 14.0));
        painter.rect_filled(bar.expand(3.0), 10, plate());
        let fraction = (health / max).clamp(0.0, 1.0);
        let fill = theme::mix(theme::DANGER, theme::SUCCESS, fraction);
        painter.rect_filled(Rect::from_min_size(bar.min, egui::vec2(bar.width() * fraction, bar.height())), 7, fill);
        painter.text(bar.center(), Align2::CENTER_CENTER, format!("{health:.0} / {max:.0}"), theme::semibold(10.5), Color32::WHITE);
    }

    fn notifications(&mut self, painter: &egui::Painter, frame: &HudFrame) {
        for command in frame.commands {
            if let Command::Notify { player, text, duration } = command {
                // Messages addressed to another player are theirs to see.
                if player.is_none() || *player == frame.runtime.local_player() {
                    self.notices.push(Notice { text: text.clone(), age: 0.0, duration: duration.clamp(0.5, 30.0) });
                }
            }
        }
        let overflow = self.notices.len().saturating_sub(4);
        self.notices.drain(..overflow);
        let mut y = frame.area.top() + 70.0;
        for notice in &mut self.notices {
            notice.age += frame.dt;
            let fade = (notice.age * 6.0).min(1.0) * ((notice.duration - notice.age) * 3.0).clamp(0.0, 1.0);
            let galley = painter.layout_no_wrap(notice.text.clone(), theme::semibold(15.0), theme::with_alpha(Color32::WHITE, fade));
            let size = galley.size() + egui::vec2(36.0, 20.0);
            let slide = (1.0 - (notice.age * 6.0).min(1.0)) * -12.0;
            let rect = Rect::from_center_size(Pos2::new(frame.area.center().x, y + size.y * 0.5 + slide), size);
            painter.rect_filled(rect, 18, theme::with_alpha(theme::BG, 0.82 * fade));
            painter.rect_stroke(rect, 18, Stroke::new(1.0, theme::with_alpha(theme::ACCENT_HI, 0.6 * fade)), StrokeKind::Inside);
            painter.galley(rect.min + egui::vec2(18.0, 10.0), galley, Color32::WHITE);
            y += size.y + 8.0;
        }
        self.notices.retain(|n| n.age < n.duration);
    }
}

/// Billboard labels and character name tags.
fn world_labels(painter: &egui::Painter, dm: &DataModel, project: &dyn Fn(Vec3) -> Option<Pos2>, eye: Vec3, area: Rect) {
    for id in dm.descendants(dm.workspace()) {
        let Some(instance) = dm.get(id) else { continue };
        let (text, world, color, size, max_distance, background) = match instance.class_name() {
            "BillboardLabel" if instance.bool("Enabled") => {
                let Some(world) = dm.world_position(id) else { continue };
                (instance.str("Text"), world, rgba(instance.color("Color"), 1.0), instance.f32("TextSize"), instance.f32("MaxDistance"), instance.bool("Background"))
            }
            "Character" if instance.bool("ShowName") && !instance.str("DisplayName").is_empty() => {
                let world = instance.vec3("Position") + Vec3::Y * 2.35 * instance.f32("Scale");
                (instance.str("DisplayName"), world, Color32::WHITE, 14.0, 45.0, true)
            }
            _ => continue,
        };
        let distance = eye.distance(world);
        if distance > max_distance || text.is_empty() {
            continue;
        }
        let Some(center) = project(world) else { continue };
        if !area.expand(80.0).contains(center) {
            continue;
        }
        // Fade out over the last fifth of the visible range.
        let fade = ((max_distance - distance) / (max_distance * 0.2).max(0.01)).clamp(0.0, 1.0);
        let galley = painter.layout_no_wrap(text.to_string(), theme::semibold(size), theme::with_alpha(color, fade));
        let rect = Rect::from_center_size(center, galley.size() + egui::vec2(18.0, 8.0));
        if background {
            painter.rect_filled(rect, CornerRadius::same(9), theme::with_alpha(theme::BG, 0.68 * fade));
        }
        painter.galley(rect.min + egui::vec2(9.0, 4.0), galley, Color32::WHITE);
    }
}

/// The key hint for the prompt in reach.
fn prompt(painter: &egui::Painter, dm: &DataModel, project: &dyn Fn(Vec3) -> Option<Pos2>, id: InstanceId, progress: f32) {
    let Some(instance) = dm.get(id) else { return };
    let Some(center) = dm.world_position(id).and_then(|world| project(world + Vec3::Y * 0.6)) else { return };
    let action = painter.layout_no_wrap(instance.str("ActionText").to_string(), theme::semibold(15.0), Color32::WHITE);
    let object = (!instance.str("ObjectText").is_empty()).then(|| painter.layout_no_wrap(instance.str("ObjectText").to_string(), theme::regular(11.5), theme::TEXT_DIM));
    let text_width = action.size().x.max(object.as_ref().map_or(0.0, |o| o.size().x));
    let key_size = 34.0;
    let size = egui::vec2(key_size + 14.0 + text_width + 26.0, 50.0);
    let rect = Rect::from_center_size(center, size);
    painter.rect_filled(rect, 14, plate());
    painter.rect_stroke(rect, 14, Stroke::new(1.0, theme::with_alpha(Color32::WHITE, 0.14)), StrokeKind::Inside);
    let key = Rect::from_min_size(rect.min + egui::vec2(10.0, 8.0), egui::vec2(key_size, key_size));
    painter.rect_filled(key, 9, theme::ACCENT);
    if progress > 0.0 {
        // Hold-to-interact: the key cap fills from the bottom.
        let filled = Rect::from_min_max(Pos2::new(key.left(), key.bottom() - key.height() * progress), key.max);
        painter.with_clip_rect(filled).rect_filled(key, 9, theme::ACCENT_2);
    }
    painter.text(key.center(), Align2::CENTER_CENTER, instance.str("Key"), theme::bold(if instance.str("Key").len() > 1 { 10.0 } else { 16.0 }), Color32::WHITE);
    let text_x = key.right() + 12.0;
    match object {
        Some(object) => {
            painter.galley(Pos2::new(text_x, rect.top() + 8.0), object, theme::TEXT_DIM);
            painter.galley(Pos2::new(text_x, rect.top() + 22.0), action, Color32::WHITE);
        }
        None => painter.galley(Pos2::new(text_x, rect.center().y - action.size().y * 0.5), action, Color32::WHITE),
    }
}

fn stat_text(value: &Variant) -> String {
    match value {
        Variant::Number(n) if n.fract() == 0.0 => format!("{n:.0}"),
        Variant::Number(n) => format!("{n:.1}"),
        Variant::String(s) => s.clone(),
        Variant::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Player stats set with `player:SetStat`, shown top-right.
fn leaderboard(painter: &egui::Painter, dm: &DataModel, area: Rect) {
    let players: Vec<&Instance> = dm.children(dm.service("Players")).iter().filter_map(|p| dm.get(*p)).collect();
    let mut columns: Vec<&str> = Vec::new();
    for player in &players {
        for key in player.attributes.keys() {
            if let Some(name) = key.strip_prefix("stat:") {
                if !columns.contains(&name) {
                    columns.push(name);
                }
            }
        }
    }
    if columns.is_empty() {
        return;
    }
    columns.truncate(3);
    let (name_width, column_width, row_height) = (118.0, 74.0, 26.0);
    let width = name_width + column_width * columns.len() as f32 + 20.0;
    let height = 30.0 + row_height * players.len() as f32 + 8.0;
    let rect = Rect::from_min_size(Pos2::new(area.right() - width - 16.0, area.top() + 16.0), egui::vec2(width, height));
    painter.rect_filled(rect, 14, plate());
    for (i, column) in columns.iter().enumerate() {
        let x = rect.left() + 10.0 + name_width + column_width * (i as f32 + 0.5);
        painter.text(Pos2::new(x, rect.top() + 16.0), Align2::CENTER_CENTER, column.to_uppercase(), theme::semibold(10.0), theme::TEXT_FAINT);
    }
    for (row, player) in players.iter().enumerate() {
        let y = rect.top() + 30.0 + row_height * (row as f32 + 0.5);
        painter.circle_filled(Pos2::new(rect.left() + 18.0, y), 5.0, rgba(player.color("Color"), 1.0));
        let font = if player.bool("IsLocal") { theme::semibold(13.0) } else { theme::regular(13.0) };
        painter.text(Pos2::new(rect.left() + 30.0, y), Align2::LEFT_CENTER, &player.name, font, theme::TEXT);
        for (i, column) in columns.iter().enumerate() {
            let x = rect.left() + 10.0 + name_width + column_width * (i as f32 + 0.5);
            let text = player.attributes.get(&format!("stat:{column}")).map(stat_text).unwrap_or_default();
            painter.text(Pos2::new(x, y), Align2::CENTER_CENTER, text, theme::semibold(13.0), Color32::WHITE);
        }
    }
}
