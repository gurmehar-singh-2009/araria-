//! The game camera: orbits the local player's character while playing and
//! flies freely while editing.

use araria_runtime::Mode;
use bevy::camera::Viewport;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::input::{GameViewport, UiCapture};
use crate::{EngineSet, Game};

/// Marks the camera that renders the 3D world.
#[derive(Component)]
pub struct GameCamera;

/// Orbit (play) and fly (edit) camera state.
#[derive(Resource)]
pub struct CameraRig {
    /// Yaw of the viewing direction in degrees (0 looks along +Z).
    pub yaw: f32,
    /// Degrees below the horizon the camera looks.
    pub pitch: f32,
    pub distance: f32,
    /// Smoothed point the orbit camera looks at.
    pub focus: Vec3,
    /// Position of the free-fly camera used while editing.
    pub fly_position: Vec3,
    pub sensitivity: f32,
}

impl Default for CameraRig {
    fn default() -> Self {
        CameraRig { yaw: 180.0, pitch: 18.0, distance: 9.0, focus: Vec3::ZERO, fly_position: Vec3::new(0.0, 8.0, 18.0), sensitivity: 0.22 }
    }
}

impl CameraRig {
    /// Unit vector the camera looks along.
    pub fn direction(&self) -> Vec3 {
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        Vec3::new(yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos())
    }
}

/// User-controlled rendering options (from the settings screen).
#[derive(Resource, Clone, PartialEq)]
pub struct RenderSettings {
    pub shadows: bool,
    pub bloom: bool,
    pub antialiasing: bool,
    /// Multiplier on each game's fog distances.
    pub view_distance: f32,
    /// The user's preferred field of view in degrees; games' own values
    /// are scaled relative to the 70° default.
    pub field_of_view: f32,
    pub invert_y: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings { shadows: true, bloom: true, antialiasing: true, view_distance: 1.0, field_of_view: 70.0, invert_y: false }
    }
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraRig>()
            .init_resource::<RenderSettings>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (update_camera, apply_viewport, apply_render_settings).in_set(EngineSet::Present));
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        GameCamera,
        Camera3d::default(),
        Tonemapping::TonyMcMapface,
        Bloom::NATURAL,
        Msaa::Sample4,
        DistanceFog { color: Color::srgb(0.75, 0.89, 1.0), falloff: FogFalloff::Linear { start: 80.0, end: 400.0 }, ..default() },
        Transform::from_xyz(0.0, 8.0, 18.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[allow(clippy::too_many_arguments)]
fn update_camera(
    game: NonSend<Game>,
    mut rig: ResMut<CameraRig>,
    capture: Res<UiCapture>,
    render: Res<RenderSettings>,
    time: Res<Time>,
    mut camera: Single<(&mut Transform, &mut Projection), With<GameCamera>>,
) {
    let Some(runtime) = game.runtime.as_ref() else { return };
    let dt = time.delta_secs();
    let (looking, wheel, mouse_delta, axis, up_down, fast) = {
        let input = runtime.input.borrow();
        (
            input.mouse_buttons[1],
            input.wheel,
            input.mouse_delta,
            input.move_axis,
            f32::from(i8::from(input.key("E")) - i8::from(input.key("Q"))),
            input.run,
        )
    };
    if looking {
        rig.yaw = (rig.yaw - mouse_delta.x * rig.sensitivity).rem_euclid(360.0);
        let vertical = if render.invert_y { -mouse_delta.y } else { mouse_delta.y };
        rig.pitch = (rig.pitch + vertical * rig.sensitivity).clamp(-85.0, 85.0);
    }
    let (transform, projection) = &mut *camera;

    let dm = runtime.dm.borrow();
    let settings = dm.get(dm.service("Camera"));
    if let (Some(settings), Projection::Perspective(perspective)) = (settings, &mut **projection) {
        perspective.fov = (settings.f32("FieldOfView") * render.field_of_view / 70.0).clamp(20.0, 120.0).to_radians();
    }

    let character = runtime.local_character().and_then(|c| dm.get(c));
    match (runtime.mode(), character) {
        (Mode::Play | Mode::Paused, Some(character)) => {
            let (min, max) = settings.map_or((0.0, 30.0), |s| (s.f32("MinDistance"), s.f32("MaxDistance")));
            if !capture.pointer {
                rig.distance = (rig.distance * (1.0 - wheel * 0.12)).clamp(min.max(0.0), max.max(0.5));
            }
            let head = character.vec3("Position") + Vec3::Y * 1.55 * character.f32("Scale");
            let target = head + settings.map_or(Vec3::ZERO, |s| s.vec3("Offset"));
            // Follow smoothly, but never lag far behind (respawns, teleports).
            let blend = 1.0 - (-18.0 * dt).exp();
            rig.focus = if rig.focus.distance(target) > 8.0 { target } else { rig.focus.lerp(target, blend) };

            let direction = rig.direction();
            let mut distance = rig.distance;
            // Pull the camera in front of anything between it and the character.
            if distance > 0.3 {
                let ignore = [character.id];
                if let Some(hit) = runtime.physics.borrow().raycast(rig.focus, -direction, distance + 0.3, &ignore) {
                    distance = (hit.distance - 0.3).max(0.15);
                }
            }
            let position = rig.focus - direction * distance;
            **transform = Transform::from_translation(position).looking_to(direction, Vec3::Y);
        }
        _ => {
            // Free-fly: hold the right mouse button and use the movement keys.
            if looking && !capture.keyboard {
                let direction = rig.direction();
                let right = Vec3::new(-direction.z, 0.0, direction.x).normalize_or_zero();
                let speed = if fast { 40.0 } else { 14.0 };
                let step = (direction * axis.y + right * axis.x + Vec3::Y * up_down) * speed * dt;
                rig.fly_position += step;
            }
            if !capture.pointer && wheel != 0.0 {
                let direction = rig.direction();
                rig.fly_position += direction * wheel * 2.5;
            }
            **transform = Transform::from_translation(rig.fly_position).looking_to(rig.direction(), Vec3::Y);
        }
    }
    drop(dm);
    runtime.input.borrow_mut().camera_yaw = rig.yaw;
}

/// Restricts 3D rendering to the area left free by the interface.
fn apply_viewport(
    viewport: Res<GameViewport>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<&mut Camera, With<GameCamera>>,
) {
    let target = viewport.0.and_then(|rect| {
        let scale = window.scale_factor();
        let min = (rect.min * scale).round().as_uvec2();
        let max = (rect.max * scale).round().as_uvec2().min(UVec2::new(window.physical_width(), window.physical_height()));
        (max.x > min.x + 1 && max.y > min.y + 1).then(|| Viewport { physical_position: min, physical_size: max - min, ..default() })
    });
    let unchanged = match (&camera.viewport, &target) {
        (None, None) => true,
        (Some(a), Some(b)) => a.physical_position == b.physical_position && a.physical_size == b.physical_size,
        _ => false,
    };
    if !unchanged {
        camera.viewport = target;
    }
}

/// Turns bloom and anti-aliasing on or off.
fn apply_render_settings(
    mut commands: Commands,
    settings: Res<RenderSettings>,
    mut camera: Single<(Entity, &mut Msaa, Has<Bloom>), With<GameCamera>>,
) {
    let (entity, msaa, has_bloom) = &mut *camera;
    let wanted = if settings.antialiasing { Msaa::Sample4 } else { Msaa::Off };
    if **msaa != wanted {
        **msaa = wanted;
    }
    if settings.bloom && !*has_bloom {
        commands.entity(*entity).insert(Bloom::NATURAL);
    } else if !settings.bloom && *has_bloom {
        commands.entity(*entity).remove::<Bloom>();
    }
}
