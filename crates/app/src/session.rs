//! Ties the platform interface to the engine: launching games, the in-game
//! menu, and applying settings.

use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_engine::Game;
use araria_engine::camera::{CameraRig, RenderSettings};
use araria_engine::input::{KeyBindings, UiCapture};
use araria_engine::scene_sync::AssetLibrary;
use araria_platform::widgets::{self, ButtonKind};
use araria_platform::{Action, GameKey, Library, Page, PlatformUi, Settings, ToastKind, icons, theme};
use araria_storage::Storage;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::Options;

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

impl Session {
    /// Loads a game into the engine and switches to it.
    fn launch(&mut self, key: &GameKey, editing: bool, game: &mut Game, assets: &AssetLibrary, rig: &mut CameraRig) {
        match self.library.load(key) {
            Ok(loaded) => {
                for warning in &loaded.warnings {
                    warn!("{}: {warning}", loaded.manifest.name);
                }
                let runtime = game.load(loaded.model, assets.0.clone());
                if editing {
                    self.library.record_project_opened(&key.id);
                } else {
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
            }
            Err(e) => self.ui.toast(ToastKind::Error, format!("Could not start the game: {e}")),
        }
    }

    fn leave(&mut self, game: &mut Game) {
        game.unload();
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
    time: Res<Time>,
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
    match &session.screen {
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
            if !options.hide_ui {
                hud(&root, &name, editing, session.settings.performance.show_fps, time.delta_secs(), &mut session.menu_open);
            }
            if session.menu_open {
                (leave, restart) = pause_menu(&ctx, &name, editing, &mut session.menu_open);
            }
            capture.pointer = session.menu_open || ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input();
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
