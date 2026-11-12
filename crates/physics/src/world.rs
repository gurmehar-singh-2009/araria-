//! The Rapier world and its synchronization with the data model.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use araria_core::assets::{AssetManifest, ColliderShape};
use araria_core::geometry::{effective_size, wedge_points};
use araria_core::{Change, DataModel, Instance, InstanceId, Quat, Variant, Vec3, euler_deg_to_quat};
use rapier3d::control::{CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::*;

use crate::character::{CharacterBody, CharacterInput, CharacterParams, CharacterState};

const SOLID: Group = Group::GROUP_1;
/// Non-colliding parts that still raise touch events.
const TRIGGER: Group = Group::GROUP_2;
/// Parts that neither collide nor touch; kept only so the editor can pick them.
const PICK_ONLY: Group = Group::GROUP_3;
const CHARACTER: Group = Group::GROUP_4;

const MAX_STEP: f32 = 1.0 / 30.0;
const MAX_SUBSTEPS: u32 = 4;
/// Gap the character controller keeps between a character and the world.
const SKIN: f32 = 0.02;
/// Tallest ledge a character walks up without jumping.
const STEP_HEIGHT: f32 = 0.38;
/// How far below its feet a walking character looks for ground to stay on
/// (keeps it glued to slopes and stairs when going down).
const SNAP_DISTANCE: f32 = 0.3;
/// Extra reach of the character's touch sensor beyond its capsule.
const TOUCH_MARGIN: f32 = 0.06;

/// Result of a ray cast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub instance: InstanceId,
    pub position: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

struct PartBody {
    body: RigidBodyHandle,
    anchored: bool,
}

#[derive(Default)]
struct ContactCollector {
    events: Mutex<Vec<(ColliderHandle, ColliderHandle, bool)>>,
}

impl EventHandler for ContactCollector {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        if let Ok(mut events) = self.events.lock() {
            events.push(match event {
                CollisionEvent::Started(a, b, _) => (a, b, true),
                CollisionEvent::Stopped(a, b, _) => (a, b, false),
            });
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: Real,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: Real,
    ) {
    }

    fn handle_soft_body_tear_event(&self, _soft_bodies: &SoftBodySet, _event: &SoftBodyTearEvent) {}
}

/// Finds the resting height of a character capsule whose center is at
/// `center`, and returns the vertical correction needed to stand there.
///
/// The capsule is lifted by the step height and cast straight down with a
/// penetration-aware shape cast. A positive result means the capsule was
/// sunk into the ground or standing against a low ledge (step up); a
/// negative one, limited to `snap`, means there is ground just below
/// (stay attached when walking downhill). `None` means free fall.
fn settle_on_ground(queries: &QueryPipeline, shape: &Capsule, center: Vec3, snap: f32) -> Option<f32> {
    let options = ShapeCastOptions {
        target_distance: SKIN,
        stop_at_penetration: true,
        max_time_of_impact: STEP_HEIGHT + snap,
        compute_impact_geometry_on_penetration: true,
    };
    let lifted = Pose::from_translation(center + Vec3::Y * STEP_HEIGHT);
    let (_, hit) = queries.cast_shape(&lifted, -Vec3::Y, shape, options)?;
    // Starting inside something (a low ceiling) says nothing about the ground.
    if hit.time_of_impact < 1e-4 {
        return None;
    }
    // Too steep to stand on: let the controller slide the character instead.
    if hit.normal1.y.abs() < 0.62 {
        return None;
    }
    let correction = STEP_HEIGHT - hit.time_of_impact;
    (correction.abs() > 1e-4 || snap > 0.0).then_some(correction)
}

fn pair(a: InstanceId, b: InstanceId) -> (InstanceId, InstanceId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn owner(collider: &Collider) -> InstanceId {
    InstanceId(collider.user_data as u64)
}

fn groups(memberships: Group, filter: Group) -> InteractionGroups {
    InteractionGroups::new(memberships, filter, InteractionTestMode::And)
}

/// The physics simulation for one game.
pub struct Physics {
    world: PhysicsWorld,
    parts: HashMap<InstanceId, PartBody>,
    characters: HashMap<InstanceId, CharacterBody>,
    /// Pairs in solid contact as reported by Rapier.
    contacts: HashSet<(InstanceId, InstanceId)>,
    /// All touching pairs after the previous step.
    touching: HashSet<(InstanceId, InstanceId)>,
    assets: Arc<AssetManifest>,
    controller: KinematicCharacterController,
    /// Forces queued by scripts for the next step.
    pending_forces: Vec<(InstanceId, Vec3)>,
    /// Latest movement request per character. Kept separately from the
    /// bodies so input set before a body exists is not lost.
    inputs: HashMap<InstanceId, CharacterInput>,
    jump_requests: HashSet<InstanceId>,
}

impl Physics {
    pub fn new(assets: Arc<AssetManifest>) -> Self {
        // Rapier's own ground snapping and auto-stepping are disabled: on some
        // frame alignments they push the capsule through thin platforms. Both
        // are handled by `settle_on_ground` with a penetration-safe cast.
        let controller = KinematicCharacterController {
            offset: CharacterLength::Absolute(SKIN),
            autostep: None,
            max_slope_climb_angle: 50f32.to_radians(),
            min_slope_slide_angle: 58f32.to_radians(),
            snap_to_ground: None,
            ..KinematicCharacterController::default()
        };
        Physics {
            world: PhysicsWorld::default(),
            parts: HashMap::new(),
            characters: HashMap::new(),
            contacts: HashSet::new(),
            touching: HashSet::new(),
            assets,
            controller,
            pending_forces: Vec::new(),
            inputs: HashMap::new(),
            jump_requests: HashSet::new(),
        }
    }

    pub fn set_assets(&mut self, assets: Arc<AssetManifest>) {
        self.assets = assets;
    }

    /// Number of simulated bodies (parts and characters).
    pub fn body_count(&self) -> usize {
        self.parts.len() + self.characters.len()
    }

    // ------------------------------------------------------------ syncing

    /// Discards everything and rebuilds the world from the data model.
    pub fn rebuild(&mut self, dm: &DataModel) {
        self.world = PhysicsWorld::default();
        self.parts.clear();
        self.characters.clear();
        self.contacts.clear();
        self.touching.clear();
        self.pending_forces.clear();
        for id in dm.descendants(dm.workspace()) {
            self.add(dm, id);
        }
        self.refresh_queries();
    }

    /// Applies the data model's pending journal entries. Call once per frame
    /// before [`Physics::step`] (or [`Physics::refresh_queries`] in the editor).
    pub fn sync(&mut self, dm: &DataModel) {
        let mut rebuild: HashSet<InstanceId> = HashSet::new();
        let mut moved: HashSet<InstanceId> = HashSet::new();
        let mut velocity: HashSet<InstanceId> = HashSet::new();
        for change in dm.journal() {
            match change {
                Change::Removed(id) => {
                    self.remove(*id);
                    self.inputs.remove(id);
                    self.jump_requests.remove(id);
                    rebuild.remove(id);
                }
                Change::Added(id) => {
                    rebuild.insert(*id);
                }
                Change::Reparented(id) => {
                    rebuild.insert(*id);
                    rebuild.extend(dm.descendants(*id));
                }
                Change::Prop(id, index) => {
                    let Some(instance) = dm.get(*id) else { continue };
                    if !(instance.is_a("BasePart") || instance.is_a("Character")) {
                        continue;
                    }
                    match instance.info().props[*index as usize].name {
                        "Position" | "Rotation" | "Facing" => {
                            moved.insert(*id);
                        }
                        "Velocity" | "AngularVelocity" => {
                            velocity.insert(*id);
                        }
                        "Size" | "Shape" | "Scale" | "Asset" | "Collision" | "CanCollide" | "CanTouch" | "Anchored"
                        | "Density" | "Friction" | "Elasticity" => {
                            // Characters read `Anchored` live; only parts need new bodies.
                            if instance.is_a("BasePart") || instance.info().props[*index as usize].name == "Scale" {
                                rebuild.insert(*id);
                            }
                        }
                        _ => {}
                    }
                }
                Change::Moved(_) | Change::Renamed(_) | Change::Attribute(_) => {}
            }
        }
        for id in &rebuild {
            self.remove(*id);
            if dm.in_workspace(*id) {
                self.add(dm, *id);
            }
        }
        for id in moved.difference(&rebuild) {
            self.teleport(dm, *id);
        }
        for id in velocity.difference(&rebuild) {
            let Some(instance) = dm.get(*id) else { continue };
            if let Some(character) = self.characters.get_mut(id) {
                character.velocity = instance.vec3("Velocity");
            } else if let Some(part) = self.parts.get(id) {
                if !part.anchored {
                    if let Some(body) = self.world.bodies.get_mut(part.body) {
                        body.set_linvel(instance.vec3("Velocity"), true);
                        body.set_angvel(instance.vec3("AngularVelocity"), true);
                    }
                }
            }
        }
    }

    fn remove(&mut self, id: InstanceId) {
        if let Some(part) = self.parts.remove(&id) {
            self.world.remove_body(part.body);
        }
        if let Some(character) = self.characters.remove(&id) {
            self.world.remove_body(character.body);
        }
        self.contacts.retain(|(a, b)| *a != id && *b != id);
    }

    fn add(&mut self, dm: &DataModel, id: InstanceId) {
        let Some(instance) = dm.get(id) else { return };
        if instance.is_a("BasePart") {
            self.add_part(instance);
        } else if instance.is_a("Character") {
            self.add_character(instance);
        }
    }

    fn part_colliders(&self, instance: &Instance) -> Vec<ColliderBuilder> {
        if !instance.is_a("MeshPart") {
            let shape = instance.str("Shape");
            let size = effective_size(shape, instance.vec3("Size")).max(Vec3::splat(0.01));
            let half = size * 0.5;
            let builder = match shape {
                "Sphere" => ColliderBuilder::ball(half.x),
                "Cylinder" => ColliderBuilder::cylinder(half.y, half.x),
                "Cone" => ColliderBuilder::cone(half.y, half.x),
                "Wedge" => ColliderBuilder::convex_hull(&wedge_points(size))
                    .unwrap_or_else(|| ColliderBuilder::cuboid(half.x, half.y, half.z)),
                _ => ColliderBuilder::cuboid(half.x, half.y, half.z),
            };
            return vec![builder];
        }
        let scale = instance.vec3("Scale");
        let asset = self.assets.models.get(instance.str("Asset"));
        let bounding_box = |asset: Option<&araria_core::assets::ModelAsset>| {
            let (size, center) = asset.map_or((Vec3::ONE, Vec3::ZERO), |a| (a.size(), a.center()));
            let half = (size * scale * 0.5).max(Vec3::splat(0.01));
            vec![ColliderBuilder::cuboid(half.x, half.y, half.z).translation(center * scale)]
        };
        match (instance.str("Collision"), asset) {
            ("Asset", Some(model)) if !model.colliders.is_empty() => model
                .colliders
                .iter()
                .map(|def| {
                    let size = (Vec3::from(def.size) * scale).max(Vec3::splat(0.01));
                    let builder = match def.shape {
                        ColliderShape::Box => ColliderBuilder::cuboid(size.x * 0.5, size.y * 0.5, size.z * 0.5),
                        ColliderShape::Sphere => ColliderBuilder::ball(size.x * 0.5),
                        ColliderShape::Cylinder => ColliderBuilder::cylinder(size.y * 0.5, size.x * 0.5),
                        ColliderShape::Capsule => {
                            let radius = size.x * 0.5;
                            ColliderBuilder::capsule_y((size.y * 0.5 - radius).max(0.01), radius)
                        }
                    };
                    let rotation = euler_deg_to_quat(Vec3::from(def.rotation));
                    builder.position(Pose::from_parts(Vec3::from(def.center) * scale, rotation))
                })
                .collect(),
            (_, asset) => bounding_box(asset),
        }
    }

    fn add_part(&mut self, instance: &Instance) {
        let anchored = instance.bool("Anchored");
        let can_collide = instance.bool("CanCollide");
        let can_touch = instance.bool("CanTouch");
        let pose = Pose::from_parts(instance.vec3("Position"), instance.quat("Rotation"));
        let body = if anchored {
            RigidBodyBuilder::fixed()
        } else {
            RigidBodyBuilder::dynamic()
                .linvel(instance.vec3("Velocity"))
                .angvel(instance.vec3("AngularVelocity"))
                .ccd_enabled(instance.vec3("Size").min_element() < 0.6)
        }
        .pose(pose)
        .user_data(u128::from(instance.id.0));
        let handle = self.world.insert_body(body);

        let membership = if can_collide {
            SOLID
        } else if can_touch {
            TRIGGER
        } else {
            PICK_ONLY
        };
        // "None" collision on a mesh keeps a pick-only box for the editor.
        let mesh_without_collision = instance.is_a("MeshPart") && instance.str("Collision") == "None";
        let membership = if mesh_without_collision { PICK_ONLY } else { membership };
        let sensor = membership != SOLID;
        for builder in self.part_colliders(instance) {
            let mut builder = builder
                .sensor(sensor)
                .friction(instance.f32("Friction"))
                .restitution(instance.f32("Elasticity"))
                .density(instance.f32("Density"))
                .collision_groups(groups(membership, Group::ALL))
                .user_data(u128::from(instance.id.0));
            if can_touch && membership != PICK_ONLY {
                builder = builder.active_events(ActiveEvents::COLLISION_EVENTS);
            }
            self.world.insert_collider(builder, Some(handle));
        }
        self.parts.insert(instance.id, PartBody { body: handle, anchored });
    }

    fn add_character(&mut self, instance: &Instance) {
        let scale = instance.f32("Scale").max(0.05);
        let mut character = CharacterBody {
            body: RigidBodyHandle::invalid(),
            collider: ColliderHandle::invalid(),
            scale,
            position: instance.vec3("Position"),
            facing: instance.f32("Facing"),
            velocity: Vec3::ZERO,
            grounded: false,
            air_time: 1.0,
            jump_buffer: 0.0,
            jump_was_held: false,
            jumping: false,
            landing_timer: 0.0,
            carry: Vec3::ZERO,
            state: CharacterState::Falling,
            input: CharacterInput::default(),
            just_jumped: false,
            just_landed: None,
        };
        let body = RigidBodyBuilder::kinematic_position_based()
            .translation(character.center())
            .user_data(u128::from(instance.id.0));
        let (radius, height) = (character.radius(), character.height());
        let collider = ColliderBuilder::capsule_y((height * 0.5 - radius).max(0.01), radius)
            .collision_groups(groups(CHARACTER, SOLID | TRIGGER))
            .sensor(!instance.bool("CanCollide"))
            .user_data(u128::from(instance.id.0));
        let (body, collider) = self.world.insert(body, collider);
        character.body = body;
        character.collider = collider;
        self.characters.insert(instance.id, character);
    }

    /// Moves a body to where the data model says it is.
    fn teleport(&mut self, dm: &DataModel, id: InstanceId) {
        let Some(instance) = dm.get(id) else { return };
        if let Some(character) = self.characters.get_mut(&id) {
            let position = instance.vec3("Position");
            if (position - character.position).length_squared() > 1e-8 {
                character.position = position;
                character.grounded = false;
                if let Some(body) = self.world.bodies.get_mut(character.body) {
                    body.set_translation(character.center(), true);
                }
            }
            character.facing = instance.f32("Facing");
            return;
        }
        let Some(part) = self.parts.get(&id) else { return };
        let pose = Pose::from_parts(instance.vec3("Position"), instance.quat("Rotation"));
        let Some(body) = self.world.bodies.get_mut(part.body) else { return };
        if part.anchored {
            // A moving anchored part becomes kinematic so it pushes and
            // carries other bodies instead of teleporting through them.
            if body.is_fixed() {
                body.set_body_type(RigidBodyType::KinematicPositionBased, true);
            }
            body.set_next_kinematic_position(pose);
        } else {
            body.set_position(pose, true);
        }
    }

    /// Updates the acceleration structures without simulating, so queries
    /// see bodies that were just added or moved (used by the editor).
    pub fn refresh_queries(&mut self) {
        for (_, body) in self.world.bodies.iter_mut() {
            if body.is_kinematic() {
                let next = *body.next_position();
                body.set_position(next, false);
            }
        }
        self.world.detect_collisions(&(), &());
    }

    // ----------------------------------------------------------- control

    pub fn set_input(&mut self, character: InstanceId, input: CharacterInput) {
        self.inputs.insert(character, input);
    }

    pub fn input(&self, character: InstanceId) -> CharacterInput {
        self.inputs.get(&character).copied().unwrap_or_default()
    }

    /// Makes a character jump on the next step if it is able to.
    pub fn request_jump(&mut self, character: InstanceId) {
        self.jump_requests.insert(character);
    }

    /// Instantly changes a body's velocity: an impulse for unanchored parts
    /// (scaled by mass) or a velocity change for characters.
    pub fn apply_impulse(&mut self, id: InstanceId, impulse: Vec3) {
        if let Some(character) = self.characters.get_mut(&id) {
            character.velocity += impulse;
            if impulse.y > 0.0 {
                character.grounded = false;
                character.air_time = crate::character::COYOTE_TIME + 1.0;
            }
        } else if let Some(part) = self.parts.get(&id) {
            if let Some(body) = self.world.bodies.get_mut(part.body) {
                body.apply_impulse(impulse, true);
            }
        }
    }

    /// Pushes an unanchored part during the next step.
    pub fn apply_force(&mut self, id: InstanceId, force: Vec3) {
        self.pending_forces.push((id, force));
    }

    pub fn mass(&self, id: InstanceId) -> f32 {
        self.parts.get(&id).and_then(|p| self.world.bodies.get(p.body)).map_or(0.0, RigidBody::mass)
    }

    // -------------------------------------------------------------- step

    /// Advances the simulation by `dt` seconds and writes results back.
    pub fn step(&mut self, dm: &mut DataModel, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let substeps = ((dt / MAX_STEP).ceil() as u32).clamp(1, MAX_SUBSTEPS);
        let h = (dt / substeps as f32).min(MAX_STEP);
        let gravity = dm.get(dm.workspace()).map_or(30.0, |w| w.f32("Gravity"));
        let kill_height = dm.get(dm.workspace()).map_or(-60.0, |w| w.f32("KillHeight"));
        self.world.gravity = Vec3::new(0.0, -gravity, 0.0);
        self.world.integration_parameters.dt = h;
        for _ in 0..substeps {
            self.step_characters(dm, gravity, h);
            for (id, force) in &self.pending_forces {
                if let Some(body) = self.parts.get(id).and_then(|p| self.world.bodies.get_mut(p.body)) {
                    body.reset_forces(false);
                    body.add_force(*force, true);
                }
            }
            let collector = ContactCollector::default();
            self.world.step_with_events(&(), &collector);
            self.record_contacts(collector);
        }
        for (id, _) in std::mem::take(&mut self.pending_forces) {
            if let Some(body) = self.parts.get(&id).and_then(|p| self.world.bodies.get_mut(p.body)) {
                body.reset_forces(false);
            }
        }
        self.write_back(dm, kill_height);
        self.update_touches(dm);
    }

    fn record_contacts(&mut self, collector: ContactCollector) {
        let events = collector.events.into_inner().unwrap_or_default();
        for (a, b, started) in events {
            let (Some(ca), Some(cb)) = (self.world.colliders.get(a), self.world.colliders.get(b)) else { continue };
            let (ia, ib) = (owner(ca), owner(cb));
            // Characters are handled by their own overlap query.
            if ia == ib || self.characters.contains_key(&ia) || self.characters.contains_key(&ib) {
                continue;
            }
            if started {
                self.contacts.insert(pair(ia, ib));
            } else {
                self.contacts.remove(&pair(ia, ib));
            }
        }
    }

    fn step_characters(&mut self, dm: &DataModel, gravity: f32, dt: f32) {
        let ids: Vec<InstanceId> = self.characters.keys().copied().collect();
        for id in ids {
            let Some(instance) = dm.get(id) else { continue };
            let params = CharacterParams::read(instance);
            let seat = instance.reference("Seat").and_then(|s| dm.get(s));
            // What is underfoot? Rapier's controller already carries characters
            // on moving (kinematic) platforms; conveyors are added here, and the
            // surface velocity is remembered so jumps inherit it.
            let (feet, grounded_now) = match self.characters.get(&id) {
                Some(c) => (c.position, c.grounded),
                None => continue,
            };
            let mut conveyor = Vec3::ZERO;
            let mut surface_velocity = Vec3::ZERO;
            if grounded_now {
                let ray = Ray::new(feet + Vec3::Y * 0.25, -Vec3::Y);
                let filter = QueryFilter::default().groups(groups(CHARACTER, SOLID)).exclude_sensors();
                if let Some((handle, _)) = self.world.cast_ray(&ray, 0.7, true, filter) {
                    if let Some(collider) = self.world.colliders.get(handle) {
                        let ground = owner(collider);
                        if let Some(body) = collider.parent().and_then(|p| self.world.bodies.get(p)) {
                            if body.is_kinematic() {
                                surface_velocity += body.velocity_at_point(feet);
                            }
                        }
                        if self.parts.get(&ground).is_some_and(|p| p.anchored) {
                            // Conveyor: an anchored part's Velocity moves what is on it.
                            conveyor = dm.get(ground).map_or(Vec3::ZERO, |g| g.vec3("Velocity"));
                        }
                    }
                }
            }
            let input = self.inputs.get(&id).copied().unwrap_or_default();
            let jump_requested = self.jump_requests.remove(&id);
            let Some(character) = self.characters.get_mut(&id) else { continue };
            character.input = input;
            character.carry = surface_velocity + conveyor;
            if jump_requested {
                character.jump_buffer = crate::character::JUMP_BUFFER;
            }

            if let Some(seat) = seat {
                // Seated characters ride their seat.
                let rotation = seat.quat("Rotation");
                let top = seat.vec3("Size").y * 0.5;
                character.position = seat.vec3("Position") + rotation * Vec3::new(0.0, top - 0.45 * character.scale, 0.0);
                let forward = rotation * Vec3::Z;
                character.facing = araria_core::geometry::yaw_toward(forward);
                character.velocity = Vec3::ZERO;
                character.grounded = true;
                character.air_time = 0.0;
                character.state = if params.dead { CharacterState::Dead } else { CharacterState::Seated };
                let center = character.center();
                if let Some(body) = self.world.bodies.get_mut(character.body) {
                    body.set_next_kinematic_position(Pose::from_translation(center));
                }
                continue;
            }

            let step_velocity = character.integrate(&params, gravity, dt);
            let attempted = step_velocity + if character.grounded { conveyor * dt } else { Vec3::ZERO };
            let (radius, height) = (character.radius(), character.height());
            let shape = Capsule::new_y((height * 0.5 - radius).max(0.01), radius);
            let start = Pose::from_translation(character.center());
            let body_handle = character.body;
            let collides = instance.bool("CanCollide");

            let was_grounded = character.grounded;
            let rising = character.velocity.y > 0.0;
            let mut hits: Vec<ColliderHandle> = Vec::new();
            let (applied, grounded) = if collides && !params.anchored {
                let filter = QueryFilter::default()
                    .groups(groups(CHARACTER, SOLID))
                    .exclude_sensors()
                    .exclude_rigid_body(body_handle);
                let queries = self.world.query_pipeline_with_filter(filter);
                let movement = self
                    .controller
                    .move_shape(dt, &queries, &shape, &start, attempted, |collision| hits.push(collision.handle));
                let mut applied = movement.translation;
                let mut grounded = movement.grounded;
                // Blocked while walking on the ground: retry from one step
                // height up, which carries the character over low ledges.
                let wanted = Vec3::new(attempted.x, 0.0, attempted.z);
                let moved = Vec3::new(applied.x, 0.0, applied.z);
                if was_grounded && !rising && wanted.length() > 1e-4 && moved.length() < wanted.length() * 0.8 {
                    let lifted = Pose::from_translation(character.center() + Vec3::Y * STEP_HEIGHT);
                    let over = self.controller.move_shape(dt, &queries, &shape, &lifted, wanted, |_| {});
                    let over_h = Vec3::new(over.translation.x, 0.0, over.translation.z);
                    if over_h.length() > moved.length() + 1e-4 {
                        let landing = character.center() + Vec3::Y * STEP_HEIGHT + over_h;
                        // Only step if there is ground to stand on at the top.
                        if let Some(drop) = settle_on_ground(&queries, &shape, landing - Vec3::Y * STEP_HEIGHT, 0.0) {
                            if drop > 0.0 {
                                applied = over_h + Vec3::Y * drop;
                                grounded = true;
                            }
                        }
                    }
                }
                if !rising {
                    let snap = if was_grounded { SNAP_DISTANCE } else { 0.0 };
                    if let Some(correction) = settle_on_ground(&queries, &shape, character.center() + applied, snap) {
                        applied.y += correction;
                        grounded = true;
                    }
                }
                (applied, grounded)
            } else {
                (attempted, false)
            };

            // Shove loose objects the character walks into.
            let push = Vec3::new(character.velocity.x, 0.0, character.velocity.z);
            for handle in hits {
                let Some(parent) = self.world.colliders.get(handle).and_then(Collider::parent) else { continue };
                if let Some(body) = self.world.bodies.get_mut(parent) {
                    if body.is_dynamic() && push.length_squared() > 0.01 {
                        let strength = (body.mass() * 0.6).min(12.0);
                        body.apply_impulse(push.normalize() * strength * dt * 8.0, true);
                    }
                }
            }

            let character = self.characters.get_mut(&id).expect("character exists");
            character.resolve(attempted, applied, grounded, &params, dt);
            let center = character.center();
            if let Some(body) = self.world.bodies.get_mut(body_handle) {
                body.set_next_kinematic_position(Pose::from_translation(center));
            }
        }
    }

    fn write_back(&mut self, dm: &mut DataModel, kill_height: f32) {
        let mut fallen = Vec::new();
        for (id, part) in &self.parts {
            if part.anchored {
                continue;
            }
            let Some(body) = self.world.bodies.get(part.body) else { continue };
            if body.is_sleeping() {
                continue;
            }
            let position = body.translation();
            if position.y < kill_height {
                fallen.push(*id);
                continue;
            }
            dm.set_simulated_transform(*id, position, Some(*body.rotation()));
            dm.set_quiet(*id, "Velocity", Variant::Vec3(body.linvel()));
            dm.set_quiet(*id, "AngularVelocity", Variant::Vec3(body.angvel()));
        }
        for id in fallen {
            // Loose parts that fall out of the world are cleaned up.
            let _ = dm.destroy(id);
            self.remove(id);
        }
        for (id, character) in &mut self.characters {
            let Some(instance) = dm.get(*id) else { continue };
            let old_state = instance.str("State").to_string();
            let health = instance.number("Health");
            dm.set_simulated_transform(*id, character.position, None);
            dm.set_quiet(*id, "Facing", Variant::Number(f64::from(character.facing)));
            dm.set_quiet(*id, "Velocity", Variant::Vec3(character.velocity));
            dm.set_quiet(*id, "MoveDirection", Variant::Vec3(character.input.move_dir));
            dm.set_quiet(*id, "Grounded", Variant::Bool(character.grounded));
            let new_state = character.state.name();
            if old_state != new_state {
                // Journaled so the animation system and scripts notice.
                let _ = dm.set(*id, "State", new_state);
                dm.fire(*id, "StateChanged", vec![old_state.into(), new_state.into()]);
            }
            if character.just_jumped {
                dm.fire(*id, "Jumped", Vec::new());
            }
            if let Some(speed) = character.just_landed {
                dm.fire(*id, "Landed", vec![speed.into()]);
            }
            if character.position.y < kill_height && health > 0.0 {
                let _ = dm.set(*id, "Health", 0.0);
            }
        }
    }

    fn update_touches(&mut self, dm: &mut DataModel) {
        let mut current: HashSet<(InstanceId, InstanceId)> = self.contacts.clone();
        for (id, character) in &self.characters {
            let Some(instance) = dm.get(*id) else { continue };
            if instance.number("Health") <= 0.0 {
                continue;
            }
            let (radius, height) = (character.radius() + TOUCH_MARGIN, character.height() + TOUCH_MARGIN * 2.0);
            let shape = Capsule::new_y((height * 0.5 - radius).max(0.01), radius);
            let filter = QueryFilter::default().groups(groups(CHARACTER, SOLID | TRIGGER));
            let queries = self.world.query_pipeline_with_filter(filter);
            for (_, collider) in queries.intersect_shape(Pose::from_translation(character.center()), &shape) {
                let other = owner(collider);
                if other != *id && dm.get(other).is_some_and(|o| o.bool("CanTouch")) {
                    current.insert(pair(*id, other));
                }
            }
        }
        for (a, b) in current.difference(&self.touching) {
            if dm.contains(*a) && dm.contains(*b) {
                dm.fire(*a, "Touched", vec![(*b).into()]);
                dm.fire(*b, "Touched", vec![(*a).into()]);
            }
        }
        for (a, b) in self.touching.difference(&current) {
            if dm.contains(*a) && dm.contains(*b) {
                dm.fire(*a, "TouchEnded", vec![(*b).into()]);
                dm.fire(*b, "TouchEnded", vec![(*a).into()]);
            }
        }
        self.touching = current;
    }

    // ------------------------------------------------------------ queries

    fn cast(&self, origin: Vec3, direction: Vec3, max_distance: f32, mask: Group, ignore: &[InstanceId]) -> Option<RayHit> {
        let length = direction.length();
        if length < 1e-6 || max_distance <= 0.0 {
            return None;
        }
        let dir = direction / length;
        let predicate = |_: ColliderHandle, collider: &Collider| !ignore.contains(&owner(collider));
        let filter = QueryFilter::default().groups(groups(Group::ALL, mask)).predicate(&predicate);
        let ray = Ray::new(origin, dir);
        let queries = self.world.query_pipeline_with_filter(filter);
        let (handle, hit) = queries.cast_ray_and_get_normal(&ray, max_distance, true)?;
        let collider = self.world.colliders.get(handle)?;
        Some(RayHit {
            instance: owner(collider),
            position: origin + dir * hit.time_of_impact,
            normal: hit.normal,
            distance: hit.time_of_impact,
        })
    }

    /// Casts a ray against solid parts and characters.
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32, ignore: &[InstanceId]) -> Option<RayHit> {
        self.cast(origin, direction, max_distance, SOLID | CHARACTER, ignore)
    }

    /// Casts a ray against everything, including non-colliding parts. Used
    /// for selecting objects in the editor.
    pub fn pick(&self, origin: Vec3, direction: Vec3, max_distance: f32, ignore: &[InstanceId]) -> Option<RayHit> {
        self.cast(origin, direction, max_distance, Group::ALL, ignore)
    }

    /// Parts and characters overlapping a sphere.
    pub fn overlap_sphere(&self, center: Vec3, radius: f32) -> Vec<InstanceId> {
        let shape = Ball::new(radius.max(0.001));
        let filter = QueryFilter::default().groups(groups(Group::ALL, SOLID | TRIGGER | CHARACTER));
        let queries = self.world.query_pipeline_with_filter(filter);
        let mut found: Vec<InstanceId> =
            queries.intersect_shape(Pose::from_translation(center), &shape).map(|(_, c)| owner(c)).collect();
        found.sort();
        found.dedup();
        found
    }

    /// Instances currently touching `id`.
    pub fn touching(&self, id: InstanceId) -> Vec<InstanceId> {
        let mut out: Vec<InstanceId> = self
            .touching
            .iter()
            .filter_map(|(a, b)| if *a == id { Some(*b) } else if *b == id { Some(*a) } else { None })
            .collect();
        out.sort();
        out
    }

    /// Current rotation of a simulated part, if it has a body.
    pub fn rotation(&self, id: InstanceId) -> Option<Quat> {
        self.parts.get(&id).and_then(|p| self.world.bodies.get(p.body)).map(|b| *b.rotation())
    }
}
