//! Mirrors instances under `Workspace` into Bevy entities.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_core::geometry::{CHARACTER_HEIGHT, CHARACTER_RADIUS, effective_size};
use araria_core::{Change, DataModel, Instance, InstanceId};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::materials::{PartMaterials, ShapeMeshes};
use crate::{EngineSet, Game, color};

/// Links an entity to the instance it presents.
#[derive(Component, Clone, Copy)]
pub struct InstanceLink(pub InstanceId);

/// The asset library used to resolve model ids to files.
#[derive(Resource, Clone, Default)]
pub struct AssetLibrary(pub Arc<AssetManifest>);

#[derive(Resource, Default)]
pub struct EntityMap {
    map: HashMap<InstanceId, Entity>,
    generation: u64,
}

impl EntityMap {
    pub fn get(&self, id: InstanceId) -> Option<Entity> {
        self.map.get(&id).copied()
    }
}

pub struct SceneSyncPlugin;

impl Plugin for SceneSyncPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EntityMap>()
            .init_resource::<AssetLibrary>()
            .add_systems(Update, sync_scene.in_set(EngineSet::Present));
    }
}

fn is_presentable(instance: &Instance) -> bool {
    instance.is_a("BasePart") || instance.is_a("Character") || instance.is_a("Light")
}

/// Property changes that only need a transform update, not a respawn.
fn is_transform_prop(name: &str) -> bool {
    matches!(name, "Position" | "Rotation" | "Facing" | "Size" | "Scale")
}

/// Runtime-only state that never affects how an instance looks.
fn is_invisible_prop(name: &str) -> bool {
    matches!(
        name,
        "Velocity" | "AngularVelocity" | "Anchored" | "CanCollide" | "CanTouch" | "Locked" | "Density" | "Friction"
            | "Elasticity" | "State" | "Grounded" | "MoveDirection" | "Health" | "MaxHealth" | "WalkSpeed" | "RunSpeed"
            | "JumpPower" | "Seat" | "Emote" | "AutoRotate" | "Collision"
    )
}

fn transform_of(dm: &DataModel, instance: &Instance) -> Transform {
    if instance.is_a("Character") {
        return Transform::from_translation(instance.vec3("Position"))
            .with_rotation(Quat::from_rotation_y(instance.f32("Facing").to_radians()))
            .with_scale(Vec3::splat(instance.f32("Scale")));
    }
    if instance.is_a("MeshPart") {
        return Transform::from_translation(instance.vec3("Position"))
            .with_rotation(instance.quat("Rotation"))
            .with_scale(instance.vec3("Scale"));
    }
    if instance.is_a("BasePart") {
        let shape = if instance.is_a("SpawnPoint") { "Cylinder" } else { instance.str("Shape") };
        return Transform::from_translation(instance.vec3("Position"))
            .with_rotation(instance.quat("Rotation"))
            .with_scale(effective_size(shape, instance.vec3("Size")).max(Vec3::splat(0.001)));
    }
    // Attachments: lights and friends follow their parent part.
    let position = dm.world_position(instance.id).unwrap_or(Vec3::ZERO);
    let parent_rotation = instance
        .parent
        .and_then(|p| dm.get(p))
        .filter(|p| p.is_a("BasePart"))
        .map_or(Quat::IDENTITY, |p| p.quat("Rotation"));
    Transform::from_translation(position).with_rotation(parent_rotation * instance.quat("Rotation"))
}

struct Spawner<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    meshes: &'a ShapeMeshes,
    part_materials: &'a mut PartMaterials,
    materials: &'a mut Assets<StandardMaterial>,
    asset_server: &'a AssetServer,
    library: &'a AssetManifest,
}

impl Spawner<'_, '_, '_> {
    fn spawn(&mut self, dm: &DataModel, instance: &Instance) -> Entity {
        let transform = transform_of(dm, instance);
        let link = InstanceLink(instance.id);
        if instance.is_a("Character") {
            return self.character(instance, transform, link);
        }
        if instance.is_a("Light") {
            return self.light(instance, transform, link);
        }
        if instance.is_a("MeshPart") {
            let visibility = if instance.f32("Transparency") >= 0.999 { Visibility::Hidden } else { Visibility::Inherited };
            return match self.library.models.get(instance.str("Asset")) {
                Some(model) => {
                    let scene = self.asset_server.load(GltfAssetLabel::Scene(0).from_asset(model.file.clone()));
                    self.commands.spawn((WorldAssetRoot(scene), transform, visibility, link)).id()
                }
                None => {
                    // Unknown asset: a visible placeholder beats an invisible object.
                    let material =
                        self.part_materials.get(self.materials, araria_core::Color::rgb(1.0, 0.0, 0.8), "Plastic", 0.0);
                    self.commands.spawn((Mesh3d(self.meshes.block.clone()), MeshMaterial3d(material), transform, visibility, link)).id()
                }
            };
        }
        let (shape, material_name) = if instance.is_a("SpawnPoint") { ("Cylinder", "Neon") } else { (instance.str("Shape"), instance.str("Material")) };
        let transparency = instance.f32("Transparency");
        let material = self.part_materials.get(self.materials, instance.color("Color"), material_name, transparency);
        let visibility = if transparency >= 0.999 { Visibility::Hidden } else { Visibility::Inherited };
        let mut entity = self.commands.spawn((Mesh3d(self.meshes.for_shape(shape)), MeshMaterial3d(material), transform, visibility, link));
        if !instance.bool("CastShadow") || transparency > 0.5 {
            entity.insert(NotShadowCaster);
        }
        entity.id()
    }

    /// Temporary stand-in until the rigged character model is loaded.
    fn character(&mut self, instance: &Instance, transform: Transform, link: InstanceLink) -> Entity {
        let body = self.part_materials.get(self.materials, instance.color("ShirtColor"), "Plastic", 0.0);
        let skin = self.part_materials.get(self.materials, instance.color("SkinColor"), "Plastic", 0.0);
        let (meshes, radius, height) = (self.meshes, CHARACTER_RADIUS, CHARACTER_HEIGHT);
        self.commands
            .spawn((transform, Visibility::Inherited, link))
            .with_children(|parent| {
                parent.spawn((
                    Mesh3d(meshes.capsule.clone()),
                    MeshMaterial3d(body),
                    Transform::from_xyz(0.0, height * 0.5, 0.0).with_scale(Vec3::new(radius * 2.0, height * 0.5, radius * 2.0)),
                ));
                // A face plate so the facing direction (+Z) is visible.
                parent.spawn((
                    Mesh3d(meshes.block.clone()),
                    MeshMaterial3d(skin),
                    Transform::from_xyz(0.0, height * 0.8, radius * 0.75).with_scale(Vec3::new(0.4, 0.3, 0.3)),
                ));
            })
            .id()
    }

    fn light(&mut self, instance: &Instance, transform: Transform, link: InstanceLink) -> Entity {
        let enabled = instance.bool("Enabled");
        let intensity = if enabled { instance.f32("Brightness") * 60_000.0 } else { 0.0 };
        let (light_color, range, shadows) = (color(instance.color("Color")), instance.f32("Range"), instance.bool("Shadows"));
        if instance.str("Kind") == "Spot" {
            let angle = instance.f32("Angle").to_radians();
            // Spot lights shine along the instance's -Y axis by default (downward).
            let aim = transform.rotation * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            self.commands
                .spawn((
                    SpotLight { color: light_color, intensity, range, outer_angle: angle, inner_angle: angle * 0.7, shadow_maps_enabled: shadows, ..default() },
                    transform.with_rotation(aim),
                    link,
                ))
                .id()
        } else {
            self.commands
                .spawn((PointLight { color: light_color, intensity, range, shadow_maps_enabled: shadows, ..default() }, transform, link))
                .id()
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_scene(
    game: NonSend<Game>,
    mut commands: Commands,
    mut map: ResMut<EntityMap>,
    meshes: Res<ShapeMeshes>,
    mut part_materials: ResMut<PartMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    library: Res<AssetLibrary>,
    mut transforms: Query<&mut Transform, With<InstanceLink>>,
) {
    let rebuild_all = map.generation != game.generation;
    if rebuild_all {
        for (_, entity) in map.map.drain() {
            commands.entity(entity).despawn();
        }
        map.generation = game.generation;
    }
    let Some(runtime) = game.runtime.as_ref() else { return };
    let dm = runtime.dm.borrow();
    let mut spawner = Spawner {
        commands: &mut commands,
        meshes: &meshes,
        part_materials: &mut part_materials,
        materials: &mut materials,
        asset_server: &asset_server,
        library: &library.0,
    };

    if rebuild_all {
        for id in dm.descendants(dm.workspace()) {
            if let Some(instance) = dm.get(id).filter(|i| is_presentable(i)) {
                let entity = spawner.spawn(&dm, instance);
                map.map.insert(id, entity);
            }
        }
        return;
    }

    let mut respawn: HashSet<InstanceId> = HashSet::new();
    let mut moved: HashSet<InstanceId> = HashSet::new();
    for change in dm.journal() {
        match change {
            Change::Removed(id) => {
                if let Some(entity) = map.map.remove(id) {
                    spawner.commands.entity(entity).despawn();
                }
                respawn.remove(id);
                moved.remove(id);
            }
            Change::Added(id) => {
                respawn.insert(*id);
            }
            Change::Reparented(id) => {
                respawn.insert(*id);
                respawn.extend(dm.descendants(*id));
            }
            Change::Moved(id) => {
                moved.insert(*id);
            }
            Change::Prop(id, index) => {
                let Some(instance) = dm.get(*id) else { continue };
                let name = instance.info().props[*index as usize].name;
                if is_transform_prop(name) {
                    moved.insert(*id);
                } else if !is_invisible_prop(name) {
                    respawn.insert(*id);
                }
            }
            Change::Renamed(_) | Change::Attribute(_) => {}
        }
    }
    for id in &respawn {
        if let Some(entity) = map.map.remove(id) {
            spawner.commands.entity(entity).despawn();
        }
        if let Some(instance) = dm.get(*id).filter(|i| is_presentable(i) && dm.in_workspace(i.id)) {
            let entity = spawner.spawn(&dm, instance);
            map.map.insert(*id, entity);
        }
    }
    for id in moved.difference(&respawn) {
        let Some(instance) = dm.get(*id) else { continue };
        if let Some(mut transform) = map.map.get(id).and_then(|e| transforms.get_mut(*e).ok()) {
            *transform = transform_of(&dm, instance);
        }
        // Attachments ride along with their parent.
        for child in &instance.children {
            let Some(attachment) = dm.get(*child).filter(|c| !c.is_a("BasePart") && !c.is_a("Character")) else { continue };
            if let Some(mut transform) = map.map.get(child).and_then(|e| transforms.get_mut(*e).ok()) {
                let rotation = transform.rotation;
                *transform = transform_of(&dm, attachment);
                if attachment.str("Kind") == "Spot" {
                    transform.rotation = rotation;
                }
            }
        }
    }
}
