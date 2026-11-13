//! # Araria scripting
//!
//! Connects the Lua VM to a running game: the object API (`Instance`,
//! `Workspace`, `Players`, ...), value types, events, the task scheduler,
//! tweens, and script lifecycle management with live hot reloading.
//!
//! Each `Script` instance runs in its own environment (its globals do not
//! leak into other scripts) and its own coroutine. Everything a script
//! registers — event connections, waiting coroutines, tweens, instances it
//! created — is tracked, so a script can be stopped or reloaded without
//! restarting the game and without leaving duplicate listeners behind.

mod globals;
pub mod host;
mod instance;
mod signal;
pub mod tween;
mod types;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use araria_core::input::InputState;
use araria_core::{Change, DataModel, InstanceId};
use araria_lua::{Error, Lua, Ret, TableRef, Value};
use araria_physics::Physics;

pub use host::{Command, Host, HostState, LogEntry, LogLevel, SaveBackend, ScriptId};
use host::SignalKey;

/// How often (in seconds) script files are checked for outside changes.
const CHANGE_CHECK_INTERVAL: f32 = 0.5;
/// Most coroutine resumes processed from the deferred queue per frame.
const MAX_DEFERRED_PER_FRAME: usize = 10_000;

const WAIT_FOR_CHILD: &str = r#"
return function(self, name, timeout)
    local deadline = time() + (timeout or 5)
    while true do
        local child = self:FindFirstChild(name)
        if child then return child end
        if time() >= deadline then
            warn("WaitForChild: '" .. tostring(name) .. "' did not appear in " .. self:GetFullName())
            return nil
        end
        task.wait()
    end
end
"#;

/// What the editor and terminal need to know about a script.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptInfo {
    pub id: ScriptId,
    pub name: String,
    /// Project path of the source file.
    pub path: String,
    pub instance: InstanceId,
    pub running: bool,
    /// The error that stopped the script from loading, if any.
    pub error: Option<String>,
}

struct ScriptState {
    info: ScriptInfo,
    /// Survives reloads: scripts keep data they want to preserve here.
    persistent: TableRef,
}

pub struct ScriptRuntime {
    lua: Lua,
    pub host: Rc<Host>,
    scripts: Vec<ScriptState>,
    next_script: ScriptId,
    fingerprint: Box<dyn Fn(&str) -> Option<u64>>,
    fingerprints: HashMap<String, u64>,
    change_timer: f32,
    previous_keys: HashSet<String>,
    previous_buttons: [bool; 3],
    console_env: TableRef,
    heartbeat: SignalKey,
    /// Reload scripts automatically when their files change.
    pub auto_reload: bool,
}

impl ScriptRuntime {
    /// Creates the scripting runtime for a game. `read_source` loads a
    /// script by project path; `fingerprint` reports a value that changes
    /// when that file changes.
    pub fn new(
        dm: Rc<RefCell<DataModel>>,
        physics: Rc<RefCell<Physics>>,
        input: Rc<RefCell<InputState>>,
        read_source: impl Fn(&str) -> Option<String> + 'static,
        fingerprint: impl Fn(&str) -> Option<u64> + 'static,
    ) -> Self {
        let root = dm.borrow().root();
        let host = Rc::new(Host {
            dm,
            physics,
            input,
            state: RefCell::new(HostState::default()),
            save: RefCell::new(None),
            methods: RefCell::new(HashMap::new()),
            signal_methods: RefCell::new(None),
            shared_env: RefCell::new(None),
            read_source: RefCell::new(Box::new(read_source)),
        });
        let mut lua = Lua::new();
        // Scripts must not reach the unsandboxed loaders.
        for name in ["load", "loadstring"] {
            lua.set_global(name, Value::Nil);
        }
        types::register(&mut lua);
        signal::register(&mut lua, &host);
        instance::register(&host);
        globals::register(&mut lua, &host);
        register_require(&mut lua, &host);
        match lua.exec(WAIT_FOR_CHILD, "builtin") {
            Ok(values) => {
                host.methods.borrow_mut().insert("WaitForChild", values.into_iter().next().unwrap_or(Value::Nil));
            }
            Err(e) => host.log(LogLevel::Error, format!("internal: {}", e.message())),
        }
        let console_env = globals::script_env(&mut lua);
        host.state.borrow_mut().script_names.insert(0, ("console".into(), "console".into()));
        ScriptRuntime {
            lua,
            host,
            scripts: Vec::new(),
            next_script: 1,
            fingerprint: Box::new(fingerprint),
            fingerprints: HashMap::new(),
            change_timer: 0.0,
            previous_keys: HashSet::new(),
            previous_buttons: [false; 3],
            console_env,
            heartbeat: (root.0, Rc::from("Heartbeat")),
            auto_reload: true,
        }
    }

    pub fn set_save_backend(&mut self, backend: Box<dyn SaveBackend>) {
        *self.host.save.borrow_mut() = Some(backend);
    }

    // ------------------------------------------------------- lifecycle

    /// Whether a script instance should be running.
    fn eligible(dm: &DataModel, id: InstanceId) -> bool {
        let Some(instance) = dm.get(id) else { return false };
        instance.class_name() == "Script" && instance.bool("Enabled") && !instance.str("Source").is_empty() && !dm.is_descendant_of(id, dm.service("Storage"))
    }

    /// Starts every enabled script in the game.
    pub fn start_all(&mut self) {
        let ids: Vec<InstanceId> = {
            let dm = self.host.dm.borrow();
            dm.descendants(dm.root()).into_iter().filter(|id| Self::eligible(&dm, *id)).collect()
        };
        for id in ids {
            self.add_script(id);
        }
    }

    fn add_script(&mut self, instance: InstanceId) {
        if self.scripts.iter().any(|s| s.info.instance == instance) {
            return;
        }
        let (name, path) = {
            let dm = self.host.dm.borrow();
            let Some(script) = dm.get(instance) else { return };
            (script.name.clone(), script.str("Source").to_string())
        };
        let id = self.next_script;
        self.next_script += 1;
        self.host.state.borrow_mut().script_names.insert(id, (name.clone(), path.clone()));
        let persistent = self.lua.new_table();
        self.scripts.push(ScriptState { info: ScriptInfo { id, name, path, instance, running: false, error: None }, persistent });
        self.run_script(self.scripts.len() - 1);
    }

    /// Loads and starts a script from its current source.
    fn run_script(&mut self, index: usize) {
        let (id, path, instance, persistent) = {
            let s = &self.scripts[index];
            (s.info.id, s.info.path.clone(), s.info.instance, s.persistent.clone())
        };
        let source = (self.host.read_source.borrow())(&path);
        let Some(source) = source else {
            let message = format!("script file '{path}' was not found");
            self.scripts[index].info.error = Some(message.clone());
            self.host.log(LogLevel::Error, message);
            return;
        };
        if let Some(print) = (self.fingerprint)(&path) {
            self.fingerprints.insert(path.clone(), print);
        }
        let env = globals::script_env(&mut self.lua);
        env.set_str("script", instance::instance_value(&self.host, instance));
        env.set_str("state", persistent);
        match self.lua.load(&source, &path, Some(env)) {
            Ok(main) => {
                self.scripts[index].info.error = None;
                self.scripts[index].info.running = true;
                let thread = self.lua.create_thread(main);
                if !self.host.run_thread(&mut self.lua, &thread, Vec::new(), id) {
                    self.scripts[index].info.error = Some("stopped by an error (see output)".into());
                }
            }
            Err(error) => {
                self.scripts[index].info.error = Some(error.message());
                self.scripts[index].info.running = false;
                self.host.log_error(id, &error);
            }
        }
    }

    /// Stops a script and releases everything it registered or created.
    fn halt_script(&mut self, index: usize) {
        let id = self.scripts[index].info.id;
        self.host.release_script(&mut self.lua, id);
        let created: Vec<InstanceId> = self.host.dm.borrow().instances().filter(|i| i.creator == id).map(|i| i.id).collect();
        {
            let mut dm = self.host.dm.borrow_mut();
            for instance in created {
                if dm.contains(instance) {
                    let _ = dm.destroy(instance);
                }
            }
        }
        self.scripts[index].info.running = false;
    }

    fn find(&self, name_or_path: &str) -> Vec<usize> {
        self.scripts.iter().enumerate().filter(|(_, s)| s.info.name == name_or_path || s.info.path == name_or_path).map(|(i, _)| i).collect()
    }

    pub fn scripts(&self) -> Vec<ScriptInfo> {
        self.scripts.iter().map(|s| s.info.clone()).collect()
    }

    /// Stops scripts by name or path. Returns how many were stopped.
    pub fn stop(&mut self, name_or_path: &str) -> usize {
        let targets: Vec<usize> = self.find(name_or_path).into_iter().filter(|i| self.scripts[*i].info.running).collect();
        for index in &targets {
            self.halt_script(*index);
        }
        targets.len()
    }

    /// Starts stopped scripts by name or path. Returns how many started.
    pub fn start(&mut self, name_or_path: &str) -> usize {
        let targets: Vec<usize> = self.find(name_or_path).into_iter().filter(|i| !self.scripts[*i].info.running).collect();
        for index in &targets {
            self.run_script(*index);
        }
        targets.len()
    }

    /// Stops every script (when leaving play mode).
    pub fn stop_all(&mut self) {
        for index in 0..self.scripts.len() {
            if self.scripts[index].info.running {
                self.halt_script(index);
            }
        }
    }

    // ------------------------------------------------------ hot reload

    /// Reloads a script or module file in the running game.
    ///
    /// The new source is compiled first: if it has a syntax error the old
    /// version keeps running and the error is returned. Otherwise every
    /// script using the file is stopped (its listeners, coroutines, tweens
    /// and spawned instances are cleaned up) and started again; the rest of
    /// the game — other scripts, physics, players — is untouched, and each
    /// script's `state` table is preserved.
    pub fn reload(&mut self, path: &str) -> Result<usize, String> {
        let Some(source) = (self.host.read_source.borrow())(path) else {
            return Err(format!("'{path}' was not found"));
        };
        if let Some(print) = (self.fingerprint)(path) {
            self.fingerprints.insert(path.to_string(), print);
        }
        if let Err(error) = araria_lua::compiler::compile(&source, path) {
            self.host.state.borrow_mut().current_script = 0;
            let mut state = self.host.state.borrow_mut();
            let time = state.time;
            state.log.push_back(LogEntry {
                level: LogLevel::Error,
                text: format!("{} — keeping the previous version running", error.message()),
                source: Some(path.to_string()),
                location: error.line.map(|line| (path.to_string(), line)),
                time,
            });
            return Err(error.message());
        }
        // Scripts running this file, plus scripts that required it as a module.
        let mut affected: HashSet<ScriptId> = self.scripts.iter().filter(|s| s.info.path == path).map(|s| s.info.id).collect();
        {
            let mut state = self.host.state.borrow_mut();
            if state.modules.remove(path).is_some() {
                affected.extend(state.module_users.remove(path).unwrap_or_default());
            }
        }
        let targets: Vec<usize> = self.scripts.iter().enumerate().filter(|(_, s)| affected.contains(&s.info.id)).map(|(i, _)| i).collect();
        for index in &targets {
            self.halt_script(*index);
        }
        for index in &targets {
            self.run_script(*index);
        }
        let names: Vec<String> = targets.iter().map(|i| self.scripts[*i].info.name.clone()).collect();
        if !names.is_empty() {
            self.host.state.borrow_mut().current_script = 0;
            self.host.log(LogLevel::Info, format!("Reloaded {}", names.join(", ")));
        }
        Ok(targets.len())
    }

    /// Reloads every script.
    pub fn reload_all(&mut self) -> usize {
        self.host.state.borrow_mut().modules.clear();
        let mut paths: Vec<String> = self.scripts.iter().map(|s| s.info.path.clone()).collect();
        paths.sort();
        paths.dedup();
        paths.iter().filter_map(|p| self.reload(p).ok()).sum()
    }

    /// Reloads scripts whose files changed since they were loaded. Returns
    /// the paths that were reloaded.
    pub fn check_for_changes(&mut self) -> Vec<String> {
        let mut watched: Vec<String> = self.scripts.iter().map(|s| s.info.path.clone()).collect();
        watched.extend(self.host.state.borrow().modules.keys().cloned());
        watched.sort();
        watched.dedup();
        let mut changed = Vec::new();
        for path in watched {
            let current = (self.fingerprint)(&path);
            let known = self.fingerprints.get(&path).copied();
            if current.is_some() && current != known {
                if let Some(print) = current {
                    self.fingerprints.insert(path.clone(), print);
                }
                // A syntax error keeps the old version; it is reported in the log.
                let _ = self.reload(&path);
                changed.push(path);
            }
        }
        changed
    }

    // ----------------------------------------------------------- frame

    fn fire(&mut self, key: &SignalKey, args: &[Value]) {
        if self.host.has_listeners(key) {
            self.host.fire(&mut self.lua, key, args);
        }
    }

    /// Runs one frame of scripting: new and removed scripts, input and
    /// engine events, `Heartbeat`, sleeping coroutines and tweens.
    pub fn update(&mut self, dt: f32) {
        {
            let mut state = self.host.state.borrow_mut();
            state.time += f64::from(dt);
            state.dt = f64::from(dt);
        }
        self.track_script_instances();
        if self.auto_reload {
            self.change_timer += dt;
            if self.change_timer >= CHANGE_CHECK_INTERVAL {
                self.change_timer = 0.0;
                self.check_for_changes();
            }
        }
        self.dispatch_input();
        self.dispatch_engine_events();
        let heartbeat = self.heartbeat.clone();
        self.fire(&heartbeat, &[Value::Num(f64::from(dt))]);
        self.wake_sleepers();
        self.step_tweens(dt);
        instance::drive_characters(&self.host);
        self.run_deferred();
        // Events raised by this frame's script activity (ChildAdded, ...).
        self.dispatch_engine_events();
        self.host.state.borrow_mut().instances.retain(|id, _| self.host.dm.borrow().contains(*id));
    }

    fn track_script_instances(&mut self) {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        {
            let dm = self.host.dm.borrow();
            for change in dm.journal() {
                match change {
                    Change::Added(id) | Change::Reparented(id) | Change::Prop(id, _) => {
                        if dm.get(*id).is_some_and(|i| i.class_name() == "Script") {
                            if Self::eligible(&dm, *id) { added.push(*id) } else { removed.push(*id) }
                        }
                    }
                    Change::Removed(id) => removed.push(*id),
                    _ => {}
                }
            }
        }
        for id in removed {
            if let Some(index) = self.scripts.iter().position(|s| s.info.instance == id) {
                self.halt_script(index);
                let script = self.scripts.remove(index);
                self.host.state.borrow_mut().script_names.remove(&script.info.id);
            }
        }
        for id in added {
            self.add_script(id);
        }
    }

    fn dispatch_input(&mut self) {
        let root = self.heartbeat.0;
        let (keys, buttons, position, wheel, captured) = {
            let input = self.host.input.borrow();
            (input.keys.clone(), input.mouse_buttons, input.mouse_position, input.wheel, input.ui_captured)
        };
        let keys = if captured { HashSet::new() } else { keys };
        let pressed: Vec<String> = keys.difference(&self.previous_keys).cloned().collect();
        let released: Vec<String> = self.previous_keys.difference(&keys).cloned().collect();
        for key in pressed {
            self.fire(&(root, Rc::from("KeyDown")), &[Value::from(key)]);
        }
        for key in released {
            self.fire(&(root, Rc::from("KeyUp")), &[Value::from(key)]);
        }
        for (i, (now, before)) in buttons.iter().zip(self.previous_buttons).enumerate() {
            if *now != before {
                let name = if *now { "MouseButtonDown" } else { "MouseButtonUp" };
                let args = [Value::Int(i as i64 + 1), Lua::userdata(types::Vector2(position))];
                self.fire(&(root, Rc::from(name)), &args);
            }
        }
        if wheel != 0.0 {
            self.fire(&(root, Rc::from("MouseWheel")), &[Value::Num(f64::from(wheel))]);
        }
        self.previous_keys = keys;
        self.previous_buttons = buttons;
    }

    fn dispatch_engine_events(&mut self) {
        // Handlers can raise more events; a few rounds settle them.
        for _ in 0..4 {
            let events = self.host.dm.borrow_mut().take_events();
            if events.is_empty() {
                break;
            }
            for event in events {
                let key: SignalKey = (event.target.0, Rc::from(event.name.as_str()));
                let specific = (event.name == "Changed").then(|| event.args.first().and_then(|a| a.as_str()).map(|p| (event.target.0, Rc::from(format!("Changed:{p}").as_str())))).flatten();
                if !self.host.has_listeners(&key) && !specific.as_ref().is_some_and(|k| self.host.has_listeners(k)) {
                    continue;
                }
                let args: Vec<Value> = event.args.iter().map(|a| types::variant_to_value(&self.host, a)).collect();
                self.fire(&key, &args);
                if let Some(specific) = specific {
                    self.fire(&specific, &[]);
                }
            }
        }
    }

    fn wake_sleepers(&mut self) {
        let due = {
            let mut state = self.host.state.borrow_mut();
            let now = state.time;
            let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut state.sleeping).into_iter().partition(|s| s.wake <= now);
            state.sleeping = waiting;
            due
        };
        let now = self.host.state.borrow().time;
        for sleeper in due {
            let args = sleeper.args.unwrap_or_else(|| vec![Value::Num(now - sleeper.started)]);
            self.host.run_thread(&mut self.lua, &sleeper.thread, args, sleeper.script);
        }
    }

    fn step_tweens(&mut self, dt: f32) {
        let finished: Vec<u64> = {
            let mut state = self.host.state.borrow_mut();
            let mut dm = self.host.dm.borrow_mut();
            let mut finished = Vec::new();
            for tween in &mut state.tweens {
                if tween.step(&mut dm, dt) {
                    finished.push(tween.id);
                }
            }
            state.tweens.retain(|t| !finished.contains(&t.id));
            finished
        };
        for id in finished {
            self.fire(&((1 << 61) | id, Rc::from("Completed")), &[]);
        }
    }

    fn run_deferred(&mut self) {
        for _ in 0..MAX_DEFERRED_PER_FRAME {
            let next = self.host.state.borrow_mut().ready.pop_front();
            let Some((thread, args, script)) = next else { break };
            self.host.run_thread(&mut self.lua, &thread, args, script);
        }
    }

    // --------------------------------------------------------- console

    /// Runs Lua typed into the terminal, in the game's context. Expressions
    /// are evaluated and their values returned as text.
    pub fn eval(&mut self, code: &str) -> Result<String, String> {
        let env = Some(self.console_env.clone());
        let chunk = match self.lua.load(&format!("return {code}"), "console", env.clone()) {
            Ok(f) => f,
            Err(_) => self.lua.load(code, "console", env).map_err(|e| e.message())?,
        };
        self.host.state.borrow_mut().current_script = 0;
        self.lua.set_budget(host::RESUME_BUDGET);
        let values = self.lua.call(&chunk, &[]).map_err(|e| e.report())?;
        let mut parts = Vec::new();
        for value in &values {
            parts.push(self.describe(value));
        }
        Ok(parts.join(", "))
    }

    /// A readable one-line description of a value (tables are expanded one level).
    fn describe(&mut self, value: &Value) -> String {
        match value {
            Value::Str(s) => format!("\"{}\"", s.to_str()),
            Value::Table(t) => {
                let entries: Vec<(Value, Value)> = t.borrow().iter().take(13).collect();
                let mut parts: Vec<String> = entries
                    .iter()
                    .take(12)
                    .map(|(k, v)| {
                        let v = self.lua.tostring(v).unwrap_or_else(|_| "?".into());
                        match k {
                            Value::Int(_) => v,
                            other => format!("{} = {v}", other.to_display()),
                        }
                    })
                    .collect();
                if entries.len() > 12 {
                    parts.push("…".into());
                }
                format!("{{{}}}", parts.join(", "))
            }
            other => self.lua.tostring(other).unwrap_or_else(|e| e.message()),
        }
    }

    pub fn log(&self, level: LogLevel, text: impl Into<String>) {
        self.host.state.borrow_mut().current_script = 0;
        self.host.log(level, text);
    }

    /// Removes and returns the output produced since the last call.
    pub fn take_log(&mut self) -> Vec<LogEntry> {
        self.host.state.borrow_mut().log.drain(..).collect()
    }

    /// Removes and returns requests for the presentation layer.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.host.state.borrow_mut().commands)
    }

    /// Frees unreachable Lua objects; call occasionally (it is cheap when
    /// little has been allocated).
    pub fn collect_garbage(&mut self) -> araria_lua::GcStats {
        self.lua.collect_garbage()
    }

    /// Scheduler statistics for the terminal: (sleeping, connections, tweens).
    pub fn stats(&self) -> (usize, usize, usize) {
        let state = self.host.state.borrow();
        (state.sleeping.len(), state.connections.values().map(Vec::len).sum(), state.tweens.len())
    }
}

/// `require("name")`, `require("scripts/name.lua")` or `require(moduleScript)`.
fn register_require(lua: &mut Lua, host: &Rc<Host>) {
    let h = host.clone();
    lua.set_global(
        "require",
        Lua::native("require", move |lua, args| {
            let path = match args.first() {
                Some(Value::Str(name)) => {
                    let name = name.to_str();
                    if name.ends_with(".lua") { name.into_owned() } else { format!("scripts/{name}.lua") }
                }
                Some(other) => match other.as_userdata::<instance::InstanceRef>() {
                    Some(module) => {
                        let dm = h.dm.borrow();
                        let instance = dm.get(module.id).ok_or_else(|| Error::runtime("the module has been destroyed"))?;
                        if instance.class_name() != "ModuleScript" {
                            return Err(Error::runtime("require expects a ModuleScript or a module name"));
                        }
                        instance.str("Source").to_string()
                    }
                    None => return Err(Error::runtime("require expects a ModuleScript or a module name")),
                },
                None => return Err(Error::runtime("require expects a ModuleScript or a module name")),
            };
            let user = h.current_script();
            h.state.borrow_mut().module_users.entry(path.clone()).or_default().insert(user);
            if let Some(cached) = h.state.borrow().modules.get(&path) {
                return Ok(Ret::One(cached.clone()));
            }
            let source = (h.read_source.borrow())(&path).ok_or_else(|| Error::runtime(format!("module '{path}' was not found")))?;
            let env = globals::script_env(lua);
            let chunk = lua.load(&source, &path, Some(env))?;
            let value = lua.call1(&chunk, &[])?;
            h.state.borrow_mut().modules.insert(path, value.clone());
            Ok(Ret::One(value))
        }),
    );
}
