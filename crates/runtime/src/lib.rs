//! # Araria runtime
//!
//! A [`Runtime`] is one running (or editable) game: the data model, its
//! physics world and its players. It has no rendering dependencies, so the
//! same code drives the windowed engine, automated tests and tools.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_core::input::InputState;
use araria_core::{DataModel, InstanceId, Vec3};
use araria_physics::{CharacterInput, Physics};

/// Whether the game is being edited or played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Nothing is simulated; the editor manipulates the scene directly.
    Edit,
    Play,
    Paused,
}

pub struct Runtime {
    pub dm: Rc<RefCell<DataModel>>,
    pub physics: Rc<RefCell<Physics>>,
    pub input: Rc<RefCell<InputState>>,
    mode: Mode,
    /// Seconds of simulated play time.
    time: f64,
    local_player: Option<InstanceId>,
    /// Seconds until each dead player's character respawns.
    respawns: HashMap<InstanceId, f32>,
    next_user_id: u32,
}

impl Runtime {
    /// Wraps a scene in edit mode.
    pub fn new(dm: DataModel, assets: Arc<AssetManifest>) -> Self {
        let mut physics = Physics::new(assets);
        physics.rebuild(&dm);
        let mut dm = dm;
        dm.clear_journal();
        Runtime {
            dm: Rc::new(RefCell::new(dm)),
            physics: Rc::new(RefCell::new(physics)),
            input: Rc::new(RefCell::new(InputState::default())),
            mode: Mode::Edit,
            time: 0.0,
            local_player: None,
            respawns: HashMap::new(),
            next_user_id: 1,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn local_player(&self) -> Option<InstanceId> {
        self.local_player
    }

    /// The character controlled by the player on this device.
    pub fn local_character(&self) -> Option<InstanceId> {
        let dm = self.dm.borrow();
        self.local_player.and_then(|p| dm.get(p)).and_then(|p| p.reference("Character"))
    }

    /// Starts playing: adds the local player and spawns their character.
    pub fn start(&mut self) {
        if self.mode != Mode::Edit {
            return;
        }
        self.mode = Mode::Play;
        self.time = 0.0;
        let player = self.add_player("Player", true);
        self.local_player = Some(player);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.mode = match (self.mode, paused) {
            (Mode::Play, true) => Mode::Paused,
            (Mode::Paused, false) => Mode::Play,
            (mode, _) => mode,
        };
    }

    /// Adds a player to the session and spawns their character if the game
    /// loads characters automatically.
    pub fn add_player(&mut self, name: &str, is_local: bool) -> InstanceId {
        let user_id = self.next_user_id;
        self.next_user_id += 1;
        let (player, auto_load) = {
            let mut dm = self.dm.borrow_mut();
            let players = dm.service("Players");
            let player = dm.create_internal("Player", name, Some(players));
            dm.set_quiet(player, "UserId", f64::from(user_id).into());
            dm.set_quiet(player, "IsLocal", is_local.into());
            let hue = (user_id as f32 * 137.5) % 360.0;
            dm.set_quiet(player, "Color", araria_core::Color::from_hsv(hue, 0.65, 0.95).into());
            if is_local {
                dm.set_quiet(players, "LocalPlayer", player.into());
            }
            dm.fire(players, "PlayerAdded", vec![player.into()]);
            (player, dm.get(players).is_some_and(|p| p.bool("CharacterAutoLoads")))
        };
        if auto_load {
            self.load_character(player);
        }
        player
    }

    pub fn remove_player(&mut self, player: InstanceId) {
        let mut dm = self.dm.borrow_mut();
        let players = dm.service("Players");
        dm.fire(players, "PlayerRemoving", vec![player.into()]);
        if let Some(character) = dm.get(player).and_then(|p| p.reference("Character")) {
            let _ = dm.destroy(character);
        }
        let _ = dm.destroy(player);
        self.respawns.remove(&player);
        if self.local_player == Some(player) {
            self.local_player = None;
        }
    }

    /// Where a player's character should appear.
    fn spawn_position(dm: &DataModel, player: InstanceId) -> (Vec3, f32) {
        let preferred = dm.get(player).and_then(|p| p.reference("RespawnPoint")).and_then(|s| dm.get(s));
        let spawn = preferred.or_else(|| {
            dm.descendants(dm.workspace())
                .into_iter()
                .filter_map(|id| dm.get(id))
                .find(|i| i.is_a("SpawnPoint") && i.bool("Enabled"))
        });
        match spawn {
            Some(point) => {
                let top = point.vec3("Size").y * 0.5 + 0.1;
                let yaw = araria_core::geometry::yaw_toward(point.quat("Rotation") * Vec3::Z);
                (point.vec3("Position") + Vec3::Y * top, yaw)
            }
            None => (Vec3::new(0.0, 5.0, 0.0), 0.0),
        }
    }

    /// Spawns (or respawns) a player's character.
    pub fn load_character(&mut self, player: InstanceId) {
        let mut dm = self.dm.borrow_mut();
        let Some(instance) = dm.get(player) else { return };
        let (name, color) = (instance.name.clone(), instance.color("Color"));
        if let Some(old) = instance.reference("Character") {
            dm.fire(player, "CharacterRemoving", vec![old.into()]);
            let _ = dm.destroy(old);
        }
        let (position, yaw) = Self::spawn_position(&dm, player);
        let workspace = dm.workspace();
        let character = dm.create_internal("Character", &name, Some(workspace));
        let _ = dm.set(character, "Position", position);
        let _ = dm.set(character, "Facing", f64::from(yaw));
        let _ = dm.set(character, "ShirtColor", color);
        let _ = dm.set(character, "DisplayName", name);
        dm.set_quiet(player, "Character", character.into());
        dm.fire(player, "CharacterAdded", vec![character.into()]);
        self.respawns.remove(&player);
    }

    /// Advances the game by `dt` seconds. Does nothing unless playing.
    pub fn step(&mut self, dt: f32) {
        if self.mode != Mode::Play {
            // Keep queries (editor picking) in sync with edits.
            let dm = self.dm.borrow();
            let mut physics = self.physics.borrow_mut();
            if !dm.journal().is_empty() {
                physics.sync(&dm);
                physics.refresh_queries();
            }
            return;
        }
        self.time += f64::from(dt);

        // Local player's controls.
        if let Some(character) = self.local_character() {
            let input = self.input.borrow();
            let request = if input.ui_captured {
                CharacterInput::default()
            } else {
                CharacterInput { move_dir: input.move_direction(), run: input.run, jump: input.jump }
            };
            self.physics.borrow_mut().set_input(character, request);
        }

        {
            let mut dm = self.dm.borrow_mut();
            let mut physics = self.physics.borrow_mut();
            physics.sync(&dm);
            physics.step(&mut dm, dt);
        }
        self.update_respawns(dt);
    }

    fn update_respawns(&mut self, dt: f32) {
        let mut due = Vec::new();
        {
            let mut dm = self.dm.borrow_mut();
            let players = dm.service("Players");
            let respawn_time = dm.get(players).map_or(2.5, |p| p.f32("RespawnTime"));
            for player in dm.children(players).to_vec() {
                let Some(character) = dm.get(player).and_then(|p| p.reference("Character")) else { continue };
                let dead = dm.get(character).is_none_or(|c| c.number("Health") <= 0.0);
                if !dead {
                    continue;
                }
                match self.respawns.get_mut(&player) {
                    None => {
                        self.respawns.insert(player, respawn_time);
                        if dm.contains(character) {
                            dm.fire(character, "Died", Vec::new());
                        }
                    }
                    Some(timer) => {
                        *timer -= dt;
                        if *timer <= 0.0 {
                            due.push(player);
                        }
                    }
                }
            }
        }
        for player in due {
            self.load_character(player);
        }
    }

    /// Call after every system has observed this frame's changes.
    pub fn end_frame(&mut self) {
        self.dm.borrow_mut().clear_journal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> DataModel {
        let mut dm = DataModel::new();
        let ws = dm.workspace();
        let floor = dm.create("Part", Some(ws)).unwrap();
        dm.set(floor, "Size", Vec3::new(100.0, 1.0, 100.0)).unwrap();
        dm.set(floor, "Position", Vec3::new(0.0, -0.5, 0.0)).unwrap();
        let spawn = dm.create("SpawnPoint", Some(ws)).unwrap();
        dm.set(spawn, "Position", Vec3::new(10.0, 0.2, 5.0)).unwrap();
        dm.set(spawn, "Size", Vec3::new(4.0, 0.4, 4.0)).unwrap();
        dm.set(dm.workspace(), "KillHeight", -20.0).unwrap();
        dm
    }

    fn run(rt: &mut Runtime, seconds: f32) {
        for _ in 0..(seconds * 60.0) as usize {
            rt.step(1.0 / 60.0);
            rt.end_frame();
        }
    }

    #[test]
    fn play_spawns_a_controllable_character_at_the_spawn_point() {
        let mut rt = Runtime::new(scene(), Arc::new(AssetManifest::default()));
        assert_eq!(rt.mode(), Mode::Edit);
        assert!(rt.local_character().is_none());
        rt.start();
        let character = rt.local_character().expect("character spawned");
        run(&mut rt, 1.0);
        let start = rt.dm.borrow().get(character).unwrap().vec3("Position");
        assert!((start - Vec3::new(10.0, 0.42, 5.0)).length() < 0.1, "{start:?}");

        // Camera looks along -Z; pressing forward walks toward -Z.
        {
            let mut input = rt.input.borrow_mut();
            input.camera_yaw = 180.0;
            input.move_axis = araria_core::Vec2::new(0.0, 1.0);
        }
        run(&mut rt, 1.0);
        let moved = rt.dm.borrow().get(character).unwrap().vec3("Position");
        assert!(moved.z < start.z - 4.0, "{moved:?}");

        // While paused nothing moves.
        rt.set_paused(true);
        run(&mut rt, 1.0);
        assert_eq!(rt.dm.borrow().get(character).unwrap().vec3("Position"), moved);
    }

    #[test]
    fn dead_characters_respawn() {
        let mut rt = Runtime::new(scene(), Arc::new(AssetManifest::default()));
        rt.start();
        let first = rt.local_character().unwrap();
        run(&mut rt, 0.5);
        rt.dm.borrow_mut().take_events();
        rt.dm.borrow_mut().set(first, "Position", Vec3::new(500.0, 0.0, 0.0)).unwrap();
        run(&mut rt, 2.0);
        let died = rt.dm.borrow_mut().take_events().iter().filter(|e| e.name == "Died").count();
        assert_eq!(died, 1);
        run(&mut rt, 3.0);
        let second = rt.local_character().unwrap();
        assert_ne!(first, second);
        assert!(!rt.dm.borrow().contains(first));
        let position = rt.dm.borrow().get(second).unwrap().vec3("Position");
        assert!((position - Vec3::new(10.0, 0.42, 5.0)).length() < 0.2, "{position:?}");
    }
}
