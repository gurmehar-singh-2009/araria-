//! Araria application entry point (native + WebAssembly).

mod demo;

use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_engine::input::UiCapture;
use araria_engine::scene_sync::AssetLibrary;
use araria_engine::{EnginePlugin, Game};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

/// Command-line options (native builds only).
#[derive(Resource, Default)]
struct Options {
    /// Save a screenshot to this path after `screenshot_frame` frames, then exit.
    screenshot: Option<String>,
    screenshot_frame: u32,
}

fn parse_options() -> Options {
    let mut options = Options { screenshot_frame: 90, ..default() };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--screenshot" => options.screenshot = args.next(),
            "--dump-demo" => {
                // Writes the demo scene as a scene file (used to seed `games/playground`).
                if let Some(path) = args.next() {
                    let text = araria_core::scene::save_scene_string(&demo::scene());
                    std::fs::write(&path, text).unwrap_or_else(|e| eprintln!("cannot write {path}: {e}"));
                }
                std::process::exit(0);
            }
            "--frames" => options.screenshot_frame = args.next().and_then(|v| v.parse().ok()).unwrap_or(90),
            _ => {}
        }
    }
    options
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin { file_path: asset_root(), ..default() }).set(WindowPlugin {
            primary_window: Some(Window {
                title: "Araria".into(),
                canvas: Some("#araria-canvas".into()),
                fit_canvas_to_parent: true,
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .add_plugins(EnginePlugin)
        .insert_resource(parse_options())
        .add_systems(Startup, start_demo)
        .add_systems(Update, capture_screenshot)
        .add_systems(EguiPrimaryContextPass, overlay)
        .run();
}

/// Where the asset library lives: next to the page on the web; on desktop
/// `$ARARIA_ASSETS`, `./assets`, or `assets` beside the executable.
fn asset_root() -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(path) = std::env::var("ARARIA_ASSETS") {
            return path;
        }
        if let Ok(cwd) = std::env::current_dir() {
            let local = cwd.join("assets");
            if local.join("manifest.json").exists() {
                return local.to_string_lossy().into_owned();
            }
        }
    }
    "assets".to_string()
}

fn start_demo(mut game: NonSendMut<Game>, mut library: ResMut<AssetLibrary>) {
    // The manifest is small, so it is compiled in rather than fetched.
    let manifest = AssetManifest::parse(include_str!("../../../assets/manifest.json")).unwrap_or_else(|e| {
        error!("built-in asset manifest is invalid: {e}");
        AssetManifest::default()
    });
    library.0 = Arc::new(manifest);
    let runtime = game.load(demo::scene(), library.0.clone());
    runtime.start();
}

fn overlay(mut contexts: EguiContexts, game: NonSend<Game>, mut capture: ResMut<UiCapture>) -> Result {
    let ctx = contexts.ctx_mut()?;
    capture.pointer = ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input();
    capture.keyboard = ctx.egui_wants_keyboard_input();
    egui::Window::new("Araria").anchor(egui::Align2::LEFT_TOP, [12.0, 12.0]).resizable(false).show(ctx, |ui| {
        ui.label("WASD move · Space jump · Shift run");
        ui.label("Hold right mouse to look · wheel to zoom");
        if let Some(runtime) = game.runtime.as_ref() {
            let dm = runtime.dm.borrow();
            ui.label(format!("{} instances · {} bodies", dm.len(), runtime.physics.borrow().body_count()));
            if let Some(character) = runtime.local_character().and_then(|c| dm.get(c)) {
                let p = character.vec3("Position");
                ui.label(format!("{} at {:.1}, {:.1}, {:.1}", character.str("State"), p.x, p.y, p.z));
            }
        }
    });
    Ok(())
}

fn capture_screenshot(mut commands: Commands, options: Res<Options>, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let Some(path) = options.screenshot.clone() else { return };
    *frame += 1;
    if *frame == options.screenshot_frame {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    } else if *frame == options.screenshot_frame + 30 {
        exit.write(AppExit::Success);
    }
}
