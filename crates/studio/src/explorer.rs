//! The Explorer: the game tree, with selection, renaming, drag-and-drop
//! reparenting and a context menu for inserting and organizing objects.

use std::collections::HashSet;

use araria_core::schema::schema;
use araria_core::{DataModel, InstanceId};
use araria_platform::{icons, theme};
use egui::{Align2, CornerRadius, Id, Key, RichText, Sense, Stroke, Ui, vec2};

const ROW: f32 = 22.0;
const INDENT: f32 = 16.0;

/// Something the user asked the Explorer to do to the scene.
#[derive(Clone, Debug, PartialEq)]
pub enum ExplorerAction {
    /// Select an object; `extend` adds to (or removes from) the selection.
    Select { id: InstanceId, extend: bool },
    Rename(InstanceId, String),
    Reparent { id: InstanceId, parent: InstanceId },
    Insert { class: &'static str, parent: InstanceId },
    Delete,
    Duplicate,
    Group,
    Ungroup,
    /// Open a script's source file in the editor.
    OpenScript(InstanceId),
    /// Move the camera to look at an object.
    Focus(InstanceId),
    Copy,
    Paste(InstanceId),
}

/// The icon for an instance class.
pub fn class_icon(class: &str) -> &'static str {
    match class {
        "Workspace" => icons::GLOBE,
        "Players" | "Player" => icons::USERS,
        "Lighting" => icons::SUN,
        "Storage" => icons::PACKAGE,
        "Gui" => icons::MONITOR,
        "Scripts" | "ModuleScript" => icons::CODE,
        "Camera" => icons::EYE,
        "Part" => icons::CUBE,
        "MeshPart" => icons::PACKAGE,
        "Model" => icons::SQUARES_FOUR,
        "Folder" => icons::FOLDER,
        "SpawnPoint" => icons::FLAG,
        "Character" => icons::PERSON_ARMS_SPREAD,
        "Light" => icons::LIGHTNING,
        "ParticleEmitter" => icons::SPARKLE,
        "Sound" => icons::SPEAKER_HIGH,
        "Prompt" => icons::HAND_WAVING,
        "BillboardLabel" => icons::INFO,
        "Script" => icons::FILE_CODE,
        "ImageLabel" => icons::IMAGE,
        "TextButton" => icons::CURSOR,
        "Frame" | "TextLabel" | "ProgressBar" => icons::SQUARES_FOUR,
        _ => icons::CUBE,
    }
}

pub fn class_color(class: &str) -> egui::Color32 {
    match class {
        "Script" | "ModuleScript" => theme::ACCENT_HI,
        "Model" | "Folder" => theme::WARNING,
        "Light" | "ParticleEmitter" => theme::mix(theme::WARNING, theme::TEXT, 0.4),
        "Sound" => theme::SUCCESS,
        "Character" | "Player" | "Players" => theme::ACCENT_2,
        _ => theme::TEXT_DIM,
    }
}

/// Classes offered in the Insert menu, most useful first.
pub const COMMON_CLASSES: &[&str] = &["Part", "Model", "Folder", "SpawnPoint", "Light", "Script", "ModuleScript", "Sound", "ParticleEmitter", "Prompt", "BillboardLabel", "MeshPart"];

#[derive(Default)]
pub struct Explorer {
    pub filter: String,
    expanded: HashSet<InstanceId>,
    renaming: Option<(InstanceId, String)>,
    /// Scroll to this object next frame.
    reveal: Option<InstanceId>,
}

struct Row {
    id: InstanceId,
    depth: usize,
    has_children: bool,
}

impl Explorer {
    /// Expands the ancestors of `id` and scrolls to it.
    pub fn reveal(&mut self, dm: &DataModel, id: InstanceId) {
        let mut current = dm.get(id).and_then(|i| i.parent);
        while let Some(parent) = current {
            self.expanded.insert(parent);
            current = dm.get(parent).and_then(|i| i.parent);
        }
        self.reveal = Some(id);
    }

    pub fn start_rename(&mut self, dm: &DataModel, id: InstanceId) {
        if let Some(instance) = dm.get(id) {
            self.renaming = Some((id, instance.name.clone()));
        }
    }

    fn rows(&self, dm: &DataModel) -> Vec<Row> {
        let mut rows = Vec::new();
        let needle = self.filter.trim().to_lowercase();
        // With a filter: matches plus the path to them.
        let visible: Option<HashSet<InstanceId>> = (!needle.is_empty()).then(|| {
            let mut keep = HashSet::new();
            for instance in dm.instances() {
                if instance.name.to_lowercase().contains(&needle) || instance.class_name().to_lowercase() == needle {
                    let mut current = Some(instance.id);
                    while let Some(id) = current {
                        if !keep.insert(id) {
                            break;
                        }
                        current = dm.get(id).and_then(|i| i.parent);
                    }
                }
            }
            keep
        });
        fn walk(explorer: &Explorer, dm: &DataModel, id: InstanceId, depth: usize, visible: Option<&HashSet<InstanceId>>, rows: &mut Vec<Row>) {
            if visible.is_some_and(|v| !v.contains(&id)) {
                return;
            }
            let children = dm.children(id);
            rows.push(Row { id, depth, has_children: !children.is_empty() });
            if explorer.expanded.contains(&id) || visible.is_some() {
                for child in children {
                    walk(explorer, dm, *child, depth + 1, visible, rows);
                }
            }
        }
        for service in dm.children(dm.root()) {
            walk(self, dm, *service, 0, visible.as_ref(), &mut rows);
        }
        rows
    }

    #[allow(clippy::too_many_lines)]
    pub fn show(&mut self, ui: &mut Ui, dm: &DataModel, selection: &[InstanceId], editing: bool) -> Vec<ExplorerAction> {
        let mut actions = Vec::new();
        if self.expanded.is_empty() {
            self.expanded.insert(dm.workspace());
        }
        // Right to left: the button first, then the filter takes exactly the
        // remaining width (so the panel never grows to fit).
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let parent = selection.first().copied().filter(|id| dm.contains(*id)).unwrap_or(dm.workspace());
            ui.menu_button(RichText::new(icons::PLUS).font(theme::regular(14.0)), |ui| {
                insert_menu(ui, parent, &mut actions);
            })
            .response
            .on_hover_text("Insert an object");
            ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(format!("{}  Filter objects", icons::MAGNIFYING_GLASS)).desired_width(ui.available_width()).font(theme::regular(12.5)));
        });
        ui.add_space(4.0);
        let rows = self.rows(dm);
        let reveal = self.reveal.take();
        let reveal_index = reveal.and_then(|id| rows.iter().position(|r| r.id == id));
        let mut toggle = None;
        egui::ScrollArea::vertical().id_salt("explorer").auto_shrink([false, false]).show_rows(ui, ROW, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Measured inside the scroll area, so rows never push it wider.
            let width = ui.available_width();
            if let Some(index) = reveal_index {
                if !range.contains(&index) {
                    // Scroll so the row comes into view.
                    let top = ui.min_rect().top() + (index as f32 - range.start as f32) * ROW;
                    ui.scroll_to_rect(egui::Rect::from_min_size(egui::pos2(ui.min_rect().left(), top), vec2(width, ROW)), Some(egui::Align::Center));
                }
            }
            for row in &rows[range] {
                let Some(instance) = dm.get(row.id) else { continue };
                let (rect, response) = ui.allocate_exact_size(vec2(width, ROW), Sense::click_and_drag());
                let selected = selection.contains(&row.id);
                let painter = ui.painter();
                let drop_target = response.dnd_hover_payload::<InstanceId>().filter(|dragged| **dragged != row.id && !dm.is_descendant_of(row.id, **dragged));
                if drop_target.is_some() {
                    painter.rect_stroke(rect.shrink(1.0), CornerRadius::same(5), Stroke::new(1.5, theme::ACCENT_2), egui::StrokeKind::Inside);
                } else if selected {
                    painter.rect_filled(rect, CornerRadius::same(5), theme::with_alpha(theme::ACCENT, 0.32));
                } else if response.hovered() {
                    painter.rect_filled(rect, CornerRadius::same(5), theme::SURFACE_HOVER);
                }
                let x = rect.left() + 4.0 + row.depth as f32 * INDENT;
                // Expand arrow.
                if row.has_children {
                    let arrow = egui::Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), vec2(16.0, ROW));
                    let arrow_response = ui.interact(arrow, Id::new(("explorer-expand", row.id)), Sense::click());
                    let open = self.expanded.contains(&row.id) || !self.filter.trim().is_empty();
                    painter.text(arrow.center(), Align2::CENTER_CENTER, if open { icons::CARET_DOWN } else { icons::CARET_RIGHT }, theme::regular(11.0), if arrow_response.hovered() { theme::TEXT } else { theme::TEXT_FAINT });
                    if arrow_response.clicked() {
                        toggle = Some(row.id);
                    }
                }
                let class = instance.class_name();
                painter.text(egui::pos2(x + 24.0, rect.center().y), Align2::CENTER_CENTER, class_icon(class), theme::regular(13.0), class_color(class));
                let name_x = x + 36.0;
                let renaming = self.renaming.as_ref().is_some_and(|(id, _)| *id == row.id);
                if renaming {
                    let name_rect = egui::Rect::from_min_max(egui::pos2(name_x, rect.top() + 1.0), egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0));
                    let (_, text) = self.renaming.as_mut().expect("renaming");
                    let edit = ui.put(name_rect, egui::TextEdit::singleline(text).font(theme::regular(12.5)).margin(egui::Margin::symmetric(4, 1)));
                    edit.request_focus();
                    let done = edit.lost_focus();
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.renaming = None;
                    } else if done {
                        let (id, name) = self.renaming.take().expect("renaming");
                        let name = name.trim().to_string();
                        if !name.is_empty() && dm.get(id).is_some_and(|i| i.name != name) {
                            actions.push(ExplorerAction::Rename(id, name));
                        }
                    }
                } else {
                    let color = if selected { theme::TEXT } else { theme::mix(theme::TEXT, theme::TEXT_DIM, 0.3) };
                    let name = araria_platform::widgets::elide(ui, &instance.name, theme::regular(12.5), rect.right() - name_x - 8.0);
                    painter.text(egui::pos2(name_x, rect.center().y), Align2::LEFT_CENTER, name, theme::regular(12.5), color);
                    if instance.class_name() == "Script" && !instance.bool("Enabled") {
                        painter.text(rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, "off", theme::regular(10.5), theme::TEXT_FAINT);
                    }
                }

                // Interaction.
                let service = instance.info().def.service;
                if response.drag_started() && !service && editing {
                    response.dnd_set_drag_payload(row.id);
                }
                if let Some(dragged) = response.dnd_release_payload::<InstanceId>() {
                    if *dragged != row.id && !dm.is_descendant_of(row.id, *dragged) {
                        actions.push(ExplorerAction::Reparent { id: *dragged, parent: row.id });
                        self.expanded.insert(row.id);
                    }
                }
                if response.clicked() {
                    let extend = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                    actions.push(ExplorerAction::Select { id: row.id, extend });
                }
                if response.double_clicked() {
                    if matches!(class, "Script" | "ModuleScript") {
                        actions.push(ExplorerAction::OpenScript(row.id));
                    } else if instance.is_a("BasePart") || instance.is_a("Model") || class == "Character" {
                        actions.push(ExplorerAction::Focus(row.id));
                    } else if row.has_children {
                        toggle = Some(row.id);
                    }
                }
                response.context_menu(|ui| {
                    if !selection.contains(&row.id) {
                        actions.push(ExplorerAction::Select { id: row.id, extend: false });
                    }
                    ui.set_min_width(190.0);
                    ui.label(RichText::new(format!("{} {}", class_icon(class), instance.name)).font(theme::semibold(12.5)));
                    ui.separator();
                    ui.menu_button(format!("{}  Insert object", icons::PLUS), |ui| insert_menu(ui, row.id, &mut actions));
                    if matches!(class, "Script" | "ModuleScript") && ui.button(format!("{}  Open script", icons::FILE_CODE)).clicked() {
                        actions.push(ExplorerAction::OpenScript(row.id));
                        ui.close();
                    }
                    if (instance.is_a("BasePart") || instance.is_a("Model")) && ui.button(format!("{}  Focus camera     F", icons::EYE)).clicked() {
                        actions.push(ExplorerAction::Focus(row.id));
                        ui.close();
                    }
                    if !service && editing {
                        ui.separator();
                        if ui.button(format!("{}  Rename           F2", icons::PENCIL_SIMPLE)).clicked() {
                            self.renaming = Some((row.id, instance.name.clone()));
                            ui.close();
                        }
                        if ui.button(format!("{}  Duplicate    Ctrl+D", icons::COPY)).clicked() {
                            actions.push(ExplorerAction::Duplicate);
                            ui.close();
                        }
                        if ui.button(format!("{}  Copy         Ctrl+C", icons::COPY)).clicked() {
                            actions.push(ExplorerAction::Copy);
                            ui.close();
                        }
                        if ui.button(format!("{}  Group        Ctrl+G", icons::SQUARES_FOUR)).clicked() {
                            actions.push(ExplorerAction::Group);
                            ui.close();
                        }
                        if matches!(class, "Model" | "Folder") && ui.button(format!("{}  Ungroup  Ctrl+Shift+G", icons::ARROWS_OUT)).clicked() {
                            actions.push(ExplorerAction::Ungroup);
                            ui.close();
                        }
                        if ui.button(RichText::new(format!("{}  Delete           Del", icons::TRASH)).color(theme::DANGER)).clicked() {
                            actions.push(ExplorerAction::Delete);
                            ui.close();
                        }
                    }
                    if editing && ui.button(format!("{}  Paste into   Ctrl+V", icons::DOWNLOAD_SIMPLE)).clicked() {
                        actions.push(ExplorerAction::Paste(row.id));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(format!("{}  Copy path", icons::LINK)).clicked() {
                        ui.ctx().copy_text(dm.path(row.id));
                        ui.close();
                    }
                });
            }
        });
        if let Some(id) = toggle {
            if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
        }
        // F2 renames the selection when the explorer has the pointer.
        if editing && ui.ui_contains_pointer() && ui.input(|i| i.key_pressed(Key::F2)) {
            if let Some(id) = selection.first() {
                self.start_rename(dm, *id);
            }
        }
        actions
    }
}

fn insert_menu(ui: &mut Ui, parent: InstanceId, actions: &mut Vec<ExplorerAction>) {
    ui.set_min_width(180.0);
    for class in COMMON_CLASSES {
        if ui.button(format!("{}  {class}", class_icon(class))).clicked() {
            actions.push(ExplorerAction::Insert { class, parent });
            ui.close();
        }
    }
    ui.menu_button("More…", |ui| {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for class in schema().classes().filter(|c| c.def.creatable && !COMMON_CLASSES.contains(&c.name())) {
                if ui.button(format!("{}  {}", class_icon(class.name()), class.name())).on_hover_text(class.def.doc).clicked() {
                    actions.push(ExplorerAction::Insert { class: class.name(), parent });
                    ui.close();
                }
            }
        });
    });
}
