//! The game camera: orbits the local player's character while playing and
//! flies freely while editing.

use araria_runtime::Mode;
use bevy::camera::Viewport;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use araria_core::{InstanceId, Variant};
use araria_runtime::Command;

use crate::input::{GameViewport, UiCapture};
use crate::scene_sync::EntityMap;
use crate::{EngineSet, Game, PresentationCommands};

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
    /// Remaining camera shake: (intensity, seconds left, total seconds).
    shake: (f32, f32, f32),
    /// The scene the rig was last reset for.
    scene: (u64, u64),
    /// The character hidden because the camera is inside it.
    hidden: Option<InstanceId>,
}

impl Default for CameraRig {
    fn default() -> Self {
        CameraRig { yaw: 180.0, pitch: 18.0, distance: 9.0, focus: Vec3::ZERO, fly_position: Vec3::new(0.0, 8.0, 18.0), sensitivity: 0.22, shake: (0.0, 0.0, 0.0), scene: (0, 0), hidden: None }
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

/// A smooth pseudo-random offset for camera shake.
fn shake_offset(time: f32, strength: f32) -> Vec3 {
    Vec3::new((time * 47.0).sin() + (time * 91.0).cos() * 0.5, (time * 53.0).cos() + (time * 83.0).sin() * 0.5, (time * 61.0).sin() * 0.6) * strength
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn update_camera(
    game: NonSend<Game>,
    mut rig: ResMut<CameraRig>,
    capture: Res<UiCapture>,
    render: Res<RenderSettings>,
    queue: Res<PresentationCommands>,
    map: Res<EntityMap>,
    time: Res<Time>,
    mut camera: Single<(&mut Transform, &mut Projection), With<GameCamera>>,
    mut visibility: Query<&mut Visibility>,
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
    for command in &queue.0 {
        if let Command::CameraShake { intensity, duration } = command {
            rig.shake = (intensity.clamp(0.0, 10.0), duration.clamp(0.0, 10.0), duration.clamp(0.01, 10.0));
        }
    }
    let (transform, projection) = &mut *camera;

    let local_character = runtime.local_character();
    let mut dm = runtime.dm.borrow_mut();
    let camera_id = dm.service("Camera");
    let settings = dm.get(camera_id);
    if let (Some(settings), Projection::Perspective(perspective)) = (settings, &mut **projection) {
        perspective.fov = (settings.f32("FieldOfView") * render.field_of_view / 70.0).clamp(20.0, 120.0).to_radians();
    }
    // A new scene, or a script changing `Camera.Distance`, sets the zoom.
    let distance_index = settings.and_then(|s| s.info().prop_index("Distance"));
    let distance_written = dm.journal().iter().any(|c| matches!(c, araria_core::Change::Prop(id, i) if *id == camera_id && Some(*i as usize) == distance_index));
    if rig.scene != game.scene_id() || distance_written {
        rig.scene = game.scene_id();
        rig.distance = settings.map_or(9.0, |s| s.f32("Distance"));
    }

    let playing = matches!(runtime.mode(), Mode::Play | Mode::Paused);
    let mode = settings.map_or("Follow", |s| s.str("Mode"));
    let subject = settings.and_then(|s| s.reference("Subject")).or(local_character).and_then(|id| dm.get(id));
    let mut hide: Option<InstanceId> = None;
    let mut publish = true;
    match (playing, mode, subject) {
        (true, "Scriptable", _) => {
            let (position, rotation) = settings.map_or((Vec3::ZERO, Quat::IDENTITY), |s| (s.vec3("Position"), s.quat("Rotation")));
            **transform = Transform::from_translation(position).with_rotation(rotation);
            publish = false;
        }
        (true, "Fixed", subject) => {
            let position = settings.map_or(Vec3::new(0.0, 6.0, 12.0), |s| s.vec3("Position"));
            **transform = match subject.and_then(|s| dm.world_position(s.id)) {
                Some(target) if target.distance(position) > 0.01 => Transform::from_translation(position).looking_at(target + Vec3::Y, Vec3::Y),
                _ => Transform::from_translation(position).with_rotation(settings.map_or(Quat::IDENTITY, |s| s.quat("Rotation"))),
            };
            publish = false;
        }
        (true, _, Some(subject)) => {
            let first_person = mode == "FirstPerson";
            let (min, max) = settings.map_or((0.0, 30.0), |s| (s.f32("MinDistance"), s.f32("MaxDistance")));
            if !capture.pointer && !first_person {
                rig.distance = (rig.distance.max(0.4) * (1.0 - wheel * 0.12)).clamp(min.max(0.0), max.max(0.5));
            }
            let is_character = subject.is_a("Character");
            let height = if is_character { 1.55 * subject.f32("Scale") } else { 0.0 };
            let base = dm.world_position(subject.id).unwrap_or(Vec3::ZERO);
            let target = base + Vec3::Y * height + settings.map_or(Vec3::ZERO, |s| s.vec3("Offset"));
            // Follow smoothly, but never lag far behind (respawns, teleports).
            let blend = 1.0 - (-18.0 * dt).exp();
            rig.focus = if first_person || rig.focus.distance(target) > 8.0 { target } else { rig.focus.lerp(target, blend) };

            let direction = rig.direction();
            let mut distance = if first_person { 0.0 } else { rig.distance };
            // Pull the camera in front of anything between it and the subject.
            if distance > 0.3 {
                let ignore = [subject.id];
                if let Some(hit) = runtime.physics.borrow().raycast(rig.focus, -direction, distance + 0.3, &ignore) {
                    distance = (hit.distance - 0.3).max(0.15);
                }
            }
            if is_character && distance < 0.9 {
                hide = Some(subject.id);
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
            publish = false;
        }
    }
    if publish {
        // Scripts can read where the camera ended up.
        dm.set_quiet(camera_id, "Position", Variant::Vec3(transform.translation));
        dm.set_quiet(camera_id, "Rotation", Variant::Quat(transform.rotation));
    }
    drop(dm);

    if rig.shake.1 > 0.0 {
        let (intensity, left, total) = rig.shake;
        let strength = intensity * 0.12 * (left / total);
        let offset = transform.rotation * shake_offset(time.elapsed_secs(), strength);
        transform.translation += offset;
        rig.shake.1 = (left - dt).max(0.0);
    }
    // First person: the player should not see the inside of their own head.
    if rig.hidden != hide {
        for (id, wanted) in [(rig.hidden, Visibility::Inherited), (hide, Visibility::Hidden)] {
            if let Some(mut current) = id.and_then(|id| map.get(id)).and_then(|e| visibility.get_mut(e).ok()) {
                *current = wanted;
            }
        }
        // Keep trying until the character's entity exists.
        if hide.is_none() || hide.and_then(|id| map.get(id)).is_some() {
            rig.hidden = hide;
        }
    }
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
