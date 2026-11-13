//! `ParticleEmitter` instances: a small CPU particle system.
//!
//! Every emitter owns one mesh of camera-facing quads that is rebuilt each
//! frame, so thousands of particles cost a handful of draw calls and no
//! per-particle entities.

use std::collections::HashMap;

use araria_core::{Change, Instance, InstanceId};
use araria_runtime::{Command, Mode};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::Image;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::camera::GameCamera;
use crate::{EngineSet, Game, PresentationCommands};

/// Upper bound per emitter, so a runaway script cannot stall rendering.
const MAX_PARTICLES: usize = 3000;
/// Brightness multiplier that lets glowing particles bloom.
const GLOW_BOOST: f32 = 2.6;

struct Particle {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    /// Per-particle size variation.
    scale: f32,
}

#[derive(Component)]
struct Emitter {
    instance: InstanceId,
    particles: Vec<Particle>,
    /// Fractional particles carried over between frames.
    carry: f32,
    mesh: Handle<Mesh>,
    glow: bool,
    seed: u32,
}

#[derive(Resource)]
struct ParticleAssets {
    glow: Handle<StandardMaterial>,
    soft: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
struct Emitters {
    map: HashMap<InstanceId, Entity>,
    scene: (u64, u64),
}

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Emitters>()
            .add_systems(Startup, setup)
            .add_systems(Update, (sync_emitters, simulate).chain().in_set(EngineSet::Present));
    }
}

/// A soft round dot: white with a radial alpha falloff.
fn dot_texture() -> Image {
    const SIZE: usize = 48;
    let mut data = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = ((x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0, (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0);
            let falloff = (1.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            let alpha = falloff * falloff * (3.0 - 2.0 * falloff);
            data.extend([255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d { width: SIZE as u32, height: SIZE as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let dot = images.add(dot_texture());
    let base = StandardMaterial { base_color_texture: Some(dot), unlit: true, cull_mode: None, double_sided: true, ..default() };
    commands.insert_resource(ParticleAssets {
        glow: materials.add(StandardMaterial { alpha_mode: AlphaMode::Add, fog_enabled: false, ..base.clone() }),
        soft: materials.add(StandardMaterial { alpha_mode: AlphaMode::Blend, ..base }),
    });
}

/// Builds the quad mesh. The renderer rejects meshes without vertices, so
/// an idle emitter gets a single invisible, zero-sized triangle.
fn quad_mesh(mut positions: Vec<[f32; 3]>, mut uvs: Vec<[f32; 2]>, mut colors: Vec<[f32; 4]>, mut indices: Vec<u32>) -> Mesh {
    if positions.is_empty() {
        positions = vec![[0.0; 3]; 3];
        uvs = vec![[0.0; 2]; 3];
        colors = vec![[0.0; 4]; 3];
        indices = vec![0, 1, 2];
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices))
}

/// Creates and removes emitter entities as instances come and go.
fn sync_emitters(
    mut commands: Commands,
    game: NonSend<Game>,
    mut emitters: ResMut<Emitters>,
    assets: Res<ParticleAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let scene = game.scene_id();
    let rebuild = emitters.scene != scene;
    if rebuild {
        for (_, entity) in emitters.map.drain() {
            commands.entity(entity).despawn();
        }
        emitters.scene = scene;
    }
    let Some(runtime) = game.runtime.as_ref() else { return };
    let dm = runtime.dm.borrow();
    let is_emitter = |i: &Instance| i.class_name() == "ParticleEmitter";
    let mut wanted: Vec<InstanceId> = Vec::new();
    if rebuild {
        wanted.extend(dm.descendants(dm.workspace()).into_iter().filter(|id| dm.get(*id).is_some_and(is_emitter)));
    }
    for change in dm.journal() {
        match change {
            Change::Added(id) | Change::Reparented(id) => {
                wanted.push(*id);
                wanted.extend(dm.descendants(*id));
            }
            Change::Removed(id) => {
                if let Some(entity) = emitters.map.remove(id) {
                    commands.entity(entity).despawn();
                }
            }
            _ => {}
        }
    }
    for id in wanted {
        let present = dm.get(id).is_some_and(is_emitter) && dm.in_workspace(id);
        match (present, emitters.map.contains_key(&id)) {
            (true, false) => {
                let mesh = meshes.add(quad_mesh(Vec::new(), Vec::new(), Vec::new(), Vec::new()));
                let glow = dm.get(id).is_some_and(|e| e.bool("Glow"));
                let material = if glow { assets.glow.clone() } else { assets.soft.clone() };
                let emitter = Emitter { instance: id, particles: Vec::new(), carry: 0.0, mesh: mesh.clone(), glow, seed: id.0 as u32 ^ 0x9e37_79b9 };
                let entity = commands.spawn((emitter, Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY, NoFrustumCulling, NotShadowCaster)).id();
                emitters.map.insert(id, entity);
            }
            (false, true) => {
                if let Some(entity) = emitters.map.remove(&id) {
                    commands.entity(entity).despawn();
                }
            }
            _ => {}
        }
    }
}

/// A fast deterministic generator: particles need variety, not quality.
fn random(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed >> 8) as f32 / 16_777_216.0
}

/// A unit vector within `spread` degrees of `direction`.
fn scatter(direction: Vec3, spread: f32, seed: &mut u32) -> Vec3 {
    let axis = direction.normalize_or(Vec3::Y);
    let cos_limit = spread.to_radians().cos();
    let cos = 1.0 - random(seed) * (1.0 - cos_limit);
    let sin = (1.0 - cos * cos).max(0.0).sqrt();
    let turn = random(seed) * std::f32::consts::TAU;
    let side = axis.any_orthonormal_vector();
    let other = axis.cross(side);
    axis * cos + (side * turn.cos() + other * turn.sin()) * sin
}

#[allow(clippy::too_many_arguments)]
fn simulate(
    game: NonSend<Game>,
    time: Res<Time>,
    queue: Res<PresentationCommands>,
    assets: Res<ParticleAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&GlobalTransform, With<GameCamera>>,
    mut emitters: Query<(&mut Emitter, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(runtime) = game.runtime.as_ref() else { return };
    let dm = runtime.dm.borrow();
    let dt = if runtime.mode() == Mode::Paused { 0.0 } else { time.delta_secs().min(0.1) };
    let (right, up) = camera.single().map_or((Vec3::X, Vec3::Y), |t| (*t.right(), *t.up()));

    for (mut emitter, mut material) in &mut emitters {
        let Some(instance) = dm.get(emitter.instance) else { continue };
        let origin = dm.world_position(emitter.instance).unwrap_or(Vec3::ZERO);
        let glow = instance.bool("Glow");
        if glow != emitter.glow {
            emitter.glow = glow;
            material.0 = if glow { assets.glow.clone() } else { assets.soft.clone() };
        }
        let (speed, direction, spread) = (instance.f32("Speed"), instance.vec3("Direction"), instance.f32("Spread"));
        let (life, area, acceleration) = (instance.f32("Lifetime"), instance.vec3("Area"), instance.vec3("Acceleration"));

        // Continuous emission plus any bursts requested this frame.
        let mut births = 0usize;
        if instance.bool("Enabled") {
            emitter.carry += instance.f32("Rate") * dt;
            births = emitter.carry as usize;
            emitter.carry -= births as f32;
        }
        for command in &queue.0 {
            if let Command::EmitParticles { emitter: target, count } = command {
                if *target == emitter.instance {
                    births += *count as usize;
                }
            }
        }
        let emitter = &mut *emitter;
        for _ in 0..births.min(MAX_PARTICLES.saturating_sub(emitter.particles.len())) {
            let seed = &mut emitter.seed;
            let offset = Vec3::new(random(seed) - 0.5, random(seed) - 0.5, random(seed) - 0.5) * area;
            let velocity = scatter(direction, spread, seed) * speed * (0.7 + 0.6 * random(seed));
            let particle = Particle { position: origin + offset, velocity, age: 0.0, life: life * (0.75 + 0.5 * random(seed)), scale: 0.8 + 0.4 * random(seed) };
            emitter.particles.push(particle);
        }
        for particle in &mut emitter.particles {
            particle.age += dt;
            particle.velocity += acceleration * dt;
            particle.position += particle.velocity * dt;
        }
        emitter.particles.retain(|p| p.age < p.life);

        // Rebuild the quads.
        let (size_start, size_end) = (instance.f32("Size"), instance.f32("SizeEnd"));
        let (color_start, color_end, opacity) = (instance.color("Color"), instance.color("ColorEnd"), instance.f32("Opacity"));
        let boost = if glow { GLOW_BOOST } else { 1.0 };
        let count = emitter.particles.len();
        let (mut positions, mut uvs, mut colors, mut indices) = (Vec::with_capacity(count * 4), Vec::with_capacity(count * 4), Vec::with_capacity(count * 4), Vec::with_capacity(count * 6));
        for (i, particle) in emitter.particles.iter().enumerate() {
            let t = (particle.age / particle.life).clamp(0.0, 1.0);
            let half = (size_start + (size_end - size_start) * t) * particle.scale * 0.5;
            let tint = LinearRgba::from(crate::color(color_start.lerp(color_end, t)));
            // Fade in quickly, fade out over the second half of the life.
            let alpha = opacity * (t * 12.0).min(1.0) * (1.0 - ((t - 0.5) * 2.0).clamp(0.0, 1.0));
            let rgba = [tint.red * boost, tint.green * boost, tint.blue * boost, alpha];
            let (r, u) = (right * half, up * half);
            for (corner, uv) in [(-r - u, [0.0, 1.0]), (r - u, [1.0, 1.0]), (r + u, [1.0, 0.0]), (-r + u, [0.0, 0.0])] {
                positions.push((particle.position + corner).to_array());
                uvs.push(uv);
                colors.push(rgba);
            }
            let base = i as u32 * 4;
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        if let Err(e) = meshes.insert(emitter.mesh.id(), quad_mesh(positions, uvs, colors, indices)) {
            warn!("could not update particle mesh: {e}");
        }
    }
}
