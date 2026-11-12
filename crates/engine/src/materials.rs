//! Shared meshes and a cache of materials for primitive parts.

use std::collections::HashMap;

use araria_core::geometry::wedge_points;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Unit-sized meshes scaled per part through its transform.
#[derive(Resource)]
pub struct ShapeMeshes {
    pub block: Handle<Mesh>,
    pub sphere: Handle<Mesh>,
    pub cylinder: Handle<Mesh>,
    pub cone: Handle<Mesh>,
    pub wedge: Handle<Mesh>,
    pub capsule: Handle<Mesh>,
}

impl ShapeMeshes {
    pub fn for_shape(&self, shape: &str) -> Handle<Mesh> {
        match shape {
            "Sphere" => self.sphere.clone(),
            "Cylinder" => self.cylinder.clone(),
            "Cone" => self.cone.clone(),
            "Wedge" => self.wedge.clone(),
            _ => self.block.clone(),
        }
    }
}

/// A flat-shaded wedge matching [`wedge_points`].
fn wedge_mesh() -> Mesh {
    let p = wedge_points(Vec3::ONE);
    // Faces as lists of corner indices, counter-clockwise seen from outside.
    let faces: [&[usize]; 5] = [
        &[0, 1, 3, 2], // bottom
        &[2, 3, 5, 4], // back
        &[0, 4, 5, 1], // slope
        &[0, 2, 4],    // left
        &[1, 5, 3],    // right
    ];
    let (mut positions, mut normals, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for face in faces {
        let base = positions.len() as u32;
        let normal = (p[face[1]] - p[face[0]]).cross(p[face[2]] - p[face[0]]).normalize();
        for (i, &corner) in face.iter().enumerate() {
            positions.push(p[corner].to_array());
            normals.push(normal.to_array());
            uvs.push([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][i]);
        }
        for i in 1..face.len() as u32 - 1 {
            indices.extend([base, base + i, base + i + 1]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MaterialKey {
    rgb: [u8; 3],
    material: String,
    /// Transparency in 1/100ths.
    transparency: u8,
}

/// Materials are shared between all parts with the same look.
#[derive(Resource, Default)]
pub struct PartMaterials {
    cache: HashMap<MaterialKey, Handle<StandardMaterial>>,
}

impl PartMaterials {
    pub fn get(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        color: araria_core::Color,
        material: &str,
        transparency: f32,
    ) -> Handle<StandardMaterial> {
        let key = MaterialKey {
            rgb: color.to_rgb8(),
            material: material.to_string(),
            transparency: (transparency.clamp(0.0, 1.0) * 100.0).round() as u8,
        };
        self.cache.entry(key).or_insert_with(|| materials.add(build(color, material, transparency))).clone()
    }
}

fn build(color: araria_core::Color, material: &str, transparency: f32) -> StandardMaterial {
    let (roughness, metallic, reflectance): (f32, f32, f32) = match material {
        "Smooth" => (0.25, 0.0, 0.5),
        "Wood" => (0.8, 0.0, 0.3),
        "Metal" => (0.32, 0.9, 0.6),
        "Glass" => (0.06, 0.0, 0.7),
        "Neon" => (0.5, 0.0, 0.5),
        "Grass" => (0.95, 0.0, 0.2),
        "Sand" => (0.95, 0.0, 0.25),
        "Rock" => (0.9, 0.0, 0.3),
        "Ice" => (0.1, 0.0, 0.7),
        "Fabric" => (1.0, 0.0, 0.15),
        "Brick" => (0.85, 0.0, 0.3),
        "Concrete" => (0.88, 0.0, 0.3),
        "Water" => (0.05, 0.0, 0.6),
        _ => (0.6, 0.0, 0.4),
    };
    // Some materials are see-through even at Transparency 0.
    let base_alpha = match material {
        "Glass" => 0.35,
        "Water" => 0.6,
        "Ice" => 0.85,
        _ => 1.0,
    };
    let alpha = base_alpha * (1.0 - transparency.clamp(0.0, 1.0));
    let base = Color::srgba(color.r, color.g, color.b, alpha);
    let mut out = StandardMaterial {
        base_color: base,
        perceptual_roughness: roughness,
        metallic,
        reflectance,
        ..default()
    };
    if material == "Neon" {
        let glow = LinearRgba::from(Color::srgb(color.r, color.g, color.b)) * 5.0;
        out.emissive = glow;
    }
    if alpha < 0.999 {
        out.alpha_mode = AlphaMode::Blend;
    }
    out
}

pub struct MaterialsPlugin;

impl Plugin for MaterialsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartMaterials>().add_systems(PreStartup, setup_meshes);
    }
}

fn setup_meshes(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(ShapeMeshes {
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        sphere: meshes.add(Sphere::new(0.5).mesh().ico(5).expect("valid subdivision count")),
        cylinder: meshes.add(Cylinder::new(0.5, 1.0).mesh().resolution(32)),
        cone: meshes.add(Cone::new(0.5, 1.0).mesh().resolution(32)),
        wedge: meshes.add(wedge_mesh()),
        capsule: meshes.add(Capsule3d::new(0.5, 1.0)),
    });
}
