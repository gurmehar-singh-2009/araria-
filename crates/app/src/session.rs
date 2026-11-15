//! Ties the platform interface to the engine: launching games, the in-game
//! menu, and applying settings.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_engine::{Game, PresentationCommands};
use araria_engine::camera::{CameraRig, RenderSettings};
use araria_engine::input::{GameViewport, KeyBindings, UiCapture};
use araria_engine::scene_sync::AssetLibrary;
use araria_platform::widgets::{self, ButtonKind};
use araria_platform::{Action, GameKey, Library, Page, PlatformUi, Settings, ToastKind, icons, theme};
use araria_runtime::{LogLevel, ProjectFiles, SaveBackend};
use araria_storage::Storage;
use araria_studio::Studio;
use araria_studio::project::ProjectFolder;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::Options;
use crate::hud::{Hud, HudFrame};

/// What is on screen.
enum Screen {
    Platform,
    /// A game is running. `editing` is true for projects opened from Create.
    InGame { key: GameKey, name: String, editing: bool },
}

/// Application state that is not part of the engine.
pub struct Session {
    library: Library,
    settings: Settings,
    ui: PlatformUi,
    screen: Screen,
    menu_open: bool,
    started: bool,
    hud: Hud,
    /// Textures loaded for `ImageLabel` elements, by asset id.
    textures: HashMap<String, (Handle<Image>, egui::TextureId)>,
    /// The development environment, while a project is open for editing.
    studio: Option<Studio>,
}

pub fn setup(world: &mut World) {
    let storage = Storage::open();
    let settings = Settings::load(&storage);
    // The manifest is small, so it is compiled in rather than fetched.
    let manifest = AssetManifest::parse(include_str!("../../../assets/manifest.json")).unwrap_or_else(|e| {
        error!("built-in asset manifest is invalid: {e}");
        AssetManifest::default()
    });
    world.resource_mut::<AssetLibrary>().0 = Arc::new(manifest);
    world.insert_non_send(Session {
        library: Library::new(storage),
        settings,
        ui: PlatformUi::default(),
        screen: Screen::Platform,
        menu_open: false,
        started: false,
        hud: Hud::default(),
        textures: HashMap::new(),
        studio: None,
    });
}

fn apply_settings(settings: &Settings, render: &mut RenderSettings, bindings: &mut KeyBindings, rig: &mut CameraRig) {
    let g = &settings.graphics;
    *render = RenderSettings {
        shadows: g.shadows,
        bloom: g.bloom,
        antialiasing: g.antialiasing,
        view_distance: g.view_distance,
        field_of_view: g.field_of_view,
        invert_y: settings.input.invert_y,
    };
    let i = &settings.input;
    *bindings = KeyBindings { forward: i.forward.clone(), back: i.back.clone(), left: i.left.clone(), right: i.right.clone(), jump: i.jump.clone(), run: i.run.clone() };
    rig.sensitivity = 0.22 * i.mouse_sensitivity;
}

/// Script sources come straight from the game's folder in local storage, so
/// edits (from the built-in editor or, on desktop, any other tool) are
/// picked up by hot reload.
pub fn project_files(storage: &Storage, key: &GameKey) -> ProjectFiles {
    let root = key.root();
    let (read_storage, read_root) = (storage.clone(), root.clone());
    let print_storage = storage.clone();
    ProjectFiles::new(move |path| read_storage.read_string(&format!("{read_root}/{path}")), move |path| print_storage.fingerprint(&format!("{root}/{path}")))
}

/// Game progress (`Save:Get` / `Save:Set`), one small JSON file per game.
struct GameSaves {
    storage: Storage,
    path: String,
}

impl GameSaves {
    fn load(&self) -> BTreeMap<String, String> {
        self.storage.read_string(&self.path).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }
}

impl SaveBackend for GameSaves {
    fn get(&self, key: &str) -> Option<String> {
        self.load().remove(key)
    }

    fn set(&self, key: &str, value: Option<&str>) {
        let mut values = self.load();
        match value {
            Some(value) => values.insert(key.to_string(), value.to_string()),
            None => values.remove(key),
        };
        let text = serde_json::to_string(&values).expect("a string map is always serializable");
        if let Err(e) = self.storage.write(&self.path, text.as_bytes()) {
            warn!("could not save game progress: {e}");
        }
    }
}

impl Session {
    /// Loads a game into the engine and switches to it.
    fn launch(&mut self, key: &GameKey, editing: bool, game: &mut Game, assets: &AssetLibrary, rig: &mut CameraRig) {
        match self.library.load(key) {
            Ok(loaded) => {
                for warning in &loaded.warnings {
                    warn!("{}: {warning}", loaded.manifest.name);
                }
                let runtime = game.load(loaded.model, assets.0.clone(), project_files(self.library.storage(), key));
                runtime.set_save_backend(Rc::new(GameSaves { storage: self.library.storage().clone(), path: format!("saves/{}-{}.json", if key.origin == araria_platform::Origin::Catalog { "game" } else { "project" }, key.id) }));
                if editing {
                    self.library.record_project_opened(&key.id);
                    let folder = ProjectFolder::new(self.library.storage().clone(), key.root());
                    self.studio = Some(Studio::new(&loaded.manifest.name, folder, assets.0.clone(), runtime));
                    // Start the editing camera with an overview of the scene.
                    rig.yaw = 180.0;
                    rig.pitch = 24.0;
                    rig.fly_position = Vec3::new(0.0, 14.0, 30.0);
                } else {
                    self.studio = None;
                    runtime.start();
                    // Start with the camera behind the character.
                    if let Some(character) = runtime.local_character() {
                        rig.yaw = runtime.dm.borrow().get(character).map_or(180.0, |c| c.f32("Facing"));
                        rig.pitch = 16.0;
                    }
                    self.library.record_play(key);
                }
                self.screen = Screen::InGame { key: key.clone(), name: loaded.manifest.name, editing };
                self.menu_open = false;
                self.hud.clear();
            }
            Err(e) => self.ui.toast(ToastKind::Error, format!("Could not start the game: {e}")),
        }
    }

    fn leave(&mut self, game: &mut Game) {
        game.unload();
        self.studio = None;
        self.hud.clear();
        self.library.refresh();
        self.screen = Screen::Platform;
        self.menu_open = false;
    }
}

#[cfg(target_arch = "wasm32")]
fn download(name: &str, contents: &str) {
    use wasm_bindgen::JsCast;
    let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
    let Ok(anchor) = document.create_element("a") else { return };
    let href = format!("data:application/json;charset=utf-8,{}", js_sys::encode_uri_component(contents));
    let _ = anchor.set_attribute("href", &href);
    let _ = anchor.set_attribute("download", name);
    if let Ok(element) = anchor.dyn_into::<web_sys::HtmlElement>() {
        element.click();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn download(_name: &str, _contents: &str) {}

#[allow(clippy::too_many_arguments)]
pub fn interface(
    mut contexts: EguiContexts,
    mut session: NonSendMut<Session>,
    mut game: NonSendMut<Game>,
    assets: Res<AssetLibrary>,
    options: Res<Options>,
    mut capture: ResMut<UiCapture>,
    mut render: ResMut<RenderSettings>,
    mut bindings: ResMut<KeyBindings>,
    mut rig: ResMut<CameraRig>,
    mut viewport: ResMut<GameViewport>,
    time: Res<Time>,
    presentation: Res<PresentationCommands>,
    asset_server: Res<AssetServer>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let session = &mut *session;
    if !session.ui.apply_style(&ctx, &session.settings) {
        return Ok(());
    }

    // Command-line shortcuts, once storage is ready.
    if !session.started && session.library.storage().is_ready() {
        session.started = true;
        session.library.refresh();
        session.settings = Settings::load(session.library.storage());
        apply_settings(&session.settings, &mut render, &mut bindings, &mut rig);
        if let Some(id) = &options.play {
            session.launch(&GameKey::catalog(id), false, &mut game, &assets, &mut rig);
        }
        if let Some(id) = &options.edit {
            let key = GameKey::project(id);
            let key = if session.library.find(&key).is_some() {
                Some(key)
            } else {
                // A catalog id: open (or make) the user's remix of it.
                let existing = session.library.projects.iter().find(|p| p.manifest.description.contains(&format!("remix of {}", session.library.find(&GameKey::catalog(id)).map_or(String::new(), |g| g.manifest.name.clone())))).map(|p| p.key.clone());
                existing.or_else(|| session.library.create_project(&format!("My {id}"), Some(id)).map_err(|e| error!("cannot create a project from {id}: {e}")).ok())
            };
            if let Some(key) = key {
                session.launch(&key, true, &mut game, &assets, &mut rig);
                if let (Some(studio), Some(runtime)) = (session.studio.as_mut(), game.runtime.as_mut()) {
                    if let Some(path) = &options.select {
                        let found = runtime.dm.borrow().find_by_path(path);
                        if let Some(found) = found {
                            studio.selection = vec![found];
                        }
                    }
                    if let Some(path) = &options.open_script {
                        studio.open_file(path, 1);
                    }
                    if options.studio_play {
                        runtime.start();
                    }
                }
            }
        }
        session.ui.page = match options.page.as_deref() {
            Some("discover") => Page::Discover,
            Some("store") => Page::Store,
            Some("create") => Page::Create,
            Some("library") => Page::Library,
            Some("settings") => Page::Settings,
            _ => Page::Home,
        };
    }

    let mut root = egui::Ui::new(
        ctx.clone(),
        "araria-root".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.content_rect()),
    );

    let mut actions = Vec::new();
    if !matches!(session.screen, Screen::InGame { editing: true, .. }) && viewport.0.is_some() {
        viewport.0 = None;
    }
    match &session.screen {
        Screen::InGame { editing: true, .. } if session.studio.is_some() && game.runtime.is_some() => {
            let studio = session.studio.as_mut().expect("checked");
            let runtime = game.runtime.as_mut().expect("checked");
            let output = studio.show(&mut root, runtime);
            // egui works in points; the engine wants window pixels (logical).
            let zoom = ctx.zoom_factor();
            let area = output.viewport.map(|r| Rect::new(r.min.x * zoom, r.min.y * zoom, r.max.x * zoom, r.max.y * zoom));
            if viewport.0 != area {
                viewport.0 = area;
            }
            // The game's own interface shows inside the 3D view while playing.
            let mut over_interface = false;
            if let (Some(area), true) = (output.viewport, runtime.mode() != araria_runtime::Mode::Edit) {
                let textures = &mut session.textures;
                let library = assets.0.clone();
                let mut texture = |asset: &str| -> Option<egui::TextureId> {
                    if let Some((_, id)) = textures.get(asset) {
                        return Some(*id);
                    }
                    let file = library.textures.get(asset)?.file.clone();
                    let handle: Handle<Image> = asset_server.load(file);
                    let id = contexts.add_image(bevy_egui::EguiTextureHandle::Strong(handle.clone()));
                    textures.insert(asset.to_string(), (handle, id));
                    Some(id)
                };
                let mut frame = HudFrame { runtime, commands: &presentation.0, area, dt: time.delta_secs(), interactive: true, texture: &mut texture };
                // Keep the game's interface inside the 3D view.
                let mut hud_ui = root.new_child(egui::UiBuilder::new().max_rect(area));
                hud_ui.set_clip_rect(area);
                over_interface = session.hud.draw(&hud_ui, &mut frame);
            } else {
                session.hud.clear();
            }
            capture.pointer = output.capture_pointer || over_interface;
            capture.keyboard = output.capture_keyboard;
            if let Some((center, radius)) = output.focus {
                let distance = radius * 2.2 + 4.0;
                rig.fly_position = center - rig.direction() * distance;
            }
            if output.started_playing {
                if let Some(character) = runtime.local_character() {
                    rig.yaw = runtime.dm.borrow().get(character).map_or(180.0, |c| c.f32("Facing"));
                    rig.pitch = 16.0;
                }
            }
            if output.leave {
                session.leave(&mut game);
                viewport.0 = None;
            }
        }
        Screen::Platform => {
            actions = session.ui.show(&mut root, &mut session.library, &mut session.settings);
            capture.pointer = true;
            capture.keyboard = true;
        }
        Screen::InGame { key, name, editing } => {
            let (key, name, editing) = (key.clone(), name.clone(), *editing);
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                session.menu_open = !session.menu_open;
            }
            if let Some(runtime) = game.runtime.as_mut() {
                runtime.set_paused(session.menu_open);
            }
            let mut leave = false;
            let mut restart = false;
            let mut over_interface = false;
            if let Some(runtime) = game.runtime.as_mut() {
                // Script output has no console outside the studio; keep it in the log.
                for entry in runtime.scripts.take_log() {
                    match entry.level {
                        LogLevel::Error => error!("[{}] {}", entry.source.as_deref().unwrap_or("script"), entry.text),
                        LogLevel::Warn => warn!("[{}] {}", entry.source.as_deref().unwrap_or("script"), entry.text),
                        LogLevel::Info => info!("[{}] {}", entry.source.as_deref().unwrap_or("script"), entry.text),
                    }
                }
                let textures = &mut session.textures;
                let library = assets.0.clone();
                let mut texture = |asset: &str| -> Option<egui::TextureId> {
                    if let Some((_, id)) = textures.get(asset) {
                        return Some(*id);
                    }
                    let file = library.textures.get(asset)?.file.clone();
                    let handle: Handle<Image> = asset_server.load(file);
                    let id = contexts.add_image(bevy_egui::EguiTextureHandle::Strong(handle.clone()));
                    textures.insert(asset.to_string(), (handle, id));
                    Some(id)
                };
                let mut frame = HudFrame { runtime, commands: &presentation.0, area: root.max_rect(), dt: time.delta_secs(), interactive: !session.menu_open, texture: &mut texture };
                over_interface = session.hud.draw(&root, &mut frame);
            }
            if !options.hide_ui {
                hud(&root, &name, editing, session.settings.performance.show_fps, time.delta_secs(), &mut session.menu_open);
            }
            if session.menu_open {
                (leave, restart) = pause_menu(&ctx, &name, editing, &mut session.menu_open);
            }
            capture.pointer = session.menu_open || over_interface || ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input();
            capture.keyboard = session.menu_open || ctx.egui_wants_keyboard_input();
            if restart {
                session.launch(&key, editing, &mut game, &assets, &mut rig);
            } else if leave {
                session.leave(&mut game);
            }
        }
    }

    for action in actions {
        match action {
            Action::Play(key) => session.launch(&key, false, &mut game, &assets, &mut rig),
            Action::Edit(key) => session.launch(&key, true, &mut game, &assets, &mut rig),
            Action::SettingsChanged => apply_settings(&session.settings, &mut render, &mut bindings, &mut rig),
            Action::Download { name, contents } => download(&name, &contents),
            Action::Exit => {
                exit.write(AppExit::Success);
            }
        }
    }
    Ok(())
}

/// The always-visible in-game overlay: menu button, title and frame rate.
fn hud(root: &egui::Ui, name: &str, editing: bool, show_fps: bool, dt: f32, menu_open: &mut bool) {
    let area = root.max_rect();
    let painter = root.painter();
    let button = egui::Rect::from_min_size(area.min + egui::vec2(16.0, 16.0), egui::vec2(44.0, 44.0));
    let response = root.interact(button, egui::Id::new("hud-menu"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Menu (Esc)");
    let fill = if response.hovered() { theme::ACCENT } else { theme::with_alpha(theme::BG, 0.72) };
    painter.rect_filled(button, egui::CornerRadius::same(14), fill);
    painter.text(button.center(), egui::Align2::CENTER_CENTER, icons::LIST, theme::regular(22.0), egui::Color32::WHITE);
    if response.clicked() {
        *menu_open = !*menu_open;
    }
    let label = if editing { format!("{name}  ·  viewing project") } else { name.to_string() };
    let galley = painter.layout_no_wrap(label, theme::semibold(14.0), egui::Color32::WHITE);
    let pill = egui::Rect::from_min_size(button.right_top() + egui::vec2(10.0, 6.0), galley.size() + egui::vec2(28.0, 14.0));
    painter.rect_filled(pill, egui::CornerRadius::same(16), theme::with_alpha(theme::BG, 0.72));
    painter.galley(pill.min + egui::vec2(14.0, 7.0), galley, egui::Color32::WHITE);
    if show_fps && dt > 0.0 {
        let text = format!("{:.0} fps", 1.0 / dt);
        painter.text(area.right_top() + egui::vec2(-18.0, 22.0), egui::Align2::RIGHT_CENTER, text, theme::mono(13.0), theme::SUCCESS);
    }
}

/// Returns (leave, restart).
fn pause_menu(ctx: &egui::Context, name: &str, editing: bool, open: &mut bool) -> (bool, bool) {
    let (mut leave, mut restart) = (false, false);
    let frame = egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(18))
        .inner_margin(egui::Margin::same(28));
    let modal = egui::Modal::new(egui::Id::new("pause-menu")).frame(frame).backdrop_color(theme::with_alpha(egui::Color32::from_rgb(5, 6, 10), 0.6)).show(ctx, |ui| {
        ui.set_width(300.0);
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(if editing { "Project" } else { "Paused" }).font(theme::regular(13.0)).color(theme::TEXT_DIM));
            ui.label(egui::RichText::new(name).font(theme::bold(24.0)));
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            if widgets::button_sized(ui, ButtonKind::Primary, icons::PLAY, "Resume", 46.0).clicked() {
                *open = false;
            }
            if widgets::button_sized(ui, ButtonKind::Secondary, icons::ARROW_COUNTER_CLOCKWISE, "Restart", 46.0).clicked() {
                restart = true;
            }
            if widgets::button_sized(ui, ButtonKind::Danger, icons::SIGN_OUT, "Leave game", 46.0).clicked() {
                leave = true;
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new("WASD move · Space jump · Shift run\nHold right mouse to look · wheel to zoom").font(theme::regular(12.0)).color(theme::TEXT_FAINT));
        });
    });
    if modal.should_close() {
        *open = false;
    }
    (leave, restart)
}
