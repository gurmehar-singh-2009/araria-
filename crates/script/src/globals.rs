//! Global functions and services available to every script.

use std::any::Any;
use std::rc::Rc;

use araria_core::{InstanceId, Variant, Vec2, Vec3};
use araria_lua::{Error, Lua, Result, Ret, TableRef, UserData, Value, func_arg, num_arg, opt_num, str_arg, table_arg};

use crate::host::{Command, Host, LogLevel, Sleeper};
use crate::instance::{InstanceRef, instance_value};
use crate::signal::signal_value;
use crate::tween::{Direction, STYLES, Tween, TweenInfo};
use crate::types::{Vector2, json_to_value, value_to_json, value_to_variant, vec, vec3_arg};

/// Signals not tied to an instance use the data model's root id.
fn global_signal(host: &Rc<Host>, name: &str) -> Value {
    let root = host.dm.borrow().root();
    signal_value(host, root.0, name, false)
}

fn native(host: &Rc<Host>, name: &'static str, f: impl Fn(&Rc<Host>, &mut Lua, &[Value]) -> Result<Ret> + 'static) -> Value {
    let h = host.clone();
    Lua::native(name, move |lua, args| f(&h, lua, args))
}

/// `Workspace:Raycast(origin, direction, ignore?)`; arguments start at `origin`.
pub fn raycast(host: &Rc<Host>, lua: &mut Lua, args: &[Value]) -> Result<Ret> {
    let origin = vec3_arg(args, 0, "Raycast")?;
    let direction = vec3_arg(args, 1, "Raycast")?;
    let mut ignore: Vec<InstanceId> = Vec::new();
    match args.get(2) {
        Some(Value::Table(list)) => {
            ignore.extend(list.borrow().array().iter().filter_map(|v| v.as_userdata::<InstanceRef>()).map(|r| r.id));
        }
        Some(other) => ignore.extend(other.as_userdata::<InstanceRef>().map(|r| r.id)),
        None => {}
    }
    // Ignoring a model or character ignores everything inside it.
    let extra: Vec<InstanceId> = ignore.iter().flat_map(|id| host.dm.borrow().descendants(*id)).collect();
    ignore.extend(extra);
    let hit = host.physics.borrow().raycast(origin, direction, direction.length(), &ignore);
    Ok(Ret::One(match hit {
        Some(hit) => {
            let result = lua.new_table();
            result.set_str("Instance", instance_value(host, hit.instance));
            result.set_str("Position", vec(hit.position));
            result.set_str("Normal", vec(hit.normal));
            result.set_str("Distance", f64::from(hit.distance));
            Value::Table(result)
        }
        None => Value::Nil,
    }))
}

// ------------------------------------------------------------------ tweens

struct TweenRef {
    id: u64,
    host: Rc<Host>,
}

impl UserData for TweenRef {
    fn type_name(&self) -> &'static str {
        "Tween"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        let (id, host) = (self.id, self.host.clone());
        let set_playing = move |playing: bool, restart: bool| {
            Lua::native("Tween.control", move |_, _| {
                let mut state = host.state.borrow_mut();
                if let Some(tween) = state.tweens.iter_mut().find(|t| t.id == id) {
                    tween.playing = playing;
                    if restart {
                        tween.elapsed = 0.0;
                    }
                }
                Ok(Ret::None)
            })
        };
        match key.as_str().as_deref() {
            Some("Play") => Ok(set_playing(true, false)),
            Some("Pause") => Ok(set_playing(false, false)),
            Some("Cancel") => {
                let host = self.host.clone();
                Ok(Lua::native("Tween.Cancel", move |_, _| {
                    host.state.borrow_mut().tweens.retain(|t| t.id != id);
                    Ok(Ret::None)
                }))
            }
            Some("Completed") => Ok(signal_value(&self.host, (1 << 61) | id, "Completed", false)),
            _ => Err(Error::runtime(format!("{} is not a valid member of Tween", key.to_display()))),
        }
    }
}

fn tween_info(value: &Value) -> Result<TweenInfo> {
    let mut info = TweenInfo::default();
    match value {
        Value::Int(_) | Value::Num(_) => info.duration = value.to_number().unwrap_or(1.0) as f32,
        Value::Table(t) => {
            let t = t.borrow();
            let number = |name: &str, default: f32| t.get_str(name).to_number().map_or(default, |n| n as f32);
            info.duration = number("Time", info.duration);
            info.delay = number("DelayTime", 0.0);
            info.repeat = number("RepeatCount", 0.0) as i32;
            info.reverses = t.get_str("Reverses").truthy();
            if let Some(style) = t.get_str("EasingStyle").as_str() {
                info.style = STYLES.iter().find(|(name, _)| *name == &*style).map(|(_, s)| *s).ok_or_else(|| Error::runtime(format!("unknown easing style '{style}'")))?;
            }
            info.direction = match t.get_str("EasingDirection").as_str().as_deref() {
                Some("In") => Direction::In,
                Some("InOut") => Direction::InOut,
                Some("Out") | None => Direction::Out,
                Some(other) => return Err(Error::runtime(format!("unknown easing direction '{other}'"))),
            };
        }
        _ => return Err(Error::runtime("TweenInfo expected")),
    }
    Ok(info)
}

// ------------------------------------------------------------- registration

#[allow(clippy::too_many_lines)]
pub fn register(lua: &mut Lua, host: &Rc<Host>) {
    // -- output ----------------------------------------------------------
    for (name, level) in [("print", LogLevel::Info), ("warn", LogLevel::Warn)] {
        let h = host.clone();
        lua.set_global(
            name,
            Lua::native(name, move |lua, args| {
                let mut parts = Vec::with_capacity(args.len());
                for v in args {
                    parts.push(lua.tostring(v)?);
                }
                h.log(level, parts.join(" "));
                Ok(Ret::None)
            }),
        );
    }
    lua.set_global(
        "typeof",
        Lua::native("typeof", |_, args| {
            Ok(Ret::One(Value::from(match args.first() {
                Some(Value::UserData(u)) => u.type_name(),
                Some(Value::Vector(..)) => "Vector3",
                Some(other) => other.type_name(),
                None => "nil",
            })))
        }),
    );

    // -- services --------------------------------------------------------
    {
        let dm = host.dm.borrow();
        let game = instance_value(host, dm.root());
        lua.set_global("game", game);
        for (global, service) in [("workspace", "Workspace"), ("Workspace", "Workspace"), ("Players", "Players"), ("Lighting", "Lighting"), ("Storage", "Storage"), ("Gui", "Gui"), ("Scripts", "Scripts"), ("Camera", "Camera")] {
            lua.set_global(global, instance_value(host, dm.service(service)));
        }
    }
    let instance = lua.new_table();
    instance.set_str(
        "new",
        native(host, "Instance.new", |host, _, args| {
            let class = str_arg(args, 0, "Instance.new")?;
            let parent = args.get(1).and_then(|v| v.as_userdata::<InstanceRef>()).map(|r| r.id);
            let mut dm = host.dm.borrow_mut();
            let id = dm.create(&class.to_str(), parent).map_err(|e| Error::runtime(e.to_string()))?;
            dm.set_creator(id, host.current_script());
            drop(dm);
            Ok(Ret::One(instance_value(host, id)))
        }),
    );
    lua.set_global("Instance", instance);

    // -- task scheduler --------------------------------------------------
    let wait = native(host, "task.wait", |host, lua, args| {
        let seconds = opt_num(args, 0, "wait", 0.0)?.max(0.0);
        let Some(thread) = lua.current_thread() else {
            return Err(Error::runtime("wait can only be used inside a script"));
        };
        let mut state = host.state.borrow_mut();
        let (now, script) = (state.time, state.current_script);
        state.sleeping.push(Sleeper { wake: now + seconds, started: now, thread, script, args: None });
        Ok(Ret::Yield(Vec::new()))
    });
    let spawn = native(host, "task.spawn", |host, lua, args| {
        let func = func_arg(args, 0, "spawn")?;
        let thread = host.spawn(lua, func, args[1..].to_vec(), host.current_script());
        Ok(Ret::One(Value::Thread(thread)))
    });
    let defer = native(host, "task.defer", |host, lua, args| {
        let func = func_arg(args, 0, "defer")?;
        let thread = lua.create_thread(func);
        let script = host.current_script();
        host.state.borrow_mut().ready.push_back((thread.clone(), args[1..].to_vec(), script));
        Ok(Ret::One(Value::Thread(thread)))
    });
    let delay = native(host, "task.delay", |host, lua, args| {
        let seconds = num_arg(args, 0, "delay")?.max(0.0);
        let func = func_arg(args, 1, "delay")?;
        let thread = lua.create_thread(func);
        let mut state = host.state.borrow_mut();
        let (now, script) = (state.time, state.current_script);
        state.sleeping.push(Sleeper { wake: now + seconds, started: now, thread: thread.clone(), script, args: Some(args[2..].to_vec()) });
        Ok(Ret::One(Value::Thread(thread)))
    });
    let cancel = native(host, "task.cancel", |host, lua, args| {
        let Some(Value::Thread(thread)) = args.first() else {
            return Err(Error::runtime("task.cancel expects a thread returned by task.spawn, task.defer or task.delay"));
        };
        let mut state = host.state.borrow_mut();
        state.sleeping.retain(|s| !s.thread.ptr_eq(thread));
        state.ready.retain(|(t, _, _)| !t.ptr_eq(thread));
        for list in state.waiters.values_mut() {
            list.retain(|(t, _)| !t.ptr_eq(thread));
        }
        drop(state);
        lua.close_thread(thread);
        Ok(Ret::None)
    });
    let task = lua.new_table();
    for (name, value) in [("wait", &wait), ("spawn", &spawn), ("defer", &defer), ("delay", &delay), ("cancel", &cancel)] {
        task.set_str(name, value.clone());
    }
    lua.set_global("task", task);
    lua.set_global("wait", wait);
    lua.set_global("spawn", spawn);
    lua.set_global("delay", delay);
    let clock = native(host, "time", |host, _, _| Ok(Ret::One(Value::Num(host.state.borrow().time))));
    lua.set_global("time", clock.clone());
    lua.set_global("tick", clock);

    // -- RunService ------------------------------------------------------
    let run_service = lua.new_table();
    for name in ["Heartbeat", "Stepped", "RenderStepped"] {
        run_service.set_str(name, global_signal(host, "Heartbeat"));
    }
    run_service.set_str("IsRunning", Lua::native("RunService.IsRunning", |_, _| Ok(Ret::One(Value::Bool(true)))));
    lua.set_global("RunService", run_service);

    // -- Input -----------------------------------------------------------
    let input = lua.new_table();
    for name in ["KeyDown", "KeyUp", "MouseButtonDown", "MouseButtonUp", "MouseWheel"] {
        input.set_str(name, global_signal(host, name));
    }
    input.set_str(
        "IsKeyDown",
        native(host, "Input.IsKeyDown", |host, _, args| {
            let key = str_arg(args, 1, "IsKeyDown")?;
            Ok(Ret::One(Value::Bool(host.input.borrow().key(&key.to_str()))))
        }),
    );
    input.set_str(
        "IsMouseButtonDown",
        native(host, "Input.IsMouseButtonDown", |host, _, args| {
            let button = araria_lua::int_arg(args, 1, "IsMouseButtonDown")?;
            let buttons = host.input.borrow().mouse_buttons;
            Ok(Ret::One(Value::Bool((1..=3).contains(&button) && buttons[button as usize - 1])))
        }),
    );
    input.set_str("GetMousePosition", native(host, "Input.GetMousePosition", |host, _, _| Ok(Ret::One(Lua::userdata(Vector2(host.input.borrow().mouse_position))))));
    input.set_str("GetMouseDelta", native(host, "Input.GetMouseDelta", |host, _, _| Ok(Ret::One(Lua::userdata(Vector2(host.input.borrow().mouse_delta))))));
    input.set_str("GetViewportSize", native(host, "Input.GetViewportSize", |host, _, _| Ok(Ret::One(Lua::userdata(Vector2(host.input.borrow().viewport))))));
    input.set_str(
        "GetMoveVector",
        native(host, "Input.GetMoveVector", |host, _, _| {
            let input = host.input.borrow();
            Ok(Ret::One(if input.ui_captured { vec(Vec3::ZERO) } else { vec(input.move_direction()) }))
        }),
    );
    input.set_str(
        "GetMoveAxis",
        native(host, "Input.GetMoveAxis", |host, _, _| {
            let input = host.input.borrow();
            Ok(Ret::One(Lua::userdata(Vector2(if input.ui_captured { Vec2::ZERO } else { input.move_axis }))))
        }),
    );
    input.set_str(
        "GetMouseRay",
        native(host, "Input.GetMouseRay", |host, _, _| {
            Ok(match host.input.borrow().mouse_ray {
                Some((origin, direction)) => Ret::Values(vec![vec(origin), vec(direction)]),
                None => Ret::One(Value::Nil),
            })
        }),
    );
    input.set_str(
        "GetMouseHit",
        native(host, "Input.GetMouseHit", |host, lua, args| {
            let Some((origin, direction)) = host.input.borrow().mouse_ray else { return Ok(Ret::One(Value::Nil)) };
            let mut ray_args = vec![vec(origin), vec(direction * 500.0)];
            ray_args.extend(args.get(1).cloned());
            raycast(host, lua, &ray_args)
        }),
    );
    lua.set_global("Input", input);

    // -- tweens ----------------------------------------------------------
    let tween_info_lib = lua.new_table();
    tween_info_lib.set_str(
        "new",
        Lua::native("TweenInfo.new", |lua, args| {
            let info = lua.new_table();
            info.set_str("Time", args.first().cloned().unwrap_or(Value::Num(1.0)));
            info.set_str("EasingStyle", args.get(1).cloned().unwrap_or(Value::from("Quad")));
            info.set_str("EasingDirection", args.get(2).cloned().unwrap_or(Value::from("Out")));
            info.set_str("RepeatCount", args.get(3).cloned().unwrap_or(Value::Int(0)));
            info.set_str("Reverses", args.get(4).cloned().unwrap_or(Value::Bool(false)));
            info.set_str("DelayTime", args.get(5).cloned().unwrap_or(Value::Int(0)));
            Ok(Ret::One(Value::Table(info)))
        }),
    );
    lua.set_global("TweenInfo", tween_info_lib);
    let tween_service = lua.new_table();
    tween_service.set_str(
        "Create",
        native(host, "TweenService.Create", |host, _, args| {
            let instance = crate::instance::instance_arg(args, 1, "Create")?;
            let info = tween_info(args.get(2).unwrap_or(&Value::Nil))?;
            let goals = table_arg(args, 3, "Create")?;
            let mut tracks = Vec::new();
            {
                let dm = host.dm.borrow();
                let target = dm.get(instance).ok_or_else(|| Error::runtime("the instance has been destroyed"))?;
                for (key, value) in goals.borrow().iter() {
                    let name = key.as_str().ok_or_else(|| Error::runtime("tween goals must be named by property"))?.into_owned();
                    let name = if name == "Orientation" { "Rotation".to_string() } else { name };
                    let def = target.info().prop(&name).ok_or_else(|| Error::runtime(format!("{name} is not a property of {}", target.class_name())))?;
                    let goal = def.coerce(value_to_variant(&value, &def.ty).map_err(|e| Error::runtime(format!("{name}: {e}")))?).map_err(Error::runtime)?;
                    let start = target.get(&name).cloned().unwrap_or(Variant::Nil);
                    tracks.push((name, start, goal));
                }
            }
            let id = host.next_id();
            let script = host.current_script();
            host.state.borrow_mut().tweens.push(Tween { id, script, instance, info, tracks, elapsed: 0.0, playing: false, completed_cycles: 0 });
            Ok(Ret::One(Lua::userdata(TweenRef { id, host: host.clone() })))
        }),
    );
    lua.set_global("TweenService", tween_service);

    // -- Events: a message bus between scripts ---------------------------
    let events = lua.new_table();
    events.set_str(
        "Fire",
        native(host, "Events.Fire", |host, lua, args| {
            let name = str_arg(args, 1, "Fire")?;
            let key = (1u64 << 62, Rc::from(&*name.to_str()));
            host.fire(lua, &key, &args[2..]);
            Ok(Ret::None)
        }),
    );
    events.set_str(
        "Get",
        native(host, "Events.Get", |host, _, args| {
            let name = str_arg(args, 1, "Get")?;
            Ok(Ret::One(signal_value(host, 1 << 62, &name.to_str(), true)))
        }),
    );
    lua.set_global("Events", events);

    // -- Audio -----------------------------------------------------------
    let audio = lua.new_table();
    audio.set_str(
        "Play",
        native(host, "Audio.Play", |host, _, args| {
            let asset = str_arg(args, 1, "Play")?.to_str().into_owned();
            let (volume, pitch) = (opt_num(args, 2, "Play", 1.0)? as f32, opt_num(args, 3, "Play", 1.0)? as f32);
            host.state.borrow_mut().commands.push(Command::PlaySound { asset, volume, pitch, position: None });
            Ok(Ret::None)
        }),
    );
    audio.set_str(
        "PlayAt",
        native(host, "Audio.PlayAt", |host, _, args| {
            let asset = str_arg(args, 1, "PlayAt")?.to_str().into_owned();
            let position = vec3_arg(args, 2, "PlayAt")?;
            let volume = opt_num(args, 3, "PlayAt", 1.0)? as f32;
            host.state.borrow_mut().commands.push(Command::PlaySound { asset, volume, pitch: 1.0, position: Some(position) });
            Ok(Ret::None)
        }),
    );
    audio.set_str(
        "PlayMusic",
        native(host, "Audio.PlayMusic", |host, _, args| {
            let asset = str_arg(args, 1, "PlayMusic")?.to_str().into_owned();
            let volume = opt_num(args, 2, "PlayMusic", 0.5)? as f32;
            host.state.borrow_mut().commands.push(Command::PlayMusic { asset, volume });
            Ok(Ret::None)
        }),
    );
    audio.set_str(
        "StopMusic",
        native(host, "Audio.StopMusic", |host, _, _| {
            host.state.borrow_mut().commands.push(Command::StopMusic);
            Ok(Ret::None)
        }),
    );
    lua.set_global("Audio", audio);

    // -- Physics ---------------------------------------------------------
    let physics = lua.new_table();
    physics.set_str("Raycast", native(host, "Physics.Raycast", |host, lua, args| raycast(host, lua, &args[1..])));
    physics.set_str(
        "OverlapSphere",
        native(host, "Physics.OverlapSphere", |host, lua, args| {
            let found = host.physics.borrow().overlap_sphere(vec3_arg(args, 1, "OverlapSphere")?, num_arg(args, 2, "OverlapSphere")? as f32);
            let values: Vec<Value> = found.into_iter().map(|id| instance_value(host, id)).collect();
            Ok(Ret::One(Value::Table(lua.new_array(values))))
        }),
    );
    lua.set_global("Physics", physics);

    // -- Save: local game progress ----------------------------------------
    let save = lua.new_table();
    save.set_str(
        "Get",
        native(host, "Save.Get", |host, lua, args| {
            let key = str_arg(args, 1, "Get")?.to_str().into_owned();
            let stored = match host.save.borrow().as_ref() {
                Some(backend) => backend.get(&key),
                None => host.state.borrow().memory_saves.get(&key).cloned(),
            };
            let value = stored.and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok()).map(|json| json_to_value(lua, &json));
            Ok(Ret::One(match value {
                Some(v) if !v.is_nil() => v,
                _ => args.get(2).cloned().unwrap_or(Value::Nil),
            }))
        }),
    );
    save.set_str(
        "Set",
        native(host, "Save.Set", |host, _, args| {
            let key = str_arg(args, 1, "Set")?.to_str().into_owned();
            let value = args.get(2).cloned().unwrap_or(Value::Nil);
            let text = if value.is_nil() { None } else { Some(value_to_json(&value, 0).map_err(Error::runtime)?.to_string()) };
            match host.save.borrow().as_ref() {
                Some(backend) => backend.set(&key, text.as_deref()),
                None => {
                    let mut state = host.state.borrow_mut();
                    match text {
                        Some(text) => state.memory_saves.insert(key, text),
                        None => state.memory_saves.remove(&key),
                    };
                }
            }
            Ok(Ret::None)
        }),
    );
    lua.set_global("Save", save);

    // -- shared state and enums -------------------------------------------
    let shared = lua.new_table();
    lua.set_global("shared", shared);
    let enums = lua.new_table();
    let add_enum = |lua: &mut Lua, name: &str, options: &[&str]| {
        let table = lua.new_table();
        for option in options {
            table.set_str(option, *option);
        }
        enums.set_str(name, table);
    };
    add_enum(lua, "Material", araria_core::schema::MATERIALS);
    add_enum(lua, "Shape", araria_core::schema::SHAPES);
    add_enum(lua, "PartType", araria_core::schema::SHAPES);
    add_enum(lua, "CameraMode", araria_core::schema::CAMERA_MODES);
    add_enum(lua, "TextAlign", araria_core::schema::TEXT_ALIGN);
    add_enum(lua, "CharacterState", araria_core::schema::CHARACTER_STATES);
    add_enum(lua, "EasingStyle", &STYLES.iter().map(|(name, _)| *name).collect::<Vec<_>>());
    add_enum(lua, "EasingDirection", &["In", "Out", "InOut"]);
    add_enum(lua, "KeyCode", &["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Space", "LeftShift", "RightShift", "LeftControl", "RightControl", "Enter", "Escape", "Tab", "Backspace", "Up", "Down", "Left", "Right"]);
    lua.set_global("Enum", enums);
}

/// Creates the environment table a script runs in: its own globals on top
/// of the shared ones.
pub fn script_env(lua: &mut Lua) -> TableRef {
    let env = lua.new_table();
    let meta = lua.new_table();
    meta.set_str("__index", lua.globals());
    env.borrow_mut().set_metatable(Some(meta));
    env
}
