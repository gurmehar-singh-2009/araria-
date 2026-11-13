//! Value types exposed to Lua (`Vector3`, `Vector2`, `Color3`, `UDim2`,
//! `CFrame`) and conversions between Lua values and property variants.

use std::any::Any;
use std::rc::Rc;

use araria_core::schema::PropType;
use araria_core::{Color, Quat, UDim2, Variant, Vec2, Vec3, euler_deg_to_quat, quat_to_euler_deg};
use araria_lua::{ArithOp, Error, Lua, Result, Ret, TableRef, UserData, Value, num_arg, opt_num};

use crate::host::Host;
use crate::instance::{InstanceRef, instance_value};

pub fn vec(v: Vec3) -> Value {
    Value::Vector(v.x, v.y, v.z)
}

pub fn to_vec3(v: &Value) -> Option<Vec3> {
    match v {
        Value::Vector(x, y, z) => Some(Vec3::new(*x, *y, *z)),
        _ => None,
    }
}

pub fn vec3_arg(args: &[Value], i: usize, func: &str) -> Result<Vec3> {
    args.get(i).and_then(to_vec3).ok_or_else(|| araria_lua::arg_error(i, func, "Vector3", args.get(i).unwrap_or(&Value::Nil)))
}

fn num(v: f32) -> Value {
    Value::Num(f64::from(v))
}

fn method(name: &'static str, f: impl Fn(&mut Lua, &[Value]) -> Result<Ret> + 'static) -> Result<Value> {
    Ok(Lua::native(name, f))
}

fn no_member(type_name: &str, key: &Value) -> Error {
    Error::runtime(format!("{} is not a valid member of {type_name}", key.to_display()))
}

// ---------------------------------------------------------------- Color3

pub struct Color3(pub Color);

impl UserData for Color3 {
    fn type_name(&self) -> &'static str {
        "Color3"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        let c = self.0;
        match key.as_str().as_deref() {
            Some("R" | "r") => Ok(num(c.r)),
            Some("G" | "g") => Ok(num(c.g)),
            Some("B" | "b") => Ok(num(c.b)),
            Some("Lerp") => method("Color3.Lerp", move |_, args| {
                let other = args.get(1).and_then(|v| v.as_userdata::<Color3>()).ok_or_else(|| Error::runtime("Color3:Lerp expects a Color3"))?;
                Ok(Ret::One(Lua::userdata(Color3(c.lerp(other.0, num_arg(args, 2, "Lerp")? as f32)))))
            }),
            Some("ToHex") => method("Color3.ToHex", move |_, _| Ok(Ret::One(Value::from(c.to_hex())))),
            Some("ToHSV") => method("Color3.ToHSV", move |_, _| {
                let (h, s, v) = c.to_hsv();
                Ok(Ret::Values(vec![num(h / 360.0), num(s), num(v)]))
            }),
            _ => Err(no_member("Color3", key)),
        }
    }
    fn equals(&self, other: &dyn UserData) -> bool {
        other.as_any().downcast_ref::<Color3>().is_some_and(|o| o.0 == self.0)
    }
    fn to_string(&self) -> String {
        format!("{}, {}, {}", self.0.r, self.0.g, self.0.b)
    }
}

// --------------------------------------------------------------- Vector2

pub struct Vector2(pub Vec2);

impl UserData for Vector2 {
    fn type_name(&self) -> &'static str {
        "Vector2"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        match key.as_str().as_deref() {
            Some("X" | "x") => Ok(num(self.0.x)),
            Some("Y" | "y") => Ok(num(self.0.y)),
            Some("Magnitude") => Ok(num(self.0.length())),
            Some("Unit") => Ok(Lua::userdata(Vector2(self.0.normalize_or_zero()))),
            _ => Err(no_member("Vector2", key)),
        }
    }
    fn arith(&self, _lua: &mut Lua, op: ArithOp, lhs: &Value, rhs: &Value) -> Result<Option<Value>> {
        let operand = |v: &Value| v.as_userdata::<Vector2>().map(|u| u.0).or_else(|| v.to_number().map(|n| Vec2::splat(n as f32)));
        let (Some(a), Some(b)) = (operand(lhs), operand(rhs)) else { return Ok(None) };
        Ok(Some(Lua::userdata(Vector2(match op {
            ArithOp::Add => a + b,
            ArithOp::Sub => a - b,
            ArithOp::Mul => a * b,
            ArithOp::Div => a / b,
            _ => return Ok(None),
        }))))
    }
    fn neg(&self, _lua: &mut Lua) -> Result<Option<Value>> {
        Ok(Some(Lua::userdata(Vector2(-self.0))))
    }
    fn equals(&self, other: &dyn UserData) -> bool {
        other.as_any().downcast_ref::<Vector2>().is_some_and(|o| o.0 == self.0)
    }
    fn to_string(&self) -> String {
        format!("{}, {}", self.0.x, self.0.y)
    }
}

// ----------------------------------------------------------------- UDim2

pub struct UDim2Value(pub UDim2);

impl UserData for UDim2Value {
    fn type_name(&self) -> &'static str {
        "UDim2"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        let u = self.0;
        match key.as_str().as_deref() {
            Some("XScale") => Ok(num(u.x_scale)),
            Some("XOffset") => Ok(num(u.x_offset)),
            Some("YScale") => Ok(num(u.y_scale)),
            Some("YOffset") => Ok(num(u.y_offset)),
            Some("Lerp") => method("UDim2.Lerp", move |_, args| {
                let other = args.get(1).and_then(|v| v.as_userdata::<UDim2Value>()).ok_or_else(|| Error::runtime("UDim2:Lerp expects a UDim2"))?;
                Ok(Ret::One(Lua::userdata(UDim2Value(u.lerp(other.0, num_arg(args, 2, "Lerp")? as f32)))))
            }),
            _ => Err(no_member("UDim2", key)),
        }
    }
    fn arith(&self, _lua: &mut Lua, op: ArithOp, lhs: &Value, rhs: &Value) -> Result<Option<Value>> {
        let (Some(a), Some(b)) = (lhs.as_userdata::<UDim2Value>(), rhs.as_userdata::<UDim2Value>()) else { return Ok(None) };
        let (a, b) = (a.0, b.0);
        let sign = match op {
            ArithOp::Add => 1.0,
            ArithOp::Sub => -1.0,
            _ => return Ok(None),
        };
        Ok(Some(Lua::userdata(UDim2Value(UDim2::new(a.x_scale + sign * b.x_scale, a.x_offset + sign * b.x_offset, a.y_scale + sign * b.y_scale, a.y_offset + sign * b.y_offset)))))
    }
    fn equals(&self, other: &dyn UserData) -> bool {
        other.as_any().downcast_ref::<UDim2Value>().is_some_and(|o| o.0 == self.0)
    }
    fn to_string(&self) -> String {
        let u = self.0;
        format!("{{{}, {}}}, {{{}, {}}}", u.x_scale, u.x_offset, u.y_scale, u.y_offset)
    }
}

// ---------------------------------------------------------------- CFrame

/// A position and orientation.
#[derive(Clone, Copy)]
pub struct CFrame {
    pub position: Vec3,
    pub rotation: Quat,
}

impl UserData for CFrame {
    fn type_name(&self) -> &'static str {
        "CFrame"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        let cf = *self;
        match key.as_str().as_deref() {
            Some("Position" | "p") => Ok(vec(cf.position)),
            Some("X") => Ok(num(cf.position.x)),
            Some("Y") => Ok(num(cf.position.y)),
            Some("Z") => Ok(num(cf.position.z)),
            Some("Rotation") => Ok(vec(quat_to_euler_deg(cf.rotation))),
            Some("LookVector") => Ok(vec(cf.rotation * Vec3::NEG_Z)),
            Some("RightVector") => Ok(vec(cf.rotation * Vec3::X)),
            Some("UpVector") => Ok(vec(cf.rotation * Vec3::Y)),
            Some("Inverse") => method("CFrame.Inverse", move |_, _| {
                let inverse = cf.rotation.inverse();
                Ok(Ret::One(Lua::userdata(CFrame { position: inverse * -cf.position, rotation: inverse })))
            }),
            Some("Lerp") => method("CFrame.Lerp", move |_, args| {
                let other = args.get(1).and_then(|v| v.as_userdata::<CFrame>()).ok_or_else(|| Error::runtime("CFrame:Lerp expects a CFrame"))?;
                let t = num_arg(args, 2, "Lerp")? as f32;
                Ok(Ret::One(Lua::userdata(CFrame { position: cf.position.lerp(other.position, t), rotation: cf.rotation.slerp(other.rotation, t) })))
            }),
            Some("PointToWorldSpace") => method("CFrame.PointToWorldSpace", move |_, args| Ok(Ret::One(vec(cf.position + cf.rotation * vec3_arg(args, 1, "PointToWorldSpace")?)))),
            Some("PointToObjectSpace") => method("CFrame.PointToObjectSpace", move |_, args| Ok(Ret::One(vec(cf.rotation.inverse() * (vec3_arg(args, 1, "PointToObjectSpace")? - cf.position))))),
            _ => Err(no_member("CFrame", key)),
        }
    }
    fn arith(&self, _lua: &mut Lua, op: ArithOp, lhs: &Value, rhs: &Value) -> Result<Option<Value>> {
        let Some(a) = lhs.as_userdata::<CFrame>() else { return Ok(None) };
        Ok(match (op, rhs) {
            (ArithOp::Mul, Value::Vector(..)) => to_vec3(rhs).map(|v| vec(a.position + a.rotation * v)),
            (ArithOp::Add, Value::Vector(..)) => to_vec3(rhs).map(|v| Lua::userdata(CFrame { position: a.position + v, rotation: a.rotation })),
            (ArithOp::Sub, Value::Vector(..)) => to_vec3(rhs).map(|v| Lua::userdata(CFrame { position: a.position - v, rotation: a.rotation })),
            (ArithOp::Mul, other) => other.as_userdata::<CFrame>().map(|b| Lua::userdata(CFrame { position: a.position + a.rotation * b.position, rotation: (a.rotation * b.rotation).normalize() })),
            _ => None,
        })
    }
    fn equals(&self, other: &dyn UserData) -> bool {
        other.as_any().downcast_ref::<CFrame>().is_some_and(|o| o.position == self.position && o.rotation == self.rotation)
    }
    fn to_string(&self) -> String {
        let r = quat_to_euler_deg(self.rotation);
        format!("{}, {}, {} ({}, {}, {})", self.position.x, self.position.y, self.position.z, r.x, r.y, r.z)
    }
}

/// A rotation that turns -Z toward `direction` (the engine's "front").
pub fn look_rotation(direction: Vec3) -> Quat {
    let forward = direction.normalize_or_zero();
    if forward == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let up = if forward.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
    let right = up.cross(-forward).normalize();
    let up = (-forward).cross(right);
    Quat::from_mat3(&araria_core::Mat3::from_cols(right, up, -forward)).normalize()
}

// ------------------------------------------------------------ constructors

fn table_of(lua: &mut Lua, name: &str, entries: Vec<(&'static str, Value)>) -> TableRef {
    let table = lua.new_table();
    for (key, value) in entries {
        table.set_str(key, value);
    }
    lua.set_global(name, table.clone());
    table
}

/// Registers the value-type libraries and vector methods.
pub fn register(lua: &mut Lua) {
    let n = |args: &[Value], i: usize, f: &str| -> Result<f32> { Ok(opt_num(args, i, f, 0.0)? as f32) };

    table_of(
        lua,
        "Vector3",
        vec![
            ("new", Lua::native("Vector3.new", move |_, a| Ok(Ret::One(Value::Vector(n(a, 0, "new")?, n(a, 1, "new")?, n(a, 2, "new")?))))),
            ("zero", vec(Vec3::ZERO)),
            ("one", vec(Vec3::ONE)),
            ("xAxis", vec(Vec3::X)),
            ("yAxis", vec(Vec3::Y)),
            ("zAxis", vec(Vec3::Z)),
        ],
    );
    // `vector.Magnitude`, `vector:Dot(other)`, ...
    lua.set_vector_index(Lua::native("Vector3.__index", |_, args| {
        let v = to_vec3(&args[0]).unwrap_or(Vec3::ZERO);
        let key = args.get(1).cloned().unwrap_or(Value::Nil);
        Ok(Ret::One(match key.as_str().as_deref() {
            Some("Magnitude") => num(v.length()),
            Some("Unit") => vec(v.normalize_or_zero()),
            Some("Dot") => Lua::native("Vector3.Dot", |_, a| Ok(Ret::One(num(vec3_arg(a, 0, "Dot")?.dot(vec3_arg(a, 1, "Dot")?))))),
            Some("Cross") => Lua::native("Vector3.Cross", |_, a| Ok(Ret::One(vec(vec3_arg(a, 0, "Cross")?.cross(vec3_arg(a, 1, "Cross")?))))),
            Some("Lerp") => Lua::native("Vector3.Lerp", |_, a| Ok(Ret::One(vec(vec3_arg(a, 0, "Lerp")?.lerp(vec3_arg(a, 1, "Lerp")?, num_arg(a, 2, "Lerp")? as f32))))),
            Some("FuzzyEq") => Lua::native("Vector3.FuzzyEq", |_, a| {
                let epsilon = opt_num(a, 2, "FuzzyEq", 1e-5)? as f32;
                Ok(Ret::One(Value::Bool(vec3_arg(a, 0, "FuzzyEq")?.distance(vec3_arg(a, 1, "FuzzyEq")?) <= epsilon)))
            }),
            _ => return Err(no_member("Vector3", &key)),
        }))
    }));

    table_of(
        lua,
        "Vector2",
        vec![
            ("new", Lua::native("Vector2.new", move |_, a| Ok(Ret::One(Lua::userdata(Vector2(Vec2::new(n(a, 0, "new")?, n(a, 1, "new")?))))))),
            ("zero", Lua::userdata(Vector2(Vec2::ZERO))),
            ("one", Lua::userdata(Vector2(Vec2::ONE))),
        ],
    );
    table_of(
        lua,
        "Color3",
        vec![
            ("new", Lua::native("Color3.new", move |_, a| Ok(Ret::One(Lua::userdata(Color3(Color::rgb(n(a, 0, "new")?, n(a, 1, "new")?, n(a, 2, "new")?))))))),
            ("fromRGB", Lua::native("Color3.fromRGB", move |_, a| Ok(Ret::One(Lua::userdata(Color3(Color::rgb(n(a, 0, "fromRGB")? / 255.0, n(a, 1, "fromRGB")? / 255.0, n(a, 2, "fromRGB")? / 255.0))))))),
            ("fromHSV", Lua::native("Color3.fromHSV", move |_, a| Ok(Ret::One(Lua::userdata(Color3(Color::from_hsv(n(a, 0, "fromHSV")? * 360.0, n(a, 1, "fromHSV")?, n(a, 2, "fromHSV")?))))))),
            ("fromHex", Lua::native("Color3.fromHex", |_, a| {
                let text = araria_lua::str_arg(a, 0, "fromHex")?;
                let color = Color::from_hex(&text.to_str()).ok_or_else(|| Error::runtime(format!("'{}' is not a hex color", text.to_str())))?;
                Ok(Ret::One(Lua::userdata(Color3(color))))
            })),
        ],
    );
    table_of(
        lua,
        "UDim2",
        vec![
            ("new", Lua::native("UDim2.new", move |_, a| Ok(Ret::One(Lua::userdata(UDim2Value(UDim2::new(n(a, 0, "new")?, n(a, 1, "new")?, n(a, 2, "new")?, n(a, 3, "new")?))))))),
            ("fromScale", Lua::native("UDim2.fromScale", move |_, a| Ok(Ret::One(Lua::userdata(UDim2Value(UDim2::from_scale(n(a, 0, "fromScale")?, n(a, 1, "fromScale")?))))))),
            ("fromOffset", Lua::native("UDim2.fromOffset", move |_, a| Ok(Ret::One(Lua::userdata(UDim2Value(UDim2::from_offset(n(a, 0, "fromOffset")?, n(a, 1, "fromOffset")?))))))),
        ],
    );
    table_of(
        lua,
        "CFrame",
        vec![
            ("new", Lua::native("CFrame.new", move |_, a| {
                // CFrame.new(), CFrame.new(position), CFrame.new(position, lookAt) or CFrame.new(x, y, z)
                let cf = match (a.first().and_then(to_vec3), a.get(1).and_then(to_vec3)) {
                    (Some(position), Some(target)) => CFrame { position, rotation: look_rotation(target - position) },
                    (Some(position), None) => CFrame { position, rotation: Quat::IDENTITY },
                    _ => CFrame { position: Vec3::new(n(a, 0, "new")?, n(a, 1, "new")?, n(a, 2, "new")?), rotation: Quat::IDENTITY },
                };
                Ok(Ret::One(Lua::userdata(cf)))
            })),
            ("lookAt", Lua::native("CFrame.lookAt", |_, a| {
                let (position, target) = (vec3_arg(a, 0, "lookAt")?, vec3_arg(a, 1, "lookAt")?);
                Ok(Ret::One(Lua::userdata(CFrame { position, rotation: look_rotation(target - position) })))
            })),
            ("Angles", Lua::native("CFrame.Angles", move |_, a| {
                // Radians, like the API scripters know.
                let degrees = Vec3::new(n(a, 0, "Angles")?, n(a, 1, "Angles")?, n(a, 2, "Angles")?) * (180.0 / std::f32::consts::PI);
                Ok(Ret::One(Lua::userdata(CFrame { position: Vec3::ZERO, rotation: euler_deg_to_quat(degrees) })))
            })),
            ("fromOrientation", Lua::native("CFrame.fromOrientation", move |_, a| {
                let degrees = Vec3::new(n(a, 0, "fromOrientation")?, n(a, 1, "fromOrientation")?, n(a, 2, "fromOrientation")?);
                Ok(Ret::One(Lua::userdata(CFrame { position: Vec3::ZERO, rotation: euler_deg_to_quat(degrees) })))
            })),
        ],
    );
}

// ------------------------------------------------------------ conversions

/// Converts a property value to a Lua value.
pub fn variant_to_value(host: &Rc<Host>, value: &Variant) -> Value {
    match value {
        Variant::Nil => Value::Nil,
        Variant::Bool(b) => Value::Bool(*b),
        Variant::Number(n) => match araria_lua::float_to_int_exact(*n) {
            Some(i) if n.abs() < 1e15 => Value::Int(i),
            _ => Value::Num(*n),
        },
        Variant::String(s) => Value::from(s.as_str()),
        Variant::Vec2(v) => Lua::userdata(Vector2(*v)),
        Variant::Vec3(v) => vec(*v),
        Variant::Quat(q) => vec(quat_to_euler_deg(*q)),
        Variant::Color(c) => Lua::userdata(Color3(*c)),
        Variant::UDim2(u) => Lua::userdata(UDim2Value(*u)),
        Variant::Ref(Some(id)) => instance_value(host, *id),
        Variant::Ref(None) => Value::Nil,
    }
}

/// Converts a Lua value for a property of the given type.
pub fn value_to_variant(value: &Value, ty: &PropType) -> std::result::Result<Variant, String> {
    let mismatch = |expected: &str| {
        let got = match value {
            Value::UserData(u) => u.type_name(),
            other => other.type_name(),
        };
        Err(format!("{expected} expected, got {got}"))
    };
    Ok(match ty {
        PropType::Bool => match value {
            Value::Bool(b) => Variant::Bool(*b),
            _ => return mismatch("boolean"),
        },
        PropType::Number { .. } => match value {
            Value::Int(_) | Value::Num(_) => Variant::Number(value.to_number().unwrap_or(0.0)),
            _ => return mismatch("number"),
        },
        PropType::String | PropType::Enum(_) | PropType::Asset(_) => match value {
            Value::Str(s) => Variant::String(s.to_str().into_owned()),
            _ => return mismatch("string"),
        },
        PropType::Vec2 => match value.as_userdata::<Vector2>() {
            Some(v) => Variant::Vec2(v.0),
            None => return mismatch("Vector2"),
        },
        PropType::Vec3 => match to_vec3(value) {
            Some(v) => Variant::Vec3(v),
            None => return mismatch("Vector3"),
        },
        PropType::Rotation => match (to_vec3(value), value.as_userdata::<CFrame>()) {
            (Some(degrees), _) => Variant::Quat(euler_deg_to_quat(degrees)),
            (_, Some(cf)) => Variant::Quat(cf.rotation),
            _ => return mismatch("Vector3"),
        },
        PropType::Color => match value.as_userdata::<Color3>() {
            Some(c) => Variant::Color(c.0),
            None => return mismatch("Color3"),
        },
        PropType::UDim2 => match value.as_userdata::<UDim2Value>() {
            Some(u) => Variant::UDim2(u.0),
            None => return mismatch("UDim2"),
        },
        PropType::Ref => match value {
            Value::Nil => Variant::Ref(None),
            other => match other.as_userdata::<InstanceRef>() {
                Some(instance) => Variant::Ref(Some(instance.id)),
                None => return mismatch("Instance"),
            },
        },
    })
}

/// Converts a Lua value to an untyped variant (attributes, event payloads).
pub fn value_to_variant_any(value: &Value) -> Option<Variant> {
    Some(match value {
        Value::Nil => Variant::Nil,
        Value::Bool(b) => Variant::Bool(*b),
        Value::Int(_) | Value::Num(_) => Variant::Number(value.to_number()?),
        Value::Str(s) => Variant::String(s.to_str().into_owned()),
        Value::Vector(..) => Variant::Vec3(to_vec3(value)?),
        Value::UserData(_) => {
            if let Some(c) = value.as_userdata::<Color3>() {
                Variant::Color(c.0)
            } else if let Some(v) = value.as_userdata::<Vector2>() {
                Variant::Vec2(v.0)
            } else if let Some(u) = value.as_userdata::<UDim2Value>() {
                Variant::UDim2(u.0)
            } else if let Some(i) = value.as_userdata::<InstanceRef>() {
                Variant::Ref(Some(i.id))
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

/// Converts a Lua value to JSON for saving (numbers, strings, booleans and
/// plain tables).
pub fn value_to_json(value: &Value, depth: u32) -> std::result::Result<serde_json::Value, String> {
    use serde_json::Value as J;
    if depth > 32 {
        return Err("table is nested too deeply to save".into());
    }
    Ok(match value {
        Value::Nil => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::from(*i),
        Value::Num(n) => serde_json::Number::from_f64(*n).map_or(J::Null, J::Number),
        Value::Str(s) => J::String(s.to_str().into_owned()),
        Value::Table(t) => {
            let table = t.borrow();
            let length = table.len();
            let pairs: Vec<(Value, Value)> = table.iter().collect();
            if length > 0 && pairs.len() as i64 == length {
                J::Array((1..=length).map(|i| value_to_json(&table.get_int(i), depth + 1)).collect::<std::result::Result<_, _>>()?)
            } else {
                let mut map = serde_json::Map::new();
                for (k, v) in pairs {
                    let key = match &k {
                        Value::Str(s) => s.to_str().into_owned(),
                        Value::Int(i) => i.to_string(),
                        other => return Err(format!("cannot save a table with {} keys", other.type_name())),
                    };
                    map.insert(key, value_to_json(&v, depth + 1)?);
                }
                J::Object(map)
            }
        }
        other => return Err(format!("cannot save a {} value", other.type_name())),
    })
}

pub fn json_to_value(lua: &mut Lua, json: &serde_json::Value) -> Value {
    use serde_json::Value as J;
    match json {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => n.as_i64().map_or_else(|| Value::Num(n.as_f64().unwrap_or(0.0)), Value::Int),
        J::String(s) => Value::from(s.as_str()),
        J::Array(items) => {
            let values: Vec<Value> = items.iter().map(|v| json_to_value(lua, v)).collect();
            Value::Table(lua.new_array(values))
        }
        J::Object(map) => {
            let table = lua.new_table();
            for (k, v) in map {
                let value = json_to_value(lua, v);
                table.set_str(k, value);
            }
            Value::Table(table)
        }
    }
}
