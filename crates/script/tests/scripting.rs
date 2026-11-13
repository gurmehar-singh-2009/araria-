//! End-to-end tests of the scripting layer running headless.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_core::input::InputState;
use araria_core::{DataModel, InstanceId, Vec3};
use araria_physics::Physics;
use araria_script::{LogLevel, ScriptRuntime};

struct Game {
    dm: Rc<RefCell<DataModel>>,
    physics: Rc<RefCell<Physics>>,
    input: Rc<RefCell<InputState>>,
    sources: Rc<RefCell<HashMap<String, String>>>,
    scripts: ScriptRuntime,
}

impl Game {
    fn new() -> Self {
        let dm = Rc::new(RefCell::new(DataModel::new()));
        let physics = Rc::new(RefCell::new(Physics::new(Arc::new(AssetManifest::default()))));
        let input = Rc::new(RefCell::new(InputState::default()));
        let sources: Rc<RefCell<HashMap<String, String>>> = Rc::default();
        let (read, print) = (sources.clone(), sources.clone());
        let scripts = ScriptRuntime::new(
            dm.clone(),
            physics.clone(),
            input.clone(),
            move |path| read.borrow().get(path).cloned(),
            move |path| print.borrow().get(path).map(|s| s.bytes().fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)))),
        );
        Game { dm, physics, input, sources, scripts }
    }

    /// Adds a script instance backed by `source`.
    fn script(&mut self, name: &str, source: &str) -> InstanceId {
        let path = format!("scripts/{name}.lua");
        self.sources.borrow_mut().insert(path.clone(), source.to_string());
        let mut dm = self.dm.borrow_mut();
        let parent = dm.service("Scripts");
        let id = dm.create("Script", Some(parent)).unwrap();
        dm.set_name(id, name).unwrap();
        dm.set(id, "Source", path).unwrap();
        id
    }

    fn edit(&mut self, name: &str, source: &str) {
        self.sources.borrow_mut().insert(format!("scripts/{name}.lua"), source.to_string());
    }

    /// Runs frames the way the runtime does: scripts, then physics.
    fn run(&mut self, seconds: f32) {
        for _ in 0..(seconds * 60.0).round() as usize {
            self.scripts.update(1.0 / 60.0);
            {
                let mut dm = self.dm.borrow_mut();
                let mut physics = self.physics.borrow_mut();
                physics.sync(&dm);
                dm.clear_journal();
                physics.step(&mut dm, 1.0 / 60.0);
            }
        }
    }

    fn output(&mut self) -> Vec<String> {
        self.scripts.take_log().into_iter().map(|e| e.text).collect()
    }

    fn errors(&mut self) -> Vec<String> {
        self.scripts.take_log().into_iter().filter(|e| e.level == LogLevel::Error).map(|e| e.text).collect()
    }

    fn eval(&mut self, code: &str) -> String {
        self.scripts.eval(code).unwrap_or_else(|e| panic!("{e}"))
    }

    fn count(&self, name: &str) -> usize {
        self.dm.borrow().instances().filter(|i| i.name == name).count()
    }
}

#[test]
fn scripts_create_and_modify_objects() {
    let mut game = Game::new();
    game.script(
        "builder",
        r#"
        local part = Instance.new("Part", workspace)
        part.Name = "Tower"
        part.Position = Vector3.new(1, 2, 3)
        part.Size = Vector3.new(2, 8, 2)
        part.Color = Color3.fromRGB(255, 128, 0)
        part.Material = "Neon"
        part.Rotation = Vector3.new(0, 90, 0)
        part:SetAttribute("Level", 3)
        part:AddTag("landmark")
        local light = Instance.new("Light")
        light.Parent = part
        print(part:GetFullName(), part.ClassName, part.Position.Y, part.Size.Magnitude > 8)
        print(workspace.Tower == part, part:IsA("BasePart"), part:GetAttribute("Level"), #workspace:GetTagged("landmark"))
        print(typeof(part), typeof(part.Position), typeof(part.Color), part.Color:ToHex())
        print(workspace:FindFirstChild("Nope") == nil, #part:GetChildren(), light.Parent.Name)
        "#,
    );
    game.scripts.start_all();
    assert_eq!(
        game.output(),
        vec!["Workspace.Tower Part 2 true", "true true 3 1", "Instance Vector3 Color3 #ff8000", "true 1 Tower"]
    );
    let dm = game.dm.borrow();
    let tower = dm.find_by_path("Workspace.Tower").unwrap();
    assert_eq!(dm.get(tower).unwrap().str("Material"), "Neon");
    assert_eq!(dm.get(tower).unwrap().vec3("Size"), Vec3::new(2.0, 8.0, 2.0));
}

#[test]
fn helpful_errors_do_not_stop_the_game() {
    let mut game = Game::new();
    game.script("broken", "local p = Instance.new('Part', workspace)\np.Colour = 5");
    game.script("typo", "local p = Instance.new('Part', workspace)\np.Position = 'up'");
    game.script("syntax", "local x = = 1");
    game.script("fine", "print('still running')");
    game.script("later", "task.wait(0.1)\nerror('boom')");
    game.scripts.start_all();
    game.run(0.3);
    let log = game.scripts.take_log();
    let errors: Vec<&str> = log.iter().filter(|e| e.level == LogLevel::Error).map(|e| e.text.as_str()).collect();
    assert!(errors.iter().any(|e| e.contains("scripts/broken.lua:2: Colour is not a valid property of Part")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("scripts/typo.lua:2: Position: Vector3 expected, got string")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("scripts/syntax.lua:1:")), "{errors:?}");
    assert!(errors.iter().any(|e| e.contains("scripts/later.lua:2: boom") && e.contains("stack traceback")), "{errors:?}");
    assert!(log.iter().any(|e| e.text == "still running" && e.source.as_deref() == Some("fine")));
    // Errors carry a location the editor can jump to.
    let located = log.iter().find(|e| e.text.contains("Colour")).unwrap();
    assert_eq!(located.location, Some(("scripts/broken.lua".to_string(), 2)));
    let info = game.scripts.scripts();
    assert!(info.iter().find(|s| s.name == "syntax").unwrap().error.is_some());
    assert!(info.iter().find(|s| s.name == "fine").unwrap().running);
}

#[test]
fn waits_events_and_heartbeat() {
    let mut game = Game::new();
    game.script(
        "timing",
        r#"
        local frames = 0
        RunService.Heartbeat:Connect(function(dt) frames = frames + 1 end)
        local elapsed = task.wait(0.5)
        print("waited", elapsed >= 0.5 and elapsed < 0.6, frames >= 29)
        task.delay(0.2, function(msg) print(msg) end, "delayed")
        task.spawn(function() task.wait(0.1) print("spawned") end)
        local custom = Signal.new()
        custom:Once(function(a, b) print("custom", a, b) end)
        custom:Fire(1, 2)
        custom:Fire(3, 4)
        task.spawn(function() print("got", custom:Wait()) end)
        custom:Fire("x")
        "#,
    );
    game.scripts.start_all();
    game.run(1.0);
    assert_eq!(game.output(), vec!["waited true true", "custom 1 2", "got x", "spawned", "delayed"]);
}

#[test]
fn engine_events_reach_scripts() {
    let mut game = Game::new();
    game.script(
        "events",
        r#"
        local floor = Instance.new("Part", workspace)
        floor.Size = Vector3.new(50, 1, 50)
        floor.Position = Vector3.new(0, -0.5, 0)
        local ball = Instance.new("Part", workspace)
        ball.Name = "Ball"
        ball.Shape = "Sphere"
        ball.Anchored = false
        ball.Position = Vector3.new(0, 4, 0)
        ball.Touched:Connect(function(other) print("touched", other.Name) end)
        workspace.ChildAdded:Connect(function(child) print("added", child.Name) end)
        ball:GetPropertyChangedSignal("Color"):Connect(function() print("recolored") end)
        task.wait(0.2)
        ball.Color = Color3.new(1, 0, 0)
        local extra = Instance.new("Folder", workspace)
        local hit = workspace:Raycast(Vector3.new(5, 10, 5), Vector3.new(0, -50, 0))
        print("ray", hit.Instance == floor, hit.Position.Y, hit.Normal.Y)
        "#,
    );
    game.scripts.start_all();
    game.run(2.0);
    let output = game.output();
    assert!(output.contains(&"touched Part".to_string()), "{output:?}");
    assert!(output.contains(&"added Folder".to_string()), "{output:?}");
    assert!(output.contains(&"recolored".to_string()), "{output:?}");
    assert!(output.contains(&"ray true 0 1".to_string()), "{output:?}");
}

#[test]
fn input_and_tweens() {
    let mut game = Game::new();
    game.script(
        "controls",
        r#"
        local door = Instance.new("Part", workspace)
        door.Name = "Door"
        Input.KeyDown:Connect(function(key)
            if key == "E" then
                local tween = TweenService:Create(door, TweenInfo.new(0.5, "Linear"), {Position = Vector3.new(0, 10, 0), Transparency = 1})
                tween.Completed:Connect(function() print("opened", door.Position.Y) end)
                tween:Play()
            end
        end)
        "#,
    );
    game.scripts.start_all();
    game.run(0.1);
    game.input.borrow_mut().keys.insert("E".into());
    game.run(0.25);
    let halfway = game.dm.borrow().get(game.dm.borrow().find_by_path("Workspace.Door").unwrap()).unwrap().vec3("Position").y;
    assert!((halfway - 5.0).abs() < 0.6, "{halfway}");
    game.run(0.5);
    assert_eq!(game.output(), vec!["opened 10"]);
    assert_eq!(game.eval("Input:IsKeyDown('E'), Input:IsKeyDown('Q')"), "true, false");
}

#[test]
fn hot_reload_replaces_behaviour_without_duplicates_or_restarts() {
    let mut game = Game::new();
    game.scripts.auto_reload = false;
    // A second script whose state must survive the first one's reload.
    game.script("counter", "local n = 0\nRunService.Heartbeat:Connect(function() n = n + 1 shared.frames = n end)");
    game.script(
        "spawner",
        r#"
        state.runs = (state.runs or 0) + 1
        local marker = Instance.new("Part", workspace)
        marker.Name = "Marker"
        RunService.Heartbeat:Connect(function() shared.version = "one" shared.ticks = (shared.ticks or 0) + 1 end)
        task.spawn(function() while true do task.wait(0.1) shared.loops = (shared.loops or 0) + 1 end end)
        "#,
    );
    game.scripts.start_all();
    game.run(1.0);
    assert_eq!(game.eval("shared.version, shared.ticks, shared.frames"), "\"one\", 60, 60");
    assert_eq!(game.count("Marker"), 1);

    // Edit the script while the game runs.
    game.edit(
        "spawner",
        r#"
        state.runs = (state.runs or 0) + 1
        shared.runs = state.runs
        local marker = Instance.new("Part", workspace)
        marker.Name = "Marker"
        RunService.Heartbeat:Connect(function() shared.version = "two" shared.ticks = shared.ticks + 1 end)
        "#,
    );
    assert_eq!(game.scripts.reload("scripts/spawner.lua"), Ok(1));
    game.run(1.0);
    // New behaviour is live, the old listener and loop are gone (ticks
    // advanced by exactly one per frame, loops stopped counting)...
    assert_eq!(game.eval("shared.version, shared.ticks, shared.runs"), "\"two\", 120, 2");
    let loops_after_reload = game.eval("shared.loops");
    game.run(0.5);
    assert_eq!(game.eval("shared.loops"), loops_after_reload);
    // ...objects the old version spawned were cleaned up, not duplicated...
    assert_eq!(game.count("Marker"), 1);
    // ...and the rest of the game never restarted.
    assert_eq!(game.eval("shared.frames"), "150");

    // A syntax error keeps the working version running.
    game.edit("spawner", "this is not lua");
    assert!(game.scripts.reload("scripts/spawner.lua").is_err());
    game.run(0.5);
    assert_eq!(game.eval("shared.version, shared.ticks"), "\"two\", 180");
    let errors = game.errors();
    assert!(errors.iter().any(|e| e.contains("keeping the previous version running")), "{errors:?}");
}

#[test]
fn file_changes_are_detected_automatically() {
    let mut game = Game::new();
    game.script("mood", "shared.mood = 'calm'");
    game.scripts.start_all();
    game.run(0.2);
    game.edit("mood", "shared.mood = 'excited'");
    game.run(1.0);
    assert_eq!(game.eval("shared.mood"), "\"excited\"");
    assert!(game.output().iter().any(|l| l == "Reloaded mood"));
}

#[test]
fn modules_are_shared_and_reload_their_users() {
    let mut game = Game::new();
    game.scripts.auto_reload = false;
    game.sources.borrow_mut().insert("scripts/config.lua".into(), "local M = { speed = 5 }\nfunction M.double(x) return x * 2 end\nreturn M".into());
    game.script("a", "local config = require('config')\nshared.a = config.double(config.speed)\nconfig.touched = true");
    game.script("b", "local config = require('scripts/config.lua')\nshared.same = config.touched == true");
    game.scripts.start_all();
    assert_eq!(game.eval("shared.a, shared.same"), "10, true");
    game.sources.borrow_mut().insert("scripts/config.lua".into(), "return { speed = 50, double = function(x) return x * 2 end }".into());
    assert_eq!(game.scripts.reload("scripts/config.lua"), Ok(2));
    assert_eq!(game.eval("shared.a"), "100");
    game.script("missing", "require('nowhere')");
    game.scripts.update(0.016);
    assert!(game.errors().iter().any(|e| e.contains("module 'scripts/nowhere.lua' was not found")));
}

#[test]
fn scripts_are_isolated_and_limited() {
    let mut game = Game::new();
    game.script("one", "secret = 42\nlocal hidden = 1");
    game.script("two", "print(secret, hidden, io, os.execute, load, loadstring, require ~= nil)");
    game.script("runaway", "while true do end");
    game.script("after", "print('after runaway')");
    game.scripts.start_all();
    let log = game.scripts.take_log();
    let text: Vec<&str> = log.iter().map(|e| e.text.as_str()).collect();
    assert!(text.contains(&"nil nil nil nil nil nil true"), "{text:?}");
    assert!(text.iter().any(|t| t.contains("exceeded its execution budget")), "{text:?}");
    assert!(text.contains(&"after runaway"));
    // A runaway event handler is stopped each time, the game keeps going.
    game.script("spin", "RunService.Heartbeat:Connect(function() while true do end end)");
    game.run(0.05);
    assert!(game.errors().len() >= 2);
    assert_eq!(game.eval("1 + 1"), "2");
}

#[test]
fn stopping_and_starting_scripts() {
    let mut game = Game::new();
    game.script("pulse", "RunService.Heartbeat:Connect(function() shared.pulses = (shared.pulses or 0) + 1 end)");
    game.scripts.start_all();
    game.run(0.5);
    assert_eq!(game.scripts.stop("pulse"), 1);
    let frozen = game.eval("shared.pulses");
    game.run(0.5);
    assert_eq!(game.eval("shared.pulses"), frozen);
    assert_eq!(game.scripts.start("pulse"), 1);
    game.run(0.5);
    assert_eq!(game.eval("shared.pulses"), "60");

    // Destroying the Script instance stops it; adding one starts it.
    game.eval("Scripts.pulse:Destroy()");
    game.run(0.2);
    assert!(game.scripts.scripts().is_empty());
    game.script("late", "shared.late = true");
    game.run(0.1);
    assert_eq!(game.eval("shared.late"), "true");
}

#[test]
fn characters_follow_script_commands() {
    let mut game = Game::new();
    game.script(
        "npc",
        r#"
        local floor = Instance.new("Part", workspace)
        floor.Size = Vector3.new(100, 1, 100)
        floor.Position = Vector3.new(0, -0.5, 0)
        local npc = Instance.new("Character", workspace)
        npc.Name = "Guide"
        npc.Position = Vector3.new(0, 0.2, 0)
        npc.MoveToFinished:Connect(function(reached) print("arrived", reached, math.floor(npc.Position.X + 0.5)) end)
        npc.Died:Connect(function() print("died") end)
        task.wait(0.5)
        npc:MoveTo(Vector3.new(6, 0, 0))
        npc.MoveToFinished:Wait()
        npc:PlayAnimation("Wave")
        print(npc.Emote, npc.State)
        npc:TakeDamage(30)
        print(npc.Health)
        "#,
    );
    game.scripts.start_all();
    game.run(3.0);
    assert_eq!(game.output(), vec!["arrived true 6", "Wave Idle", "70"]);
}

#[test]
fn saved_values_round_trip() {
    let mut game = Game::new();
    assert_eq!(game.eval("Save:Get('best', 0)"), "0");
    game.eval("Save:Set('best', {score = 12, name = 'ann', tags = {'a', 'b'}})");
    assert_eq!(game.eval("Save:Get('best').score, Save:Get('best').tags[2]"), "12, \"b\"");
    assert!(game.scripts.eval("Save:Set('bad', print)").is_err());
}

#[test]
fn console_evaluates_in_the_game() {
    let mut game = Game::new();
    assert_eq!(game.eval("workspace.Name, #workspace:GetChildren()"), "\"Workspace\", 1");
    game.eval("x = Instance.new('Part', workspace); x.Name = 'FromConsole'");
    assert_eq!(game.eval("x:GetFullName()"), "\"Workspace.FromConsole\"");
    assert_eq!(game.eval("{1, 2, a = 'b'}"), "{1, 2, a = b}");
    assert_eq!(game.eval("Vector3.new(1, 2, 3) * 2, CFrame.new(1, 2, 3).Position.Z"), "2, 4, 6, 3");
    let error = game.scripts.eval("nothing.here").unwrap_err();
    assert!(error.contains("attempt to index nil with 'here'"), "{error}");
    assert!(game.scripts.eval("1 +").is_err());
}
