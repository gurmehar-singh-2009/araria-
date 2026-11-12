//! The class schema: every instance class, its properties, and documentation
//! for its script-visible methods and events.
//!
//! The schema is the single source of truth used by the data model (defaults
//! and type checking), the scene format (what to serialize), the property
//! inspector (what to show), the script editor (autocomplete) and the
//! generated API reference.

use std::collections::HashMap;
use std::sync::LazyLock;

use glam::{Quat, Vec2, Vec3};

use crate::types::{Color, UDim2, Variant};

/// What kind of asset an asset-reference property points to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Model,
    Audio,
    Texture,
    Script,
}

/// The static type of a property.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropType {
    Bool,
    Number { min: f64, max: f64 },
    String,
    Vec2,
    Vec3,
    Rotation,
    Color,
    UDim2,
    Ref,
    Enum(&'static [&'static str]),
    Asset(AssetKind),
}

#[derive(Clone, Debug)]
pub struct PropDef {
    pub name: &'static str,
    pub ty: PropType,
    pub default: Variant,
    pub doc: &'static str,
    /// Not shown in the property inspector.
    pub hidden: bool,
    /// Runtime state that is not saved with the scene.
    pub transient: bool,
    /// Scripts may read but not assign it.
    pub read_only: bool,
}

impl PropDef {
    fn new(name: &'static str, ty: PropType, default: Variant, doc: &'static str) -> Self {
        PropDef { name, ty, default, doc, hidden: false, transient: false, read_only: false }
    }

    fn transient(mut self) -> Self {
        self.transient = true;
        self.hidden = true;
        self
    }

    fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Checks (and where sensible converts) a value for this property.
    pub fn coerce(&self, value: Variant) -> Result<Variant, String> {
        let ok = match (&self.ty, &value) {
            (PropType::Bool, Variant::Bool(_))
            | (PropType::String | PropType::Asset(_), Variant::String(_))
            | (PropType::Vec2, Variant::Vec2(_))
            | (PropType::Vec3, Variant::Vec3(_))
            | (PropType::Rotation, Variant::Quat(_))
            | (PropType::Color, Variant::Color(_))
            | (PropType::UDim2, Variant::UDim2(_))
            | (PropType::Ref, Variant::Ref(_)) => true,
            (PropType::Ref, Variant::Nil) => return Ok(Variant::Ref(None)),
            (PropType::Number { min, max }, Variant::Number(n)) => {
                if n.is_nan() {
                    return Err(format!("{} cannot be NaN", self.name));
                }
                return Ok(Variant::Number(n.clamp(*min, *max)));
            }
            (PropType::Rotation, Variant::Vec3(deg)) => {
                return Ok(Variant::Quat(crate::types::euler_deg_to_quat(*deg)));
            }
            (PropType::Enum(options), Variant::String(s)) => {
                if let Some(option) = options.iter().find(|o| o.eq_ignore_ascii_case(s)) {
                    return Ok(Variant::String((*option).to_string()));
                }
                return Err(format!("'{s}' is not a valid {} (expected one of: {})", self.name, options.join(", ")));
            }
            _ => false,
        };
        if ok {
            Ok(value)
        } else {
            Err(format!("{} expects {}, got {}", self.name, self.type_label(), value.type_name()))
        }
    }

    /// Human-readable type name used in docs and error messages.
    pub fn type_label(&self) -> &'static str {
        match self.ty {
            PropType::Bool => "boolean",
            PropType::Number { .. } => "number",
            PropType::String | PropType::Enum(_) | PropType::Asset(_) => "string",
            PropType::Vec2 => "Vector2",
            PropType::Vec3 | PropType::Rotation => "Vector3",
            PropType::Color => "Color3",
            PropType::UDim2 => "UDim2",
            PropType::Ref => "Instance",
        }
    }
}

/// Documentation for a method or event implemented by the scripting layer.
#[derive(Clone, Copy, Debug)]
pub struct MemberDoc {
    pub name: &'static str,
    /// e.g. `(name: string): Instance?` for methods, `(other: Part)` for events.
    pub signature: &'static str,
    pub doc: &'static str,
}

const fn m(name: &'static str, signature: &'static str, doc: &'static str) -> MemberDoc {
    MemberDoc { name, signature, doc }
}

#[derive(Clone, Debug)]
pub struct ClassDef {
    pub name: &'static str,
    pub parent: Option<&'static str>,
    pub doc: &'static str,
    /// Can be created with `Instance.new` and from the editor's insert menu.
    pub creatable: bool,
    /// A singleton child of the root created with every data model.
    pub service: bool,
    pub props: Vec<PropDef>,
    pub methods: Vec<MemberDoc>,
    pub events: Vec<MemberDoc>,
}

/// Index of a class in the [`Schema`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClassId(pub u16);

/// A class with inherited members flattened in.
pub struct ClassInfo {
    pub id: ClassId,
    pub def: ClassDef,
    /// All properties, base classes first, so a base property keeps the same
    /// index in every derived class.
    pub props: Vec<PropDef>,
    index: HashMap<&'static str, usize>,
    /// This class followed by its ancestors.
    pub lineage: Vec<ClassId>,
}

impl ClassInfo {
    pub fn name(&self) -> &'static str {
        self.def.name
    }

    pub fn prop_index(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    pub fn prop(&self, name: &str) -> Option<&PropDef> {
        self.prop_index(name).map(|i| &self.props[i])
    }

    pub fn is_a(&self, class: ClassId) -> bool {
        self.lineage.contains(&class)
    }

    /// Methods of this class and its ancestors, most derived first.
    pub fn all_methods(&self) -> impl Iterator<Item = &MemberDoc> {
        self.lineage.iter().flat_map(|c| schema().class(*c).def.methods.iter())
    }

    pub fn all_events(&self) -> impl Iterator<Item = &MemberDoc> {
        self.lineage.iter().flat_map(|c| schema().class(*c).def.events.iter())
    }
}

pub struct Schema {
    classes: Vec<ClassInfo>,
    by_name: HashMap<&'static str, ClassId>,
}

impl Schema {
    pub fn class(&self, id: ClassId) -> &ClassInfo {
        &self.classes[id.0 as usize]
    }

    pub fn find(&self, name: &str) -> Option<&ClassInfo> {
        self.by_name.get(name).map(|id| self.class(*id))
    }

    /// Looks a class up by name, panicking for names that are compiled in.
    pub fn id(&self, name: &str) -> ClassId {
        *self.by_name.get(name).unwrap_or_else(|| panic!("unknown built-in class {name}"))
    }

    pub fn classes(&self) -> impl Iterator<Item = &ClassInfo> {
        self.classes.iter()
    }

    pub fn services(&self) -> impl Iterator<Item = &ClassInfo> {
        self.classes.iter().filter(|c| c.def.service)
    }

    fn build(defs: Vec<ClassDef>) -> Schema {
        let mut by_name = HashMap::new();
        for (i, def) in defs.iter().enumerate() {
            by_name.insert(def.name, ClassId(i as u16));
        }
        let mut classes: Vec<ClassInfo> = Vec::with_capacity(defs.len());
        for (i, def) in defs.into_iter().enumerate() {
            let id = ClassId(i as u16);
            let (mut props, mut lineage) = match def.parent {
                Some(parent) => {
                    // Parents are declared before children, so they are built already.
                    let parent = &classes[by_name[parent].0 as usize];
                    (parent.props.clone(), parent.lineage.clone())
                }
                None => (Vec::new(), Vec::new()),
            };
            props.extend(def.props.iter().cloned());
            lineage.insert(0, id);
            let index = props.iter().enumerate().map(|(i, p)| (p.name, i)).collect();
            classes.push(ClassInfo { id, def, props, index, lineage });
        }
        Schema { classes, by_name }
    }
}

/// The global class schema.
pub fn schema() -> &'static Schema {
    static SCHEMA: LazyLock<Schema> = LazyLock::new(|| Schema::build(definitions()));
    &SCHEMA
}

// ------------------------------------------------------------ definitions

const UNBOUNDED: PropType = PropType::Number { min: f64::MIN, max: f64::MAX };

fn bool(name: &'static str, default: bool, doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::Bool, Variant::Bool(default), doc)
}
fn num(name: &'static str, default: f64, doc: &'static str) -> PropDef {
    PropDef::new(name, UNBOUNDED, Variant::Number(default), doc)
}
fn ranged(name: &'static str, default: f64, min: f64, max: f64, doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::Number { min, max }, Variant::Number(default), doc)
}
fn string(name: &'static str, default: &str, doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::String, Variant::String(default.to_string()), doc)
}
fn vec3(name: &'static str, default: [f32; 3], doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::Vec3, Variant::Vec3(Vec3::from(default)), doc)
}
fn vec2(name: &'static str, default: [f32; 2], doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::Vec2, Variant::Vec2(Vec2::from(default)), doc)
}
fn rotation(name: &'static str, doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::Rotation, Variant::Quat(Quat::IDENTITY), doc)
}
fn color(name: &'static str, hex: &str, doc: &'static str) -> PropDef {
    let default = Color::from_hex(hex).expect("valid default color");
    PropDef::new(name, PropType::Color, Variant::Color(default), doc)
}
fn udim2(name: &'static str, default: UDim2, doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::UDim2, Variant::UDim2(default), doc)
}
fn reference(name: &'static str, doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::Ref, Variant::Ref(None), doc)
}
fn choice(name: &'static str, options: &'static [&'static str], default: &str, doc: &'static str) -> PropDef {
    debug_assert!(options.contains(&default));
    PropDef::new(name, PropType::Enum(options), Variant::String(default.to_string()), doc)
}
fn asset(name: &'static str, kind: AssetKind, default: &str, doc: &'static str) -> PropDef {
    PropDef::new(name, PropType::Asset(kind), Variant::String(default.to_string()), doc)
}

fn class(name: &'static str, parent: Option<&'static str>, doc: &'static str) -> ClassDef {
    ClassDef { name, parent, doc, creatable: true, service: false, props: Vec::new(), methods: Vec::new(), events: Vec::new() }
}

impl ClassDef {
    fn abstract_class(mut self) -> Self {
        self.creatable = false;
        self
    }
    fn service(mut self) -> Self {
        self.service = true;
        self.creatable = false;
        self
    }
    fn props(mut self, props: Vec<PropDef>) -> Self {
        self.props = props;
        self
    }
    fn methods(mut self, methods: Vec<MemberDoc>) -> Self {
        self.methods = methods;
        self
    }
    fn events(mut self, events: Vec<MemberDoc>) -> Self {
        self.events = events;
        self
    }
}

pub const MATERIALS: &[&str] = &[
    "Plastic", "Smooth", "Wood", "Metal", "Glass", "Neon", "Grass", "Sand", "Rock", "Ice", "Fabric", "Brick",
    "Concrete", "Water",
];
pub const SHAPES: &[&str] = &["Block", "Sphere", "Cylinder", "Wedge", "Cone", "Plane"];
pub const CAMERA_MODES: &[&str] = &["Follow", "FirstPerson", "Fixed", "Scriptable"];
pub const TEXT_ALIGN: &[&str] = &["Left", "Center", "Right"];
pub const CHARACTER_STATES: &[&str] =
    &["Idle", "Walking", "Running", "Jumping", "Falling", "Landing", "Seated", "Dead"];

#[allow(clippy::too_many_lines)]
fn definitions() -> Vec<ClassDef> {
    vec![
        class("Instance", None, "Base class of every object in the game tree.")
            .abstract_class()
            .methods(vec![
                m("Destroy", "()", "Removes the instance and all of its descendants from the game."),
                m("Clone", "(): Instance", "Returns a deep copy with no parent."),
                m("FindFirstChild", "(name: string, recursive: boolean?): Instance?", "First child (or descendant) with the given name."),
                m("FindFirstChildOfClass", "(className: string): Instance?", "First child that is of the given class."),
                m("WaitForChild", "(name: string, timeout: number?): Instance?", "Yields until a child with the name exists."),
                m("GetChildren", "(): {Instance}", "The instance's direct children."),
                m("GetDescendants", "(): {Instance}", "Every instance below this one."),
                m("IsA", "(className: string): boolean", "Whether the instance is of the class or inherits from it."),
                m("IsDescendantOf", "(ancestor: Instance): boolean", "Whether `ancestor` is somewhere above this instance."),
                m("GetFullName", "(): string", "Dot-separated path from the root, e.g. `Workspace.Door.Handle`."),
                m("SetAttribute", "(name: string, value: any)", "Stores a custom value on the instance."),
                m("GetAttribute", "(name: string): any", "Reads a custom value stored with SetAttribute."),
                m("GetAttributes", "(): {[string]: any}", "All custom attributes."),
                m("AddTag", "(tag: string)", "Adds a tag that scripts can query with `Workspace:GetTagged`."),
                m("RemoveTag", "(tag: string)", "Removes a tag."),
                m("HasTag", "(tag: string): boolean", "Whether the instance has the tag."),
                m("GetPropertyChangedSignal", "(property: string): Signal", "Event fired when the property changes."),
            ])
            .events(vec![
                m("ChildAdded", "(child: Instance)", "Fired when a child is parented to this instance."),
                m("ChildRemoved", "(child: Instance)", "Fired when a child leaves this instance."),
                m("Changed", "(property: string)", "Fired when any property changes."),
                m("Destroying", "()", "Fired just before the instance is destroyed."),
            ]),
        class("Game", Some("Instance"), "The root of the tree. Available to scripts as `game`.")
            .abstract_class()
            .methods(vec![m("GetService", "(name: string): Instance", "Returns a service such as `Workspace` or `Players`.")]),
        class("Folder", Some("Instance"), "A container for organizing instances."),
        class("Model", Some("Instance"), "A group of parts that can be moved together.")
            .props(vec![reference("PrimaryPart", "The part that defines the model's pivot.")])
            .methods(vec![
                m("GetPivot", "(): Vector3", "Position of the primary part, or the center of the model's bounds."),
                m("MoveTo", "(position: Vector3)", "Moves every part so the pivot lands on `position`."),
                m("GetBoundingBox", "(): (Vector3, Vector3)", "Center and size of the box enclosing all parts."),
            ]),
        class("BasePart", Some("Instance"), "Anything with a position in the 3D world that can collide.")
            .abstract_class()
            .props(vec![
                vec3("Position", [0.0, 0.0, 0.0], "World-space position of the part's center."),
                rotation("Rotation", "Orientation as X/Y/Z angles in degrees."),
                vec3("Size", [2.0, 1.0, 2.0], "Dimensions along the part's local axes."),
                color("Color", "#a3a9b8", "Surface color."),
                choice("Material", MATERIALS, "Plastic", "Surface look (roughness, shine, glow)."),
                ranged("Transparency", 0.0, 0.0, 1.0, "0 is opaque, 1 is invisible."),
                bool("Anchored", true, "Anchored parts never move from physics; unanchored parts fall and collide."),
                bool("CanCollide", true, "Whether other objects collide with the part."),
                bool("CanTouch", true, "Whether the part fires Touched events."),
                bool("CastShadow", true, "Whether the part casts a shadow."),
                bool("Locked", false, "Locked parts cannot be selected in the editor viewport."),
                ranged("Density", 1.0, 0.01, 100.0, "Mass per unit volume for unanchored parts."),
                ranged("Friction", 0.6, 0.0, 2.0, "Surface friction."),
                ranged("Elasticity", 0.1, 0.0, 1.0, "Bounciness."),
                vec3("Velocity", [0.0, 0.0, 0.0], "Linear velocity. For anchored parts this carries characters standing on it (conveyors).").transient(),
                vec3("AngularVelocity", [0.0, 0.0, 0.0], "Angular velocity in radians per second.").transient(),
            ])
            .methods(vec![
                m("ApplyImpulse", "(impulse: Vector3)", "Instantly changes the velocity of an unanchored part."),
                m("ApplyForce", "(force: Vector3)", "Pushes an unanchored part for the current frame."),
                m("GetTouchingParts", "(): {BasePart}", "Parts currently overlapping this one."),
                m("LookAt", "(target: Vector3)", "Rotates the part so its front faces `target`."),
            ])
            .events(vec![
                m("Touched", "(other: Instance)", "Fired when a part or character starts touching this part."),
                m("TouchEnded", "(other: Instance)", "Fired when a part or character stops touching this part."),
            ]),
        class("Part", Some("BasePart"), "A primitive solid: block, sphere, cylinder, wedge, cone or plane.")
            .props(vec![choice("Shape", SHAPES, "Block", "The primitive shape.")]),
        class("MeshPart", Some("BasePart"), "A 3D model from the asset library or the project's imported assets.")
            .props(vec![
                asset("Asset", AssetKind::Model, "", "Model asset id, e.g. `models/tree_pine`."),
                vec3("Scale", [1.0, 1.0, 1.0], "Scale applied to the model."),
                color("Tint", "#ffffff", "Color multiplied into the model's materials."),
                choice("Collision", &["Asset", "Box", "None"], "Asset", "Collider source: the asset's authored shapes, its bounding box, or none."),
                string("Animation", "", "Name of an animation clip in the asset to loop."),
            ]),
        class("SpawnPoint", Some("BasePart"), "Where characters appear when they join or respawn.")
            .props(vec![bool("Enabled", true, "Disabled spawn points are skipped.")]),
        class("Character", Some("Instance"), "A rigged, animated humanoid controlled by a player or by scripts.")
            .props(vec![
                vec3("Position", [0.0, 3.0, 0.0], "Position of the character's feet."),
                ranged("Facing", 0.0, -360.0, 360.0, "Yaw in degrees."),
                ranged("Scale", 1.0, 0.25, 4.0, "Uniform size multiplier."),
                ranged("WalkSpeed", 6.0, 0.0, 100.0, "Movement speed in units per second."),
                ranged("RunSpeed", 10.0, 0.0, 100.0, "Movement speed while sprinting."),
                ranged("JumpPower", 10.5, 0.0, 100.0, "Initial upward velocity of a jump."),
                ranged("MaxHealth", 100.0, 1.0, 1e9, "Upper limit for Health."),
                ranged("Health", 100.0, 0.0, 1e9, "Current health; the character dies at 0."),
                string("DisplayName", "", "Name shown above the character."),
                bool("ShowName", false, "Whether to show the name tag."),
                color("SkinColor", "#f2c9a0", "Skin tone."),
                color("ShirtColor", "#3d8bfd", "Torso and sleeve color."),
                color("PantsColor", "#2b3350", "Leg color."),
                color("HairColor", "#4a3628", "Hair color."),
                bool("Anchored", false, "Freezes the character in place."),
                bool("AutoRotate", true, "Turn to face the movement direction."),
                bool("CanCollide", true, "Whether the character collides with the world."),
                vec3("Velocity", [0.0, 0.0, 0.0], "Current velocity.").transient(),
                vec3("MoveDirection", [0.0, 0.0, 0.0], "Requested movement direction.").transient().read_only(),
                choice("State", CHARACTER_STATES, "Idle", "What the character is doing.").transient().read_only(),
                bool("Grounded", false, "Whether the character is standing on something.").transient().read_only(),
                reference("Seat", "The part the character is sitting on.").transient().read_only(),
                string("Emote", "", "Name of the gesture animation currently playing.").transient().read_only(),
            ])
            .methods(vec![
                m("Move", "(direction: Vector3, run: boolean?)", "Walks in a direction until told otherwise. Pass a zero vector to stop."),
                m("MoveTo", "(position: Vector3, run: boolean?)", "Walks toward a point; fires MoveToFinished on arrival or after 8 seconds."),
                m("Jump", "()", "Jumps if standing on the ground."),
                m("Teleport", "(position: Vector3)", "Moves the character instantly and resets its velocity."),
                m("TakeDamage", "(amount: number)", "Reduces Health."),
                m("ApplyImpulse", "(velocity: Vector3)", "Adds to the character's velocity (knockback, launch pads)."),
                m("PlayAnimation", "(name: string, fadeTime: number?)", "Plays a gesture such as `Wave`, `Dance`, `Cheer` or `Sit` over the movement animation."),
                m("StopAnimation", "()", "Stops the current gesture."),
                m("Sit", "(seat: BasePart)", "Attaches the character to a part in a seated pose."),
                m("Stand", "()", "Leaves the current seat."),
            ])
            .events(vec![
                m("Died", "()", "Fired when Health reaches 0."),
                m("Jumped", "()", "Fired when the character leaves the ground by jumping."),
                m("Landed", "(fallSpeed: number)", "Fired when the character touches down."),
                m("Touched", "(other: Instance)", "Fired when the character touches a part or another character."),
                m("StateChanged", "(old: string, new: string)", "Fired when State changes."),
                m("HealthChanged", "(health: number)", "Fired when Health changes."),
                m("MoveToFinished", "(reached: boolean)", "Fired when a MoveTo ends."),
            ]),
        class("Light", Some("Instance"), "A point or spot light. Parent it to a part to make it follow that part.")
            .props(vec![
                choice("Kind", &["Point", "Spot"], "Point", "Point lights shine in all directions; spot lights shine in a cone."),
                vec3("Position", [0.0, 0.0, 0.0], "Offset from the parent part, or world position without one."),
                rotation("Rotation", "Direction of a spot light."),
                color("Color", "#fff1d6", "Light color."),
                ranged("Brightness", 1.0, 0.0, 50.0, "Intensity multiplier."),
                ranged("Range", 12.0, 0.1, 200.0, "How far the light reaches."),
                ranged("Angle", 45.0, 1.0, 89.0, "Cone half-angle of a spot light, in degrees."),
                bool("Shadows", false, "Whether the light casts shadows (expensive)."),
                bool("Enabled", true, "Turns the light on or off."),
            ]),
        class("ParticleEmitter", Some("Instance"), "Emits small glowing or fading particles. Parent it to a part to follow that part.")
            .props(vec![
                vec3("Position", [0.0, 0.0, 0.0], "Offset from the parent part, or world position without one."),
                bool("Enabled", true, "Whether particles are emitted continuously."),
                ranged("Rate", 20.0, 0.0, 500.0, "Particles per second."),
                ranged("Lifetime", 1.5, 0.05, 20.0, "Seconds each particle lives."),
                ranged("Speed", 2.0, 0.0, 100.0, "Initial speed."),
                vec3("Direction", [0.0, 1.0, 0.0], "Emission direction."),
                ranged("Spread", 25.0, 0.0, 180.0, "Cone angle around Direction, in degrees."),
                vec3("Acceleration", [0.0, 0.0, 0.0], "Constant acceleration (use a negative Y for gravity)."),
                vec3("Area", [0.0, 0.0, 0.0], "Size of the box particles spawn in."),
                ranged("Size", 0.25, 0.01, 20.0, "Particle size at birth."),
                ranged("SizeEnd", 0.0, 0.0, 20.0, "Particle size at death."),
                color("Color", "#ffd166", "Color at birth."),
                color("ColorEnd", "#ff6b35", "Color at death."),
                ranged("Opacity", 1.0, 0.0, 1.0, "Opacity at birth; particles fade out over their life."),
                bool("Glow", true, "Glowing particles are additive and unlit."),
            ])
            .methods(vec![m("Emit", "(count: number)", "Emits a burst of particles immediately.")]),
        class("Sound", Some("Instance"), "Plays an audio asset. Parent it to a part for positional audio.")
            .props(vec![
                asset("Asset", AssetKind::Audio, "", "Audio asset id, e.g. `audio/coin`."),
                ranged("Volume", 0.8, 0.0, 4.0, "Playback volume."),
                ranged("Pitch", 1.0, 0.1, 4.0, "Playback speed and pitch."),
                bool("Looped", false, "Restart when the sound ends."),
                bool("Playing", false, "Set to true to start the sound; false stops it."),
                bool("Spatial", false, "Positional audio that gets quieter with distance from its parent part."),
                ranged("Range", 30.0, 1.0, 500.0, "Distance at which a spatial sound becomes inaudible."),
            ])
            .methods(vec![
                m("Play", "()", "Starts the sound from the beginning."),
                m("Stop", "()", "Stops the sound."),
            ])
            .events(vec![m("Ended", "()", "Fired when a non-looped sound finishes.")]),
        class("Prompt", Some("Instance"), "An interaction hint shown when the player is near its parent part.")
            .props(vec![
                string("ActionText", "Interact", "What the interaction does, e.g. `Open`."),
                string("ObjectText", "", "Optional name of the object, shown above the action."),
                string("Key", "E", "Key that triggers the prompt."),
                ranged("MaxDistance", 4.0, 0.5, 100.0, "How close the player must be."),
                ranged("HoldDuration", 0.0, 0.0, 10.0, "Seconds the key must be held."),
                bool("Enabled", true, "Whether the prompt is available."),
                vec3("Position", [0.0, 0.0, 0.0], "Offset from the parent part."),
            ])
            .events(vec![m("Triggered", "(player: Player)", "Fired when a player completes the interaction.")]),
        class("BillboardLabel", Some("Instance"), "Text that floats in the world above its parent part and faces the camera.")
            .props(vec![
                string("Text", "Label", "The text to show."),
                vec3("Position", [0.0, 2.0, 0.0], "Offset from the parent part."),
                color("Color", "#ffffff", "Text color."),
                ranged("TextSize", 18.0, 6.0, 96.0, "Font size in pixels."),
                ranged("MaxDistance", 40.0, 1.0, 1000.0, "Hidden beyond this distance from the camera."),
                bool("Background", true, "Draw a dark plate behind the text."),
                bool("Enabled", true, "Whether the label is shown."),
            ]),
        class("Script", Some("Instance"), "Lua code that runs when the game starts.")
            .props(vec![
                asset("Source", AssetKind::Script, "", "Path of the script file inside the project, e.g. `scripts/main.lua`."),
                bool("Enabled", true, "Disabled scripts do not run."),
            ]),
        class("ModuleScript", Some("Instance"), "Lua code that other scripts load with `require`.")
            .props(vec![asset("Source", AssetKind::Script, "", "Path of the module file inside the project.")]),
        class("GuiObject", Some("Instance"), "Base class for on-screen interface elements.")
            .abstract_class()
            .props(vec![
                udim2("Position", UDim2::new(0.0, 0.0, 0.0, 0.0), "Position within the parent (scale and pixel offset)."),
                udim2("Size", UDim2::new(0.0, 200.0, 0.0, 50.0), "Size relative to the parent (scale and pixel offset)."),
                vec2("AnchorPoint", [0.0, 0.0], "Which point of the element Position refers to; (0.5, 0.5) is its center."),
                color("BackgroundColor", "#161a26", "Fill color."),
                ranged("BackgroundTransparency", 0.0, 0.0, 1.0, "0 is opaque, 1 hides the background."),
                ranged("CornerRadius", 10.0, 0.0, 200.0, "Corner rounding in pixels."),
                color("BorderColor", "#2f364a", "Outline color."),
                ranged("BorderSize", 0.0, 0.0, 20.0, "Outline thickness in pixels."),
                bool("Visible", true, "Hidden elements also hide their children."),
                ranged("ZIndex", 0.0, -1000.0, 1000.0, "Draw order among siblings."),
            ])
            .events(vec![
                m("MouseEnter", "()", "Fired when the pointer moves over the element."),
                m("MouseLeave", "()", "Fired when the pointer leaves the element."),
            ]),
        class("Frame", Some("GuiObject"), "A rectangle that can contain other interface elements."),
        class("TextLabel", Some("GuiObject"), "Displays text.")
            .props(vec![
                string("Text", "Label", "The text to show."),
                color("TextColor", "#f4f6fb", "Text color."),
                ranged("TextSize", 18.0, 6.0, 200.0, "Font size in pixels."),
                choice("TextAlign", TEXT_ALIGN, "Center", "Horizontal alignment."),
                bool("Bold", false, "Use the bold typeface."),
            ]),
        class("TextButton", Some("TextLabel"), "A clickable button with text.")
            .props(vec![color("HoverColor", "#2a3350", "Fill color while the pointer is over the button.")])
            .events(vec![m("Activated", "()", "Fired when the button is clicked or tapped.")]),
        class("ImageLabel", Some("GuiObject"), "Displays a texture.")
            .props(vec![
                asset("Image", AssetKind::Texture, "", "Texture asset id."),
                color("ImageColor", "#ffffff", "Tint multiplied into the image."),
            ]),
        class("ProgressBar", Some("GuiObject"), "A horizontal bar that fills from left to right.")
            .props(vec![
                ranged("Value", 0.5, 0.0, 1.0, "Fill amount from 0 to 1."),
                color("FillColor", "#4ade80", "Color of the filled portion."),
            ]),
        class("Camera", Some("Instance"), "The player's view. Available as `Workspace.Camera`.")
            .service()
            .props(vec![
                choice("Mode", CAMERA_MODES, "Follow", "Follow orbits the character, FirstPerson looks through its eyes, Fixed stays put while looking at the subject, Scriptable is fully script-driven."),
                ranged("FieldOfView", 70.0, 10.0, 120.0, "Vertical field of view in degrees."),
                ranged("Distance", 9.0, 0.0, 200.0, "Distance from the subject in Follow mode."),
                ranged("MinDistance", 0.0, 0.0, 200.0, "Closest zoom; 0 allows first person."),
                ranged("MaxDistance", 30.0, 0.5, 500.0, "Furthest zoom."),
                vec3("Position", [0.0, 6.0, 12.0], "Camera position (written by scripts in Scriptable/Fixed mode, updated by the engine otherwise)."),
                rotation("Rotation", "Camera orientation."),
                reference("Subject", "The part or character to follow; defaults to the local player's character."),
                vec3("Offset", [0.0, 0.0, 0.0], "Extra offset applied to the focus point."),
            ])
            .methods(vec![
                m("LookAt", "(target: Vector3)", "Rotates the camera to face `target`."),
                m("Shake", "(intensity: number, duration: number)", "Shakes the view."),
                m("WorldToScreen", "(position: Vector3): (Vector2, boolean)", "Projects a world position to screen pixels; the second result is false when it is behind the camera."),
            ]),
        class("Workspace", Some("Instance"), "Everything that exists in the 3D world lives under Workspace.")
            .service()
            .props(vec![
                ranged("Gravity", 32.0, 0.0, 500.0, "Downward acceleration."),
                num("KillHeight", -60.0, "Characters that fall below this height die; loose parts are removed."),
            ])
            .methods(vec![
                m("Raycast", "(origin: Vector3, direction: Vector3, ignore: {Instance}?): RaycastResult?", "Casts a ray of length |direction|. Returns `{Instance, Position, Normal, Distance}` or nil."),
                m("GetPartsInRadius", "(center: Vector3, radius: number): {Instance}", "Parts and characters whose colliders overlap the sphere."),
                m("GetTagged", "(tag: string): {Instance}", "Every instance in the game with the tag."),
            ]),
        class("Lighting", Some("Instance"), "Sun, sky, ambient light and fog.")
            .service()
            .props(vec![
                ranged("ClockTime", 14.0, 0.0, 24.0, "Time of day in hours; controls the sun's position."),
                ranged("Brightness", 1.0, 0.0, 10.0, "Sun intensity multiplier."),
                color("SunColor", "#fff4e0", "Color of direct sunlight."),
                color("Ambient", "#8fa3c8", "Color of indirect light."),
                ranged("AmbientBrightness", 1.0, 0.0, 10.0, "Indirect light intensity multiplier."),
                color("SkyColor", "#3f8fe8", "Sky color overhead."),
                color("HorizonColor", "#bfe3ff", "Sky color at the horizon."),
                color("FogColor", "#bfe3ff", "Color distant objects fade to."),
                ranged("FogStart", 80.0, 0.0, 100_000.0, "Distance where fog begins."),
                ranged("FogEnd", 400.0, 1.0, 100_000.0, "Distance where fog is fully opaque."),
                bool("Shadows", true, "Whether the sun casts shadows."),
                bool("Clouds", true, "Whether clouds drift across the sky."),
                bool("Stars", false, "Whether stars are visible."),
            ]),
        class("Players", Some("Instance"), "Holds a Player for everyone in the session.")
            .service()
            .props(vec![
                ranged("RespawnTime", 2.5, 0.0, 600.0, "Seconds before a dead character respawns."),
                bool("CharacterAutoLoads", true, "Spawn characters automatically."),
                ranged("MaxPlayers", 8.0, 1.0, 32.0, "Maximum players in a multiplayer session."),
                reference("LocalPlayer", "The player on this device.").transient().read_only(),
            ])
            .methods(vec![
                m("GetPlayers", "(): {Player}", "Everyone currently in the session."),
                m("GetPlayerFromCharacter", "(character: Instance): Player?", "The player controlling a character."),
            ])
            .events(vec![
                m("PlayerAdded", "(player: Player)", "Fired when someone joins."),
                m("PlayerRemoving", "(player: Player)", "Fired when someone leaves."),
            ]),
        class("Player", Some("Instance"), "A participant in the session. Players are temporary: there are no accounts.")
            .abstract_class()
            .props(vec![
                num("UserId", 0.0, "Temporary id, unique within this session.").read_only(),
                reference("Character", "The player's character, if spawned.").read_only(),
                bool("IsLocal", false, "True for the player on this device.").read_only(),
                reference("RespawnPoint", "Spawn point used the next time the character spawns."),
                color("Color", "#3d8bfd", "The player's signature color."),
            ])
            .methods(vec![
                m("LoadCharacter", "()", "Spawns (or respawns) the character."),
                m("SetStat", "(name: string, value: number | string)", "Sets a value shown on the leaderboard."),
                m("GetStat", "(name: string): any", "Reads a leaderboard value."),
                m("Notify", "(text: string, duration: number?)", "Shows a notification to this player."),
            ])
            .events(vec![
                m("CharacterAdded", "(character: Character)", "Fired when the player's character spawns."),
                m("CharacterRemoving", "(character: Character)", "Fired before the character is removed."),
            ]),
        class("Storage", Some("Instance"), "Holds templates that scripts clone into the world. Its contents are not rendered or simulated.").service(),
        class("Scripts", Some("Instance"), "A place to keep scripts that are not attached to an object.").service(),
        class("Gui", Some("Instance"), "On-screen interface shown to the player.").service(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_consistent() {
        let schema = schema();
        for class in schema.classes() {
            if let Some(parent) = class.def.parent {
                assert!(schema.find(parent).is_some(), "{} has unknown parent {parent}", class.name());
            }
            let mut seen = std::collections::HashSet::new();
            for prop in &class.props {
                assert!(seen.insert(prop.name), "{}.{} is declared twice", class.name(), prop.name);
                assert!(prop.coerce(prop.default.clone()).is_ok(), "{}.{} default is invalid", class.name(), prop.name);
                assert!(!prop.doc.is_empty(), "{}.{} is undocumented", class.name(), prop.name);
            }
        }
        let part = schema.find("Part").unwrap();
        let mesh = schema.find("MeshPart").unwrap();
        assert_eq!(part.prop_index("Position"), mesh.prop_index("Position"));
        assert!(part.is_a(schema.id("BasePart")) && part.is_a(schema.id("Instance")));
        assert!(!part.is_a(schema.id("Model")));
        assert!(part.all_methods().any(|m| m.name == "Destroy"));
        assert!(part.all_events().any(|e| e.name == "Touched"));
    }

    #[test]
    fn coercion_validates_and_converts() {
        let part = schema().find("Part").unwrap();
        let shape = part.prop("Shape").unwrap();
        assert_eq!(shape.coerce("sphere".into()).unwrap(), Variant::from("Sphere"));
        assert!(shape.coerce("Blob".into()).is_err());
        let transparency = part.prop("Transparency").unwrap();
        assert_eq!(transparency.coerce(5.0.into()).unwrap(), Variant::Number(1.0));
        assert!(transparency.coerce("x".into()).is_err());
        let rotation = part.prop("Rotation").unwrap();
        assert!(matches!(rotation.coerce(Vec3::new(0.0, 90.0, 0.0).into()).unwrap(), Variant::Quat(_)));
    }
}
