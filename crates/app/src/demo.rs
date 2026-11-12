//! A small hand-built scene used until projects can be loaded from storage.

use araria_core::{Color, DataModel, InstanceId, Vec3};

fn part(dm: &mut DataModel, name: &str, position: [f32; 3], size: [f32; 3], color: &str) -> InstanceId {
    let id = dm.create("Part", Some(dm.workspace())).expect("Part is creatable");
    dm.set_name(id, name).expect("instance exists");
    dm.set(id, "Position", Vec3::from(position)).expect("valid position");
    dm.set(id, "Size", Vec3::from(size)).expect("valid size");
    dm.set(id, "Color", Color::from_hex(color).expect("valid color")).expect("valid color");
    id
}

pub fn scene() -> DataModel {
    let mut dm = DataModel::new();
    let ground = part(&mut dm, "Ground", [0.0, -0.5, 0.0], [120.0, 1.0, 120.0], "#5fae57");
    dm.set(ground, "Material", "Grass").expect("valid material");

    let spawn = dm.create("SpawnPoint", Some(dm.workspace())).expect("creatable");
    dm.set(spawn, "Position", Vec3::new(0.0, 0.1, 6.0)).expect("valid");
    dm.set(spawn, "Size", Vec3::new(3.0, 0.2, 3.0)).expect("valid");
    dm.set(spawn, "Color", Color::from_hex("#4cc9f0").expect("valid")).expect("valid");

    // A staircase of platforms.
    for i in 0..6 {
        let f = i as f32;
        part(&mut dm, &format!("Step{i}"), [-6.0 + f * 2.4, 0.3 + f * 0.6, -4.0], [2.2, 0.4, 3.0], "#f4a261");
    }
    part(&mut dm, "Tower", [9.0, 2.0, -4.0], [3.0, 4.0, 3.0], "#e76f51");

    let ramp = part(&mut dm, "Ramp", [-12.0, 1.0, 0.0], [4.0, 2.0, 8.0], "#8ecae6");
    dm.set(ramp, "Shape", "Wedge").expect("valid");

    let pillar = part(&mut dm, "Pillar", [6.0, 2.0, 6.0], [1.6, 4.0, 1.6], "#adb5bd");
    dm.set(pillar, "Shape", "Cylinder").expect("valid");
    dm.set(pillar, "Material", "Metal").expect("valid");

    let glow = part(&mut dm, "Glow", [6.0, 4.6, 6.0], [1.2, 1.2, 1.2], "#ffd166");
    dm.set(glow, "Shape", "Sphere").expect("valid");
    dm.set(glow, "Material", "Neon").expect("valid");
    let light = dm.create("Light", Some(glow)).expect("creatable");
    dm.set(light, "Color", Color::from_hex("#ffd166").expect("valid")).expect("valid");
    dm.set(light, "Range", 18.0).expect("valid");

    let glass = part(&mut dm, "Glass", [-3.0, 1.5, 8.0], [4.0, 3.0, 0.3], "#bde0fe");
    dm.set(glass, "Material", "Glass").expect("valid");

    // Loose objects to push around.
    for (i, color) in ["#ef476f", "#06d6a0", "#118ab2"].iter().enumerate() {
        let crate_ = part(&mut dm, &format!("Crate{i}"), [3.0, 0.5 + i as f32 * 1.05, 2.0], [1.0, 1.0, 1.0], color);
        dm.set(crate_, "Anchored", false).expect("valid");
        dm.set(crate_, "Material", "Wood").expect("valid");
    }
    let ball = part(&mut dm, "Ball", [-2.0, 3.0, 2.0], [1.4, 1.4, 1.4], "#ffffff");
    dm.set(ball, "Shape", "Sphere").expect("valid");
    dm.set(ball, "Anchored", false).expect("valid");
    dm.set(ball, "Elasticity", 0.8).expect("valid");
    dm
}
