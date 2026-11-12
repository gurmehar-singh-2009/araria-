//! The asset manifest: metadata describing models, audio and textures.
//!
//! The Blender export pipeline (`tools/blender`) writes this file next to
//! the exported `.glb` models. It lets the engine build colliders, list
//! animations and populate the asset browser without parsing the binary
//! assets up front.

use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const ASSET_MANIFEST_FILE: &str = "manifest.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColliderShape {
    Box,
    Sphere,
    /// Upright cylinder: `size.x` is the diameter, `size.y` the height.
    Cylinder,
    /// Upright capsule: `size.x` is the diameter, `size.y` the total height.
    Capsule,
}

/// One primitive of a model's collision shape, in model space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColliderDef {
    pub shape: ColliderShape,
    pub center: [f32; 3],
    pub size: [f32; 3],
    /// Rotation in degrees (X, Y, Z), applied yaw-pitch-roll.
    #[serde(default)]
    pub rotation: [f32; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelAsset {
    /// Path of the `.glb` relative to the asset root.
    pub file: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    /// Bounding box size and center in model space.
    pub size: [f32; 3],
    #[serde(default)]
    pub center: [f32; 3],
    /// Collision primitives; empty means "use the bounding box".
    #[serde(default)]
    pub colliders: Vec<ColliderDef>,
    /// Names of animation clips contained in the file.
    #[serde(default)]
    pub animations: Vec<String>,
    #[serde(default)]
    pub triangles: u32,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl ModelAsset {
    pub fn size(&self) -> Vec3 {
        Vec3::from(self.size)
    }

    pub fn center(&self) -> Vec3 {
        Vec3::from(self.center)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioAsset {
    pub file: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
    /// Length in seconds.
    #[serde(default)]
    pub duration: f32,
    #[serde(default)]
    pub looping: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureAsset {
    pub file: String,
    pub name: String,
    #[serde(default)]
    pub category: String,
}

/// All assets of one pack (the built-in library or a project's imports),
/// keyed by asset id such as `models/tree_pine`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AssetManifest {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub models: BTreeMap<String, ModelAsset>,
    #[serde(default)]
    pub audio: BTreeMap<String, AudioAsset>,
    #[serde(default)]
    pub textures: BTreeMap<String, TextureAsset>,
}

impl AssetManifest {
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifest is always serializable")
    }

    /// Adds another pack's assets; entries in `other` win on id clashes.
    pub fn merge(&mut self, other: AssetManifest) {
        self.models.extend(other.models);
        self.audio.extend(other.audio);
        self.textures.extend(other.textures);
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty() && self.audio.is_empty() && self.textures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_with_defaults() {
        let text = r#"{
            "version": 1,
            "models": {"models/crate": {"file": "models/crate.glb", "name": "Crate", "size": [1, 1, 1],
                "colliders": [{"shape": "box", "center": [0, 0.5, 0], "size": [1, 1, 1]}]}},
            "audio": {"audio/coin": {"file": "audio/coin.ogg", "name": "Coin"}}
        }"#;
        let manifest = AssetManifest::parse(text).unwrap();
        let model = &manifest.models["models/crate"];
        assert_eq!(model.colliders[0].shape, ColliderShape::Box);
        assert_eq!(model.colliders[0].rotation, [0.0; 3]);
        assert_eq!(AssetManifest::parse(&manifest.to_json()).unwrap(), manifest);
    }
}
