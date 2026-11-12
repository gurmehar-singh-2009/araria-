//! Araria application entry point (native + WebAssembly).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};

fn main() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
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
        .add_systems(Startup, setup)
        .add_systems(EguiPrimaryContextPass, ui)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.9, 0.4, 0.3))),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));
    commands.spawn((
        DirectionalLight { shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-2.5, 4.5, 9.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn ui(mut contexts: EguiContexts) -> Result {
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("Araria").show(ctx, |ui| {
        ui.label("Toolchain check");
    });
    Ok(())
}
