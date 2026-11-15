//! The in-studio API reference: every global, library and class with its
//! members and documentation, searchable.

use araria_core::schema::schema;
use araria_platform::{icons, theme};
use egui::{RichText, Ui};

use super::api::{self, Kind};
use super::highlight::{TokenKind, color};

#[derive(Default)]
pub struct Reference {
    filter: String,
    /// Show this class or library expanded (from "View docs" elsewhere).
    pub focus: Option<String>,
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Property => "property",
        Kind::Method => "method",
        Kind::Event => "event",
        Kind::Function => "function",
        _ => "",
    }
}

fn member(ui: &mut Ui, name: &str, kind: Kind, signature: &str, doc: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let tint = match kind {
            Kind::Property => color(TokenKind::Member),
            Kind::Event => theme::WARNING,
            _ => color(TokenKind::Call),
        };
        ui.label(RichText::new(name).font(theme::mono_bold(12.0)).color(tint));
        if !signature.is_empty() {
            let shown = if matches!(kind, Kind::Property) { format!(": {signature}") } else { signature.to_string() };
            ui.label(RichText::new(shown).font(theme::mono(11.5)).color(theme::TEXT_FAINT));
        }
        let label = kind_label(kind);
        if !label.is_empty() {
            ui.label(RichText::new(label).font(theme::regular(10.5)).color(theme::with_alpha(theme::TEXT_FAINT, 0.8)));
        }
    });
    if !doc.is_empty() {
        ui.label(RichText::new(doc).font(theme::regular(11.5)).color(theme::TEXT_DIM));
    }
    ui.add_space(3.0);
}

impl Reference {
    pub fn show(&mut self, ui: &mut Ui) {
        if let Some(focus) = &self.focus {
            self.filter.clone_from(focus);
        }
        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(format!("{}  Search the API (e.g. Tween, Touched)", icons::MAGNIFYING_GLASS)).desired_width(f32::INFINITY).font(theme::regular(12.5)));
        let needle = self.filter.trim().to_lowercase();
        let hit = |text: &str| needle.is_empty() || text.to_lowercase().contains(&needle);
        let focus = self.focus.take();
        egui::ScrollArea::vertical().id_salt("api-reference").auto_shrink([false, false]).show(ui, |ui| {
            let globals: Vec<_> = api::GLOBALS.iter().filter(|g| hit(g.name) || hit(g.doc)).collect();
            if !globals.is_empty() {
                egui::CollapsingHeader::new(RichText::new("Globals").font(theme::semibold(12.5))).default_open(!needle.is_empty()).show(ui, |ui| {
                    for global in globals {
                        member(ui, global.name, if global.signature.is_empty() { Kind::Global } else { Kind::Function }, global.signature, global.doc);
                    }
                });
            }
            for library in api::LIBRARIES {
                let library_hit = hit(library.name);
                let members: Vec<_> = library.members.iter().chain(library.instance_members.iter()).filter(|m| library_hit || hit(m.name) || hit(m.doc)).collect();
                if members.is_empty() {
                    continue;
                }
                let open = !needle.is_empty() || focus.as_deref() == Some(library.name);
                egui::CollapsingHeader::new(RichText::new(format!("{}  {}", icons::BOOKS, library.name)).font(theme::semibold(12.5))).id_salt(("api-lib", library.name)).open(open.then_some(true)).show(ui, |ui| {
                    ui.label(RichText::new(library.doc).font(theme::regular(11.5)).color(theme::TEXT_DIM));
                    ui.add_space(4.0);
                    for entry in members {
                        member(ui, entry.name, if entry.signature.is_empty() { Kind::Property } else { Kind::Function }, entry.signature, entry.doc);
                    }
                });
            }
            for class in schema().classes() {
                let class_hit = hit(class.name());
                let members: Vec<_> = api::class_members(class).into_iter().filter(|m| class_hit || hit(m.name)).collect();
                if members.is_empty() {
                    continue;
                }
                let open = !needle.is_empty() && (class_hit || members.len() < 8) || focus.as_deref() == Some(class.name());
                let title = format!("{}  {}{}", crate::explorer::class_icon(class.name()), class.name(), class.def.parent.map_or(String::new(), |p| format!("  : {p}")));
                egui::CollapsingHeader::new(RichText::new(title).font(theme::semibold(12.5))).id_salt(("api-class", class.name())).open(open.then_some(true)).show(ui, |ui| {
                    ui.label(RichText::new(class.def.doc).font(theme::regular(11.5)).color(theme::TEXT_DIM));
                    ui.add_space(4.0);
                    for m in members {
                        member(ui, m.name, m.kind, &m.signature, m.doc);
                    }
                });
            }
        });
    }
}
