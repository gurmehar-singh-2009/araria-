//! Headless physics tests driven entirely through the data model.

use std::sync::Arc;

use araria_core::assets::{AssetManifest, ColliderDef, ColliderShape, ModelAsset};
use araria_core::{DataModel, InstanceId, Vec3};
use araria_physics::{CharacterInput, Physics};

const DT: f32 = 1.0 / 60.0;

struct World {
    dm: DataModel,
    physics: Physics,
}

impl World {
    fn new() -> Self {
        Self::with_assets(AssetManifest::default())
    }

    fn with_assets(assets: AssetManifest) -> Self {
        let mut world = World { dm: DataModel::new(), physics: Physics::new(Arc::new(assets)) };
        let floor = world.part("Floor", Vec3::new(0.0, -0.5, 0.0), Vec3::new(200.0, 1.0, 200.0));
        world.dm.set(floor, "Anchored", true).unwrap();
        world
    }

    fn part(&mut self, name: &str, position: Vec3, size: Vec3) -> InstanceId {
        let id = self.dm.create("Part", Some(self.dm.workspace())).unwrap();
        self.dm.set_name(id, name).unwrap();
        self.dm.set(id, "Position", position).unwrap();
        self.dm.set(id, "Size", size).unwrap();
        id
    }

    fn character(&mut self, position: Vec3) -> InstanceId {
        let id = self.dm.create("Character", Some(self.dm.workspace())).unwrap();
        self.dm.set(id, "Position", position).unwrap();
        id
    }

    /// Runs `seconds` of simulation the way the runtime does each frame.
    fn run(&mut self, seconds: f32) {
        for _ in 0..(seconds / DT).round() as usize {
            self.physics.sync(&self.dm);
            self.dm.clear_journal();
            self.physics.step(&mut self.dm, DT);
        }
    }

    fn pos(&self, id: InstanceId) -> Vec3 {
        self.dm.get(id).unwrap().vec3("Position")
    }

    fn events(&mut self, name: &str) -> Vec<(InstanceId, Vec<araria_core::Variant>)> {
        self.dm.take_events().into_iter().filter(|e| e.name == name).map(|e| (e.target, e.args)).collect()
    }
}

#[test]
fn unanchored_parts_fall_and_rest_on_the_floor() {
    let mut w = World::new();
    let crate_ = w.part("Crate", Vec3::new(0.0, 5.0, 0.0), Vec3::splat(1.0));
    w.dm.set(crate_, "Anchored", false).unwrap();
    let anchored = w.part("Shelf", Vec3::new(5.0, 5.0, 0.0), Vec3::splat(1.0));
    w.run(3.0);
    let y = w.pos(crate_).y;
    assert!((y - 0.5).abs() < 0.05, "crate should rest on the floor, y = {y}");
    assert_eq!(w.pos(anchored).y, 5.0, "anchored parts do not move");
}

#[test]
fn spheres_roll_down_a_wedge() {
    let mut w = World::new();
    let ramp = w.part("Ramp", Vec3::new(0.0, 1.0, 0.0), Vec3::new(4.0, 2.0, 8.0));
    w.dm.set(ramp, "Shape", "Wedge").unwrap();
    let ball = w.part("Ball", Vec3::new(0.0, 3.5, 3.0), Vec3::splat(1.0));
    w.dm.set(ball, "Shape", "Sphere").unwrap();
    w.dm.set(ball, "Anchored", false).unwrap();
    w.run(4.0);
    let p = w.pos(ball);
    assert!(p.z < -4.0, "ball should have rolled off the low (-Z) end of the wedge: {p:?}");
    assert!(p.y < 1.0, "{p:?}");
}

#[test]
fn character_lands_walks_and_jumps() {
    let mut w = World::new();
    let c = w.character(Vec3::new(0.0, 2.0, 0.0));
    w.run(1.5);
    assert!(w.pos(c).y.abs() < 0.08, "feet on the floor: {:?}", w.pos(c));
    assert_eq!(w.dm.get(c).unwrap().str("State"), "Idle");
    assert!(w.dm.get(c).unwrap().bool("Grounded"));
    assert_eq!(w.events("Landed").len(), 1);

    w.physics.set_input(c, CharacterInput { move_dir: Vec3::X, run: false, jump: false });
    w.run(2.0);
    let x = w.pos(c).x;
    // 6 units/s for 2 s, minus a short acceleration ramp.
    assert!((11.0..=12.1).contains(&x), "walked {x}");
    assert_eq!(w.dm.get(c).unwrap().str("State"), "Walking");
    assert!((w.dm.get(c).unwrap().f32("Facing") - 90.0).abs() < 2.0);

    w.physics.set_input(c, CharacterInput { move_dir: Vec3::X, run: true, jump: false });
    w.run(1.0);
    assert_eq!(w.dm.get(c).unwrap().str("State"), "Running");

    w.physics.set_input(c, CharacterInput { move_dir: Vec3::ZERO, run: false, jump: true });
    let mut apex = 0f32;
    let mut saw_jumping = false;
    for _ in 0..90 {
        w.run(DT);
        apex = apex.max(w.pos(c).y);
        saw_jumping |= w.dm.get(c).unwrap().str("State") == "Jumping";
    }
    // v² / 2g with JumpPower 10.5 and Gravity 32 is about 1.7.
    assert!((1.3..=1.9).contains(&apex), "jump apex {apex}");
    assert!(saw_jumping);
    assert_eq!(w.events("Jumped").len(), 1, "holding jump does not bunny-hop");
    assert!(w.pos(c).y < 0.1, "landed again");
}

#[test]
fn walls_and_steps() {
    let mut w = World::new();
    w.part("Wall", Vec3::new(5.0, 2.0, 0.0), Vec3::new(1.0, 4.0, 10.0));
    w.part("Step", Vec3::new(4.0, 0.15, 5.0), Vec3::new(4.0, 0.3, 4.0));
    let c = w.character(Vec3::new(0.0, 0.1, 0.0));
    w.physics.set_input(c, CharacterInput { move_dir: Vec3::X, run: true, jump: false });
    w.run(2.0);
    let x = w.pos(c).x;
    assert!((3.9..=4.2).contains(&x), "stopped by the wall at x = {x}");

    w.physics.set_input(c, CharacterInput { move_dir: Vec3::Z, run: false, jump: false });
    w.run(1.2);
    let p = w.pos(c);
    assert!(p.z > 3.5 && (p.y - 0.32).abs() < 0.05, "stepped up onto the low step: {p:?}");
}

#[test]
fn touch_events_fire_for_triggers_and_solids() {
    let mut w = World::new();
    let coin = w.part("Coin", Vec3::new(4.0, 1.0, 0.0), Vec3::splat(1.0));
    w.dm.set(coin, "CanCollide", false).unwrap();
    let ghost = w.part("Ghost", Vec3::new(2.0, 1.0, 0.0), Vec3::splat(1.0));
    w.dm.set(ghost, "CanCollide", false).unwrap();
    w.dm.set(ghost, "CanTouch", false).unwrap();
    let c = w.character(Vec3::new(0.0, 0.1, 0.0));
    w.run(0.5);
    let floor = w.dm.find_by_path("Workspace.Floor").unwrap();
    let touched = w.events("Touched");
    assert!(touched.contains(&(floor, vec![c.into()])), "standing on the floor touches it");
    assert!(touched.contains(&(c, vec![floor.into()])));

    w.physics.set_input(c, CharacterInput { move_dir: Vec3::X, run: false, jump: false });
    w.run(0.8);
    let touched = w.events("Touched");
    assert!(touched.contains(&(coin, vec![c.into()])), "{touched:?}");
    assert!(!touched.iter().any(|(target, _)| *target == ghost), "CanTouch = false is silent");
    assert_eq!(w.physics.touching(coin), vec![c]);
    w.run(1.0);
    let ended = w.events("TouchEnded");
    assert!(ended.contains(&(coin, vec![c.into()])), "{ended:?}");
    assert!(w.pos(c).x > 6.0, "triggers do not block movement");

    // Part-to-part contact.
    let brick = w.part("Brick", Vec3::new(-20.0, 3.0, 0.0), Vec3::splat(1.0));
    w.dm.set(brick, "Anchored", false).unwrap();
    w.dm.take_events();
    w.run(1.5);
    assert!(w.events("Touched").contains(&(brick, vec![floor.into()])));
}

#[test]
fn raycasts_and_overlaps() {
    let mut w = World::new();
    let wall = w.part("Wall", Vec3::new(0.0, 2.0, -10.0), Vec3::new(6.0, 4.0, 1.0));
    let glass = w.part("Glass", Vec3::new(0.0, 2.0, -5.0), Vec3::new(6.0, 4.0, 0.2));
    w.dm.set(glass, "CanCollide", false).unwrap();
    w.physics.sync(&w.dm);
    w.dm.clear_journal();
    w.physics.refresh_queries();

    let origin = Vec3::new(0.0, 2.0, 0.0);
    let hit = w.physics.raycast(origin, -Vec3::Z, 100.0, &[]).unwrap();
    assert_eq!(hit.instance, wall, "gameplay rays pass through non-colliding parts");
    assert!((hit.distance - 9.5).abs() < 0.01 && (hit.normal - Vec3::Z).length() < 0.01, "{hit:?}");
    assert!((hit.position - Vec3::new(0.0, 2.0, -9.5)).length() < 0.01);
    assert!(w.physics.raycast(origin, -Vec3::Z, 5.0, &[]).is_none(), "respects max distance");
    assert!(w.physics.raycast(origin, -Vec3::Z, 100.0, &[wall]).is_none(), "respects the ignore list");
    assert_eq!(w.physics.pick(origin, -Vec3::Z, 100.0, &[]).unwrap().instance, glass, "editor picking sees everything");

    let near = w.physics.overlap_sphere(Vec3::new(0.0, 2.0, -6.0), 1.5);
    assert_eq!(near, vec![glass]);

    // Moving a part in the editor is visible to queries after a refresh.
    w.dm.set(wall, "Position", Vec3::new(0.0, 2.0, -20.0)).unwrap();
    w.physics.sync(&w.dm);
    w.dm.clear_journal();
    w.physics.refresh_queries();
    let hit = w.physics.raycast(origin, -Vec3::Z, 100.0, &[]).unwrap();
    assert!((hit.distance - 19.5).abs() < 0.01, "{hit:?}");
}

#[test]
fn moving_platforms_carry_characters() {
    let mut w = World::new();
    let platform = w.part("Platform", Vec3::new(0.0, 2.0, 30.0), Vec3::new(6.0, 0.5, 6.0));
    let c = w.character(Vec3::new(0.0, 2.6, 30.0));
    w.run(0.5);
    // A script moving the platform 3 units/s along X for two seconds.
    for i in 0..120 {
        let x = 3.0 * (i + 1) as f32 * DT;
        w.dm.set(platform, "Position", Vec3::new(x, 2.0, 30.0)).unwrap();
        w.run(DT);
    }
    let p = w.pos(c);
    assert!((p.x - 6.0).abs() < 0.6, "character rode the platform: {p:?}");
    assert!((p.y - 2.25).abs() < 0.1, "still standing on it: {p:?}");

    // Conveyor: an anchored part with Velocity moves what stands on it.
    let belt = w.part("Belt", Vec3::new(0.0, 0.1, -30.0), Vec3::new(4.0, 0.2, 20.0));
    w.dm.set(belt, "Velocity", Vec3::new(0.0, 0.0, 2.0)).unwrap();
    let rider = w.character(Vec3::new(0.0, 0.3, -35.0));
    w.run(2.5);
    let z = w.pos(rider).z;
    assert!(z > -31.5, "conveyor moved the character to z = {z}");
}

#[test]
fn falling_out_of_the_world() {
    let mut w = World::new();
    w.dm.set(w.dm.workspace(), "KillHeight", -10.0).unwrap();
    let c = w.character(Vec3::new(500.0, 0.0, 0.0));
    let rock = w.part("Rock", Vec3::new(500.0, 0.0, 5.0), Vec3::splat(1.0));
    w.dm.set(rock, "Anchored", false).unwrap();
    w.run(3.0);
    assert_eq!(w.dm.get(c).unwrap().number("Health"), 0.0);
    assert_eq!(w.dm.get(c).unwrap().str("State"), "Dead");
    assert!(!w.dm.contains(rock), "loose parts below KillHeight are removed");
}

#[test]
fn impulses_teleports_and_anchoring() {
    let mut w = World::new();
    let c = w.character(Vec3::new(0.0, 0.1, 0.0));
    w.run(0.3);
    w.physics.apply_impulse(c, Vec3::new(0.0, 20.0, 0.0));
    w.run(0.4);
    assert!(w.pos(c).y > 3.0, "launch pad impulse: {:?}", w.pos(c));

    w.dm.set(c, "Position", Vec3::new(50.0, 10.0, 50.0)).unwrap();
    w.run(DT);
    assert!((w.pos(c) - Vec3::new(50.0, 10.0, 50.0)).length() < 1.0, "teleported: {:?}", w.pos(c));

    w.dm.set(c, "Anchored", true).unwrap();
    let frozen = w.pos(c);
    w.run(1.0);
    assert_eq!(w.pos(c), frozen, "anchored characters hang in place");

    let ball = w.part("Ball", Vec3::new(-30.0, 0.5, 0.0), Vec3::splat(1.0));
    w.dm.set(ball, "Shape", "Sphere").unwrap();
    w.dm.set(ball, "Anchored", false).unwrap();
    w.run(0.5);
    let mass = w.physics.mass(ball);
    assert!(mass > 0.0);
    w.physics.apply_impulse(ball, Vec3::new(8.0 * mass, 0.0, 0.0));
    w.run(1.0);
    assert!(w.pos(ball).x > -27.0, "impulse moved the ball: {:?}", w.pos(ball));
    // Anchoring at runtime freezes it where it is.
    w.dm.set(ball, "Anchored", true).unwrap();
    let frozen = w.pos(ball);
    w.run(0.5);
    assert_eq!(w.pos(ball), frozen);
}

#[test]
fn mesh_parts_use_authored_colliders() {
    let mut assets = AssetManifest::default();
    assets.models.insert(
        "models/tree".into(),
        ModelAsset {
            file: "models/tree.glb".into(),
            name: "Tree".into(),
            category: "Nature".into(),
            size: [4.0, 6.0, 4.0],
            center: [0.0, 3.0, 0.0],
            // Only the trunk is solid, not the whole canopy's bounding box.
            colliders: vec![ColliderDef { shape: ColliderShape::Cylinder, center: [0.0, 1.5, 0.0], size: [0.8, 3.0, 0.8], rotation: [0.0; 3] }],
            animations: vec![],
            triangles: 0,
            tags: vec![],
        },
    );
    let mut w = World::with_assets(assets);
    let tree = w.dm.create("MeshPart", Some(w.dm.workspace())).unwrap();
    w.dm.set(tree, "Asset", "models/tree").unwrap();
    w.dm.set(tree, "Scale", Vec3::splat(2.0)).unwrap();
    w.physics.sync(&w.dm);
    w.dm.clear_journal();
    w.physics.refresh_queries();

    let trunk = w.physics.raycast(Vec3::new(-10.0, 1.0, 0.0), Vec3::X, 50.0, &[]).unwrap();
    assert_eq!(trunk.instance, tree);
    assert!((trunk.distance - 9.2).abs() < 0.05, "trunk radius is 0.4 × scale 2: {trunk:?}");
    assert!(w.physics.raycast(Vec3::new(-10.0, 1.0, 2.5), Vec3::X, 50.0, &[]).is_none(), "misses beside the trunk");

    w.dm.set(tree, "Collision", "Box").unwrap();
    w.physics.sync(&w.dm);
    w.dm.clear_journal();
    w.physics.refresh_queries();
    let boxed = w.physics.raycast(Vec3::new(-10.0, 1.0, 2.5), Vec3::X, 50.0, &[]).unwrap();
    assert!((boxed.distance - 6.0).abs() < 0.05, "bounding box half-width is 2 × scale 2: {boxed:?}");
}
