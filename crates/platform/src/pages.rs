//! The platform's pages and dialogs.

use araria_core::project::CATEGORIES;
use araria_storage::now_secs;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::cards::{self, CardResponse, players_label};
use crate::icons;
use crate::library::{GameEntry, GameKey, Library, Origin};
use crate::settings::{Quality, Settings};
use crate::shell::{Action, NewProject, Page, PlatformUi, Sort, ToastKind, ago, mini_cover, size_label};
use crate::theme::{self, ACCENT, ACCENT_HI, BORDER, SURFACE, SURFACE_HI, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::widgets::{self, ButtonKind};

const CARD_GAP: f32 = 18.0;

fn handle_card(st: &mut PlatformUi, response: CardResponse, entry: &GameEntry, actions: &mut Vec<Action>) {
    if response.play {
        actions.push(Action::Play(entry.key.clone()));
    } else if response.opened {
        st.detail = Some(entry.key.clone());
    }
}

/// Card width that fills the row evenly.
fn card_width(available: f32) -> f32 {
    let columns = ((available + CARD_GAP) / (250.0 + CARD_GAP)).floor().max(1.0);
    ((available + CARD_GAP) / columns - CARD_GAP).floor()
}

/// A wrapping grid of game cards.
fn grid(ui: &mut Ui, st: &mut PlatformUi, library: &Library, entries: &[GameEntry], actions: &mut Vec<Action>) {
    let width = card_width(ui.available_width());
    ui.spacing_mut().item_spacing = vec2(CARD_GAP, CARD_GAP);
    ui.horizontal_wrapped(|ui| {
        for entry in entries {
            let response = cards::game_card(ui, &mut st.thumbs, library, entry, width);
            handle_card(st, response, entry, actions);
        }
    });
}

/// A single horizontally scrolling row of game cards.
fn shelf(ui: &mut Ui, st: &mut PlatformUi, library: &Library, id: &str, entries: &[GameEntry], actions: &mut Vec<Action>) {
    let width = card_width(ui.available_width()).min(300.0);
    egui::ScrollArea::horizontal().id_salt(id).auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            ui.add_space(2.0);
            for entry in entries {
                // Leave room for the hover lift and shadow.
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    let response = cards::game_card(ui, &mut st.thumbs, library, entry, width);
                    handle_card(st, response, entry, actions);
                    ui.add_space(10.0);
                });
            }
        });
    });
}

// ------------------------------------------------------------------ home

pub fn home(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, actions: &mut Vec<Action>) {
    let now = ui.input(|i| i.time);
    let featured = library.catalog.first().cloned();
    if let Some(entry) = &featured {
        let height = (ui.available_width() * 0.3).clamp(260.0, 360.0);
        let (play, details) = cards::hero(ui, &mut st.thumbs, library, entry, height);
        if play {
            actions.push(Action::Play(entry.key.clone()));
        }
        if details {
            st.detail = Some(entry.key.clone());
        }
        ui.add_space(14.0);
    }

    let recent: Vec<GameEntry> = library.recent().into_iter().map(|(e, _)| e.clone()).collect();
    if !recent.is_empty() {
        widgets::section_header(ui, "Continue playing", "Pick up where you left off");
        shelf(ui, st, library, "recent", &recent, actions);
    }

    widgets::section_header(ui, "Popular", "The most played experiences on this device");
    let popular: Vec<GameEntry> = library.popular().into_iter().cloned().collect();
    shelf(ui, st, library, "popular", &popular, actions);

    widgets::section_header(ui, "Recommended for you", "Something different from what you usually play");
    let recommended: Vec<GameEntry> = library.recommended().into_iter().cloned().collect();
    shelf(ui, st, library, "recommended", &recommended, actions);

    widgets::section_header(ui, "Your projects", "Keep building");
    if library.projects.is_empty() {
        widgets::empty_state(ui, icons::HAMMER, "You haven't made anything yet", "Start from a blank world or remix one of the built-in games.", |ui| {
            if widgets::button(ui, ButtonKind::Primary, icons::PLUS, "Create your first game").clicked() {
                st.go(Page::Create, now);
                st.new_project = Some(NewProject { name: String::new(), template: None });
            }
        });
    } else {
        let projects: Vec<GameEntry> = library.projects.iter().take(8).cloned().collect();
        shelf(ui, st, library, "projects", &projects, actions);
    }
}

// -------------------------------------------------------------- discover

pub fn discover(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        if widgets::chip(ui, "All", st.category.is_none()).clicked() {
            st.category = None;
        }
        for category in CATEGORIES.iter().filter(|c| **c != "Other") {
            if widgets::chip(ui, category, st.category == Some(category)).clicked() {
                st.category = if st.category == Some(category) { None } else { Some(category) };
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Sort by").color(TEXT_DIM));
        for (sort, label) in [(Sort::Popular, "Most played"), (Sort::Newest, "Recently added"), (Sort::Name, "A–Z")] {
            if widgets::chip(ui, label, st.sort == sort).clicked() {
                st.sort = sort;
            }
        }
    });
    ui.add_space(10.0);

    let query = st.search.trim().to_lowercase();
    let mut results: Vec<GameEntry> = library
        .catalog
        .iter()
        .chain(library.projects.iter())
        .filter(|e| st.category.is_none_or(|c| e.manifest.category == c))
        .filter(|e| {
            query.is_empty()
                || e.manifest.name.to_lowercase().contains(&query)
                || e.manifest.description.to_lowercase().contains(&query)
                || e.manifest.category.to_lowercase().contains(&query)
                || e.manifest.tags.iter().any(|t| t.to_lowercase().contains(&query))
        })
        .cloned()
        .collect();
    match st.sort {
        Sort::Popular => results.sort_by(|a, b| library.play_count(&b.key).cmp(&library.play_count(&a.key)).then(b.manifest.featured.cmp(&a.manifest.featured))),
        Sort::Newest => results.sort_by(|a, b| b.manifest.created.cmp(&a.manifest.created)),
        Sort::Name => results.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase())),
    }

    let summary = match (results.len(), query.is_empty()) {
        (n, true) => format!("{n} experience{}", if n == 1 { "" } else { "s" }),
        (n, false) => format!("{n} result{} for “{}”", if n == 1 { "" } else { "s" }, st.search.trim()),
    };
    ui.label(egui::RichText::new(summary).color(TEXT_FAINT).font(theme::regular(13.0)));
    ui.add_space(6.0);
    if results.is_empty() {
        widgets::empty_state(ui, icons::MAGNIFYING_GLASS, "Nothing matches", "Try a different search or clear the filters.", |ui| {
            if widgets::button(ui, ButtonKind::Secondary, icons::X, "Clear filters").clicked() {
                st.search.clear();
                st.category = None;
            }
        });
    } else {
        grid(ui, st, library, &results, actions);
    }
}

// ----------------------------------------------------------------- store

pub fn store(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, actions: &mut Vec<Action>) {
    let installed = library.installed().count();
    ui.label(egui::RichText::new(format!("{} games ship with Araria · {installed} installed · free, offline, no account", library.catalog.len())).color(TEXT_DIM));
    ui.add_space(10.0);
    let catalog = library.catalog.clone();
    for entry in &catalog {
        store_row(ui, st, library, entry, actions);
        ui.add_space(4.0);
    }
}

fn store_row(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, entry: &GameEntry, actions: &mut Vec<Action>) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 132.0), Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), widgets::anim_time(ui));
    ui.painter().rect(rect, CornerRadius::same(theme::RADIUS), theme::mix(SURFACE, SURFACE_HI, hover), Stroke::new(1.0, theme::mix(BORDER, ACCENT, hover * 0.6)), StrokeKind::Inside);
    let cover = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(192.0, 108.0));
    cards::cover(ui, &mut st.thumbs, library, entry, cover, CornerRadius::same(theme::RADIUS_SMALL));

    let x = cover.right() + 20.0;
    ui.painter().text(pos2(x, rect.top() + 16.0), Align2::LEFT_TOP, &entry.manifest.name, theme::semibold(18.0), TEXT);
    let badge = widgets::badge(ui, pos2(x, rect.top() + 46.0), &entry.manifest.category, theme::with_alpha(theme::category_color(&entry.manifest.category), 0.16), theme::category_color(&entry.manifest.category));
    let status = if entry.installed { ("Installed", theme::SUCCESS) } else { ("Available", TEXT_DIM) };
    widgets::badge(ui, pos2(badge.right() + 8.0, rect.top() + 46.0), status.0, theme::with_alpha(status.1, 0.14), status.1);
    let text_width = (rect.right() - 230.0 - x).max(120.0);
    let description = ui.painter().layout(entry.manifest.description.clone(), theme::regular(13.0), TEXT_DIM, text_width);
    let clip = Rect::from_min_size(pos2(x, rect.top() + 76.0), vec2(text_width, 40.0));
    ui.painter().with_clip_rect(clip).galley(clip.min, description, TEXT_DIM);

    let buttons = Rect::from_min_max(pos2(rect.right() - 220.0, rect.top() + 24.0), pos2(rect.right() - 16.0, rect.bottom() - 24.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::top_down(egui::Align::Max)), |ui| {
        if entry.installed {
            if widgets::button(ui, ButtonKind::Primary, icons::PLAY, "Play").clicked() {
                actions.push(Action::Play(entry.key.clone()));
            }
        } else if widgets::button(ui, ButtonKind::Primary, icons::DOWNLOAD_SIMPLE, "Install").clicked() {
            match library.install(&entry.key.id) {
                Ok(()) => st.toast(ToastKind::Success, format!("Installed “{}”", entry.manifest.name)),
                Err(e) => st.toast(ToastKind::Error, format!("Install failed: {e}")),
            }
        }
        ui.label(egui::RichText::new(format!("{} · {}", size_label(entry.size), players_label(entry.manifest.max_players))).font(theme::regular(12.0)).color(TEXT_FAINT));
    });
    if response.clicked() {
        st.detail = Some(entry.key.clone());
    }
}

// ---------------------------------------------------------------- create

pub fn create(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, actions: &mut Vec<Action>) {
    // Call-to-action banner.
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
    let radius = CornerRadius::same(18);
    ui.painter().rect_filled(rect, radius, Color32::from_rgb(38, 28, 92));
    let painter = ui.painter().with_clip_rect(rect);
    painter.circle_filled(rect.right_top() + vec2(-90.0, 20.0), 150.0, theme::with_alpha(ACCENT, 0.5));
    painter.circle_filled(rect.right_bottom() + vec2(-260.0, 40.0), 110.0, theme::with_alpha(theme::ACCENT_2, 0.25));
    painter.text(rect.right_center() + vec2(-110.0, 0.0), Align2::CENTER_CENTER, icons::ROCKET, theme::regular(84.0), theme::with_alpha(Color32::WHITE, 0.9));
    ui.painter().text(rect.left_top() + vec2(32.0, 26.0), Align2::LEFT_TOP, "Build a game of your own", theme::bold(26.0), Color32::WHITE);
    ui.painter().text(rect.left_top() + vec2(32.0, 62.0), Align2::LEFT_TOP, "Place parts, write Lua, press Play. Everything saves to this device.", theme::regular(14.0), theme::with_alpha(Color32::WHITE, 0.82));
    let buttons = Rect::from_min_size(rect.left_top() + vec2(32.0, 92.0), vec2(400.0, 40.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(buttons).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        if widgets::button_sized(ui, ButtonKind::Primary, icons::PLUS, "New project", 40.0).clicked() {
            st.new_project = Some(NewProject { name: String::new(), template: None });
        }
    });
    ui.add_space(10.0);

    widgets::section_header(ui, "Templates", "Start from something that already works");
    let templates = library.catalog.clone();
    let width = card_width(ui.available_width()).min(280.0);
    egui::ScrollArea::horizontal().id_salt("templates").auto_shrink([false, true]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = CARD_GAP;
            ui.vertical(|ui| {
                ui.add_space(6.0);
                if blank_template_card(ui, width).clicked() {
                    st.new_project = Some(NewProject { name: String::new(), template: None });
                }
                ui.add_space(10.0);
            });
            for entry in &templates {
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    let response = cards::game_card(ui, &mut st.thumbs, library, entry, width);
                    if response.opened || response.play {
                        st.new_project = Some(NewProject { name: format!("My {}", entry.manifest.name), template: Some(entry.key.id.clone()) });
                    }
                    ui.add_space(10.0);
                });
            }
        });
    });

    widgets::section_header(ui, "Your projects", "Open one in the studio to keep working");
    if library.projects.is_empty() {
        widgets::empty_state(ui, icons::FOLDER_OPEN, "No projects yet", "Projects you create or import will appear here.", |_| {});
    } else {
        let projects = library.projects.clone();
        for entry in &projects {
            project_row(ui, st, library, entry, actions);
            ui.add_space(2.0);
        }
    }

    widgets::section_header(ui, "Import", "Bring in a project exported from another device");
    widgets::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(icons::UPLOAD_SIMPLE).font(theme::regular(26.0)).color(ACCENT_HI));
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Drop an .araria file anywhere on this window").font(theme::medium(14.0)));
                ui.label(egui::RichText::new("Imported projects get their own copy, so nothing is overwritten.").color(TEXT_DIM).font(theme::regular(12.5)));
            });
        });
        #[cfg(not(target_arch = "wasm32"))]
        ui.horizontal(|ui| {
            ui.add_sized([ui.available_width() - 120.0, 32.0], egui::TextEdit::singleline(&mut st.import_path).hint_text("…or paste the path to a .araria file"));
            if ui.add_enabled_ui(!st.import_path.trim().is_empty(), |ui| widgets::button(ui, ButtonKind::Secondary, icons::UPLOAD_SIMPLE, "Import")).inner.clicked() {
                st.import_from_path(library);
            }
        });
    });
}

fn blank_template_card(ui: &mut Ui, width: f32) -> egui::Response {
    let cover_height = (width * 9.0 / 16.0).round();
    let (rect, response) = ui.allocate_exact_size(vec2(width, cover_height + 68.0), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered(), widgets::anim_time(ui));
    let rect = rect.translate(vec2(0.0, -4.0 * hover));
    ui.painter().rect(rect, CornerRadius::same(theme::RADIUS), theme::mix(SURFACE, SURFACE_HI, hover), Stroke::new(1.5, theme::mix(BORDER, ACCENT, hover)), StrokeKind::Inside);
    let cover = Rect::from_min_size(rect.min, vec2(width, cover_height));
    ui.painter().circle_filled(cover.center(), 30.0 + 3.0 * hover, theme::with_alpha(ACCENT, 0.18 + 0.2 * hover));
    ui.painter().text(cover.center(), Align2::CENTER_CENTER, icons::PLUS, theme::regular(30.0), ACCENT_HI);
    ui.painter().text(pos2(rect.left() + 14.0, cover.bottom() + 12.0), Align2::LEFT_TOP, "Blank world", theme::semibold(15.0), TEXT);
    ui.painter().text(pos2(rect.left() + 14.0, cover.bottom() + 36.0), Align2::LEFT_TOP, "A baseplate and a spawn point", theme::regular(12.5), TEXT_DIM);
    response
}

fn project_row(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, entry: &GameEntry, actions: &mut Vec<Action>) {
    egui::Frame::new().fill(SURFACE).corner_radius(CornerRadius::same(theme::RADIUS)).stroke(Stroke::new(1.0, BORDER)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            mini_cover(ui, st, library, &entry.key, vec2(112.0, 63.0));
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(&entry.manifest.name).font(theme::semibold(15.0)));
                ui.label(egui::RichText::new(format!("Edited {} · {} · {}", ago(now_secs(), entry.manifest.modified), entry.manifest.category, size_label(entry.size))).font(theme::regular(12.5)).color(TEXT_DIM));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::TRASH, "Delete").clicked() {
                    st.confirm_delete = Some(entry.key.clone());
                }
                if widgets::icon_button(ui, icons::EXPORT, "Export as a file").clicked() {
                    st.export(library, &entry.key, actions);
                }
                if widgets::icon_button(ui, icons::COPY, "Duplicate").clicked() {
                    match library.duplicate_project(&entry.key.id) {
                        Ok(_) => st.toast(ToastKind::Success, format!("Duplicated “{}”", entry.manifest.name)),
                        Err(e) => st.toast(ToastKind::Error, format!("Could not duplicate: {e}")),
                    }
                }
                if widgets::button(ui, ButtonKind::Secondary, icons::PLAY, "Play").clicked() {
                    actions.push(Action::Play(entry.key.clone()));
                }
                if widgets::button(ui, ButtonKind::Primary, icons::PENCIL_SIMPLE, "Edit").clicked() {
                    actions.push(Action::Edit(entry.key.clone()));
                }
            });
        });
    });
}

/// Returns true when the dialog should close.
pub fn new_project_dialog(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, actions: &mut Vec<Action>) -> bool {
    let Some(dialog) = st.new_project.as_mut() else { return true };
    let mut close = false;
    let mut create = false;
    ui.set_width(460.0);
    ui.label(egui::RichText::new("New project").font(theme::semibold(21.0)));
    ui.label(egui::RichText::new("Give it a name and pick a starting point. You can change everything later.").color(TEXT_DIM));
    ui.add_space(10.0);
    ui.label(egui::RichText::new("Name").font(theme::medium(13.0)).color(TEXT_DIM));
    let name = ui.add_sized([460.0, 38.0], egui::TextEdit::singleline(&mut dialog.name).hint_text("My Awesome Game").font(theme::regular(15.0)).margin(egui::Margin::symmetric(12, 8)));
    if name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        create = true;
    }
    ui.add_space(8.0);
    ui.label(egui::RichText::new("Start from").font(theme::medium(13.0)).color(TEXT_DIM));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        if widgets::chip(ui, "Blank world", dialog.template.is_none()).clicked() {
            dialog.template = None;
        }
        for entry in &library.catalog {
            let selected = dialog.template.as_deref() == Some(entry.key.id.as_str());
            if widgets::chip(ui, &entry.manifest.name, selected).clicked() {
                dialog.template = Some(entry.key.id.clone());
            }
        }
    });
    ui.add_space(18.0);
    ui.horizontal(|ui| {
        create |= widgets::button(ui, ButtonKind::Primary, icons::HAMMER, "Create and open").clicked();
        close |= widgets::button(ui, ButtonKind::Secondary, "", "Cancel").clicked();
    });
    if create {
        let (name, template) = (dialog.name.clone(), dialog.template.clone());
        match library.create_project(&name, template.as_deref()) {
            Ok(key) => {
                st.toast(ToastKind::Success, "Project created");
                actions.push(Action::Edit(key));
                close = true;
            }
            Err(e) => st.toast(ToastKind::Error, format!("Could not create the project: {e}")),
        }
    }
    close
}

// --------------------------------------------------------------- library

pub fn library(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, actions: &mut Vec<Action>) {
    let now = ui.input(|i| i.time);
    let counts = [library.installed().count(), library.projects.len(), library.recent().len()];
    ui.horizontal(|ui| {
        for (i, label) in ["Installed games", "My projects", "Recently played"].iter().enumerate() {
            if widgets::chip(ui, &format!("{label}  {}", counts[i]), st.library_tab == i).clicked() {
                st.library_tab = i;
            }
        }
    });
    ui.add_space(12.0);
    match st.library_tab {
        0 => {
            let installed: Vec<GameEntry> = library.installed().cloned().collect();
            if installed.is_empty() {
                widgets::empty_state(ui, icons::STOREFRONT, "No games installed", "Games you install from the Store are kept here and work offline.", |ui| {
                    if widgets::button(ui, ButtonKind::Primary, icons::STOREFRONT, "Browse the Store").clicked() {
                        st.go(Page::Store, now);
                    }
                });
            } else {
                grid(ui, st, library, &installed, actions);
            }
        }
        1 => {
            if library.projects.is_empty() {
                widgets::empty_state(ui, icons::HAMMER, "No projects yet", "Create a game and it will be saved here automatically.", |ui| {
                    if widgets::button(ui, ButtonKind::Primary, icons::PLUS, "New project").clicked() {
                        st.go(Page::Create, now);
                        st.new_project = Some(NewProject { name: String::new(), template: None });
                    }
                });
            } else {
                let projects = library.projects.clone();
                for entry in &projects {
                    project_row(ui, st, library, entry, actions);
                    ui.add_space(2.0);
                }
            }
        }
        _ => {
            let recent: Vec<(GameEntry, u64)> = library.recent().into_iter().map(|(e, t)| (e.clone(), t)).collect();
            if recent.is_empty() {
                widgets::empty_state(ui, icons::CLOCK, "Nothing played yet", "Games you play show up here so you can jump back in.", |ui| {
                    if widgets::button(ui, ButtonKind::Primary, icons::COMPASS, "Discover games").clicked() {
                        st.go(Page::Discover, now);
                    }
                });
            }
            for (entry, time) in &recent {
                egui::Frame::new().fill(SURFACE).corner_radius(CornerRadius::same(theme::RADIUS)).stroke(Stroke::new(1.0, BORDER)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        mini_cover(ui, st, library, &entry.key, vec2(112.0, 63.0));
                        ui.vertical(|ui| {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(&entry.manifest.name).font(theme::semibold(15.0)));
                            let plays = library.play_count(&entry.key);
                            ui.label(egui::RichText::new(format!("Played {} · {plays} session{}", ago(now_secs(), *time), if plays == 1 { "" } else { "s" })).font(theme::regular(12.5)).color(TEXT_DIM));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(ui, ButtonKind::Primary, icons::PLAY, "Play").clicked() {
                                actions.push(Action::Play(entry.key.clone()));
                            }
                            if widgets::button(ui, ButtonKind::Ghost, icons::INFO, "Details").clicked() {
                                st.detail = Some(entry.key.clone());
                            }
                        });
                    });
                });
                ui.add_space(2.0);
            }
        }
    }
}

// ---------------------------------------------------------------- detail

/// The game detail sheet. Returns true when it should close.
pub fn detail(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, key: &GameKey, actions: &mut Vec<Action>) -> bool {
    let Some(entry) = library.find(key).cloned() else { return true };
    let mut close = false;
    let width = 760.0f32.min(ui.ctx().content_rect().width() - 48.0);
    ui.set_width(width);
    ui.spacing_mut().item_spacing.y = 0.0;

    let (cover, _) = ui.allocate_exact_size(vec2(width, 280.0), Sense::hover());
    cards::cover(ui, &mut st.thumbs, library, &entry, cover, CornerRadius { nw: 18, ne: 18, sw: 0, se: 0 });
    let lower = Rect::from_min_max(pos2(cover.left(), cover.bottom() - 150.0), cover.max);
    widgets::gradient(ui, lower, theme::with_alpha(SURFACE, 0.0), SURFACE);
    ui.painter().text(cover.left_bottom() + vec2(28.0, -46.0), Align2::LEFT_BOTTOM, &entry.manifest.name, theme::bold(32.0), Color32::WHITE);
    let badge = widgets::badge(ui, cover.left_bottom() + vec2(28.0, -38.0), &entry.manifest.category.to_uppercase(), theme::with_alpha(Color32::BLACK, 0.5), theme::category_color(&entry.manifest.category));
    for (i, tag) in entry.manifest.tags.iter().take(4).enumerate() {
        widgets::badge(ui, pos2(badge.right() + 8.0 + i as f32 * 84.0, badge.top()), tag, theme::with_alpha(Color32::BLACK, 0.4), TEXT_DIM);
    }
    let close_rect = Rect::from_center_size(cover.right_top() + vec2(-28.0, 28.0), vec2(34.0, 34.0));
    let close_button = ui.interact(close_rect, ui.id().with("close"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.painter().circle_filled(close_rect.center(), 17.0, theme::with_alpha(Color32::BLACK, if close_button.hovered() { 0.8 } else { 0.55 }));
    ui.painter().text(close_rect.center(), Align2::CENTER_CENTER, icons::X, theme::regular(17.0), Color32::WHITE);
    close |= close_button.clicked();

    egui::Frame::new().inner_margin(egui::Margin { left: 28, right: 28, top: 14, bottom: 26 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.horizontal(|ui| {
            let is_project = entry.key.origin == Origin::Project;
            if entry.installed {
                if widgets::button_sized(ui, ButtonKind::Primary, icons::PLAY, "Play", 44.0).clicked() {
                    actions.push(Action::Play(entry.key.clone()));
                    close = true;
                }
            } else if widgets::button_sized(ui, ButtonKind::Primary, icons::DOWNLOAD_SIMPLE, "Install", 44.0).clicked() {
                match library.install(&entry.key.id) {
                    Ok(()) => st.toast(ToastKind::Success, format!("Installed “{}”", entry.manifest.name)),
                    Err(e) => st.toast(ToastKind::Error, format!("Install failed: {e}")),
                }
            }
            if is_project {
                if widgets::button_sized(ui, ButtonKind::Secondary, icons::PENCIL_SIMPLE, "Edit in Studio", 44.0).clicked() {
                    actions.push(Action::Edit(entry.key.clone()));
                    close = true;
                }
                if widgets::button_sized(ui, ButtonKind::Ghost, icons::EXPORT, "Export", 44.0).clicked() {
                    st.export(library, &entry.key, actions);
                }
            } else if widgets::button_sized(ui, ButtonKind::Secondary, icons::PAINT_BRUSH, "Remix", 44.0).on_hover_text("Make your own editable copy").clicked() {
                st.new_project = Some(NewProject { name: format!("My {}", entry.manifest.name), template: Some(entry.key.id.clone()) });
                close = true;
            }
            if is_project || entry.installed {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if is_project { "Delete" } else { "Uninstall" };
                    if widgets::button_sized(ui, ButtonKind::Danger, icons::TRASH, label, 44.0).clicked() {
                        st.confirm_delete = Some(entry.key.clone());
                    }
                });
            }
        });
        ui.add_space(6.0);
        ui.columns(2, |columns| {
            columns[0].label(egui::RichText::new("About").font(theme::semibold(15.0)));
            let description = if entry.manifest.description.is_empty() { "No description yet." } else { &entry.manifest.description };
            columns[0].label(egui::RichText::new(description).color(TEXT_DIM));
            if !entry.manifest.controls.is_empty() {
                columns[0].add_space(6.0);
                columns[0].label(egui::RichText::new("Controls").font(theme::semibold(15.0)));
                for control in &entry.manifest.controls {
                    columns[0].label(egui::RichText::new(format!("{}  {control}", icons::KEYBOARD)).color(TEXT_DIM).font(theme::regular(13.0)));
                }
            }
            let info = &mut columns[1];
            info.label(egui::RichText::new("Details").font(theme::semibold(15.0)));
            let plays = library.play_count(&entry.key);
            let rows = [
                (icons::USERS, "Players", players_label(entry.manifest.max_players)),
                (icons::PACKAGE, "Size", size_label(entry.size)),
                (icons::TROPHY, "Played here", if plays == 0 { "Not yet".to_string() } else { format!("{plays} time{}", if plays == 1 { "" } else { "s" }) }),
                (icons::USER, "Made by", if entry.manifest.author.is_empty() { "You".to_string() } else { entry.manifest.author.clone() }),
                (icons::CLOCK, "Updated", ago(now_secs(), entry.manifest.modified)),
                (icons::WIFI_SLASH, "Works offline", "Yes".to_string()),
            ];
            for (icon, label, value) in rows {
                info.horizontal(|ui| {
                    ui.label(egui::RichText::new(icon).color(ACCENT_HI).font(theme::regular(16.0)));
                    ui.label(egui::RichText::new(label).color(TEXT_DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(egui::RichText::new(value).font(theme::medium(13.5))));
                });
            }
        });
    });
    close
}

// -------------------------------------------------------------- settings

const SETTINGS_TABS: [(&str, &str); 7] = [
    (icons::MONITOR, "Graphics"),
    (icons::SPEAKER_HIGH, "Audio"),
    (icons::KEYBOARD, "Controls"),
    (icons::PERSON_ARMS_SPREAD, "Accessibility"),
    (icons::GAUGE, "Performance"),
    (icons::CODE, "Editor"),
    (icons::FLOPPY_DISK, "Storage"),
];

/// One settings row: label and description on the left, control on the right.
fn row(ui: &mut Ui, title: &str, description: &str, control: impl FnOnce(&mut Ui) -> bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(46.0);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 300.0).max(160.0));
            ui.label(egui::RichText::new(title).font(theme::medium(14.5)));
            if !description.is_empty() {
                ui.add_space(-5.0);
                ui.label(egui::RichText::new(description).font(theme::regular(12.5)).color(TEXT_DIM));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| changed = control(ui));
    });
    ui.add_space(2.0);
    let line = ui.available_rect_before_wrap();
    ui.painter().line_segment([line.left_top(), pos2(line.right(), line.top())], Stroke::new(1.0, theme::with_alpha(BORDER, 0.6)));
    ui.add_space(6.0);
    changed
}

fn slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, percent: bool) -> bool {
    ui.spacing_mut().slider_width = 190.0;
    let mut slider = egui::Slider::new(value, range).trailing_fill(true);
    slider = if percent { slider.custom_formatter(|v, _| format!("{:.0}%", v * 100.0)) } else { slider.max_decimals(1) };
    ui.add(slider).changed()
}

fn switch(ui: &mut Ui, value: &mut bool) -> bool {
    widgets::toggle(ui, value).changed()
}

/// Script-facing name of an egui key (matches the engine's key names).
fn key_name(key: egui::Key) -> Option<String> {
    use egui::Key;
    Some(
        match key {
            Key::ArrowUp => "Up",
            Key::ArrowDown => "Down",
            Key::ArrowLeft => "Left",
            Key::ArrowRight => "Right",
            Key::Num0 => "Zero",
            Key::Num1 => "One",
            Key::Num2 => "Two",
            Key::Num3 => "Three",
            Key::Num4 => "Four",
            Key::Num5 => "Five",
            Key::Num6 => "Six",
            Key::Num7 => "Seven",
            Key::Num8 => "Eight",
            Key::Num9 => "Nine",
            Key::Space | Key::Enter | Key::Tab | Key::Backspace => key.name(),
            other => {
                let name = other.name();
                return (name.len() == 1 && name.chars().all(|c| c.is_ascii_uppercase())).then(|| name.to_string());
            }
        }
        .to_string(),
    )
}

pub fn settings(ui: &mut Ui, st: &mut PlatformUi, library: &mut Library, settings: &mut Settings, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for (i, (icon, label)) in SETTINGS_TABS.iter().enumerate() {
            if widgets::chip(ui, &format!("{icon}  {label}"), st.settings_tab == i).clicked() {
                st.settings_tab = i;
                st.rebinding = None;
            }
        }
    });
    ui.add_space(12.0);
    let before = settings.clone();
    let max_width = ui.available_width().min(860.0);
    ui.scope(|ui| {
        ui.set_max_width(max_width);
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            match st.settings_tab {
                0 => {
                    let g = &mut settings.graphics;
                    row(ui, "Quality preset", "Sets the options below for your device", |ui| {
                        let mut picked = None;
                        for (quality, label) in [(Quality::High, "High"), (Quality::Medium, "Medium"), (Quality::Low, "Low")] {
                            if widgets::chip(ui, label, g.quality == quality).clicked() {
                                picked = Some(quality);
                            }
                        }
                        if let Some(quality) = picked {
                            g.apply_preset(quality);
                        }
                        picked.is_some()
                    });
                    row(ui, "Shadows", "Sunlight casts shadows", |ui| switch(ui, &mut g.shadows));
                    row(ui, "Bloom", "Bright and glowing objects shine", |ui| switch(ui, &mut g.bloom));
                    row(ui, "Anti-aliasing", "Smooths jagged edges", |ui| switch(ui, &mut g.antialiasing));
                    row(ui, "Particles", "Sparks, smoke and other effects", |ui| switch(ui, &mut g.particles));
                    row(ui, "View distance", "How far you can see before fog", |ui| slider(ui, &mut g.view_distance, 0.5..=2.0, true));
                    row(ui, "Field of view", "Wider shows more of the world", |ui| slider(ui, &mut g.field_of_view, 50.0..=100.0, false));
                }
                1 => {
                    let a = &mut settings.audio;
                    row(ui, "Mute everything", "", |ui| switch(ui, &mut a.muted));
                    row(ui, "Master volume", "", |ui| slider(ui, &mut a.master, 0.0..=1.0, true));
                    row(ui, "Music", "", |ui| slider(ui, &mut a.music, 0.0..=1.0, true));
                    row(ui, "Sound effects", "", |ui| slider(ui, &mut a.effects, 0.0..=1.0, true));
                }
                2 => {
                    row(ui, "Mouse sensitivity", "How fast the camera turns", |ui| slider(ui, &mut settings.input.mouse_sensitivity, 0.2..=3.0, false));
                    row(ui, "Invert vertical look", "", |ui| switch(ui, &mut settings.input.invert_y));
                    // Capture a key for the binding being changed.
                    if let Some(index) = st.rebinding {
                        let pressed = ui.input(|i| {
                            if i.modifiers.shift {
                                return Some("LeftShift".to_string());
                            }
                            if i.modifiers.ctrl {
                                return Some("LeftControl".to_string());
                            }
                            i.events.iter().find_map(|e| match e {
                                egui::Event::Key { key, pressed: true, .. } => Some(if *key == egui::Key::Escape { String::new() } else { key_name(*key).unwrap_or_default() }),
                                _ => None,
                            })
                        });
                        if let Some(name) = pressed {
                            if !name.is_empty() {
                                *settings.input.bindings_mut()[index].1 = name;
                            }
                            st.rebinding = None;
                        }
                    }
                    let rebinding = st.rebinding;
                    let mut start = None;
                    for (i, (label, binding)) in settings.input.bindings_mut().into_iter().enumerate() {
                        row(ui, label, "", |ui| {
                            let waiting = rebinding == Some(i);
                            let text = if waiting { "Press a key…".to_string() } else { binding.clone() };
                            if widgets::button(ui, if waiting { ButtonKind::Primary } else { ButtonKind::Secondary }, "", &text).clicked() {
                                start = Some(i);
                            }
                            false
                        });
                    }
                    if let Some(i) = start {
                        st.rebinding = Some(i);
                    }
                    ui.horizontal(|ui| {
                        if widgets::button(ui, ButtonKind::Ghost, icons::ARROW_COUNTER_CLOCKWISE, "Reset controls").clicked() {
                            settings.input = Default::default();
                        }
                    });
                }
                3 => {
                    let a = &mut settings.accessibility;
                    row(ui, "Interface size", "Scales text and controls", |ui| {
                        ui.spacing_mut().slider_width = 190.0;
                        // Applied on release so the slider does not move under the pointer.
                        let mut scale = a.ui_scale;
                        let response = ui.add(egui::Slider::new(&mut scale, 0.75..=1.75).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).trailing_fill(true));
                        if response.drag_stopped() || (response.changed() && !response.dragged()) {
                            a.ui_scale = scale;
                            return true;
                        }
                        false
                    });
                    row(ui, "Reduce motion", "Turns off interface animations", |ui| switch(ui, &mut a.reduce_motion));
                    row(ui, "Camera shake", "Allow games to shake the view", |ui| switch(ui, &mut a.camera_shake));
                    row(ui, "High contrast", "Stronger outlines on interface elements", |ui| switch(ui, &mut a.high_contrast));
                    row(ui, "Display name", "Shown above your character in multiplayer. Not an account.", |ui| {
                        ui.add_sized([200.0, 32.0], egui::TextEdit::singleline(&mut settings.display_name).hint_text("Player").char_limit(20)).changed()
                    });
                }
                4 => {
                    let p = &mut settings.performance;
                    row(ui, "Show frame rate", "Displays frames per second while playing", |ui| switch(ui, &mut p.show_fps));
                    row(ui, "Frame rate limit", "Lower limits save battery", |ui| {
                        let mut changed = false;
                        for (limit, label) in [(0, "Unlimited"), (120, "120"), (60, "60"), (30, "30")] {
                            if widgets::chip(ui, label, p.frame_limit == limit).clicked() {
                                p.frame_limit = limit;
                                changed = true;
                            }
                        }
                        changed
                    });
                    row(ui, "Pause in background", "Stop simulating when the window is not focused", |ui| switch(ui, &mut p.pause_when_unfocused));
                }
                5 => {
                    let e = &mut settings.editor;
                    row(ui, "Code font size", "", |ui| slider(ui, &mut e.font_size, 10.0..=24.0, false));
                    row(ui, "Tab width", "Spaces per indent level", |ui| {
                        let mut changed = false;
                        for width in [8, 4, 2] {
                            if widgets::chip(ui, &width.to_string(), e.tab_width == width).clicked() {
                                e.tab_width = width;
                                changed = true;
                            }
                        }
                        changed
                    });
                    row(ui, "Line numbers", "", |ui| switch(ui, &mut e.line_numbers));
                    row(ui, "Autocomplete", "Suggest engine APIs while typing", |ui| switch(ui, &mut e.autocomplete));
                    row(ui, "Autosave", "Save scripts and scenes as you work", |ui| switch(ui, &mut e.autosave));
                    row(ui, "Hot reload", "Apply script changes to the running game", |ui| switch(ui, &mut e.hot_reload));
                    row(ui, "Move snap", "Grid size when dragging objects (0 = off)", |ui| slider(ui, &mut e.snap_move, 0.0..=4.0, false));
                    row(ui, "Rotate snap", "Angle step in degrees (0 = off)", |ui| slider(ui, &mut e.snap_rotate, 0.0..=90.0, false));
                }
                _ => {
                    let storage = library.storage();
                    let files = storage.list("");
                    let bytes: u64 = files.iter().filter_map(|p| storage.read(p)).map(|d| d.len() as u64).sum();
                    row(ui, "Where your data lives", &storage.location(), |_| false);
                    row(ui, "Space used", &format!("{} files", files.len()), |ui| {
                        ui.label(egui::RichText::new(size_label(bytes)).font(theme::semibold(15.0)));
                        false
                    });
                    row(ui, "Accounts", "Araria has none. Nothing is uploaded; projects and progress stay on this device.", |ui| {
                        ui.label(egui::RichText::new(icons::LOCK_OPEN).color(theme::SUCCESS).font(theme::regular(22.0)));
                        false
                    });
                    if let Some(warning) = storage.warning() {
                        ui.label(egui::RichText::new(format!("{}  {warning}", icons::WARNING)).color(theme::WARNING));
                    }
                    row(ui, "Reset settings", "Restore every option to its default", |ui| {
                        if widgets::button(ui, ButtonKind::Danger, icons::ARROW_COUNTER_CLOCKWISE, "Reset").clicked() {
                            *settings = Settings::default();
                            return true;
                        }
                        false
                    });
                }
            }
        });
    });
    if *settings != before {
        settings.save(library.storage());
        actions.push(Action::SettingsChanged);
    }
}
