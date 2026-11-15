//! The Properties inspector: every property of the selected object, edited
//! with a widget that suits its type, plus custom attributes and tags.
//! Edits go straight into the data model, so the 3D view, physics and
//! running scripts see them immediately.

use araria_core::assets::AssetManifest;
use araria_core::schema::{AssetKind, PropDef, PropType};
use araria_core::{Color, DataModel, InstanceId, UDim2, Variant, Vec2, Vec3, euler_deg_to_quat, quat_to_euler_deg};
use araria_platform::{icons, theme};
use egui::{RichText, Ui};

use crate::explorer::{class_color, class_icon};

/// What the inspector reports back to the studio.
#[derive(Default)]
pub struct InspectorOutput {
    /// A widget is mid-edit (dragging a value, typing): do not checkpoint
    /// the undo history yet.
    pub editing: bool,
    /// Open a script file.
    pub open_script: Option<String>,
    /// Create a new script file and point the selected script at it.
    pub new_script: bool,
    /// Select another object (from a reference property).
    pub select: Option<InstanceId>,
}

#[derive(Default)]
pub struct Inspector {
    new_attribute: String,
    new_attribute_kind: usize,
    new_tag: String,
    error: Option<String>,
    filter: String,
}

const CATEGORIES: &[(&str, &[&str])] = &[
    ("Transform", &["Position", "Rotation", "Size", "Scale", "Facing", "Offset", "Direction"]),
    ("Appearance", &["Color", "Material", "Transparency", "Shape", "CastShadow", "Tint", "Asset", "Animation", "Brightness", "Range", "Angle", "Shadows", "Kind", "ShirtColor", "PantsColor", "SkinColor", "HairColor", "Glow", "Opacity", "ColorEnd", "SizeEnd", "Text", "TextSize", "TextColor", "Font", "BackgroundColor", "BackgroundTransparency", "Image", "ShowName", "DisplayName"]),
    ("Physics", &["Anchored", "CanCollide", "CanTouch", "Density", "Friction", "Elasticity", "Collision", "Velocity", "AngularVelocity", "WalkSpeed", "RunSpeed", "JumpPower", "Gravity", "KillHeight"]),
];

fn category(name: &str) -> usize {
    CATEGORIES.iter().position(|(_, names)| names.contains(&name)).unwrap_or(CATEGORIES.len())
}

const ATTRIBUTE_KINDS: &[&str] = &["Number", "Text", "Boolean", "Vector3", "Color"];

/// A row of labelled number fields; returns the new values if changed.
fn vector_fields<const N: usize>(ui: &mut Ui, values: [f32; N], labels: [&str; N], speed: f64, editing: &mut bool) -> Option<[f32; N]> {
    let mut values = values;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let width = ((ui.available_width() - 18.0 * N as f32) / N as f32).max(32.0);
        for (value, label) in values.iter_mut().zip(labels) {
            ui.label(RichText::new(label).font(theme::regular(11.0)).color(theme::TEXT_FAINT));
            let response = ui.add_sized([width, 20.0], egui::DragValue::new(value).speed(speed).max_decimals(3));
            changed |= response.changed();
            *editing |= response.dragged() || response.has_focus();
        }
    });
    changed.then_some(values)
}

fn assets_of(assets: &AssetManifest, kind: AssetKind) -> Vec<(String, String)> {
    match kind {
        AssetKind::Model => assets.models.iter().map(|(id, m)| (id.clone(), m.name.clone())).collect(),
        AssetKind::Audio => assets.audio.iter().map(|(id, a)| (id.clone(), a.name.clone())).collect(),
        AssetKind::Texture => assets.textures.iter().map(|(id, t)| (id.clone(), t.name.clone())).collect(),
        AssetKind::Script => Vec::new(),
    }
}

impl Inspector {
    #[allow(clippy::too_many_lines)]
    pub fn show(&mut self, ui: &mut Ui, dm: &mut DataModel, selection: &[InstanceId], assets: &AssetManifest, scripts: &[String]) -> InspectorOutput {
        let mut out = InspectorOutput::default();
        let Some(&id) = selection.last() else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icons::SLIDERS).font(theme::regular(28.0)).color(theme::TEXT_FAINT));
                ui.label(RichText::new("Select an object to see its properties").font(theme::regular(12.5)).color(theme::TEXT_FAINT));
            });
            return out;
        };
        let Some(instance) = dm.get(id) else { return out };
        let class = instance.class_name();
        let info = instance.info();
        let mut name = instance.name.clone();
        let is_service = info.def.service;

        // Header: icon, editable name, class.
        ui.horizontal(|ui| {
            ui.label(RichText::new(class_icon(class)).font(theme::regular(18.0)).color(class_color(class)));
            ui.vertical(|ui| {
                let response = ui.add_enabled(!is_service, egui::TextEdit::singleline(&mut name).font(theme::semibold(14.0)).desired_width(ui.available_width()));
                out.editing |= response.has_focus();
                if response.changed() && !name.trim().is_empty() {
                    if let Err(e) = dm.set_name(id, name.trim()) {
                        self.error = Some(e.to_string());
                    }
                }
                let mut subtitle = format!("{class} · id {}", id.0);
                if selection.len() > 1 {
                    subtitle.push_str(&format!(" · {} selected", selection.len()));
                }
                ui.label(RichText::new(subtitle).font(theme::regular(11.5)).color(theme::TEXT_FAINT)).on_hover_text(info.def.doc);
            });
        });
        ui.add_space(4.0);
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(format!("{}  Filter properties", icons::MAGNIFYING_GLASS)).desired_width(f32::INFINITY).font(theme::regular(12.0)));
        if let Some(error) = &self.error {
            ui.label(RichText::new(format!("{} {error}", icons::WARNING)).font(theme::regular(12.0)).color(theme::DANGER));
        }
        ui.add_space(2.0);

        let props: Vec<(usize, PropDef)> = info.props.iter().cloned().enumerate().filter(|(_, p)| !p.hidden).filter(|(_, p)| self.filter.is_empty() || p.name.to_lowercase().contains(&self.filter.to_lowercase())).collect();
        let names: Vec<String> = dm.descendants(dm.root()).into_iter().filter_map(|i| dm.get(i)).filter(|i| i.is_a("BasePart") || i.is_a("Model") || i.is_a("Character")).take(400).map(|i| format!("{}\u{1f}{}", i.id.0, dm.path(i.id))).collect();
        egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
            let groups = CATEGORIES.len() + 1;
            for group in 0..groups {
                let in_group: Vec<&(usize, PropDef)> = props.iter().filter(|(_, p)| category(p.name) == group).collect();
                if in_group.is_empty() {
                    continue;
                }
                let title = CATEGORIES.get(group).map_or("Behavior", |(t, _)| t);
                egui::CollapsingHeader::new(RichText::new(title).font(theme::semibold(12.5))).id_salt(("inspector-group", title)).default_open(true).show(ui, |ui| {
                    egui::Grid::new(("inspector-grid", title)).num_columns(2).spacing([8.0, 4.0]).min_col_width(92.0).striped(false).show(ui, |ui| {
                        for (index, def) in in_group {
                            let value = dm.get(id).map(|i| i.prop_at(*index).clone()).unwrap_or_default();
                            let label = ui.label(RichText::new(def.name).font(theme::regular(12.0)).color(if value == def.default { theme::TEXT_DIM } else { theme::TEXT }));
                            label.on_hover_text(format!("{}\n\n{}", def.doc, def.type_label()));
                            let new = ui.add_enabled_ui(!def.read_only, |ui| self.property(ui, def, &value, dm, assets, scripts, &names, &mut out)).inner;
                            if let Some(new) = new {
                                match dm.set(id, def.name, new) {
                                    Ok(()) => self.error = None,
                                    Err(e) => self.error = Some(e.to_string()),
                                }
                            }
                            ui.end_row();
                        }
                    });
                });
            }

            // Attributes.
            egui::CollapsingHeader::new(RichText::new("Attributes").font(theme::semibold(12.5))).id_salt("inspector-attributes").default_open(true).show(ui, |ui| {
                let attributes: Vec<(String, Variant)> = dm.get(id).map(|i| i.attributes.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
                if attributes.is_empty() {
                    ui.label(RichText::new("Custom values scripts read with GetAttribute.").font(theme::regular(11.5)).color(theme::TEXT_FAINT));
                }
                for (key, value) in attributes {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&key).font(theme::mono(12.0)));
                        let new = match &value {
                            Variant::Bool(b) => {
                                let mut b = *b;
                                ui.checkbox(&mut b, "").changed().then_some(Variant::Bool(b))
                            }
                            Variant::Number(n) => {
                                let mut n = *n;
                                let response = ui.add(egui::DragValue::new(&mut n).speed(0.1));
                                out.editing |= response.dragged() || response.has_focus();
                                response.changed().then_some(Variant::Number(n))
                            }
                            Variant::String(s) => {
                                let mut s = s.clone();
                                let response = ui.add(egui::TextEdit::singleline(&mut s).desired_width(120.0));
                                out.editing |= response.has_focus();
                                response.changed().then_some(Variant::String(s))
                            }
                            Variant::Vec3(v) => vector_fields(ui, v.to_array(), ["X", "Y", "Z"], 0.1, &mut out.editing).map(|a| Variant::Vec3(Vec3::from_array(a))),
                            Variant::Color(c) => {
                                let mut rgb = c.to_rgb8();
                                ui.color_edit_button_srgb(&mut rgb).changed().then(|| Variant::Color(Color::from_rgb8(rgb[0], rgb[1], rgb[2])))
                            }
                            other => {
                                ui.label(RichText::new(crate::terminal::describe(dm, other)).font(theme::mono(12.0)).color(theme::TEXT_DIM));
                                None
                            }
                        };
                        if let Some(new) = new {
                            let _ = dm.set_attribute(id, &key, new);
                        }
                        if ui.small_button(icons::X).on_hover_text("Remove").clicked() {
                            let _ = dm.set_attribute(id, &key, Variant::Nil);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    let response = ui.add(egui::TextEdit::singleline(&mut self.new_attribute).hint_text("New attribute").desired_width(110.0).font(theme::regular(12.0)));
                    out.editing |= response.has_focus();
                    egui::ComboBox::from_id_salt("attribute-kind").width(80.0).selected_text(ATTRIBUTE_KINDS[self.new_attribute_kind]).show_ui(ui, |ui| {
                        for (i, kind) in ATTRIBUTE_KINDS.iter().enumerate() {
                            ui.selectable_value(&mut self.new_attribute_kind, i, *kind);
                        }
                    });
                    let name = self.new_attribute.trim().to_string();
                    if ui.add_enabled(!name.is_empty(), egui::Button::new(icons::PLUS)).clicked() {
                        let value = match self.new_attribute_kind {
                            0 => Variant::Number(0.0),
                            1 => Variant::String(String::new()),
                            2 => Variant::Bool(false),
                            3 => Variant::Vec3(Vec3::ZERO),
                            _ => Variant::Color(Color::from_rgb8(255, 255, 255)),
                        };
                        let _ = dm.set_attribute(id, &name, value);
                        self.new_attribute.clear();
                    }
                });
            });

            // Tags.
            egui::CollapsingHeader::new(RichText::new("Tags").font(theme::semibold(12.5))).id_salt("inspector-tags").default_open(true).show(ui, |ui| {
                let tags: Vec<String> = dm.get(id).map(|i| i.tags.clone()).unwrap_or_default();
                ui.horizontal_wrapped(|ui| {
                    for tag in tags {
                        if ui.button(RichText::new(format!("{tag}  {}", icons::X)).font(theme::regular(12.0))).on_hover_text("Remove tag").clicked() {
                            dm.remove_tag(id, &tag);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let response = ui.add(egui::TextEdit::singleline(&mut self.new_tag).hint_text("Add tag").desired_width(140.0).font(theme::regular(12.0)));
                    out.editing |= response.has_focus();
                    let submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button(icons::PLUS).clicked() || submit) && !self.new_tag.trim().is_empty() {
                        dm.add_tag(id, self.new_tag.trim());
                        self.new_tag.clear();
                    }
                });
            });
        });
        out
    }

    /// The editor for one property. Returns a new value when it changed.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn property(&mut self, ui: &mut Ui, def: &PropDef, value: &Variant, dm: &DataModel, assets: &AssetManifest, scripts: &[String], candidates: &[String], out: &mut InspectorOutput) -> Option<Variant> {
        let editing = &mut out.editing;
        match (&def.ty, value) {
            (PropType::Bool, Variant::Bool(b)) => {
                let mut b = *b;
                ui.checkbox(&mut b, "").changed().then_some(Variant::Bool(b))
            }
            (PropType::Number { min, max }, Variant::Number(n)) => {
                let mut n = *n;
                let response = if *min >= 0.0 && *max <= 1.0 {
                    ui.add(egui::Slider::new(&mut n, *min..=*max).max_decimals(2))
                } else {
                    let speed = (n.abs() * 0.01).clamp(0.01, 5.0);
                    ui.add(egui::DragValue::new(&mut n).speed(speed).range(*min..=*max).max_decimals(3))
                };
                *editing |= response.dragged() || response.has_focus();
                response.changed().then_some(Variant::Number(n))
            }
            (PropType::String, Variant::String(s)) => {
                let mut s = s.clone();
                let multiline = def.name == "Text";
                let response = if multiline { ui.add(egui::TextEdit::multiline(&mut s).desired_rows(2).desired_width(f32::INFINITY)) } else { ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY)) };
                *editing |= response.has_focus();
                response.changed().then_some(Variant::String(s))
            }
            (PropType::Vec2, Variant::Vec2(v)) => vector_fields(ui, v.to_array(), ["X", "Y"], 0.1, editing).map(|a| Variant::Vec2(Vec2::from_array(a))),
            (PropType::Vec3, Variant::Vec3(v)) => {
                let speed = if def.name == "Scale" { 0.01 } else { 0.1 };
                vector_fields(ui, v.to_array(), ["X", "Y", "Z"], speed, editing).map(|a| Variant::Vec3(Vec3::from_array(a)))
            }
            (PropType::Rotation, Variant::Quat(q)) => {
                let euler = quat_to_euler_deg(*q);
                let rounded = [round(euler.x), round(euler.y), round(euler.z)];
                vector_fields(ui, rounded, ["X", "Y", "Z"], 1.0, editing).map(|a| Variant::Quat(euler_deg_to_quat(Vec3::from_array(a))))
            }
            (PropType::Color, Variant::Color(c)) => {
                let mut rgb = c.to_rgb8();
                let mut result = None;
                ui.horizontal(|ui| {
                    let response = ui.color_edit_button_srgb(&mut rgb);
                    if response.changed() {
                        result = Some(Variant::Color(Color::from_rgb8(rgb[0], rgb[1], rgb[2])));
                    }
                    ui.label(RichText::new(c.to_hex()).font(theme::mono(11.5)).color(theme::TEXT_FAINT));
                });
                result
            }
            (PropType::UDim2, Variant::UDim2(u)) => {
                let values = [u.x_scale, u.x_offset, u.y_scale, u.y_offset];
                let mut result = None;
                ui.vertical(|ui| {
                    if let Some([xs, xo, ys, yo]) = vector_fields(ui, values, ["xS", "xO", "yS", "yO"], 0.5, editing) {
                        result = Some(Variant::UDim2(UDim2::new(xs, xo, ys, yo)));
                    }
                });
                result
            }
            (PropType::Enum(options), Variant::String(current)) => {
                let mut selected = current.clone();
                egui::ComboBox::from_id_salt(("enum", def.name)).selected_text(&selected).width(150.0).show_ui(ui, |ui| {
                    for option in *options {
                        ui.selectable_value(&mut selected, (*option).to_string(), *option);
                    }
                });
                (selected != *current).then_some(Variant::String(selected))
            }
            (PropType::Asset(AssetKind::Script), Variant::String(current)) => {
                let mut selected = current.clone();
                let mut result = None;
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("script-source", def.name)).selected_text(if current.is_empty() { "(none)" } else { current.as_str() }).width(150.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, String::new(), "(none)");
                        for script in scripts {
                            ui.selectable_value(&mut selected, script.clone(), script);
                        }
                    });
                    if selected != *current {
                        result = Some(Variant::String(selected.clone()));
                    }
                    if current.is_empty() {
                        if ui.small_button(icons::PLUS).on_hover_text("Create a new script file").clicked() {
                            out.new_script = true;
                        }
                    } else if ui.small_button(icons::PENCIL_SIMPLE).on_hover_text("Open in the editor").clicked() {
                        out.open_script = Some(current.clone());
                    }
                });
                result
            }
            (PropType::Asset(kind), Variant::String(current)) => {
                let options = assets_of(assets, *kind);
                let label = options.iter().find(|(id, _)| id == current).map_or_else(|| if current.is_empty() { "(none)".to_string() } else { current.clone() }, |(_, name)| name.clone());
                let mut selected = current.clone();
                egui::ComboBox::from_id_salt(("asset", def.name)).selected_text(label).width(150.0).height(320.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, String::new(), "(none)");
                    for (id, name) in &options {
                        ui.selectable_value(&mut selected, id.clone(), name).on_hover_text(id);
                    }
                });
                (selected != *current).then_some(Variant::String(selected))
            }
            (PropType::Ref, Variant::Ref(current)) => {
                let label = current.filter(|id| dm.contains(*id)).map_or("(none)".to_string(), |id| dm.get(id).map_or(String::new(), |i| i.name.clone()));
                let mut selected = *current;
                let mut result = None;
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("ref", def.name)).selected_text(label).width(130.0).height(320.0).show_ui(ui, |ui| {
                        ui.selectable_value(&mut selected, None, "(none)");
                        for entry in candidates {
                            let (raw, path) = entry.split_once('\u{1f}').unwrap_or(("0", entry));
                            let id = InstanceId(raw.parse().unwrap_or(0));
                            ui.selectable_value(&mut selected, Some(id), path);
                        }
                    });
                    if selected != *current {
                        result = Some(Variant::Ref(selected));
                    }
                    if let Some(target) = current.filter(|id| dm.contains(*id)) {
                        if ui.small_button(icons::ARROW_RIGHT).on_hover_text("Select it").clicked() {
                            out.select = Some(target);
                        }
                    }
                });
                result
            }
            (_, other) => {
                ui.label(RichText::new(crate::terminal::describe(dm, other)).font(theme::mono(12.0)).color(theme::TEXT_DIM));
                None
            }
        }
    }
}

/// Rounds away float noise from Euler angles.
fn round(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}
