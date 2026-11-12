//! The instance tree.
//!
//! [`DataModel`] is the single source of truth for a game: the renderer,
//! physics, scripts, the editor and networking all read and write it. Every
//! mutation is appended to a change journal so those systems can react
//! without polling, and engine-raised events (collisions, clicks, ...) are
//! queued here for the scripting layer to deliver.

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::{Quat, Vec2, Vec3};
use thiserror::Error;

use crate::schema::{ClassId, ClassInfo, schema};
use crate::types::{Color, InstanceId, UDim2, Variant};

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ModelError {
    #[error("unknown class '{0}'")]
    UnknownClass(String),
    #[error("'{0}' cannot be created")]
    NotCreatable(String),
    #[error("the instance no longer exists")]
    Missing,
    #[error("'{property}' is not a valid property of {class}")]
    UnknownProperty { class: &'static str, property: String },
    #[error("{0}")]
    InvalidValue(String),
    #[error("cannot parent {0} under its own descendant")]
    Cycle(String),
    #[error("{0} is a service and cannot be moved or destroyed")]
    Protected(String),
}

pub type ModelResult<T> = Result<T, ModelError>;

/// One node of the game tree.
#[derive(Clone, Debug)]
pub struct Instance {
    pub id: InstanceId,
    pub class: ClassId,
    pub name: String,
    pub parent: Option<InstanceId>,
    pub children: Vec<InstanceId>,
    props: Vec<Variant>,
    pub attributes: BTreeMap<String, Variant>,
    pub tags: Vec<String>,
    /// Id of the script that created the instance at runtime (0 = none).
    /// Hot reloading uses it to clean up after the old version of a script.
    pub creator: u32,
}

impl Instance {
    pub fn info(&self) -> &'static ClassInfo {
        schema().class(self.class)
    }

    pub fn class_name(&self) -> &'static str {
        self.info().name()
    }

    pub fn is_a(&self, class: &str) -> bool {
        schema().find(class).is_some_and(|c| self.info().is_a(c.id))
    }

    pub fn get(&self, property: &str) -> Option<&Variant> {
        self.info().prop_index(property).map(|i| &self.props[i])
    }

    pub fn prop_at(&self, index: usize) -> &Variant {
        &self.props[index]
    }

    pub fn props(&self) -> &[Variant] {
        &self.props
    }

    pub fn bool(&self, property: &str) -> bool {
        self.get(property).and_then(Variant::as_bool).unwrap_or(false)
    }

    pub fn number(&self, property: &str) -> f64 {
        self.get(property).and_then(Variant::as_f64).unwrap_or(0.0)
    }

    pub fn f32(&self, property: &str) -> f32 {
        self.number(property) as f32
    }

    pub fn str(&self, property: &str) -> &str {
        self.get(property).and_then(Variant::as_str).unwrap_or("")
    }

    pub fn vec3(&self, property: &str) -> Vec3 {
        self.get(property).and_then(Variant::as_vec3).unwrap_or(Vec3::ZERO)
    }

    pub fn vec2(&self, property: &str) -> Vec2 {
        self.get(property).and_then(Variant::as_vec2).unwrap_or(Vec2::ZERO)
    }

    pub fn quat(&self, property: &str) -> Quat {
        self.get(property).and_then(Variant::as_quat).unwrap_or(Quat::IDENTITY)
    }

    pub fn color(&self, property: &str) -> Color {
        self.get(property).and_then(Variant::as_color).unwrap_or(Color::WHITE)
    }

    pub fn udim2(&self, property: &str) -> UDim2 {
        self.get(property).and_then(Variant::as_udim2).unwrap_or_default()
    }

    pub fn reference(&self, property: &str) -> Option<InstanceId> {
        self.get(property).and_then(Variant::as_ref_id)
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }
}

/// A recorded mutation of the tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Added(InstanceId),
    Removed(InstanceId),
    Reparented(InstanceId),
    Renamed(InstanceId),
    /// A property was assigned; the payload is its index in the class.
    Prop(InstanceId, u16),
    /// Position/rotation were written by the physics simulation.
    Moved(InstanceId),
    Attribute(InstanceId),
}

impl Change {
    pub fn id(&self) -> InstanceId {
        match self {
            Change::Added(id)
            | Change::Removed(id)
            | Change::Reparented(id)
            | Change::Renamed(id)
            | Change::Prop(id, _)
            | Change::Moved(id)
            | Change::Attribute(id) => *id,
        }
    }
}

/// An event raised by the engine for scripts to observe.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub target: InstanceId,
    pub name: String,
    pub args: Vec<Variant>,
}

#[derive(Clone, Debug)]
pub struct DataModel {
    instances: HashMap<InstanceId, Instance>,
    root: InstanceId,
    next_id: u64,
    next_local_id: u64,
    services: HashMap<&'static str, InstanceId>,
    journal: Vec<Change>,
    events: Vec<Event>,
    /// Instances whose property changes should raise `Changed` events.
    watched: HashSet<InstanceId>,
}

impl Default for DataModel {
    fn default() -> Self {
        DataModel::new()
    }
}

impl DataModel {
    /// Creates a data model containing the root and every service.
    pub fn new() -> Self {
        let mut dm = DataModel {
            instances: HashMap::new(),
            root: InstanceId(1),
            next_id: 1,
            next_local_id: InstanceId::LOCAL_BASE,
            services: HashMap::new(),
            journal: Vec::new(),
            events: Vec::new(),
            watched: HashSet::new(),
        };
        let game = schema().id("Game");
        dm.root = dm.insert_raw(game, "Game", None);
        // Workspace first so the camera can be parented to it.
        let mut services: Vec<&ClassInfo> = schema().services().collect();
        services.sort_by_key(|c| (c.name() != "Workspace", c.name() == "Camera"));
        for class in services {
            let parent = if class.name() == "Camera" { dm.services["Workspace"] } else { dm.root };
            let id = dm.insert_raw(class.id, class.name(), Some(parent));
            dm.services.insert(class.name(), id);
        }
        dm.journal.clear();
        dm
    }

    fn insert_raw(&mut self, class: ClassId, name: &str, parent: Option<InstanceId>) -> InstanceId {
        let id = InstanceId(self.next_id);
        self.next_id += 1;
        self.insert_with_id(id, class, name, parent);
        id
    }

    fn insert_with_id(&mut self, id: InstanceId, class: ClassId, name: &str, parent: Option<InstanceId>) {
        let info = schema().class(class);
        let instance = Instance {
            id,
            class,
            name: name.to_string(),
            parent,
            children: Vec::new(),
            props: info.props.iter().map(|p| p.default.clone()).collect(),
            attributes: BTreeMap::new(),
            tags: Vec::new(),
            creator: 0,
        };
        self.instances.insert(id, instance);
        if let Some(parent) = parent.and_then(|p| self.instances.get_mut(&p)) {
            parent.children.push(id);
        }
        self.journal.push(Change::Added(id));
    }

    // ----------------------------------------------------------- queries

    pub fn root(&self) -> InstanceId {
        self.root
    }

    /// Id of a service such as `"Workspace"`. Panics for unknown names.
    pub fn service(&self, name: &str) -> InstanceId {
        *self.services.get(name).unwrap_or_else(|| panic!("unknown service {name}"))
    }

    pub fn find_service(&self, name: &str) -> Option<InstanceId> {
        self.services.get(name).copied()
    }

    pub fn workspace(&self) -> InstanceId {
        self.services["Workspace"]
    }

    pub fn get(&self, id: InstanceId) -> Option<&Instance> {
        self.instances.get(&id)
    }

    pub fn contains(&self, id: InstanceId) -> bool {
        self.instances.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.instances.values()
    }

    pub fn children(&self, id: InstanceId) -> &[InstanceId] {
        self.get(id).map_or(&[], |i| i.children.as_slice())
    }

    /// Every instance below `id`, parents before children.
    pub fn descendants(&self, id: InstanceId) -> Vec<InstanceId> {
        let mut out = Vec::new();
        let mut stack: Vec<InstanceId> = self.children(id).iter().rev().copied().collect();
        while let Some(current) = stack.pop() {
            out.push(current);
            stack.extend(self.children(current).iter().rev());
        }
        out
    }

    pub fn find_first_child(&self, id: InstanceId, name: &str) -> Option<InstanceId> {
        self.children(id).iter().copied().find(|c| self.get(*c).is_some_and(|i| i.name == name))
    }

    pub fn find_first_descendant(&self, id: InstanceId, name: &str) -> Option<InstanceId> {
        self.descendants(id).into_iter().find(|c| self.get(*c).is_some_and(|i| i.name == name))
    }

    pub fn find_first_child_of_class(&self, id: InstanceId, class: &str) -> Option<InstanceId> {
        self.children(id).iter().copied().find(|c| self.get(*c).is_some_and(|i| i.is_a(class)))
    }

    pub fn is_descendant_of(&self, id: InstanceId, ancestor: InstanceId) -> bool {
        let mut current = self.get(id).and_then(|i| i.parent);
        while let Some(p) = current {
            if p == ancestor {
                return true;
            }
            current = self.get(p).and_then(|i| i.parent);
        }
        false
    }

    /// Whether the instance is part of the simulated, rendered world.
    pub fn in_workspace(&self, id: InstanceId) -> bool {
        id == self.workspace() || self.is_descendant_of(id, self.workspace())
    }

    /// The nearest ancestor (excluding `id` itself) of the given class.
    pub fn ancestor_of_class(&self, id: InstanceId, class: &str) -> Option<InstanceId> {
        let mut current = self.get(id).and_then(|i| i.parent);
        while let Some(p) = current {
            let instance = self.get(p)?;
            if instance.is_a(class) {
                return Some(p);
            }
            current = instance.parent;
        }
        None
    }

    /// Dot-separated path from the root (the root itself is omitted).
    pub fn path(&self, id: InstanceId) -> String {
        let mut names = Vec::new();
        let mut current = Some(id);
        while let Some(c) = current {
            let Some(instance) = self.get(c) else { break };
            if c != self.root {
                names.push(instance.name.as_str());
            }
            current = instance.parent;
        }
        names.reverse();
        names.join(".")
    }

    /// Resolves a path produced by [`DataModel::path`].
    pub fn find_by_path(&self, path: &str) -> Option<InstanceId> {
        let mut current = self.root;
        for name in path.split('.').filter(|s| !s.is_empty()) {
            current = self.find_first_child(current, name)?;
        }
        Some(current)
    }

    pub fn tagged(&self, tag: &str) -> Vec<InstanceId> {
        let mut ids: Vec<InstanceId> = self.instances.values().filter(|i| i.has_tag(tag)).map(|i| i.id).collect();
        ids.sort();
        ids
    }

    /// World position of an instance: its own `Position`, offset by its
    /// parent's transform for attachment-style classes (lights, emitters,
    /// sounds, prompts, labels) parented to a part or character.
    pub fn world_position(&self, id: InstanceId) -> Option<Vec3> {
        let instance = self.get(id)?;
        let local = instance.get("Position").and_then(Variant::as_vec3);
        if instance.is_a("BasePart") || instance.is_a("Character") {
            return local;
        }
        let parent = instance.parent.and_then(|p| self.get(p));
        match parent {
            Some(p) if p.is_a("BasePart") => Some(p.vec3("Position") + p.quat("Rotation") * local.unwrap_or(Vec3::ZERO)),
            Some(p) if p.is_a("Character") => {
                let yaw = Quat::from_rotation_y(p.f32("Facing").to_radians());
                Some(p.vec3("Position") + yaw * local.unwrap_or(Vec3::ZERO))
            }
            _ => local,
        }
    }

    // --------------------------------------------------------- mutation

    /// Creates an instance of a creatable class.
    pub fn create(&mut self, class: &str, parent: Option<InstanceId>) -> ModelResult<InstanceId> {
        let info = schema().find(class).ok_or_else(|| ModelError::UnknownClass(class.to_string()))?;
        if !info.def.creatable {
            return Err(ModelError::NotCreatable(class.to_string()));
        }
        if let Some(parent) = parent {
            if !self.contains(parent) {
                return Err(ModelError::Missing);
            }
        }
        let id = self.insert_raw(info.id, class, parent);
        if let Some(parent) = parent {
            self.events.push(Event { target: parent, name: "ChildAdded".into(), args: vec![id.into()] });
        }
        Ok(id)
    }

    /// Creates an instance of any non-service class (players, for example,
    /// are created by the engine rather than by scripts).
    pub fn create_internal(&mut self, class: &str, name: &str, parent: Option<InstanceId>) -> InstanceId {
        let info = schema().find(class).unwrap_or_else(|| panic!("unknown built-in class {class}"));
        let id = self.insert_raw(info.id, name, parent);
        if let Some(parent) = parent {
            self.events.push(Event { target: parent, name: "ChildAdded".into(), args: vec![id.into()] });
        }
        id
    }

    /// Creates an instance with a caller-chosen id (replication, scene loading).
    pub fn create_with_id(&mut self, id: InstanceId, class: &str, name: &str, parent: Option<InstanceId>) -> ModelResult<()> {
        let info = schema().find(class).ok_or_else(|| ModelError::UnknownClass(class.to_string()))?;
        if self.contains(id) {
            return Err(ModelError::InvalidValue(format!("instance {id} already exists")));
        }
        self.insert_with_id(id, info.id, name, parent.filter(|p| self.contains(*p)));
        if !id.is_local() {
            self.next_id = self.next_id.max(id.0 + 1);
        }
        Ok(())
    }

    /// Allocates an id in the client-local range (never replicated).
    pub fn create_local(&mut self, class: &str, name: &str, parent: Option<InstanceId>) -> ModelResult<InstanceId> {
        let id = InstanceId(self.next_local_id);
        self.next_local_id += 1;
        self.create_with_id(id, class, name, parent)?;
        Ok(id)
    }

    fn is_protected(&self, id: InstanceId) -> bool {
        id == self.root || self.services.values().any(|s| *s == id)
    }

    /// Destroys an instance and all of its descendants.
    pub fn destroy(&mut self, id: InstanceId) -> ModelResult<()> {
        let instance = self.get(id).ok_or(ModelError::Missing)?;
        if self.is_protected(id) {
            return Err(ModelError::Protected(instance.name.clone()));
        }
        let parent = instance.parent;
        let mut doomed = self.descendants(id);
        doomed.insert(0, id);
        if let Some(parent) = parent {
            if let Some(p) = self.instances.get_mut(&parent) {
                p.children.retain(|c| *c != id);
            }
            self.events.push(Event { target: parent, name: "ChildRemoved".into(), args: vec![id.into()] });
        }
        // Children are removed before their parents.
        for victim in doomed.into_iter().rev() {
            self.watched.remove(&victim);
            self.events.push(Event { target: victim, name: "Destroying".into(), args: Vec::new() });
            self.instances.remove(&victim);
            self.journal.push(Change::Removed(victim));
        }
        Ok(())
    }

    /// Destroys every child of an instance.
    pub fn clear_children(&mut self, id: InstanceId) {
        for child in self.children(id).to_vec() {
            // Service children (the camera) are intentionally kept.
            let _ = self.destroy(child);
        }
    }

    pub fn set_parent(&mut self, id: InstanceId, parent: Option<InstanceId>) -> ModelResult<()> {
        let instance = self.get(id).ok_or(ModelError::Missing)?;
        if self.is_protected(id) {
            return Err(ModelError::Protected(instance.name.clone()));
        }
        let old = instance.parent;
        if old == parent {
            return Ok(());
        }
        if let Some(new_parent) = parent {
            if !self.contains(new_parent) {
                return Err(ModelError::Missing);
            }
            if new_parent == id || self.is_descendant_of(new_parent, id) {
                return Err(ModelError::Cycle(instance.name.clone()));
            }
        }
        if let Some(old) = old {
            if let Some(p) = self.instances.get_mut(&old) {
                p.children.retain(|c| *c != id);
            }
            self.events.push(Event { target: old, name: "ChildRemoved".into(), args: vec![id.into()] });
        }
        if let Some(new_parent) = parent {
            if let Some(p) = self.instances.get_mut(&new_parent) {
                p.children.push(id);
            }
            self.events.push(Event { target: new_parent, name: "ChildAdded".into(), args: vec![id.into()] });
        }
        if let Some(instance) = self.instances.get_mut(&id) {
            instance.parent = parent;
        }
        self.journal.push(Change::Reparented(id));
        Ok(())
    }

    /// Moves a child to a new position among its siblings.
    pub fn reorder_child(&mut self, id: InstanceId, index: usize) {
        let Some(parent) = self.get(id).and_then(|i| i.parent) else { return };
        if let Some(p) = self.instances.get_mut(&parent) {
            if let Some(from) = p.children.iter().position(|c| *c == id) {
                p.children.remove(from);
                let index = index.min(p.children.len());
                p.children.insert(index, id);
                self.journal.push(Change::Reparented(id));
            }
        }
    }

    pub fn set_name(&mut self, id: InstanceId, name: &str) -> ModelResult<()> {
        let instance = self.instances.get_mut(&id).ok_or(ModelError::Missing)?;
        if instance.name != name {
            instance.name = name.to_string();
            self.journal.push(Change::Renamed(id));
            self.notify_changed(id, "Name");
        }
        Ok(())
    }

    /// Assigns a property after validating it against the schema.
    pub fn set(&mut self, id: InstanceId, property: &str, value: impl Into<Variant>) -> ModelResult<()> {
        let instance = self.instances.get_mut(&id).ok_or(ModelError::Missing)?;
        let info = instance.info();
        let index = info
            .prop_index(property)
            .ok_or_else(|| ModelError::UnknownProperty { class: info.name(), property: property.to_string() })?;
        let value = info.props[index].coerce(value.into()).map_err(ModelError::InvalidValue)?;
        if instance.props[index] != value {
            instance.props[index] = value;
            self.journal.push(Change::Prop(id, index as u16));
            self.notify_changed(id, property);
        }
        Ok(())
    }

    /// Writes a transform computed by the physics simulation. Recorded as
    /// [`Change::Moved`] so the physics world does not treat it as a teleport.
    pub fn set_simulated_transform(&mut self, id: InstanceId, position: Vec3, rotation: Option<Quat>) {
        let Some(instance) = self.instances.get_mut(&id) else { return };
        let info = instance.info();
        let mut changed = false;
        if let Some(i) = info.prop_index("Position") {
            if instance.props[i] != Variant::Vec3(position) {
                instance.props[i] = Variant::Vec3(position);
                changed = true;
            }
        }
        if let (Some(rotation), Some(i)) = (rotation, info.prop_index("Rotation")) {
            if instance.props[i] != Variant::Quat(rotation) {
                instance.props[i] = Variant::Quat(rotation);
                changed = true;
            }
        }
        if changed {
            self.journal.push(Change::Moved(id));
        }
    }

    /// Writes a property without validation, journaling or events. For
    /// high-frequency engine state such as velocities.
    pub fn set_quiet(&mut self, id: InstanceId, property: &str, value: Variant) {
        if let Some(instance) = self.instances.get_mut(&id) {
            if let Some(i) = instance.info().prop_index(property) {
                instance.props[i] = value;
            }
        }
    }

    pub fn set_attribute(&mut self, id: InstanceId, name: &str, value: Variant) -> ModelResult<()> {
        let instance = self.instances.get_mut(&id).ok_or(ModelError::Missing)?;
        let changed = match &value {
            Variant::Nil => instance.attributes.remove(name).is_some(),
            _ => instance.attributes.insert(name.to_string(), value.clone()).as_ref() != Some(&value),
        };
        if changed {
            self.journal.push(Change::Attribute(id));
            self.events.push(Event { target: id, name: "AttributeChanged".into(), args: vec![name.into()] });
        }
        Ok(())
    }

    pub fn add_tag(&mut self, id: InstanceId, tag: &str) {
        if let Some(instance) = self.instances.get_mut(&id) {
            if !instance.has_tag(tag) {
                instance.tags.push(tag.to_string());
                self.journal.push(Change::Attribute(id));
            }
        }
    }

    pub fn remove_tag(&mut self, id: InstanceId, tag: &str) {
        if let Some(instance) = self.instances.get_mut(&id) {
            let before = instance.tags.len();
            instance.tags.retain(|t| t != tag);
            if instance.tags.len() != before {
                self.journal.push(Change::Attribute(id));
            }
        }
    }

    pub fn set_creator(&mut self, id: InstanceId, creator: u32) {
        if let Some(instance) = self.instances.get_mut(&id) {
            instance.creator = creator;
        }
    }

    /// Deep-copies an instance and its descendants. References that point
    /// inside the copied subtree are remapped to the copies.
    pub fn clone_tree(&mut self, source: InstanceId, parent: Option<InstanceId>) -> ModelResult<InstanceId> {
        if !self.contains(source) {
            return Err(ModelError::Missing);
        }
        if self.is_protected(source) {
            return Err(ModelError::Protected(self.instances[&source].name.clone()));
        }
        let mut order = vec![source];
        order.extend(self.descendants(source));
        let mut mapping: HashMap<InstanceId, InstanceId> = HashMap::new();
        for old in &order {
            let original = self.instances[old].clone();
            let new_parent = if *old == source { parent } else { original.parent.and_then(|p| mapping.get(&p).copied()) };
            let new_id = self.insert_raw(original.class, &original.name, new_parent);
            let copy = self.instances.get_mut(&new_id).expect("just inserted");
            copy.props = original.props;
            copy.attributes = original.attributes;
            copy.tags = original.tags;
            mapping.insert(*old, new_id);
        }
        for new_id in mapping.values() {
            let instance = self.instances.get_mut(new_id).expect("copied instance");
            for value in &mut instance.props {
                if let Variant::Ref(Some(target)) = value {
                    if let Some(remapped) = mapping.get(target) {
                        *target = *remapped;
                    }
                }
            }
        }
        let root_copy = mapping[&source];
        if let Some(parent) = parent {
            self.events.push(Event { target: parent, name: "ChildAdded".into(), args: vec![root_copy.into()] });
        }
        Ok(root_copy)
    }

    // -------------------------------------------------- journal & events

    /// Changes recorded since the journal was last cleared.
    pub fn journal(&self) -> &[Change] {
        &self.journal
    }

    pub fn clear_journal(&mut self) {
        self.journal.clear();
    }

    /// Queues an event for scripts.
    pub fn fire(&mut self, target: InstanceId, name: &str, args: Vec<Variant>) {
        self.events.push(Event { target, name: name.to_string(), args });
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Asks for `Changed` events whenever a property of `id` is assigned.
    pub fn watch(&mut self, id: InstanceId) {
        self.watched.insert(id);
    }

    fn notify_changed(&mut self, id: InstanceId, property: &str) {
        if self.watched.contains(&id) {
            self.events.push(Event { target: id, name: "Changed".into(), args: vec![property.into()] });
        }
    }

    /// Axis-aligned bounds (min, max) of all parts at or below `id`.
    pub fn bounds(&self, id: InstanceId) -> Option<(Vec3, Vec3)> {
        let mut ids = vec![id];
        ids.extend(self.descendants(id));
        let mut bounds: Option<(Vec3, Vec3)> = None;
        for part in ids.iter().filter_map(|i| self.get(*i)) {
            let (center, half) = if part.is_a("BasePart") {
                let size = if part.is_a("MeshPart") { part.vec3("Scale") * 2.0 } else { part.vec3("Size") };
                let rotation = part.quat("Rotation");
                // Extents of the rotated box along the world axes.
                let h = size * 0.5;
                let (x, y, z) = (rotation * Vec3::X * h.x, rotation * Vec3::Y * h.y, rotation * Vec3::Z * h.z);
                (part.vec3("Position"), x.abs() + y.abs() + z.abs())
            } else if part.is_a("Character") {
                let scale = part.f32("Scale");
                (part.vec3("Position") + Vec3::Y * 0.9 * scale, Vec3::new(0.4, 0.9, 0.4) * scale)
            } else {
                continue;
            };
            let (lo, hi) = (center - half, center + half);
            bounds = Some(match bounds {
                Some((min, max)) => (min.min(lo), max.max(hi)),
                None => (lo, hi),
            });
        }
        bounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_exist_and_are_protected() {
        let mut dm = DataModel::new();
        for name in ["Workspace", "Lighting", "Players", "Storage", "Scripts", "Gui", "Camera"] {
            assert!(dm.find_service(name).is_some(), "{name}");
        }
        assert_eq!(dm.get(dm.service("Camera")).unwrap().parent, Some(dm.workspace()));
        assert!(matches!(dm.destroy(dm.workspace()), Err(ModelError::Protected(_))));
        assert!(dm.set_parent(dm.workspace(), None).is_err());
        assert!(dm.create("Workspace", None).is_err());
        assert!(dm.create("Nope", None).is_err());
    }

    #[test]
    fn tree_operations() {
        let mut dm = DataModel::new();
        let ws = dm.workspace();
        let model = dm.create("Model", Some(ws)).unwrap();
        let a = dm.create("Part", Some(model)).unwrap();
        let b = dm.create("Part", Some(a)).unwrap();
        dm.set_name(model, "Tower").unwrap();
        dm.set_name(b, "Top").unwrap();
        assert_eq!(dm.path(b), "Workspace.Tower.Part.Top");
        assert_eq!(dm.find_by_path("Workspace.Tower.Part.Top"), Some(b));
        assert_eq!(dm.descendants(model), vec![a, b]);
        assert!(dm.is_descendant_of(b, model) && dm.in_workspace(b));
        assert!(matches!(dm.set_parent(model, Some(b)), Err(ModelError::Cycle(_))));

        dm.set_parent(a, Some(dm.service("Storage"))).unwrap();
        assert!(!dm.in_workspace(b));
        assert!(dm.children(model).is_empty());

        dm.destroy(a).unwrap();
        assert!(!dm.contains(a) && !dm.contains(b));
        assert!(matches!(dm.destroy(a), Err(ModelError::Missing)));
    }

    #[test]
    fn properties_are_validated_and_journaled() {
        let mut dm = DataModel::new();
        let part = dm.create("Part", Some(dm.workspace())).unwrap();
        dm.clear_journal();
        dm.set(part, "Position", Vec3::new(1.0, 2.0, 3.0)).unwrap();
        dm.set(part, "Position", Vec3::new(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(dm.journal().len(), 1, "identical writes are not journaled");
        assert!(matches!(dm.set(part, "Position", 5.0), Err(ModelError::InvalidValue(_))));
        assert!(matches!(dm.set(part, "Nope", 5.0), Err(ModelError::UnknownProperty { .. })));
        dm.set(part, "Shape", "sphere").unwrap();
        assert_eq!(dm.get(part).unwrap().str("Shape"), "Sphere");
        assert_eq!(dm.get(part).unwrap().vec3("Position"), Vec3::new(1.0, 2.0, 3.0));

        dm.take_events();
        dm.watch(part);
        dm.set(part, "Anchored", false).unwrap();
        let events = dm.take_events();
        assert_eq!(events, vec![Event { target: part, name: "Changed".into(), args: vec!["Anchored".into()] }]);
    }

    #[test]
    fn clone_remaps_internal_references() {
        let mut dm = DataModel::new();
        let model = dm.create("Model", Some(dm.workspace())).unwrap();
        let part = dm.create("Part", Some(model)).unwrap();
        dm.set(model, "PrimaryPart", part).unwrap();
        dm.add_tag(part, "coin");
        dm.set_attribute(part, "Value", Variant::Number(5.0)).unwrap();
        let copy = dm.clone_tree(model, Some(dm.workspace())).unwrap();
        let copied_part = dm.children(copy)[0];
        assert_ne!(copied_part, part);
        assert_eq!(dm.get(copy).unwrap().reference("PrimaryPart"), Some(copied_part));
        assert!(dm.get(copied_part).unwrap().has_tag("coin"));
        assert_eq!(dm.get(copied_part).unwrap().attributes["Value"], Variant::Number(5.0));
        assert_eq!(dm.tagged("coin").len(), 2);
    }

    #[test]
    fn attachments_follow_their_parent() {
        let mut dm = DataModel::new();
        let part = dm.create("Part", Some(dm.workspace())).unwrap();
        dm.set(part, "Position", Vec3::new(10.0, 0.0, 0.0)).unwrap();
        dm.set(part, "Rotation", Vec3::new(0.0, 90.0, 0.0)).unwrap();
        let light = dm.create("Light", Some(part)).unwrap();
        dm.set(light, "Position", Vec3::new(0.0, 0.0, -2.0)).unwrap();
        let world = dm.world_position(light).unwrap();
        assert!((world - Vec3::new(8.0, 0.0, 0.0)).length() < 1e-4, "{world:?}");
    }
}
