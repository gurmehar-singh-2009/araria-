//! The `Instance` userdata: properties, events, methods and child lookup.

use std::any::Any;
use std::rc::Rc;

use araria_core::{InstanceId, ModelError, Variant, Vec3, quat_to_euler_deg};
use araria_lua::{Error, Lua, Result, Ret, UserData, Value, int_arg, num_arg, opt_num, str_arg};

use crate::host::{Command, Host, MoveGoal};
use crate::signal::signal_value;
use crate::types::{CFrame, look_rotation, to_vec3, value_to_variant, value_to_variant_any, variant_to_value, vec, vec3_arg};

/// A reference to an instance in the data model. The same instance always
/// maps to the same userdata object, so references compare equal and work
/// as table keys.
pub struct InstanceRef {
    pub id: InstanceId,
    host: Rc<Host>,
}

pub fn instance_value(host: &Rc<Host>, id: InstanceId) -> Value {
    if let Some(value) = host.state.borrow().instances.get(&id) {
        return value.clone();
    }
    let value = Lua::userdata(InstanceRef { id, host: host.clone() });
    host.state.borrow_mut().instances.insert(id, value.clone());
    value
}

fn model_error(e: ModelError) -> Error {
    Error::runtime(e.to_string())
}

fn destroyed() -> Error {
    Error::runtime("the instance has been destroyed")
}

impl UserData for InstanceRef {
    fn type_name(&self) -> &'static str {
        "Instance"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        let Some(name) = key.as_str() else { return Ok(Value::Nil) };
        let name = &*name;
        let dm = self.host.dm.borrow();
        let Some(instance) = dm.get(self.id) else {
            // Destroyed instances still answer a few harmless questions.
            return match name {
                "Parent" => Ok(Value::Nil),
                "Destroy" => Ok(self.host.methods.borrow().get("Destroy").cloned().unwrap_or(Value::Nil)),
                _ => Err(destroyed()),
            };
        };
        match name {
            "Name" => return Ok(Value::from(instance.name.as_str())),
            "ClassName" => return Ok(Value::from(instance.class_name())),
            "Parent" => return Ok(instance.parent.map_or(Value::Nil, |p| instance_value(&self.host, p))),
            "CFrame" if instance.is_a("BasePart") || instance.is_a("Camera") => {
                return Ok(Lua::userdata(CFrame { position: instance.vec3("Position"), rotation: instance.quat("Rotation") }));
            }
            "Orientation" if instance.get("Rotation").is_some() => return Ok(vec(quat_to_euler_deg(instance.quat("Rotation")))),
            _ => {}
        }
        let info = instance.info();
        if let Some(value) = instance.get(name) {
            return Ok(variant_to_value(&self.host, value));
        }
        if info.all_events().any(|e| e.name == name) || name == "AttributeChanged" {
            return Ok(signal_value(&self.host, self.id.0, name, false));
        }
        if info.all_methods().any(|m| m.name == name) {
            if let Some(method) = self.host.methods.borrow().get(name) {
                return Ok(method.clone());
            }
        }
        if let Some(child) = dm.find_first_child(self.id, name) {
            return Ok(instance_value(&self.host, child));
        }
        Err(Error::runtime(format!("{name} is not a valid member of {} \"{}\"", info.name(), instance.name)))
    }

    fn new_index(&self, _lua: &mut Lua, key: &Value, value: Value) -> Result<()> {
        let Some(name) = key.as_str() else { return Err(Error::runtime("property names must be strings")) };
        let name = &*name;
        let mut dm = self.host.dm.borrow_mut();
        let Some(instance) = dm.get(self.id) else { return Err(destroyed()) };
        match name {
            "Name" => {
                let text = value.as_str().ok_or_else(|| Error::runtime("Name must be a string"))?.into_owned();
                return dm.set_name(self.id, &text).map_err(model_error);
            }
            "Parent" => {
                let parent = match &value {
                    Value::Nil => None,
                    other => Some(other.as_userdata::<InstanceRef>().ok_or_else(|| Error::runtime("Parent must be an Instance or nil"))?.id),
                };
                return dm.set_parent(self.id, parent).map_err(model_error);
            }
            "CFrame" if instance.is_a("BasePart") || instance.is_a("Camera") => {
                let cf = value.as_userdata::<CFrame>().ok_or_else(|| Error::runtime("CFrame expected"))?;
                dm.set(self.id, "Position", cf.position).map_err(model_error)?;
                return dm.set(self.id, "Rotation", cf.rotation).map_err(model_error);
            }
            "Orientation" if instance.get("Rotation").is_some() => {
                let degrees = to_vec3(&value).ok_or_else(|| Error::runtime("Vector3 expected"))?;
                return dm.set(self.id, "Rotation", degrees).map_err(model_error);
            }
            _ => {}
        }
        let info = instance.info();
        let Some(def) = info.prop(name) else {
            return Err(Error::runtime(format!("{name} is not a valid property of {} \"{}\"", info.name(), instance.name)));
        };
        if def.read_only {
            return Err(Error::runtime(format!("{name} is read-only")));
        }
        let variant = value_to_variant(&value, &def.ty).map_err(|e| Error::runtime(format!("{name}: {e}")))?;
        dm.set(self.id, name, variant).map_err(model_error)
    }

    fn to_string(&self) -> String {
        self.host.dm.borrow().get(self.id).map_or_else(|| "<destroyed>".to_string(), |i| i.name.clone())
    }
}

/// The instance a method was called on (`args[0]`).
pub fn self_id(args: &[Value], method: &str) -> Result<InstanceId> {
    args.first().and_then(|v| v.as_userdata::<InstanceRef>()).map(|i| i.id).ok_or_else(|| Error::runtime(format!("{method} must be called with ':' on an Instance")))
}

pub fn instance_arg(args: &[Value], i: usize, method: &str) -> Result<InstanceId> {
    args.get(i).and_then(|v| v.as_userdata::<InstanceRef>()).map(|r| r.id).ok_or_else(|| araria_lua::arg_error(i, method, "Instance", args.get(i).unwrap_or(&Value::Nil)))
}

fn list(lua: &mut Lua, host: &Rc<Host>, ids: impl IntoIterator<Item = InstanceId>) -> Ret {
    let values: Vec<Value> = ids.into_iter().map(|id| instance_value(host, id)).collect();
    Ret::One(Value::Table(lua.new_array(values)))
}

/// Registers every instance method in the host's method table.
#[allow(clippy::too_many_lines)]
pub fn register(host: &Rc<Host>) {
    let mut methods = host.methods.borrow_mut();
    let mut add = |name: &'static str, f: Box<dyn Fn(&Rc<Host>, &mut Lua, &[Value]) -> Result<Ret>>| {
        let h = host.clone();
        methods.insert(name, Lua::native(name, move |lua, args| f(&h, lua, args)));
    };
    macro_rules! method {
        ($name:literal, |$host:ident, $lua:ident, $args:ident| $body:expr) => {
            add($name, Box::new(|$host: &Rc<Host>, $lua: &mut Lua, $args: &[Value]| -> Result<Ret> { $body }));
        };
    }

    // ------------------------------------------------------- Instance
    method!("Destroy", |host, _lua, args| {
        let id = self_id(args, "Destroy")?;
        let mut dm = host.dm.borrow_mut();
        if dm.contains(id) {
            dm.destroy(id).map_err(model_error)?;
        }
        Ok(Ret::None)
    });
    method!("Clone", |host, _lua, args| {
        let id = self_id(args, "Clone")?;
        let copy = host.dm.borrow_mut().clone_tree(id, None).map_err(model_error)?;
        let script = host.current_script();
        host.dm.borrow_mut().set_creator(copy, script);
        Ok(Ret::One(instance_value(host, copy)))
    });
    method!("FindFirstChild", |host, _lua, args| {
        let id = self_id(args, "FindFirstChild")?;
        let name = str_arg(args, 1, "FindFirstChild")?;
        let recursive = args.get(2).is_some_and(Value::truthy);
        let dm = host.dm.borrow();
        let found = if recursive { dm.find_first_descendant(id, &name.to_str()) } else { dm.find_first_child(id, &name.to_str()) };
        Ok(Ret::One(found.map_or(Value::Nil, |c| instance_value(host, c))))
    });
    method!("FindFirstChildOfClass", |host, _lua, args| {
        let id = self_id(args, "FindFirstChildOfClass")?;
        let class = str_arg(args, 1, "FindFirstChildOfClass")?;
        let found = host.dm.borrow().find_first_child_of_class(id, &class.to_str());
        Ok(Ret::One(found.map_or(Value::Nil, |c| instance_value(host, c))))
    });
    method!("GetChildren", |host, lua, args| {
        let children = host.dm.borrow().children(self_id(args, "GetChildren")?).to_vec();
        Ok(list(lua, host, children))
    });
    method!("GetDescendants", |host, lua, args| {
        let descendants = host.dm.borrow().descendants(self_id(args, "GetDescendants")?);
        Ok(list(lua, host, descendants))
    });
    method!("IsA", |host, _lua, args| {
        let id = self_id(args, "IsA")?;
        let class = str_arg(args, 1, "IsA")?;
        Ok(Ret::One(Value::Bool(host.dm.borrow().get(id).is_some_and(|i| i.is_a(&class.to_str())))))
    });
    method!("IsDescendantOf", |host, _lua, args| {
        let (id, ancestor) = (self_id(args, "IsDescendantOf")?, instance_arg(args, 1, "IsDescendantOf")?);
        Ok(Ret::One(Value::Bool(host.dm.borrow().is_descendant_of(id, ancestor))))
    });
    method!("GetFullName", |host, _lua, args| Ok(Ret::One(Value::from(host.dm.borrow().path(self_id(args, "GetFullName")?)))));
    method!("SetAttribute", |host, _lua, args| {
        let id = self_id(args, "SetAttribute")?;
        let name = str_arg(args, 1, "SetAttribute")?;
        let value = value_to_variant_any(args.get(2).unwrap_or(&Value::Nil)).ok_or_else(|| Error::runtime("attributes can hold booleans, numbers, strings, vectors, colors and instances"))?;
        host.dm.borrow_mut().set_attribute(id, &name.to_str(), value).map_err(model_error)?;
        Ok(Ret::None)
    });
    method!("GetAttribute", |host, _lua, args| {
        let id = self_id(args, "GetAttribute")?;
        let name = str_arg(args, 1, "GetAttribute")?;
        let value = host.dm.borrow().get(id).and_then(|i| i.attributes.get(&*name.to_str()).cloned());
        Ok(Ret::One(value.map_or(Value::Nil, |v| variant_to_value(host, &v))))
    });
    method!("GetAttributes", |host, lua, args| {
        let id = self_id(args, "GetAttributes")?;
        let attributes = host.dm.borrow().get(id).map(|i| i.attributes.clone()).unwrap_or_default();
        let table = lua.new_table();
        for (name, value) in &attributes {
            table.set_str(name, variant_to_value(host, value));
        }
        Ok(Ret::One(Value::Table(table)))
    });
    method!("AddTag", |host, _lua, args| {
        host.dm.borrow_mut().add_tag(self_id(args, "AddTag")?, &str_arg(args, 1, "AddTag")?.to_str());
        Ok(Ret::None)
    });
    method!("RemoveTag", |host, _lua, args| {
        host.dm.borrow_mut().remove_tag(self_id(args, "RemoveTag")?, &str_arg(args, 1, "RemoveTag")?.to_str());
        Ok(Ret::None)
    });
    method!("HasTag", |host, _lua, args| {
        let id = self_id(args, "HasTag")?;
        let tag = str_arg(args, 1, "HasTag")?;
        Ok(Ret::One(Value::Bool(host.dm.borrow().get(id).is_some_and(|i| i.has_tag(&tag.to_str())))))
    });
    method!("GetPropertyChangedSignal", |host, _lua, args| {
        let id = self_id(args, "GetPropertyChangedSignal")?;
        let property = str_arg(args, 1, "GetPropertyChangedSignal")?;
        Ok(Ret::One(signal_value(host, id.0, &format!("Changed:{}", property.to_str()), false)))
    });
    method!("GetService", |host, _lua, args| {
        let name = str_arg(args, 1, "GetService")?;
        let name = name.to_str();
        let dm = host.dm.borrow();
        // Familiar aliases from other engines map onto Araria's services.
        let service = match &*name {
            "ReplicatedStorage" | "ServerStorage" => "Storage",
            "ServerScriptService" | "StarterPlayerScripts" => "Scripts",
            "StarterGui" => "Gui",
            other => other,
        };
        dm.find_service(service).map(|id| Ret::One(instance_value(host, id))).ok_or_else(|| Error::runtime(format!("'{name}' is not a service")))
    });

    // ------------------------------------------------------- BasePart
    method!("ApplyForce", |host, _lua, args| {
        host.physics.borrow_mut().apply_force(self_id(args, "ApplyForce")?, vec3_arg(args, 1, "ApplyForce")?);
        Ok(Ret::None)
    });
    method!("GetTouchingParts", |host, lua, args| {
        let touching = host.physics.borrow().touching(self_id(args, "GetTouchingParts")?);
        Ok(list(lua, host, touching))
    });
    method!("LookAt", |host, _lua, args| {
        let id = self_id(args, "LookAt")?;
        let target = vec3_arg(args, 1, "LookAt")?;
        let mut dm = host.dm.borrow_mut();
        let position = dm.get(id).ok_or_else(destroyed)?.vec3("Position");
        dm.set(id, "Rotation", look_rotation(target - position)).map_err(model_error)?;
        Ok(Ret::None)
    });

    // ---------------------------------------------------------- Model
    method!("GetPivot", |host, _lua, args| {
        let id = self_id(args, "GetPivot")?;
        Ok(Ret::One(vec(pivot(host, id))))
    });
    method!("GetBoundingBox", |host, _lua, args| {
        let id = self_id(args, "GetBoundingBox")?;
        let (min, max) = host.dm.borrow().bounds(id).unwrap_or((Vec3::ZERO, Vec3::ZERO));
        Ok(Ret::Values(vec![vec((min + max) * 0.5), vec(max - min)]))
    });

    // ------------------------------------------- Character and shared
    method!("MoveTo", |host, _lua, args| {
        let id = self_id(args, "MoveTo")?;
        let target = vec3_arg(args, 1, "MoveTo")?;
        let is_character = host.dm.borrow().get(id).ok_or_else(destroyed)?.is_a("Character");
        if is_character {
            let deadline = host.state.borrow().time + 8.0;
            let run = args.get(2).is_some_and(Value::truthy);
            host.state.borrow_mut().moves.insert(id, MoveGoal::Point { target, run, deadline });
        } else {
            // Models: shift every part so the pivot lands on the target.
            let offset = target - pivot(host, id);
            let mut dm = host.dm.borrow_mut();
            for part in dm.descendants(id) {
                let position = dm.get(part).filter(|p| p.is_a("BasePart") || p.is_a("Character")).map(|p| p.vec3("Position"));
                if let Some(position) = position {
                    dm.set(part, "Position", position + offset).map_err(model_error)?;
                }
            }
        }
        Ok(Ret::None)
    });
    method!("Move", |host, _lua, args| {
        let id = self_id(args, "Move")?;
        let dir = vec3_arg(args, 1, "Move")?;
        let run = args.get(2).is_some_and(Value::truthy);
        let mut state = host.state.borrow_mut();
        if dir.length_squared() < 1e-6 {
            state.moves.remove(&id);
            drop(state);
            host.physics.borrow_mut().set_input(id, Default::default());
        } else {
            state.moves.insert(id, MoveGoal::Direction { dir: Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero(), run });
        }
        Ok(Ret::None)
    });
    method!("Jump", |host, _lua, args| {
        host.physics.borrow_mut().request_jump(self_id(args, "Jump")?);
        Ok(Ret::None)
    });
    method!("Teleport", |host, _lua, args| {
        let id = self_id(args, "Teleport")?;
        let mut dm = host.dm.borrow_mut();
        dm.set(id, "Position", vec3_arg(args, 1, "Teleport")?).map_err(model_error)?;
        dm.set(id, "Velocity", Vec3::ZERO).map_err(model_error)?;
        Ok(Ret::None)
    });
    method!("TakeDamage", |host, _lua, args| {
        let id = self_id(args, "TakeDamage")?;
        let amount = num_arg(args, 1, "TakeDamage")?;
        let mut dm = host.dm.borrow_mut();
        let health = dm.get(id).ok_or_else(destroyed)?.number("Health");
        if health > 0.0 {
            let new_health = (health - amount).max(0.0);
            dm.set(id, "Health", new_health).map_err(model_error)?;
            dm.fire(id, "HealthChanged", vec![new_health.into()]);
        }
        Ok(Ret::None)
    });
    method!("ApplyImpulse", |host, _lua, args| {
        host.physics.borrow_mut().apply_impulse(self_id(args, "ApplyImpulse")?, vec3_arg(args, 1, "ApplyImpulse")?);
        Ok(Ret::None)
    });
    method!("PlayAnimation", |host, _lua, args| {
        let id = self_id(args, "PlayAnimation")?;
        let name = str_arg(args, 1, "PlayAnimation")?;
        let name = name.to_str();
        if !["Wave", "Cheer", "Dance", "Point", "Sit"].contains(&&*name) {
            return Err(Error::runtime(format!("unknown animation '{name}' (expected Wave, Cheer, Dance, Point or Sit)")));
        }
        let mut dm = host.dm.borrow_mut();
        dm.set_quiet(id, "Emote", Variant::String(name.into_owned()));
        Ok(Ret::None)
    });
    method!("StopAnimation", |host, _lua, args| {
        host.dm.borrow_mut().set_quiet(self_id(args, "StopAnimation")?, "Emote", Variant::String(String::new()));
        Ok(Ret::None)
    });
    method!("Sit", |host, _lua, args| {
        let (id, seat) = (self_id(args, "Sit")?, instance_arg(args, 1, "Sit")?);
        host.dm.borrow_mut().set_quiet(id, "Seat", Variant::Ref(Some(seat)));
        Ok(Ret::None)
    });
    method!("Stand", |host, _lua, args| {
        host.dm.borrow_mut().set_quiet(self_id(args, "Stand")?, "Seat", Variant::Ref(None));
        Ok(Ret::None)
    });

    // ------------------------------------------------------ Workspace
    method!("Raycast", |host, lua, args| crate::globals::raycast(host, lua, &args[1..]));
    method!("GetPartsInRadius", |host, lua, args| {
        let found = host.physics.borrow().overlap_sphere(vec3_arg(args, 1, "GetPartsInRadius")?, num_arg(args, 2, "GetPartsInRadius")? as f32);
        Ok(list(lua, host, found))
    });
    method!("GetTagged", |host, lua, args| {
        let tagged = host.dm.borrow().tagged(&str_arg(args, 1, "GetTagged")?.to_str());
        Ok(list(lua, host, tagged))
    });

    // -------------------------------------------------------- Players
    method!("GetPlayers", |host, lua, args| {
        let id = self_id(args, "GetPlayers")?;
        let players = host.dm.borrow().children(id).to_vec();
        Ok(list(lua, host, players))
    });
    method!("GetPlayerFromCharacter", |host, _lua, args| {
        let id = self_id(args, "GetPlayerFromCharacter")?;
        let Some(character) = args.get(1).and_then(|v| v.as_userdata::<InstanceRef>()).map(|r| r.id) else { return Ok(Ret::One(Value::Nil)) };
        let dm = host.dm.borrow();
        let player = dm.children(id).iter().copied().find(|p| dm.get(*p).and_then(|i| i.reference("Character")) == Some(character));
        Ok(Ret::One(player.map_or(Value::Nil, |p| instance_value(host, p))))
    });
    method!("LoadCharacter", |host, _lua, args| {
        let player = self_id(args, "LoadCharacter")?;
        host.state.borrow_mut().commands.push(Command::LoadCharacter { player });
        Ok(Ret::None)
    });
    method!("SetStat", |host, _lua, args| {
        let id = self_id(args, "SetStat")?;
        let name = str_arg(args, 1, "SetStat")?;
        let value = value_to_variant_any(args.get(2).unwrap_or(&Value::Nil)).ok_or_else(|| Error::runtime("stats must be numbers or strings"))?;
        host.dm.borrow_mut().set_attribute(id, &format!("stat:{}", name.to_str()), value).map_err(model_error)?;
        Ok(Ret::None)
    });
    method!("GetStat", |host, _lua, args| {
        let id = self_id(args, "GetStat")?;
        let key = format!("stat:{}", str_arg(args, 1, "GetStat")?.to_str());
        let value = host.dm.borrow().get(id).and_then(|i| i.attributes.get(&key).cloned());
        Ok(Ret::One(value.map_or(Value::Nil, |v| variant_to_value(host, &v))))
    });
    method!("Notify", |host, _lua, args| {
        let player = self_id(args, "Notify")?;
        let text = str_arg(args, 1, "Notify")?.to_str().into_owned();
        let duration = opt_num(args, 2, "Notify", 3.0)? as f32;
        host.state.borrow_mut().commands.push(Command::Notify { player: Some(player), text, duration });
        Ok(Ret::None)
    });

    // --------------------------------------------- Camera, Sound, FX
    method!("Shake", |host, _lua, args| {
        let (intensity, duration) = (num_arg(args, 1, "Shake")? as f32, opt_num(args, 2, "Shake", 0.4)? as f32);
        host.state.borrow_mut().commands.push(Command::CameraShake { intensity, duration });
        Ok(Ret::None)
    });
    method!("Play", |host, _lua, args| {
        let id = self_id(args, "Play")?;
        let mut dm = host.dm.borrow_mut();
        // Toggle off first so a playing sound restarts.
        dm.set(id, "Playing", false).map_err(model_error)?;
        dm.set(id, "Playing", true).map_err(model_error)?;
        Ok(Ret::None)
    });
    method!("Stop", |host, _lua, args| {
        host.dm.borrow_mut().set(self_id(args, "Stop")?, "Playing", false).map_err(model_error)?;
        Ok(Ret::None)
    });
    method!("Emit", |host, _lua, args| {
        let emitter = self_id(args, "Emit")?;
        let count = int_arg(args, 1, "Emit")?.clamp(0, 2000) as u32;
        host.state.borrow_mut().commands.push(Command::EmitParticles { emitter, count });
        Ok(Ret::None)
    });
}

/// A model's pivot: its primary part, or the center of its bounds.
fn pivot(host: &Rc<Host>, id: InstanceId) -> Vec3 {
    let dm = host.dm.borrow();
    let primary = dm.get(id).and_then(|m| m.reference("PrimaryPart")).and_then(|p| dm.get(p)).map(|p| p.vec3("Position"));
    primary.or_else(|| dm.bounds(id).map(|(min, max)| (min + max) * 0.5)).or_else(|| dm.world_position(id)).unwrap_or(Vec3::ZERO)
}

/// Turns script movement goals into character input. Called every frame.
pub fn drive_characters(host: &Rc<Host>) {
    let now = host.state.borrow().time;
    let goals: Vec<(InstanceId, MoveGoal)> = host.state.borrow().moves.iter().map(|(k, v)| (*k, *v)).collect();
    for (id, goal) in goals {
        let position = host.dm.borrow().get(id).map(|c| c.vec3("Position"));
        let Some(position) = position else {
            host.state.borrow_mut().moves.remove(&id);
            continue;
        };
        let (dir, run, finished) = match goal {
            MoveGoal::Direction { dir, run } => (dir, run, None),
            MoveGoal::Point { target, run, deadline } => {
                let delta = Vec3::new(target.x - position.x, 0.0, target.z - position.z);
                if delta.length() < 0.35 {
                    (Vec3::ZERO, run, Some(true))
                } else if now > deadline {
                    (Vec3::ZERO, run, Some(false))
                } else {
                    (delta.normalize(), run, None)
                }
            }
        };
        host.physics.borrow_mut().set_input(id, araria_physics::CharacterInput { move_dir: dir, run, jump: false });
        if let Some(reached) = finished {
            host.state.borrow_mut().moves.remove(&id);
            host.dm.borrow_mut().fire(id, "MoveToFinished", vec![reached.into()]);
        }
    }
}
