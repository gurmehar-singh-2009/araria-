//! Araria application entry point (native + WebAssembly).

mod demo;
mod hud;
mod session;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::camera::CameraOutputMode;
use bevy::render::render_resource::BlendState;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext};

/// Command-line options (native builds only).
#[derive(Resource, Default)]
pub struct Options {
    /// Save a screenshot to this path after `screenshot_frame` frames, then exit.
    screenshot: Option<String>,
    screenshot_frame: u32,
    /// Start directly in this catalog game instead of the home screen.
    pub play: Option<String>,
    /// Hide the in-game overlay (for capturing thumbnails).
    pub hide_ui: bool,
    /// Open the platform on this page (`home`, `discover`, `store`, ...).
    pub page: Option<String>,
    /// Open this project in the studio (a catalog game id makes a remix first).
    pub edit: Option<String>,
    /// Studio start-up helpers for screenshots: select an object by path,
    /// open a script, start playing.
    pub select: Option<String>,
    pub open_script: Option<String>,
    pub studio_play: bool,
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
            "--play" => options.play = args.next(),
            "--page" => options.page = args.next(),
            "--hide-ui" => options.hide_ui = true,
            "--edit" => options.edit = args.next(),
            "--select" => options.select = args.next(),
            "--open-script" => options.open_script = args.next(),
            "--studio-play" => options.studio_play = true,
            _ => {}
        }
    }
    options
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

fn main() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin { file_path: asset_root(), ..default() }).set(WindowPlugin {
            primary_window: Some(Window {
                title: "Araria".into(),
                resolution: (1440, 900).into(),
                canvas: Some("#araria-canvas".into()),
                fit_canvas_to_parent: true,
                prevent_default_event_handling: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        // The interface gets its own full-window camera, so the 3D camera
        // can be confined to the studio's viewport.
        .insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() })
        .add_systems(Startup, spawn_interface_camera)
        .add_plugins(araria_engine::EnginePlugin)
        .insert_resource(parse_options())
        .add_systems(Startup, session::setup)
        .add_systems(Update, capture_screenshot)
        .add_systems(EguiPrimaryContextPass, session::interface)
        .run();
}

/// Draws the egui interface over the whole window, on top of the 3D view.
fn spawn_interface_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            output_mode: CameraOutputMode::Write { blend_state: Some(BlendState::ALPHA_BLENDING), clear_color: ClearColorConfig::None },
            ..default()
        },
        Msaa::Off,
        PrimaryEguiContext,
    ));
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
