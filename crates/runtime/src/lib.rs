//! # Araria runtime
//!
//! A [`Runtime`] is one running (or editable) game: the data model, its
//! physics world, its scripts and its players. It has no rendering
//! dependencies, so the same code drives the windowed engine, automated
//! tests and tools.
//!
//! Playing never damages the scene being edited: [`Runtime::start`] takes a
//! snapshot and [`Runtime::stop`] restores it (or keeps the played state on
//! request).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_core::input::InputState;
use araria_core::{DataModel, InstanceId, Vec3};
use araria_physics::{CharacterInput, Physics};
pub use araria_script::{Command, LogEntry, LogLevel, SaveBackend, ScriptInfo, ScriptRuntime};

/// Access to the files of the project being run (script sources).
#[derive(Clone)]
pub struct ProjectFiles {
    read: Rc<dyn Fn(&str) -> Option<String>>,
    fingerprint: Rc<dyn Fn(&str) -> Option<u64>>,
}

impl ProjectFiles {
    /// `read` loads a text file by project path; `fingerprint` returns a
    /// value that changes whenever that file's content changes.
    pub fn new(read: impl Fn(&str) -> Option<String> + 'static, fingerprint: impl Fn(&str) -> Option<u64> + 'static) -> Self {
        ProjectFiles { read: Rc::new(read), fingerprint: Rc::new(fingerprint) }
    }

    /// A project without files (scenes built in code, tests).
    pub fn none() -> Self {
        ProjectFiles::new(|_| None, |_| None)
    }

    pub fn read(&self, path: &str) -> Option<String> {
        (self.read)(path)
    }
}

/// The interaction prompt the local player can currently trigger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActivePrompt {
    pub prompt: InstanceId,
    /// How far a hold-to-interact prompt has progressed, `0..=1`.
    pub progress: f32,
}

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
    /// The Lua side of the game. Always present so the terminal can evaluate
    /// code while editing; game scripts only run while playing.
    pub scripts: ScriptRuntime,
    files: ProjectFiles,
    save: Option<Rc<dyn SaveBackend>>,
    /// The scene as it was when play started.
    snapshot: Option<DataModel>,
    /// Bumped whenever the whole scene is replaced.
    epoch: u64,
    /// Requests from scripts that the presentation layer carries out.
    commands: Vec<Command>,
    prompt: Option<ActivePrompt>,
    prompt_hold: f32,
    prompt_fired: bool,
}

fn new_scripts(dm: &Rc<RefCell<DataModel>>, physics: &Rc<RefCell<Physics>>, input: &Rc<RefCell<InputState>>, files: &ProjectFiles, save: Option<&Rc<dyn SaveBackend>>) -> ScriptRuntime {
    let (read, fingerprint) = (files.read.clone(), files.fingerprint.clone());
    let mut scripts = ScriptRuntime::new(dm.clone(), physics.clone(), input.clone(), move |path| read(path), move |path| fingerprint(path));
    if let Some(save) = save {
        scripts.set_save_backend(save.clone());
    }
    scripts
}

impl Runtime {
    /// Wraps a scene without project files in edit mode.
    pub fn new(dm: DataModel, assets: Arc<AssetManifest>) -> Self {
        Runtime::with_files(dm, assets, ProjectFiles::none())
    }

    /// Wraps a project's scene in edit mode.
    pub fn with_files(dm: DataModel, assets: Arc<AssetManifest>, files: ProjectFiles) -> Self {
        let mut physics = Physics::new(assets);
        physics.rebuild(&dm);
        let mut dm = dm;
        dm.clear_journal();
        let dm = Rc::new(RefCell::new(dm));
        let physics = Rc::new(RefCell::new(physics));
        let input = Rc::new(RefCell::new(InputState::default()));
        let scripts = new_scripts(&dm, &physics, &input, &files, None);
        Runtime {
            dm,
            physics,
            input,
            mode: Mode::Edit,
            time: 0.0,
            local_player: None,
            respawns: HashMap::new(),
            next_user_id: 1,
            scripts,
            files,
            save: None,
            snapshot: None,
            epoch: 0,
            commands: Vec::new(),
            prompt: None,
            prompt_hold: 0.0,
            prompt_fired: false,
        }
    }

    /// Replaces the scene being edited with `scene`, applying only the
    /// differences (undo, redo). Ignored while playing.
    pub fn restore_scene(&mut self, scene: &DataModel) {
        if self.mode == Mode::Edit {
            self.dm.borrow_mut().restore_from(scene);
        }
    }

    /// Where `Save:Get` / `Save:Set` keep game progress.
    pub fn set_save_backend(&mut self, backend: Rc<dyn SaveBackend>) {
        self.scripts.set_save_backend(backend.clone());
        self.save = Some(backend);
    }

    pub fn files(&self) -> &ProjectFiles {
        &self.files
    }

    /// Changes whenever the scene is replaced wholesale (leaving play
    /// mode), telling presentation layers to rebuild from scratch.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The prompt the local player is close enough to use.
    pub fn active_prompt(&self) -> Option<ActivePrompt> {
        self.prompt
    }

    /// Removes and returns script requests for the presentation layer
    /// (sounds, particles, notifications, camera shake).
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// Queues a request for the presentation layer as if a script had made
    /// it (the studio previews sounds this way).
    pub fn queue_command(&mut self, command: Command) {
        self.commands.push(command);
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

    /// Starts playing: remembers the scene, adds the local player, spawns
    /// their character and starts the game's scripts.
    pub fn start(&mut self) {
        if self.mode != Mode::Edit {
            return;
        }
        {
            let mut dm = self.dm.borrow_mut();
            dm.take_events();
            self.snapshot = Some(dm.clone());
        }
        self.mode = Mode::Play;
        self.time = 0.0;
        let player = self.add_player("Player", true);
        self.local_player = Some(player);
        // Scripts start with the local player in place; the PlayerAdded and
        // CharacterAdded events stay queued until the first frame, after
        // every script has had the chance to connect to them.
        self.scripts.start_all();
    }

    /// Leaves play mode. Unless `keep_changes` is set the scene returns to
    /// exactly what it was when play started; with it, the world stays as
    /// the game left it (minus players and anything scripts spawned).
    pub fn stop(&mut self, keep_changes: bool) {
        if self.mode == Mode::Edit {
            return;
        }
        self.scripts.stop_all();
        let snapshot = self.snapshot.take();
        if keep_changes {
            let mut dm = self.dm.borrow_mut();
            let players = dm.service("Players");
            for player in dm.children(players).to_vec() {
                if let Some(character) = dm.get(player).and_then(|p| p.reference("Character")) {
                    let _ = dm.destroy(character);
                }
                let _ = dm.destroy(player);
            }
            dm.set_quiet(players, "LocalPlayer", araria_core::Variant::Ref(None));
            dm.take_events();
        } else if let Some(snapshot) = snapshot {
            *self.dm.borrow_mut() = snapshot;
        }
        {
            let mut dm = self.dm.borrow_mut();
            dm.clear_journal();
            self.physics.borrow_mut().rebuild(&dm);
        }
        self.scripts = new_scripts(&self.dm, &self.physics, &self.input, &self.files, self.save.as_ref());
        self.mode = Mode::Edit;
        self.epoch += 1;
        self.local_player = None;
        self.respawns.clear();
        self.commands.clear();
        self.prompt = None;
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

        self.scripts.update(dt);
        self.update_prompt(dt);
        {
            let mut dm = self.dm.borrow_mut();
            let mut physics = self.physics.borrow_mut();
            physics.sync(&dm);
            physics.step(&mut dm, dt);
        }
        self.update_respawns(dt);
        for command in self.scripts.take_commands() {
            match command {
                Command::LoadCharacter { player } => self.load_character(player),
                other => self.commands.push(other),
            }
        }
    }

    /// Finds the prompt nearest the local character and fires `Triggered`
    /// when its key has been pressed (or held long enough).
    fn update_prompt(&mut self, dt: f32) {
        let Some(player) = self.local_player else { return };
        let mut dm = self.dm.borrow_mut();
        let position = dm.get(player).and_then(|p| p.reference("Character")).and_then(|c| dm.get(c)).filter(|c| c.number("Health") > 0.0).map(|c| c.vec3("Position") + Vec3::Y);
        let mut nearest: Option<(InstanceId, f32)> = None;
        if let Some(position) = position {
            for id in dm.descendants(dm.workspace()) {
                let Some(prompt) = dm.get(id).filter(|i| i.class_name() == "Prompt" && i.bool("Enabled")) else { continue };
                let Some(at) = dm.world_position(id) else { continue };
                let distance = at.distance(position);
                if distance <= prompt.f32("MaxDistance") && nearest.is_none_or(|(_, best)| distance < best) {
                    nearest = Some((id, distance));
                }
            }
        }
        let Some((id, _)) = nearest else {
            self.prompt = None;
            self.prompt_hold = 0.0;
            return;
        };
        let (key, hold) = dm.get(id).map_or((String::new(), 0.0), |p| (p.str("Key").to_string(), p.f32("HoldDuration")));
        let pressed = {
            let input = self.input.borrow();
            !input.ui_captured && input.key(&key)
        };
        if self.prompt.is_none_or(|p| p.prompt != id) {
            self.prompt_hold = 0.0;
        }
        if pressed {
            self.prompt_hold += dt;
            if self.prompt_hold >= hold && !self.prompt_fired {
                self.prompt_fired = true;
                dm.fire(id, "Triggered", vec![player.into()]);
            }
        } else {
            self.prompt_hold = 0.0;
            self.prompt_fired = false;
        }
        let progress = if hold > 0.0 { (self.prompt_hold / hold).clamp(0.0, 1.0) } else { 0.0 };
        self.prompt = Some(ActivePrompt { prompt: id, progress });
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
        rt.scripts.eval("Players.LocalPlayer.Character.Died:Connect(function() shared.died = (shared.died or 0) + 1 end)").unwrap();
        rt.dm.borrow_mut().set(first, "Position", Vec3::new(500.0, 0.0, 0.0)).unwrap();
        run(&mut rt, 2.0);
        assert_eq!(rt.scripts.eval("shared.died").unwrap(), "1");
        run(&mut rt, 3.0);
        let second = rt.local_character().unwrap();
        assert_ne!(first, second);
        assert!(!rt.dm.borrow().contains(first));
        let position = rt.dm.borrow().get(second).unwrap().vec3("Position");
        assert!((position - Vec3::new(10.0, 0.42, 5.0)).length() < 0.2, "{position:?}");
    }

    fn scripted(source: &'static str) -> Runtime {
        let mut dm = scene();
        let holder = dm.service("Scripts");
        let script = dm.create("Script", Some(holder)).unwrap();
        dm.set_name(script, "main").unwrap();
        dm.set(script, "Source", "scripts/main.lua").unwrap();
        let files = ProjectFiles::new(move |path| (path == "scripts/main.lua").then(|| source.to_string()), |_| Some(1));
        Runtime::with_files(dm, Arc::new(AssetManifest::default()), files)
    }

    #[test]
    fn scripts_run_while_playing_and_stop_restores_the_scene() {
        let mut rt = scripted(
            r#"
            local made = Instance.new("Part", workspace)
            made.Name = "Made"
            workspace.Gravity = 5
            Players.PlayerAdded:Connect(function(player) shared.joined = player.Name end)
            Players.LocalPlayer.CharacterAdded:Connect(function(character) shared.spawned = character.ClassName end)
            Audio:Play("audio/coin")
            "#,
        );
        let before = rt.dm.borrow().len();
        assert_eq!(rt.scripts.eval("1 + 1").unwrap(), "2", "the console works while editing");
        rt.start();
        run(&mut rt, 0.2);
        assert_eq!(rt.scripts.eval("shared.joined, shared.spawned").unwrap(), "\"Player\", \"Character\"");
        assert!(rt.dm.borrow().find_by_path("Workspace.Made").is_some());
        assert!(matches!(rt.take_commands().as_slice(), [Command::PlaySound { .. }]));
        let epoch = rt.epoch();
        rt.stop(false);
        assert_eq!(rt.mode(), Mode::Edit);
        assert_eq!(rt.dm.borrow().len(), before, "players, characters and spawned parts are gone");
        assert_eq!(rt.dm.borrow().get(rt.dm.borrow().workspace()).unwrap().number("Gravity"), 32.0);
        assert!(rt.epoch() > epoch);
        assert!(rt.scripts.scripts().is_empty());
        // Play again from the restored scene.
        rt.start();
        run(&mut rt, 0.2);
        assert!(rt.dm.borrow().find_by_path("Workspace.Made").is_some());
    }

    #[test]
    fn stopping_can_keep_what_the_game_changed() {
        let mut rt = scripted("workspace.Gravity = 5");
        rt.start();
        run(&mut rt, 0.1);
        rt.stop(true);
        let dm = rt.dm.borrow();
        assert_eq!(dm.get(dm.workspace()).unwrap().number("Gravity"), 5.0);
        assert!(dm.children(dm.service("Players")).is_empty());
        assert!(!dm.instances().any(|i| i.is_a("Character")));
    }

    #[test]
    fn prompts_trigger_when_the_player_is_near_and_presses_the_key() {
        let mut rt = scripted(
            r#"
            local lever = Instance.new("Part", workspace)
            lever.Position = Vector3.new(10, 1, 5)
            lever.CanCollide = false
            local prompt = Instance.new("Prompt", lever)
            prompt.HoldDuration = 0.5
            prompt.Triggered:Connect(function(player) shared.pulled = (shared.pulled or 0) + 1 end)
            "#,
        );
        rt.start();
        run(&mut rt, 0.3);
        assert_eq!(rt.active_prompt().map(|p| p.progress), Some(0.0));
        rt.input.borrow_mut().keys.insert("E".into());
        run(&mut rt, 0.25);
        let progress = rt.active_prompt().unwrap().progress;
        assert!(progress > 0.3 && progress < 0.7, "{progress}");
        assert_eq!(rt.scripts.eval("shared.pulled").unwrap(), "nil");
        run(&mut rt, 1.0);
        assert_eq!(rt.scripts.eval("shared.pulled").unwrap(), "1", "holding does not re-trigger");
        // Walk away: no prompt.
        let character = rt.local_character().unwrap();
        rt.dm.borrow_mut().set(character, "Position", Vec3::new(-30.0, 1.0, -30.0)).unwrap();
        run(&mut rt, 0.1);
        assert!(rt.active_prompt().is_none());
    }
}
