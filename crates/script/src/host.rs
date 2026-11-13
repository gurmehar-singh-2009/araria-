//! State shared between the scripting runtime and the native functions it
//! exposes to Lua.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use araria_core::input::InputState;
use araria_core::{DataModel, InstanceId, Vec3};
use araria_lua::{Error, Lua, Resumed, TableRef, ThreadRef, Value};
use araria_physics::Physics;

pub type ScriptId = u32;
/// Loop iterations and calls one resume of a script may use before it is
/// stopped as runaway.
pub const RESUME_BUDGET: i64 = 4_000_000;
const MAX_LOG: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// One line of script output.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level: LogLevel,
    pub text: String,
    /// Name of the script that produced it, if any.
    pub source: Option<String>,
    /// Project path and line of an error, for jumping to it in the editor.
    pub location: Option<(String, u32)>,
    pub time: f64,
}

/// A request from scripts to the presentation layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    PlaySound { asset: String, volume: f32, pitch: f32, position: Option<Vec3> },
    PlayMusic { asset: String, volume: f32 },
    StopMusic,
    EmitParticles { emitter: InstanceId, count: u32 },
    CameraShake { intensity: f32, duration: f32 },
    Notify { player: Option<InstanceId>, text: String, duration: f32 },
    LoadCharacter { player: InstanceId },
}

/// Identifies an event: an instance (or a synthetic id) plus an event name.
pub type SignalKey = (u64, Rc<str>);

pub struct Connection {
    pub id: u64,
    pub func: Value,
    pub script: ScriptId,
    pub once: bool,
}

pub struct Sleeper {
    pub wake: f64,
    pub started: f64,
    pub thread: ThreadRef,
    pub script: ScriptId,
    /// Arguments for `task.delay`; `None` resumes with the elapsed time.
    pub args: Option<Vec<Value>>,
}

/// Where a script-driven character is heading.
#[derive(Clone, Copy)]
pub enum MoveGoal {
    Direction { dir: Vec3, run: bool },
    Point { target: Vec3, run: bool, deadline: f64 },
}

/// Persists game progress (`Save:Get` / `Save:Set`). Values are JSON text.
pub trait SaveBackend {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: Option<&str>);
}

#[derive(Default)]
pub struct HostState {
    pub time: f64,
    pub dt: f64,
    pub current_script: ScriptId,
    pub script_names: HashMap<ScriptId, (String, String)>,
    pub log: VecDeque<LogEntry>,
    pub connections: HashMap<SignalKey, Vec<Connection>>,
    pub waiters: HashMap<SignalKey, Vec<(ThreadRef, ScriptId)>>,
    pub next_id: u64,
    pub sleeping: Vec<Sleeper>,
    pub ready: VecDeque<(ThreadRef, Vec<Value>, ScriptId)>,
    pub instances: HashMap<InstanceId, Value>,
    pub commands: Vec<Command>,
    pub moves: HashMap<InstanceId, MoveGoal>,
    pub modules: HashMap<String, Value>,
    /// Which scripts required which module path.
    pub module_users: HashMap<String, HashSet<ScriptId>>,
    pub tweens: Vec<crate::tween::Tween>,
    pub memory_saves: HashMap<String, String>,
}

pub struct Host {
    pub dm: Rc<RefCell<DataModel>>,
    pub physics: Rc<RefCell<Physics>>,
    pub input: Rc<RefCell<InputState>>,
    pub state: RefCell<HostState>,
    pub save: RefCell<Option<Box<dyn SaveBackend>>>,
    /// Method tables, filled in once the globals are registered.
    pub methods: RefCell<HashMap<&'static str, Value>>,
    pub signal_methods: RefCell<Option<TableRef>>,
    pub shared_env: RefCell<Option<TableRef>>,
    /// Reads a script's source by project path.
    pub read_source: RefCell<Box<dyn Fn(&str) -> Option<String>>>,
}

impl Host {
    pub fn log(&self, level: LogLevel, text: impl Into<String>) {
        let mut state = self.state.borrow_mut();
        let source = state.script_names.get(&state.current_script).map(|(name, _)| name.clone());
        let time = state.time;
        state.log.push_back(LogEntry { level, text: text.into(), source, location: None, time });
        while state.log.len() > MAX_LOG {
            state.log.pop_front();
        }
    }

    /// Records a script error with its stack trace.
    pub fn log_error(&self, script: ScriptId, error: &Error) {
        let mut state = self.state.borrow_mut();
        let names = state.script_names.get(&script).cloned();
        let location = error.chunk.clone().zip(error.line);
        let time = state.time;
        state.log.push_back(LogEntry { level: LogLevel::Error, text: error.report(), source: names.map(|(name, _)| name), location, time });
        while state.log.len() > MAX_LOG {
            state.log.pop_front();
        }
    }

    pub fn next_id(&self) -> u64 {
        let mut state = self.state.borrow_mut();
        state.next_id += 1;
        state.next_id
    }

    pub fn current_script(&self) -> ScriptId {
        self.state.borrow().current_script
    }

    /// Resumes a coroutine on behalf of `script`, reporting any error.
    /// Returns false if the coroutine raised an error.
    pub fn run_thread(&self, lua: &mut Lua, thread: &ThreadRef, args: Vec<Value>, script: ScriptId) -> bool {
        let previous = std::mem::replace(&mut self.state.borrow_mut().current_script, script);
        lua.set_budget(RESUME_BUDGET);
        let result = lua.resume(thread, args);
        self.state.borrow_mut().current_script = previous;
        match result {
            Ok(Resumed::Yield(_) | Resumed::Return(_)) => true,
            Err(error) => {
                self.log_error(script, &error);
                false
            }
        }
    }

    /// Runs `func(args...)` in a new coroutine owned by `script`.
    pub fn spawn(&self, lua: &mut Lua, func: Value, args: Vec<Value>, script: ScriptId) -> ThreadRef {
        let thread = lua.create_thread(func);
        self.run_thread(lua, &thread, args, script);
        thread
    }

    /// Calls every listener of an event and resumes coroutines waiting on it.
    pub fn fire(&self, lua: &mut Lua, key: &SignalKey, args: &[Value]) {
        let (listeners, waiters) = {
            let mut state = self.state.borrow_mut();
            let listeners: Vec<(Value, ScriptId)> = match state.connections.get_mut(key) {
                Some(list) => {
                    let snapshot = list.iter().map(|c| (c.func.clone(), c.script)).collect();
                    list.retain(|c| !c.once);
                    snapshot
                }
                None => Vec::new(),
            };
            (listeners, state.waiters.remove(key).unwrap_or_default())
        };
        for (func, script) in listeners {
            self.spawn(lua, func, args.to_vec(), script);
        }
        for (thread, script) in waiters {
            self.run_thread(lua, &thread, args.to_vec(), script);
        }
    }

    pub fn has_listeners(&self, key: &SignalKey) -> bool {
        let state = self.state.borrow();
        state.connections.get(key).is_some_and(|l| !l.is_empty()) || state.waiters.get(key).is_some_and(|w| !w.is_empty())
    }

    /// Removes everything a script registered: listeners, sleeping and
    /// queued coroutines, tweens and movement goals it started.
    pub fn release_script(&self, lua: &mut Lua, script: ScriptId) {
        let mut state = self.state.borrow_mut();
        for list in state.connections.values_mut() {
            list.retain(|c| c.script != script);
        }
        state.connections.retain(|_, list| !list.is_empty());
        let mut threads: Vec<ThreadRef> = Vec::new();
        for list in state.waiters.values_mut() {
            threads.extend(list.iter().filter(|(_, s)| *s == script).map(|(t, _)| t.clone()));
            list.retain(|(_, s)| *s != script);
        }
        threads.extend(state.sleeping.iter().filter(|s| s.script == script).map(|s| s.thread.clone()));
        state.sleeping.retain(|s| s.script != script);
        threads.extend(state.ready.iter().filter(|(_, _, s)| *s == script).map(|(t, _, _)| t.clone()));
        state.ready.retain(|(_, _, s)| *s != script);
        state.tweens.retain(|t| t.script != script);
        for users in state.module_users.values_mut() {
            users.remove(&script);
        }
        drop(state);
        for thread in threads {
            lua.close_thread(&thread);
        }
    }
}

pub fn runtime_error(message: impl Into<String>) -> Error {
    Error::runtime(message)
}
