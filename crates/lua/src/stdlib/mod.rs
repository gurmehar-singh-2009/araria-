//! The standard library: a sandboxed subset of Lua 5.4's, plus a few of the
//! Luau conveniences game scripters expect (`table.find`, `math.clamp`,
//! `string.split`, ...). There is deliberately no `io`, no file or process
//! access, and no bytecode loading.

mod base;
mod coroutine;
mod math;
mod os;
pub mod pattern;
mod string;
mod table;

use crate::error::{Error, Result};
use crate::value::{LuaStr, TableRef, Value};
use crate::vm::Lua;

/// Installs every library into the state's globals.
pub fn open(lua: &mut Lua) {
    base::open(lua);
    string::open(lua);
    table::open(lua);
    math::open(lua);
    os::open(lua);
    coroutine::open(lua);
}

// ----------------------------------------------------- argument checking

fn arg(args: &[Value], i: usize) -> &Value {
    const NIL: Value = Value::Nil;
    args.get(i).unwrap_or(&NIL)
}

/// The standard "bad argument" error.
pub fn arg_error(i: usize, func: &str, expected: &str, got: &Value) -> Error {
    let got = match got {
        Value::Nil if expected.is_empty() => String::new(),
        Value::UserData(u) => u.type_name().to_string(),
        other => other.type_name().to_string(),
    };
    Error::runtime(format!(
        "invalid argument #{} to '{}' ({} expected, got {})",
        i + 1,
        func,
        expected,
        if got == "nil" { "no value".to_string() } else { got }
    ))
}

pub fn int_arg(args: &[Value], i: usize, func: &str) -> Result<i64> {
    match arg(args, i) {
        Value::Int(n) => Ok(*n),
        v @ (Value::Num(_) | Value::Str(_)) => v.to_integer().ok_or_else(|| {
            if v.to_number().is_some() {
                Error::runtime(format!(
                    "invalid argument #{} to '{}' (number has no integer representation)",
                    i + 1,
                    func
                ))
            } else {
                arg_error(i, func, "number", v)
            }
        }),
        other => Err(arg_error(i, func, "number", other)),
    }
}

pub fn opt_int(args: &[Value], i: usize, func: &str, default: i64) -> Result<i64> {
    if arg(args, i).is_nil() { Ok(default) } else { int_arg(args, i, func) }
}

pub fn num_arg(args: &[Value], i: usize, func: &str) -> Result<f64> {
    let v = arg(args, i);
    v.to_number().ok_or_else(|| arg_error(i, func, "number", v))
}

pub fn opt_num(args: &[Value], i: usize, func: &str, default: f64) -> Result<f64> {
    if arg(args, i).is_nil() { Ok(default) } else { num_arg(args, i, func) }
}

/// A string argument; numbers are converted, as in Lua.
pub fn str_arg(args: &[Value], i: usize, func: &str) -> Result<LuaStr> {
    match arg(args, i) {
        Value::Str(s) => Ok(s.clone()),
        v @ (Value::Int(_) | Value::Num(_)) => Ok(LuaStr::from(v.to_display())),
        other => Err(arg_error(i, func, "string", other)),
    }
}

pub fn table_arg(args: &[Value], i: usize, func: &str) -> Result<TableRef> {
    match arg(args, i) {
        Value::Table(t) => Ok(t.clone()),
        other => Err(arg_error(i, func, "table", other)),
    }
}

pub fn func_arg(args: &[Value], i: usize, func: &str) -> Result<Value> {
    match arg(args, i) {
        v @ (Value::Function(_) | Value::Native(_)) => Ok(v.clone()),
        other => Err(arg_error(i, func, "function", other)),
    }
}

/// Registers `functions` in a new table stored as global `name`.
fn register(
    lua: &mut Lua,
    name: &'static str,
    functions: Vec<(&'static str, BoxedNative)>,
) -> TableRef {
    let table = lua.new_table();
    for (fname, f) in functions {
        table.set_str(fname, Lua::native(format!("{name}.{fname}"), f));
    }
    lua.set_global(name, table.clone());
    table
}

type BoxedNative = Box<dyn Fn(&mut Lua, &[Value]) -> Result<crate::value::Ret>>;

/// Boxes a closure; going through a generic function lets the compiler infer
/// the closure's argument types.
pub(crate) fn boxed(f: impl Fn(&mut Lua, &[Value]) -> Result<crate::value::Ret> + 'static) -> BoxedNative {
    Box::new(f)
}

/// Builds the `(name, boxed closure)` list `register` expects.
macro_rules! lib {
    ($( $name:literal => $f:expr ),* $(,)?) => {
        vec![ $( ($name, $crate::stdlib::boxed($f)) ),* ]
    };
}
pub(crate) use lib;
