//! The asset browser: models, animations, textures and audio from the
//! asset library, and the project's own files.

use araria_core::assets::AssetManifest;
use araria_platform::{icons, theme};
use egui::{RichText, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetTab {
    Models,
    Animations,
    Textures,
    Audio,
    Files,
}

/// What the user asked for in the asset browser.
#[derive(Clone, Debug, PartialEq)]
pub enum AssetAction {
    /// Add a model to the scene (at `MeshPart` in front of the camera).
    InsertModel(String),
    /// Play an animation clip on the selected model part.
    ApplyAnimation { model: String, clip: String },
    PreviewSound(String),
    /// Add a `Sound` with this asset to the selection (or Workspace).
    InsertSound(String),
    /// Add an `ImageLabel` with this texture to the GUI, or set it on the
    /// selected image.
    UseTexture(String),
    OpenFile(String),
    NewScript,
    DeleteFile(String),
    RenameFile(String, String),
}

/// A drag-and-drop payload: a model dragged from the browser into the view.
#[derive(Clone, Debug, PartialEq)]
pub struct DraggedModel(pub String);

pub struct AssetBrowser {
    pub tab: AssetTab,
    filter: String,
    renaming: Option<(String, String)>,
}

impl Default for AssetBrowser {
    fn default() -> Self {
        AssetBrowser { tab: AssetTab::Models, filter: String::new(), renaming: None }
    }
}

fn matches(filter: &str, texts: &[&str]) -> bool {
    let filter = filter.trim().to_lowercase();
    filter.is_empty() || texts.iter().any(|t| t.to_lowercase().contains(&filter))
}

impl AssetBrowser {
    #[allow(clippy::too_many_lines)]
    pub fn show(&mut self, ui: &mut Ui, assets: &AssetManifest, files: &[String]) -> Vec<AssetAction> {
        let mut actions = Vec::new();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (tab, label) in [(AssetTab::Models, "Models"), (AssetTab::Animations, "Anims"), (AssetTab::Textures, "Textures"), (AssetTab::Audio, "Audio"), (AssetTab::Files, "Files")] {
                ui.selectable_value(&mut self.tab, tab, RichText::new(label).font(theme::regular(12.0)));
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(format!("{}  Search", icons::MAGNIFYING_GLASS)).desired_width(f32::INFINITY).font(theme::regular(12.5)));
        ui.add_space(4.0);
        let filter = self.filter.clone();
        egui::ScrollArea::vertical().id_salt("assets").auto_shrink([false, false]).show(ui, |ui| match self.tab {
            AssetTab::Models => {
                let mut categories: Vec<&str> = assets.models.values().map(|m| m.category.as_str()).collect();
                categories.sort_unstable();
                categories.dedup();
                for category in categories {
                    let models: Vec<_> = assets.models.iter().filter(|(_, m)| m.category == category).filter(|(id, m)| matches(&filter, &[id, &m.name, &m.category])).collect();
                    if models.is_empty() {
                        continue;
                    }
                    let title = if category.is_empty() { "Other" } else { category };
                    egui::CollapsingHeader::new(RichText::new(format!("{title} ({})", models.len())).font(theme::semibold(12.0))).id_salt(("model-category", title)).default_open(!filter.is_empty() || models.len() < 12).show(ui, |ui| {
                        for (id, model) in models {
                            let response = ui
                                .dnd_drag_source(egui::Id::new(("asset-model", id)), DraggedModel(id.clone()), |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(icons::PACKAGE).color(theme::ACCENT_2));
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new(&model.name).font(theme::regular(12.5)));
                                            let size = model.size();
                                            ui.label(RichText::new(format!("{:.1} × {:.1} × {:.1}{}", size.x, size.y, size.z, if model.animations.is_empty() { String::new() } else { format!(" · {} anims", model.animations.len()) })).font(theme::regular(10.5)).color(theme::TEXT_FAINT));
                                        });
                                    });
                                })
                                .response;
                            let response = response.on_hover_text(format!("{id}\nDrag into the view, or double-click to insert"));
                            if response.double_clicked() {
                                actions.push(AssetAction::InsertModel(id.clone()));
                            }
                            response.context_menu(|ui| {
                                if ui.button(format!("{}  Insert", icons::PLUS)).clicked() {
                                    actions.push(AssetAction::InsertModel(id.clone()));
                                    ui.close();
                                }
                                if ui.button(format!("{}  Copy id", icons::COPY)).clicked() {
                                    ui.ctx().copy_text(id.clone());
                                    ui.close();
                                }
                            });
                        }
                    });
                }
            }
            AssetTab::Animations => {
                let mut any = false;
                for (id, model) in assets.models.iter().filter(|(_, m)| !m.animations.is_empty()) {
                    let clips: Vec<&String> = model.animations.iter().filter(|c| matches(&filter, &[c, &model.name])).collect();
                    if clips.is_empty() {
                        continue;
                    }
                    any = true;
                    egui::CollapsingHeader::new(RichText::new(format!("{} {}", icons::PERSON_ARMS_SPREAD, model.name)).font(theme::semibold(12.0))).id_salt(("anims", id)).default_open(true).show(ui, |ui| {
                        for clip in clips {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(clip).font(theme::mono(12.0)));
                                if ui.small_button("Apply").on_hover_text("Loop this clip on the selected model part").clicked() {
                                    actions.push(AssetAction::ApplyAnimation { model: id.clone(), clip: clip.clone() });
                                }
                            });
                        }
                    });
                }
                if !any {
                    ui.label(RichText::new("No animated models match.").color(theme::TEXT_FAINT));
                }
            }
            AssetTab::Textures => {
                for (id, texture) in assets.textures.iter().filter(|(id, t)| matches(&filter, &[id, &t.name, &t.category])) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icons::IMAGE).color(theme::ACCENT_2));
                        ui.label(RichText::new(&texture.name).font(theme::regular(12.5))).on_hover_text(id);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Use").on_hover_text("Show on screen as an ImageLabel, or set the selected image").clicked() {
                                actions.push(AssetAction::UseTexture(id.clone()));
                            }
                        });
                    });
                }
            }
            AssetTab::Audio => {
                for (id, audio) in assets.audio.iter().filter(|(id, a)| matches(&filter, &[id, &a.name, &a.category])) {
                    ui.horizontal(|ui| {
                        if ui.small_button(icons::PLAY).on_hover_text("Preview").clicked() {
                            actions.push(AssetAction::PreviewSound(id.clone()));
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&audio.name).font(theme::regular(12.5))).on_hover_text(id);
                            ui.label(RichText::new(format!("{}{:.1}s{}", if audio.category.is_empty() { String::new() } else { format!("{} · ", audio.category) }, audio.duration, if audio.looping { " · loops" } else { "" })).font(theme::regular(10.5)).color(theme::TEXT_FAINT));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button(icons::PLUS).on_hover_text("Add a Sound object").clicked() {
                                actions.push(AssetAction::InsertSound(id.clone()));
                            }
                        });
                    });
                }
            }
            AssetTab::Files => {
                if ui.button(format!("{}  New script", icons::PLUS)).clicked() {
                    actions.push(AssetAction::NewScript);
                }
                ui.add_space(4.0);
                for file in files.iter().filter(|f| matches(&filter, &[f])) {
                    let renaming = self.renaming.as_ref().is_some_and(|(from, _)| from == file);
                    ui.horizontal(|ui| {
                        let icon = if file.ends_with(".lua") { icons::FILE_CODE } else if file.ends_with(".json") { icons::FILE } else { icons::IMAGE };
                        ui.label(RichText::new(icon).color(if file.ends_with(".lua") { theme::ACCENT_HI } else { theme::TEXT_FAINT }));
                        if renaming {
                            let (_, to) = self.renaming.as_mut().expect("renaming");
                            let response = ui.add(egui::TextEdit::singleline(to).desired_width(ui.available_width() - 8.0).font(theme::mono(12.0)));
                            response.request_focus();
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                self.renaming = None;
                            } else if response.lost_focus() {
                                let (from, to) = self.renaming.take().expect("renaming");
                                if from != to.trim() {
                                    actions.push(AssetAction::RenameFile(from, to.trim().to_string()));
                                }
                            }
                            return;
                        }
                        let response = ui.add(egui::Label::new(RichText::new(file).font(theme::mono(12.0))).sense(egui::Sense::click()));
                        if response.double_clicked() && file.ends_with(".lua") {
                            actions.push(AssetAction::OpenFile(file.clone()));
                        }
                        response.context_menu(|ui| {
                            if file.ends_with(".lua") && ui.button(format!("{}  Open", icons::FILE_CODE)).clicked() {
                                actions.push(AssetAction::OpenFile(file.clone()));
                                ui.close();
                            }
                            let protected = file == "project.json" || file.starts_with("scenes/");
                            if !protected {
                                if ui.button(format!("{}  Rename", icons::PENCIL_SIMPLE)).clicked() {
                                    self.renaming = Some((file.clone(), file.clone()));
                                    ui.close();
                                }
                                if ui.button(RichText::new(format!("{}  Delete", icons::TRASH)).color(theme::DANGER)).clicked() {
                                    actions.push(AssetAction::DeleteFile(file.clone()));
                                    ui.close();
                                }
                            }
                        });
                    });
                }
            }
        });
        actions
    }
}
