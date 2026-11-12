//! The scene file format (`*.scene.json`), version 1.
//!
//! A scene is a JSON document describing the instance tree. Property values
//! are encoded according to their schema type and only values that differ
//! from the class default are written, which keeps files small and diffable.
//! See `docs/project-format.md` for the full specification.

use std::collections::HashMap;

use glam::{Quat, Vec2, Vec3};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::datamodel::DataModel;
use crate::schema::{PropDef, PropType, schema};
use crate::types::{Color, InstanceId, UDim2, Variant, euler_deg_to_quat, quat_to_euler_deg};

pub const SCENE_FORMAT: &str = "araria.scene";
pub const SCENE_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum SceneError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not an Araria scene file")]
    WrongFormat,
    #[error("scene version {0} is newer than this build supports ({SCENE_VERSION})")]
    TooNew(u64),
    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
}

/// Rounds to five decimals so files stay readable and diff cleanly.
fn num(v: f32) -> Value {
    let rounded = (f64::from(v) * 1e5).round() / 1e5;
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 { json!(rounded as i64) } else { json!(rounded) }
}

fn vec3_json(v: Vec3) -> Value {
    json!([num(v.x), num(v.y), num(v.z)])
}

fn color_json(c: Color) -> Value {
    // Hex is exact for 8-bit colors; anything else keeps full precision.
    if Color::from_hex(&c.to_hex()) == Some(c) { json!(c.to_hex()) } else { json!([c.r, c.g, c.b]) }
}

fn floats(value: &Value, n: usize) -> Option<Vec<f32>> {
    let array = value.as_array()?;
    if array.len() != n {
        return None;
    }
    array.iter().map(|v| v.as_f64().map(|f| f as f32)).collect()
}

fn color_from(value: &Value) -> Option<Color> {
    match value {
        Value::String(hex) => Color::from_hex(hex),
        other => floats(other, 3).map(|f| Color::rgb(f[0], f[1], f[2])),
    }
}

/// Encodes a property value according to its schema type.
pub fn encode_prop(value: &Variant, refs: &HashMap<InstanceId, u64>) -> Value {
    match value {
        Variant::Nil => Value::Null,
        Variant::Bool(b) => json!(b),
        Variant::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                json!(*n as i64)
            } else {
                json!(n)
            }
        }
        Variant::String(s) => json!(s),
        Variant::Vec2(v) => json!([num(v.x), num(v.y)]),
        Variant::Vec3(v) => vec3_json(*v),
        Variant::Quat(q) => vec3_json(quat_to_euler_deg(*q)),
        Variant::Color(c) => color_json(*c),
        Variant::UDim2(u) => json!([num(u.x_scale), num(u.x_offset), num(u.y_scale), num(u.y_offset)]),
        Variant::Ref(target) => target.and_then(|t| refs.get(&t)).map_or(Value::Null, |id| json!(id)),
    }
}

fn decode_prop(def: &PropDef, value: &Value) -> Option<Variant> {
    Some(match def.ty {
        PropType::Bool => Variant::Bool(value.as_bool()?),
        PropType::Number { .. } => Variant::Number(value.as_f64()?),
        PropType::String | PropType::Enum(_) | PropType::Asset(_) => Variant::String(value.as_str()?.to_string()),
        PropType::Vec2 => floats(value, 2).map(|f| Variant::Vec2(Vec2::new(f[0], f[1])))?,
        PropType::Vec3 => floats(value, 3).map(|f| Variant::Vec3(Vec3::new(f[0], f[1], f[2])))?,
        PropType::Rotation => floats(value, 3).map(|f| Variant::Quat(euler_deg_to_quat(Vec3::new(f[0], f[1], f[2]))))?,
        PropType::Color => Variant::Color(color_from(value)?),
        PropType::UDim2 => floats(value, 4).map(|f| Variant::UDim2(UDim2::new(f[0], f[1], f[2], f[3])))?,
        // References are resolved in a second pass once every node exists.
        PropType::Ref => return None,
    })
}

/// Attributes are untyped, so non-JSON-native values carry a type tag.
pub fn encode_attribute(value: &Variant, refs: &HashMap<InstanceId, u64>) -> Value {
    let tagged = |t: &str, v: Value| json!({ "$type": t, "value": v });
    match value {
        Variant::Nil | Variant::Bool(_) | Variant::Number(_) | Variant::String(_) => encode_prop(value, refs),
        Variant::Vec2(_) => tagged("Vector2", encode_prop(value, refs)),
        Variant::Vec3(_) => tagged("Vector3", encode_prop(value, refs)),
        Variant::Quat(_) => tagged("Rotation", encode_prop(value, refs)),
        Variant::Color(_) => tagged("Color3", encode_prop(value, refs)),
        Variant::UDim2(_) => tagged("UDim2", encode_prop(value, refs)),
        Variant::Ref(_) => tagged("Instance", encode_prop(value, refs)),
    }
}

pub fn decode_attribute(value: &Value, refs: &HashMap<u64, InstanceId>) -> Option<Variant> {
    Some(match value {
        Value::Null => Variant::Nil,
        Value::Bool(b) => Variant::Bool(*b),
        Value::Number(n) => Variant::Number(n.as_f64()?),
        Value::String(s) => Variant::String(s.clone()),
        Value::Object(map) => {
            let inner = map.get("value")?;
            match map.get("$type")?.as_str()? {
                "Vector2" => floats(inner, 2).map(|f| Variant::Vec2(Vec2::new(f[0], f[1])))?,
                "Vector3" => floats(inner, 3).map(|f| Variant::Vec3(Vec3::new(f[0], f[1], f[2])))?,
                "Rotation" => floats(inner, 3).map(|f| Variant::Quat(euler_deg_to_quat(Vec3::new(f[0], f[1], f[2]))))?,
                "Color3" => Variant::Color(color_from(inner)?),
                "UDim2" => floats(inner, 4).map(|f| Variant::UDim2(UDim2::new(f[0], f[1], f[2], f[3])))?,
                "Instance" => Variant::Ref(inner.as_u64().and_then(|id| refs.get(&id).copied())),
                _ => return None,
            }
        }
        Value::Array(_) => return None,
    })
}

/// Collects every instance that some reference property in `ids` points at.
fn referenced(dm: &DataModel, ids: &[InstanceId]) -> HashMap<InstanceId, u64> {
    let mut refs = HashMap::new();
    let mut next = 1u64;
    let mut note = |target: InstanceId, refs: &mut HashMap<InstanceId, u64>| {
        refs.entry(target).or_insert_with(|| {
            next += 1;
            next - 1
        });
    };
    for instance in ids.iter().filter_map(|id| dm.get(*id)) {
        for value in instance.props().iter().chain(instance.attributes.values()) {
            if let Variant::Ref(Some(target)) = value {
                if ids.contains(target) {
                    note(*target, &mut refs);
                }
            }
        }
    }
    refs
}

fn encode_node(dm: &DataModel, id: InstanceId, refs: &HashMap<InstanceId, u64>, include_transient: bool) -> Value {
    let instance = dm.get(id).expect("node exists");
    let info = instance.info();
    let mut node = Map::new();
    node.insert("class".into(), json!(info.name()));
    if instance.name != info.name() {
        node.insert("name".into(), json!(instance.name));
    }
    if let Some(file_id) = refs.get(&id) {
        node.insert("id".into(), json!(file_id));
    }
    let mut props = Map::new();
    for (def, value) in info.props.iter().zip(instance.props()) {
        if (def.transient && !include_transient) || *value == def.default {
            continue;
        }
        // References to instances outside the serialized set are dropped.
        if let Variant::Ref(Some(target)) = value {
            if !refs.contains_key(target) {
                continue;
            }
        }
        props.insert(def.name.into(), encode_prop(value, refs));
    }
    if !props.is_empty() {
        node.insert("props".into(), Value::Object(props));
    }
    if !instance.attributes.is_empty() {
        let attrs: Map<String, Value> =
            instance.attributes.iter().map(|(k, v)| (k.clone(), encode_attribute(v, refs))).collect();
        node.insert("attrs".into(), Value::Object(attrs));
    }
    if !instance.tags.is_empty() {
        node.insert("tags".into(), json!(instance.tags));
    }
    let children: Vec<Value> = instance
        .children
        .iter()
        .filter(|c| dm.get(**c).is_some_and(|i| !i.is_a("Player")))
        .map(|c| encode_node(dm, *c, refs, include_transient))
        .collect();
    if !children.is_empty() {
        node.insert("children".into(), Value::Array(children));
    }
    Value::Object(node)
}

/// Serializes a whole data model to a scene document.
pub fn save_scene(dm: &DataModel) -> Value {
    let mut all = dm.descendants(dm.root());
    all.insert(0, dm.root());
    let refs = referenced(dm, &all);
    let nodes: Vec<Value> = dm.children(dm.root()).iter().map(|c| encode_node(dm, *c, &refs, false)).collect();
    json!({ "format": SCENE_FORMAT, "version": SCENE_VERSION, "nodes": nodes })
}

pub fn save_scene_string(dm: &DataModel) -> String {
    serde_json::to_string_pretty(&save_scene(dm)).expect("scene values are always serializable")
}

/// Serializes one instance and its descendants (clipboard, templates,
/// replication). Transient runtime state is included when requested.
pub fn save_subtree(dm: &DataModel, id: InstanceId, include_transient: bool) -> Value {
    let mut all = dm.descendants(id);
    all.insert(0, id);
    let refs = referenced(dm, &all);
    encode_node(dm, id, &refs, include_transient)
}

struct Loader<'a> {
    dm: &'a mut DataModel,
    refs: HashMap<u64, InstanceId>,
    /// (instance, property, file id) triples resolved after all nodes exist.
    pending_refs: Vec<(InstanceId, String, u64)>,
    pending_attrs: Vec<(InstanceId, String, Value)>,
    warnings: Vec<String>,
}

impl Loader<'_> {
    fn node(&mut self, node: &Value, parent: InstanceId, path: &str) -> Result<InstanceId, SceneError> {
        let invalid = |message: &str| SceneError::Invalid { path: path.to_string(), message: message.to_string() };
        let object = node.as_object().ok_or_else(|| invalid("expected an object"))?;
        let class = object.get("class").and_then(Value::as_str).ok_or_else(|| invalid("missing \"class\""))?;
        let info = schema().find(class).ok_or_else(|| invalid(&format!("unknown class '{class}'")))?;
        let name = object.get("name").and_then(Value::as_str).unwrap_or(class);
        let id = if info.def.service {
            self.dm.find_service(class).ok_or_else(|| invalid("unknown service"))?
        } else if class == "Game" || class == "Player" {
            return Err(invalid(&format!("'{class}' cannot appear in a scene")));
        } else {
            let id = self.dm.create_internal(class, name, Some(parent));
            id
        };
        let path = format!("{path}/{name}");
        if let Some(file_id) = object.get("id").and_then(Value::as_u64) {
            self.refs.insert(file_id, id);
        }
        if let Some(props) = object.get("props").and_then(Value::as_object) {
            for (key, raw) in props {
                let Some(def) = info.prop(key) else {
                    self.warnings.push(format!("{path}: unknown property '{key}' ignored"));
                    continue;
                };
                if def.ty == PropType::Ref {
                    if let Some(target) = raw.as_u64() {
                        self.pending_refs.push((id, key.clone(), target));
                    }
                    continue;
                }
                match decode_prop(def, raw) {
                    Some(value) => {
                        if let Err(e) = self.dm.set(id, key, value) {
                            self.warnings.push(format!("{path}: {e}"));
                        }
                    }
                    None => self.warnings.push(format!("{path}: property '{key}' has the wrong type and was ignored")),
                }
            }
        }
        if let Some(attrs) = object.get("attrs").and_then(Value::as_object) {
            for (key, raw) in attrs {
                self.pending_attrs.push((id, key.clone(), raw.clone()));
            }
        }
        if let Some(tags) = object.get("tags").and_then(Value::as_array) {
            for tag in tags.iter().filter_map(Value::as_str) {
                self.dm.add_tag(id, tag);
            }
        }
        if let Some(children) = object.get("children").and_then(Value::as_array) {
            for child in children {
                self.node(child, id, &path)?;
            }
        }
        Ok(id)
    }

    fn finish(mut self) -> Vec<String> {
        for (id, property, file_id) in std::mem::take(&mut self.pending_refs) {
            match self.refs.get(&file_id) {
                Some(target) => {
                    let _ = self.dm.set(id, &property, *target);
                }
                None => self.warnings.push(format!("{}: dangling reference in '{property}'", self.dm.path(id))),
            }
        }
        for (id, name, raw) in std::mem::take(&mut self.pending_attrs) {
            match decode_attribute(&raw, &self.refs) {
                Some(value) => {
                    let _ = self.dm.set_attribute(id, &name, value);
                }
                None => self.warnings.push(format!("{}: attribute '{name}' could not be read", self.dm.path(id))),
            }
        }
        self.warnings
    }
}

/// A loaded scene plus non-fatal problems found while reading it.
pub struct LoadedScene {
    pub model: DataModel,
    pub warnings: Vec<String>,
}

/// Builds a fresh data model from a scene document.
pub fn load_scene(document: &Value) -> Result<LoadedScene, SceneError> {
    if document.get("format").and_then(Value::as_str) != Some(SCENE_FORMAT) {
        return Err(SceneError::WrongFormat);
    }
    let version = document.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version > u64::from(SCENE_VERSION) {
        return Err(SceneError::TooNew(version));
    }
    let mut dm = DataModel::new();
    let root = dm.root();
    let mut loader =
        Loader { dm: &mut dm, refs: HashMap::new(), pending_refs: Vec::new(), pending_attrs: Vec::new(), warnings: Vec::new() };
    if let Some(nodes) = document.get("nodes").and_then(Value::as_array) {
        for node in nodes {
            loader.node(node, root, "")?;
        }
    }
    let warnings = loader.finish();
    dm.clear_journal();
    dm.take_events();
    Ok(LoadedScene { model: dm, warnings })
}

pub fn load_scene_str(text: &str) -> Result<LoadedScene, SceneError> {
    load_scene(&serde_json::from_str(text)?)
}

/// Instantiates a subtree produced by [`save_subtree`] under `parent`.
pub fn load_subtree(dm: &mut DataModel, node: &Value, parent: InstanceId) -> Result<InstanceId, SceneError> {
    let mut loader =
        Loader { dm, refs: HashMap::new(), pending_refs: Vec::new(), pending_attrs: Vec::new(), warnings: Vec::new() };
    let id = loader.node(node, parent, "")?;
    loader.finish();
    Ok(id)
}

/// A rotation helper for scene-building code and tests.
pub fn rotation_deg(x: f32, y: f32, z: f32) -> Quat {
    euler_deg_to_quat(Vec3::new(x, y, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DataModel {
        let mut dm = DataModel::new();
        let ws = dm.workspace();
        let model = dm.create("Model", Some(ws)).unwrap();
        dm.set_name(model, "Gate").unwrap();
        let part = dm.create("Part", Some(model)).unwrap();
        dm.set(part, "Position", Vec3::new(1.5, 2.0, -3.25)).unwrap();
        dm.set(part, "Rotation", Vec3::new(0.0, 45.0, 0.0)).unwrap();
        dm.set(part, "Color", Color::from_hex("#ff8800").unwrap()).unwrap();
        dm.set(part, "Shape", "Cylinder").unwrap();
        dm.set(part, "Velocity", Vec3::ONE).unwrap();
        dm.set(model, "PrimaryPart", part).unwrap();
        dm.add_tag(part, "door");
        dm.set_attribute(part, "Opens", Variant::Bool(true)).unwrap();
        dm.set_attribute(part, "Target", Variant::Vec3(Vec3::new(0.0, 5.0, 0.0))).unwrap();
        dm.set_attribute(part, "Owner", Variant::Ref(Some(model))).unwrap();
        dm.set(dm.service("Lighting"), "ClockTime", 18.5).unwrap();
        dm.set(dm.service("Camera"), "FieldOfView", 55.0).unwrap();
        let label = dm.create("TextLabel", Some(dm.service("Gui"))).unwrap();
        dm.set(label, "Size", UDim2::new(0.5, 0.0, 0.0, 40.0)).unwrap();
        dm
    }

    #[test]
    fn scene_round_trips() {
        let dm = sample();
        let text = save_scene_string(&dm);
        let loaded = load_scene_str(&text).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let copy = loaded.model;
        // Saving the loaded model reproduces the same document.
        assert_eq!(save_scene_string(&copy), text);

        let part = copy.find_by_path("Workspace.Gate.Part").unwrap();
        let instance = copy.get(part).unwrap();
        assert_eq!(instance.vec3("Position"), Vec3::new(1.5, 2.0, -3.25));
        assert_eq!(instance.str("Shape"), "Cylinder");
        assert!(instance.has_tag("door"));
        assert_eq!(instance.attributes["Opens"], Variant::Bool(true));
        assert_eq!(instance.attributes["Target"], Variant::Vec3(Vec3::new(0.0, 5.0, 0.0)));
        let gate = copy.find_by_path("Workspace.Gate").unwrap();
        assert_eq!(instance.attributes["Owner"], Variant::Ref(Some(gate)));
        assert_eq!(copy.get(gate).unwrap().reference("PrimaryPart"), Some(part));
        assert_eq!(instance.vec3("Velocity"), Vec3::ZERO, "transient state is not saved");
        assert_eq!(copy.get(copy.service("Lighting")).unwrap().number("ClockTime"), 18.5);
        assert_eq!(copy.get(copy.service("Camera")).unwrap().number("FieldOfView"), 55.0);
        assert_eq!(copy.len(), dm.len());
    }

    #[test]
    fn defaults_are_omitted() {
        let mut dm = DataModel::new();
        dm.create("Part", Some(dm.workspace())).unwrap();
        let text = save_scene_string(&dm);
        assert!(!text.contains("\"props\""), "{text}");
    }

    #[test]
    fn subtree_round_trips_into_another_model() {
        let dm = sample();
        let gate = dm.find_by_path("Workspace.Gate").unwrap();
        let node = save_subtree(&dm, gate, false);
        let mut other = DataModel::new();
        let storage = other.service("Storage");
        let copy = load_subtree(&mut other, &node, storage).unwrap();
        let part = other.children(copy)[0];
        assert_eq!(other.get(copy).unwrap().reference("PrimaryPart"), Some(part));
        assert_eq!(other.path(part), "Storage.Gate.Part");
    }

    #[test]
    fn bad_documents_are_rejected_or_reported() {
        assert!(matches!(load_scene_str("{}"), Err(SceneError::WrongFormat)));
        assert!(matches!(load_scene_str("not json"), Err(SceneError::Json(_))));
        let future = r#"{"format":"araria.scene","version":99,"nodes":[]}"#;
        assert!(matches!(load_scene_str(future), Err(SceneError::TooNew(99))));
        let unknown_class = r#"{"format":"araria.scene","version":1,"nodes":[{"class":"Workspace","children":[{"class":"Bogus"}]}]}"#;
        assert!(matches!(load_scene_str(unknown_class), Err(SceneError::Invalid { .. })));
        let sloppy = r#"{"format":"araria.scene","version":1,"nodes":[{"class":"Workspace","children":[
            {"class":"Part","props":{"Position":"oops","Mystery":1,"Transparency":0.5}}]}]}"#;
        let loaded = load_scene_str(sloppy).unwrap();
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
        let part = loaded.model.find_by_path("Workspace.Part").unwrap();
        assert_eq!(loaded.model.get(part).unwrap().number("Transparency"), 0.5);
    }
}
