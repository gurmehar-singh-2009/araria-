//! Lua values and the heap objects they refer to.

use std::any::Any;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::rc::Rc;

use indexmap::IndexMap;

use crate::compiler::Proto;
use crate::error::{Error, Result};
use crate::vm::{Lua, ThreadState};

/// Per-object bookkeeping used by the cycle collector (see [`crate::gc`]).
#[derive(Default)]
pub(crate) struct GcHeader {
    pub(crate) refs: Cell<u32>,
    pub(crate) marked: Cell<bool>,
}

// ---------------------------------------------------------------- strings

pub(crate) struct LStr {
    hash: u64,
    bytes: Box<[u8]>,
}

/// An immutable Lua string. Lua strings are byte strings; they are usually,
/// but not necessarily, valid UTF-8.
#[derive(Clone)]
pub struct LuaStr(Rc<LStr>);

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

impl LuaStr {
    pub fn new(bytes: &[u8]) -> Self {
        LuaStr(Rc::new(LStr { hash: fnv1a(bytes), bytes: bytes.into() }))
    }

    pub fn from_vec(bytes: Vec<u8>) -> Self {
        LuaStr(Rc::new(LStr { hash: fnv1a(&bytes), bytes: bytes.into_boxed_slice() }))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0.bytes
    }

    pub fn len(&self) -> usize {
        self.0.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.bytes.is_empty()
    }

    /// The string as UTF-8, replacing invalid sequences.
    pub fn to_str(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0.bytes)
    }

    pub(crate) fn hash_value(&self) -> u64 {
        self.0.hash
    }
}

impl PartialEq for LuaStr {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
            || (self.0.hash == other.0.hash && self.0.bytes == other.0.bytes)
    }
}

impl From<&str> for LuaStr {
    fn from(s: &str) -> Self {
        LuaStr::new(s.as_bytes())
    }
}

impl From<String> for LuaStr {
    fn from(s: String) -> Self {
        LuaStr::from_vec(s.into_bytes())
    }
}

impl fmt::Debug for LuaStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_str())
    }
}

// ----------------------------------------------------------------- tables

/// Hasher for table keys: [`Value::hash`] writes exactly one pre-mixed `u64`.
#[derive(Default)]
pub struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        self.0 = fnv1a(bytes);
    }
    fn write_u64(&mut self, n: u64) {
        // Fibonacci mixing spreads sequential integers across buckets.
        self.0 = n.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(26);
    }
}

type KeyMap = IndexMap<Value, Value, BuildHasherDefault<KeyHasher>>;

/// The contents of a Lua table: a dense array part plus an insertion-ordered
/// hash part. Iteration order is stable, which keeps `pairs` deterministic.
#[derive(Default)]
pub struct Table {
    pub(crate) arr: Vec<Value>,
    pub(crate) map: KeyMap,
    pub(crate) meta: Option<TableRef>,
    /// Number of hash entries whose value is nil (kept so `next` stays valid
    /// while a traversal clears fields).
    tombstones: usize,
}

impl Table {
    pub fn get(&self, key: &Value) -> Value {
        match key {
            Value::Int(i) => self.get_int(*i),
            Value::Num(n) => match float_to_int_exact(*n) {
                Some(i) => self.get_int(i),
                None => self.map.get(key).cloned().unwrap_or(Value::Nil),
            },
            Value::Nil => Value::Nil,
            _ => self.map.get(key).cloned().unwrap_or(Value::Nil),
        }
    }

    pub fn get_int(&self, i: i64) -> Value {
        if i >= 1 && (i as usize) <= self.arr.len() {
            return self.arr[i as usize - 1].clone();
        }
        if self.map.is_empty() {
            return Value::Nil;
        }
        self.map.get(&Value::Int(i)).cloned().unwrap_or(Value::Nil)
    }

    pub fn get_str(&self, key: &str) -> Value {
        if self.map.is_empty() {
            return Value::Nil;
        }
        self.map.get(&Value::Str(LuaStr::from(key))).cloned().unwrap_or(Value::Nil)
    }

    /// Raw assignment. Fails only for `nil` and NaN keys.
    pub fn set(&mut self, key: Value, value: Value) -> Result<()> {
        match key {
            Value::Int(i) => {
                self.set_int(i, value);
                Ok(())
            }
            Value::Num(n) => {
                if let Some(i) = float_to_int_exact(n) {
                    self.set_int(i, value);
                    Ok(())
                } else if n.is_nan() {
                    Err(Error::runtime("table index is NaN"))
                } else {
                    self.set_hash(Value::Num(n), value);
                    Ok(())
                }
            }
            Value::Nil => Err(Error::runtime("table index is nil")),
            key => {
                self.set_hash(key, value);
                Ok(())
            }
        }
    }

    pub fn set_str(&mut self, key: &str, value: Value) {
        self.set_hash(Value::Str(LuaStr::from(key)), value);
    }

    pub fn set_int(&mut self, i: i64, value: Value) {
        let len = self.arr.len();
        if i >= 1 && (i as usize) <= len {
            let idx = i as usize - 1;
            if value.is_nil() && idx + 1 == len {
                self.arr.pop();
                while matches!(self.arr.last(), Some(Value::Nil)) {
                    self.arr.pop();
                }
            } else {
                self.arr[idx] = value;
            }
        } else if i as usize == len + 1 && i >= 1 {
            if value.is_nil() {
                if !self.map.is_empty() {
                    self.set_hash(Value::Int(i), value);
                }
                return;
            }
            if !self.map.is_empty() {
                self.map.shift_remove(&Value::Int(i));
            }
            self.arr.push(value);
            // Migrate any following integer keys out of the hash part.
            while !self.map.is_empty() {
                let next = Value::Int(self.arr.len() as i64 + 1);
                match self.map.shift_remove(&next) {
                    Some(v) if !v.is_nil() => self.arr.push(v),
                    Some(_) => {
                        self.tombstones = self.tombstones.saturating_sub(1);
                        break;
                    }
                    None => break,
                }
            }
        } else {
            self.set_hash(Value::Int(i), value);
        }
    }

    fn set_hash(&mut self, key: Value, value: Value) {
        if value.is_nil() {
            if let Some(slot) = self.map.get_mut(&key) {
                if !slot.is_nil() {
                    *slot = Value::Nil;
                    self.tombstones += 1;
                }
            }
            return;
        }
        match self.map.entry(key) {
            indexmap::map::Entry::Occupied(mut e) => {
                if e.get().is_nil() {
                    self.tombstones = self.tombstones.saturating_sub(1);
                }
                e.insert(value);
            }
            indexmap::map::Entry::Vacant(e) => {
                e.insert(value);
                if self.tombstones > 16 && self.tombstones * 2 > self.map.len() {
                    self.map.retain(|_, v| !v.is_nil());
                    self.tombstones = 0;
                }
            }
        }
    }

    /// The border used by the `#` operator.
    pub fn len(&self) -> i64 {
        let n = self.arr.len() as i64;
        if self.map.is_empty() {
            return n;
        }
        let mut n = n;
        while self.map.get(&Value::Int(n + 1)).is_some_and(|v| !v.is_nil()) {
            n += 1;
        }
        n
    }

    pub fn is_empty(&self) -> bool {
        self.arr.is_empty() && self.map.values().all(Value::is_nil)
    }

    /// Stateless traversal step used by `next` and `pairs`.
    pub fn next(&self, key: &Value) -> Result<Option<(Value, Value)>> {
        let mut hash_start = 0usize;
        let arr_start = match key {
            Value::Nil => Some(0usize),
            Value::Int(i) if *i >= 1 && (*i as usize) <= self.arr.len() => Some(*i as usize),
            Value::Num(n) => match float_to_int_exact(*n) {
                Some(i) if i >= 1 && (i as usize) <= self.arr.len() => Some(i as usize),
                _ => None,
            },
            _ => None,
        };
        if let Some(start) = arr_start {
            for (idx, v) in self.arr.iter().enumerate().skip(start) {
                if !v.is_nil() {
                    return Ok(Some((Value::Int(idx as i64 + 1), v.clone())));
                }
            }
        } else {
            match self.map.get_index_of(key) {
                Some(i) => hash_start = i + 1,
                // A positive integer key that is no longer present was an
                // array element removed during the traversal (clearing
                // fields while iterating is allowed); the array part is
                // exhausted, so continue with the hash part.
                None if key.to_integer().is_some_and(|i| i >= 1) => {}
                None => return Err(Error::runtime("invalid key to 'next'")),
            }
        }
        for i in hash_start..self.map.len() {
            let (k, v) = self.map.get_index(i).expect("index in range");
            if !v.is_nil() {
                return Ok(Some((k.clone(), v.clone())));
            }
        }
        Ok(None)
    }

    pub fn array(&self) -> &[Value] {
        &self.arr
    }

    pub fn array_mut(&mut self) -> &mut Vec<Value> {
        &mut self.arr
    }

    /// Live (non-nil) key/value pairs in traversal order.
    pub fn iter(&self) -> impl Iterator<Item = (Value, Value)> + '_ {
        let arr = self
            .arr
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.is_nil())
            .map(|(i, v)| (Value::Int(i as i64 + 1), v.clone()));
        let map = self.map.iter().filter(|(_, v)| !v.is_nil()).map(|(k, v)| (k.clone(), v.clone()));
        arr.chain(map)
    }

    pub fn metatable(&self) -> Option<TableRef> {
        self.meta.clone()
    }

    pub fn set_metatable(&mut self, meta: Option<TableRef>) {
        self.meta = meta;
    }

    pub fn clear(&mut self) {
        self.arr.clear();
        self.map.clear();
        self.tombstones = 0;
    }

    pub(crate) fn clear_all(&mut self) {
        self.clear();
        self.meta = None;
    }
}

pub(crate) struct TableBox {
    pub(crate) gc: GcHeader,
    pub(crate) t: RefCell<Table>,
}

/// A shared, mutable reference to a table.
#[derive(Clone)]
pub struct TableRef(pub(crate) Rc<TableBox>);

impl TableRef {
    pub(crate) fn new_untracked(t: Table) -> Self {
        TableRef(Rc::new(TableBox { gc: GcHeader::default(), t: RefCell::new(t) }))
    }

    pub fn borrow(&self) -> std::cell::Ref<'_, Table> {
        self.0.t.borrow()
    }

    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, Table> {
        self.0.t.borrow_mut()
    }

    pub fn get(&self, key: &Value) -> Value {
        self.0.t.borrow().get(key)
    }

    pub fn get_str(&self, key: &str) -> Value {
        self.0.t.borrow().get_str(key)
    }

    pub fn get_int(&self, i: i64) -> Value {
        self.0.t.borrow().get_int(i)
    }

    pub fn set(&self, key: Value, value: Value) -> Result<()> {
        self.0.t.borrow_mut().set(key, value)
    }

    pub fn set_str(&self, key: &str, value: impl Into<Value>) {
        self.0.t.borrow_mut().set_str(key, value.into());
    }

    pub fn set_int(&self, i: i64, value: impl Into<Value>) {
        self.0.t.borrow_mut().set_int(i, value.into());
    }

    pub fn push(&self, value: impl Into<Value>) {
        let mut t = self.0.t.borrow_mut();
        let n = t.len();
        t.set_int(n + 1, value.into());
    }

    pub fn len(&self) -> i64 {
        self.0.t.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.t.borrow().is_empty()
    }

    pub fn ptr_eq(&self, other: &TableRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn addr(&self) -> usize {
        Rc::as_ptr(&self.0) as *const () as usize
    }
}

// -------------------------------------------------------------- functions

/// A heap cell holding a local variable captured by a closure.
pub(crate) struct CellBox {
    pub(crate) gc: GcHeader,
    pub(crate) v: RefCell<Value>,
}

#[derive(Clone)]
pub struct CellRef(pub(crate) Rc<CellBox>);

impl CellRef {
    pub(crate) fn get(&self) -> Value {
        self.0.v.borrow().clone()
    }
    pub(crate) fn set(&self, v: Value) {
        *self.0.v.borrow_mut() = v;
    }
}

/// A Lua function instance: compiled code plus captured variables.
pub struct Closure {
    pub(crate) gc: GcHeader,
    pub(crate) proto: Rc<Proto>,
    pub(crate) upvals: Vec<CellRef>,
    /// The table global variable accesses resolve against.
    pub(crate) env: TableRef,
}

/// What a native function hands back to the interpreter.
pub enum Ret {
    /// Return these values to the caller.
    Values(Vec<Value>),
    /// Return a single value (avoids allocating).
    One(Value),
    /// Return nothing.
    None,
    /// Suspend the running coroutine, passing these values to its resumer.
    /// The values given to the next resume become the native call's results.
    Yield(Vec<Value>),
    /// Call `func` in protected mode (the mechanism behind `pcall`/`xpcall`).
    Protected { func: Value, args: Vec<Value>, handler: Option<Value> },
    /// Replace this call with a call to `func` (keeps Lua frames yieldable).
    Call { func: Value, args: Vec<Value> },
}

impl From<Value> for Ret {
    fn from(v: Value) -> Self {
        Ret::One(v)
    }
}

impl From<Vec<Value>> for Ret {
    fn from(v: Vec<Value>) -> Self {
        Ret::Values(v)
    }
}

pub type NativeFn = dyn Fn(&mut Lua, &[Value]) -> Result<Ret>;

/// A function implemented in Rust.
pub struct Native {
    pub name: Cow<'static, str>,
    pub(crate) f: Box<NativeFn>,
}

// --------------------------------------------------------------- userdata

/// Binary operators that can be overloaded by userdata and metatables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    IDiv,
}

impl ArithOp {
    pub(crate) fn event(self) -> &'static str {
        match self {
            ArithOp::Add => "__add",
            ArithOp::Sub => "__sub",
            ArithOp::Mul => "__mul",
            ArithOp::Div => "__div",
            ArithOp::Mod => "__mod",
            ArithOp::Pow => "__pow",
            ArithOp::IDiv => "__idiv",
        }
    }
}

/// Host objects exposed to Lua. The trait methods play the role of
/// metamethods; all have conservative defaults.
pub trait UserData: 'static {
    /// The name reported by `type()` errors and `typeof`.
    fn type_name(&self) -> &'static str;
    fn as_any(&self) -> &dyn Any;

    /// `object[key]` / `object.key`.
    fn index(&self, _lua: &mut Lua, key: &Value) -> Result<Value> {
        Err(Error::runtime(format!(
            "{} is not a valid member of {}",
            key.to_display(),
            self.type_name()
        )))
    }

    /// `object[key] = value`.
    fn new_index(&self, _lua: &mut Lua, key: &Value, _value: Value) -> Result<()> {
        Err(Error::runtime(format!(
            "cannot assign {} on {}",
            key.to_display(),
            self.type_name()
        )))
    }

    /// Arithmetic where at least one operand is this userdata. Return
    /// `Ok(None)` when the operation is not supported.
    fn arith(&self, _lua: &mut Lua, _op: ArithOp, _lhs: &Value, _rhs: &Value) -> Result<Option<Value>> {
        Ok(None)
    }

    fn neg(&self, _lua: &mut Lua) -> Result<Option<Value>> {
        Ok(None)
    }

    /// `==` between two *different* userdata objects.
    fn equals(&self, _other: &dyn UserData) -> bool {
        false
    }

    fn less_than(&self, _other: &dyn UserData) -> Option<bool> {
        None
    }

    fn length(&self) -> Option<i64> {
        None
    }

    fn to_string(&self) -> String {
        self.type_name().to_string()
    }

    /// `object(...)`.
    fn call(&self, _lua: &mut Lua, _args: &[Value]) -> Result<Ret> {
        Err(Error::runtime(format!("attempt to call a {} value", self.type_name())))
    }
}

/// A suspended or running coroutine.
pub struct Thread {
    pub(crate) gc: GcHeader,
    /// `None` while the coroutine is running (its state is owned by the
    /// interpreter loop) or after it has finished.
    pub(crate) state: RefCell<Option<Box<ThreadState>>>,
    pub(crate) status: Cell<ThreadStatus>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreadStatus {
    Suspended,
    Running,
    /// Resumed another coroutine and is waiting for it.
    Normal,
    Dead,
}

impl ThreadStatus {
    pub fn name(self) -> &'static str {
        match self {
            ThreadStatus::Suspended => "suspended",
            ThreadStatus::Running => "running",
            ThreadStatus::Normal => "normal",
            ThreadStatus::Dead => "dead",
        }
    }
}

#[derive(Clone)]
pub struct ThreadRef(pub(crate) Rc<Thread>);

impl ThreadRef {
    pub fn status(&self) -> ThreadStatus {
        self.0.status.get()
    }

    pub fn ptr_eq(&self, other: &ThreadRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

// ------------------------------------------------------------------ value

/// Any Lua value.
#[derive(Clone, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    /// A 3-component vector value type (allocation-free, like Luau's `vector`).
    Vector(f32, f32, f32),
    Str(LuaStr),
    Table(TableRef),
    Function(Rc<Closure>),
    Native(Rc<Native>),
    UserData(Rc<dyn UserData>),
    Thread(ThreadRef),
    /// Interpreter-internal: a captured local living on the stack.
    #[doc(hidden)]
    Cell(CellRef),
}

impl Value {
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Lua truthiness: everything except `nil` and `false`.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn str(s: &str) -> Value {
        Value::Str(LuaStr::from(s))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Num(_) => "number",
            Value::Vector(..) => "vector",
            Value::Str(_) => "string",
            Value::Table(_) => "table",
            Value::Function(_) | Value::Native(_) => "function",
            Value::UserData(_) => "userdata",
            Value::Thread(_) => "thread",
            Value::Cell(_) => "cell",
        }
    }

    /// Numeric value with string coercion, as arithmetic does.
    pub fn to_number(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Num(n) => Some(*n),
            Value::Str(s) => match crate::lexer::parse_number(s.to_str().trim()) {
                Some(Value::Int(i)) => Some(i as f64),
                Some(Value::Num(n)) => Some(n),
                _ => None,
            },
            _ => None,
        }
    }

    /// Integer value if the number has an exact integer representation.
    pub fn to_integer(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Num(n) => float_to_int_exact(*n),
            Value::Str(s) => match crate::lexer::parse_number(s.to_str().trim()) {
                Some(Value::Int(i)) => Some(i),
                Some(Value::Num(n)) => float_to_int_exact(n),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<Cow<'_, str>> {
        match self {
            Value::Str(s) => Some(s.to_str()),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&TableRef> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Downcast a userdata value to a concrete host type.
    pub fn as_userdata<T: UserData>(&self) -> Option<&T> {
        match self {
            Value::UserData(u) => u.as_any().downcast_ref::<T>(),
            _ => None,
        }
    }

    pub fn is_function(&self) -> bool {
        matches!(self, Value::Function(_) | Value::Native(_))
    }

    /// Conversion performed by `tostring` for values without a `__tostring`
    /// metamethod.
    pub fn to_display(&self) -> String {
        match self {
            Value::Nil => "nil".into(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Num(n) => fmt_number(*n),
            Value::Vector(x, y, z) => format!("{}, {}, {}", fmt_f32(*x), fmt_f32(*y), fmt_f32(*z)),
            Value::Str(s) => s.to_str().into_owned(),
            Value::Table(t) => format!("table: 0x{:08x}", t.addr()),
            Value::Function(f) => format!("function: 0x{:08x}", Rc::as_ptr(f) as usize),
            Value::Native(f) => format!("function: builtin: {}", f.name),
            Value::UserData(u) => u.to_string(),
            Value::Thread(t) => format!("thread: 0x{:08x}", Rc::as_ptr(&t.0) as usize),
            Value::Cell(_) => "<cell>".into(),
        }
    }

    /// Raw equality (no metamethods). Integers and floats compare by value.
    pub fn raw_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Int(a), Value::Num(b)) | (Value::Num(b), Value::Int(a)) => (*a as f64) == *b,
            (Value::Vector(a, b, c), Value::Vector(x, y, z)) => a == x && b == y && c == z,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Table(a), Value::Table(b)) => a.ptr_eq(b),
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            (Value::Native(a), Value::Native(b)) => Rc::ptr_eq(a, b),
            (Value::UserData(a), Value::UserData(b)) => {
                std::ptr::addr_eq(Rc::as_ptr(a), Rc::as_ptr(b))
            }
            (Value::Thread(a), Value::Thread(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.raw_eq(other)
    }
}

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let h: u64 = match self {
            Value::Nil => 0,
            Value::Bool(b) => 1 + u64::from(*b),
            Value::Int(i) => *i as u64,
            Value::Num(n) => match float_to_int_exact(*n) {
                Some(i) => i as u64,
                None => n.to_bits(),
            },
            Value::Vector(x, y, z) => {
                (u64::from(x.to_bits()) << 32 | u64::from(y.to_bits())) ^ u64::from(z.to_bits()).rotate_left(17)
            }
            Value::Str(s) => s.hash_value(),
            Value::Table(t) => t.addr() as u64,
            Value::Function(f) => Rc::as_ptr(f) as usize as u64,
            Value::Native(f) => Rc::as_ptr(f) as usize as u64,
            Value::UserData(u) => Rc::as_ptr(u) as *const () as usize as u64,
            Value::Thread(t) => Rc::as_ptr(&t.0) as usize as u64,
            Value::Cell(c) => Rc::as_ptr(&c.0) as usize as u64,
        };
        state.write_u64(h);
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "{:?}", s.to_str()),
            other => f.write_str(&other.to_display()),
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(i64::from(v))
    }
}
impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Value::Int(v as i64)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Num(v)
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Num(f64::from(v))
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(LuaStr::from(v))
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(LuaStr::from(v))
    }
}
impl From<LuaStr> for Value {
    fn from(v: LuaStr) -> Self {
        Value::Str(v)
    }
}
impl From<TableRef> for Value {
    fn from(v: TableRef) -> Self {
        Value::Table(v)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Nil, Into::into)
    }
}

/// Converts a float to an integer only when the conversion is exact.
pub fn float_to_int_exact(n: f64) -> Option<i64> {
    if n.fract() == 0.0 && n >= -9_223_372_036_854_775_808.0 && n < 9_223_372_036_854_775_808.0 {
        Some(n as i64)
    } else {
        None
    }
}

/// Formats a float the way Lua's `%.14g` does, keeping a trailing `.0` on
/// integral values so floats stay distinguishable from integers.
pub fn fmt_number(n: f64) -> String {
    if n.is_nan() {
        return if n.is_sign_negative() { "-nan".into() } else { "nan".into() };
    }
    if n.is_infinite() {
        return if n > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if n.fract() == 0.0 && n.abs() < 1e16 {
        return format!("{n:.1}");
    }
    fmt_g(n, 14)
}

fn fmt_f32(n: f32) -> String {
    if n.fract() == 0.0 && n.abs() < 1e7 {
        format!("{n:.0}")
    } else {
        fmt_g(f64::from(n), 7)
    }
}

/// `%.{precision}g` formatting.
pub fn fmt_g(n: f64, precision: usize) -> String {
    if n == 0.0 {
        return if n.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !n.is_finite() {
        return if n.is_nan() { "nan".into() } else if n > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let precision = precision.max(1);
    let exp = n.abs().log10().floor() as i32;
    // Round to `precision` significant digits first; rounding can bump the exponent.
    let sci = format!("{:.*e}", precision - 1, n);
    let (mantissa, exp_str) = sci.split_once('e').expect("scientific format");
    let exp10: i32 = exp_str.parse().unwrap_or(exp);
    if exp10 < -4 || exp10 >= precision as i32 {
        let mantissa = trim_zeros(mantissa);
        let sign = if exp10 < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exp10.abs())
    } else {
        let decimals = (precision as i32 - 1 - exp10).max(0) as usize;
        trim_zeros(&format!("{n:.decimals$}")).to_string()
    }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}
