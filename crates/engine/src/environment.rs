//! Sun, ambient light, sky color and fog, driven by the `Lighting` service.

use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

use crate::camera::{GameCamera, RenderSettings};
use crate::{EngineSet, Game, color};

#[derive(Component)]
struct Sun;

pub struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_sun).add_systems(Update, apply_lighting.in_set(EngineSet::Present));
    }
}

fn spawn_sun(mut commands: Commands) {
    commands.spawn((
        Sun,
        DirectionalLight { shadow_maps_enabled: true, illuminance: 10_000.0, ..default() },
        CascadeShadowConfigBuilder { num_cascades: 3, maximum_distance: 120.0, first_cascade_far_bound: 12.0, ..default() }.build(),
        Transform::from_xyz(30.0, 60.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Direction sunlight travels at the given hour (sunrise 6, noon 12, sunset 18).
pub fn sun_direction(clock_time: f32) -> Vec3 {
    let angle = (clock_time - 6.0) / 12.0 * std::f32::consts::PI;
    // The sun arcs from +X over the top to -X, tilted a little toward +Z.
    let toward_sun = Vec3::new(angle.cos(), angle.sin(), 0.35).normalize();
    -toward_sun
}

fn apply_lighting(
    game: NonSend<Game>,
    render: Res<RenderSettings>,
    mut sun: Single<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    mut fog: Query<&mut DistanceFog, With<GameCamera>>,
) {
    let Some(runtime) = game.runtime.as_ref() else { return };
    let dm = runtime.dm.borrow();
    let Some(lighting) = dm.get(dm.service("Lighting")) else { return };

    let clock = lighting.f32("ClockTime");
    let direction = sun_direction(clock);
    let elevation = (-direction.y).clamp(0.0, 1.0);
    // Daylight fades smoothly around sunrise and sunset.
    let daylight = (elevation * 4.0).clamp(0.0, 1.0);
    let night = 1.0 - daylight;

    let (light, transform) = &mut *sun;
    light.illuminance = 11_000.0 * lighting.f32("Brightness") * daylight.max(0.02);
    light.color = color(lighting.color("SunColor").lerp(araria_core::Color::rgb(1.0, 0.55, 0.3), (1.0 - elevation).powi(4) * daylight));
    light.shadow_maps_enabled = render.shadows && lighting.bool("Shadows") && daylight > 0.05;
    // At night the light comes from the moon, opposite the sun.
    let light_dir = if direction.y < 0.0 { direction } else { -direction };
    **transform = Transform::IDENTITY.looking_to(light_dir, Vec3::Y);

    ambient.color = color(lighting.color("Ambient"));
    ambient.brightness = 420.0 * lighting.f32("AmbientBrightness") * (0.25 + 0.75 * daylight);

    let night_sky = araria_core::Color::rgb(0.02, 0.03, 0.07);
    let horizon = lighting.color("HorizonColor").lerp(night_sky, night * 0.92);
    clear.0 = color(horizon);
    for mut fog in &mut fog {
        fog.color = color(lighting.color("FogColor").lerp(night_sky, night * 0.92));
        let (start, end) = (lighting.f32("FogStart") * render.view_distance, lighting.f32("FogEnd") * render.view_distance);
        fog.falloff = FogFalloff::Linear { start, end: end.max(start + 1.0) };
    }
}
