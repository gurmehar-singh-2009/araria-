//! Basic functions: `print`, `type`, `pairs`, `pcall`, ...

use super::{arg, arg_error, int_arg, opt_int, str_arg, table_arg};
use crate::error::{Error, Result};
use crate::value::{Ret, Value};
use crate::vm::Lua;

pub(super) fn open(lua: &mut Lua) {
    let globals = lua.globals();
    let def = |name: &'static str, f: fn(&mut Lua, &[Value]) -> Result<Ret>| {
        globals.set_str(name, Lua::native(name, f));
    };

    def("print", |lua, args| {
        let mut parts = Vec::with_capacity(args.len());
        for v in args {
            parts.push(lua.tostring(v)?);
        }
        lua.print(&parts.join("\t"));
        Ok(Ret::None)
    });
    def("type", |_, args| {
        if args.is_empty() {
            return Err(arg_error(0, "type", "value", &Value::Nil));
        }
        Ok(Ret::One(Value::str(args[0].type_name())))
    });
    def("tostring", |lua, args| Ok(Ret::One(Value::from(lua.tostring(arg(args, 0))?))));
    def("tonumber", |_, args| {
        let v = arg(args, 0);
        if arg(args, 1).is_nil() {
            return Ok(Ret::One(match v {
                Value::Int(_) | Value::Num(_) => v.clone(),
                Value::Str(s) => crate::lexer::parse_number(s.to_str().trim()).unwrap_or(Value::Nil),
                _ => Value::Nil,
            }));
        }
        let base = int_arg(args, 1, "tonumber")?;
        if !(2..=36).contains(&base) {
            return Err(Error::runtime("invalid argument #2 to 'tonumber' (base out of range)"));
        }
        let text = str_arg(args, 0, "tonumber")?;
        let text = text.to_str();
        let text = text.trim();
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        Ok(Ret::One(match i64::from_str_radix(digits, base as u32) {
            Ok(n) if !digits.is_empty() => Value::Int(if negative { -n } else { n }),
            _ => Value::Nil,
        }))
    });
    def("next", |_, args| {
        let t = table_arg(args, 0, "next")?;
        let step = t.borrow().next(arg(args, 1))?;
        Ok(match step {
            Some((k, v)) => Ret::Values(vec![k, v]),
            None => Ret::One(Value::Nil),
        })
    });
    let next = globals.get_str("next");
    globals.set_str(
        "pairs",
        Lua::native("pairs", move |_, args| {
            let t = table_arg(args, 0, "pairs")?;
            Ok(Ret::Values(vec![next.clone(), Value::Table(t), Value::Nil]))
        }),
    );
    let ipairs_iter = Lua::native("ipairs_iterator", |lua, args| {
        let i = int_arg(args, 1, "ipairs")? + 1;
        let v = match arg(args, 0) {
            Value::Table(t) if t.borrow().metatable().is_none() => t.get_int(i),
            other => lua.index(other, &Value::Int(i))?,
        };
        Ok(if v.is_nil() { Ret::One(Value::Nil) } else { Ret::Values(vec![Value::Int(i), v]) })
    });
    globals.set_str(
        "ipairs",
        Lua::native("ipairs", move |_, args| {
            if args.is_empty() {
                return Err(arg_error(0, "ipairs", "table", &Value::Nil));
            }
            Ok(Ret::Values(vec![ipairs_iter.clone(), args[0].clone(), Value::Int(0)]))
        }),
    );
    def("select", |_, args| {
        if let Value::Str(s) = arg(args, 0) {
            if s.as_bytes() == b"#" {
                return Ok(Ret::One(Value::Int(args.len() as i64 - 1)));
            }
        }
        let n = int_arg(args, 0, "select")?;
        let count = args.len() as i64 - 1;
        let start = if n < 0 { count + n } else { n - 1 };
        if start < 0 {
            return Err(Error::runtime("invalid argument #1 to 'select' (index out of range)"));
        }
        Ok(Ret::Values(args.iter().skip(1 + start as usize).cloned().collect()))
    });
    def("rawget", |_, args| Ok(Ret::One(table_arg(args, 0, "rawget")?.get(arg(args, 1)))));
    def("rawset", |_, args| {
        let t = table_arg(args, 0, "rawset")?;
        t.set(arg(args, 1).clone(), arg(args, 2).clone())?;
        Ok(Ret::One(Value::Table(t)))
    });
    def("rawequal", |_, args| Ok(Ret::One(Value::Bool(arg(args, 0).raw_eq(arg(args, 1))))));
    def("rawlen", |_, args| match arg(args, 0) {
        Value::Table(t) => Ok(Ret::One(Value::Int(t.len()))),
        Value::Str(s) => Ok(Ret::One(Value::Int(s.len() as i64))),
        other => Err(arg_error(0, "rawlen", "table or string", other)),
    });
    def("setmetatable", |lua, args| {
        let t = table_arg(args, 0, "setmetatable")?;
        let meta = match arg(args, 1) {
            Value::Nil => None,
            Value::Table(m) => Some(m.clone()),
            other => return Err(arg_error(1, "setmetatable", "nil or table", other)),
        };
        if let Some(existing) = t.borrow().metatable() {
            if !existing.get(&lua.names.metatable).is_nil() {
                return Err(Error::runtime("cannot change a protected metatable"));
            }
        }
        t.borrow_mut().set_metatable(meta);
        Ok(Ret::One(Value::Table(t)))
    });
    def("getmetatable", |lua, args| {
        Ok(Ret::One(match arg(args, 0) {
            Value::Table(t) => match t.borrow().metatable() {
                Some(meta) => {
                    let protected = meta.get(&lua.names.metatable);
                    if protected.is_nil() { Value::Table(meta) } else { protected }
                }
                None => Value::Nil,
            },
            Value::Str(_) => match lua.string_lib() {
                Some(lib) => {
                    let meta = lua.new_table();
                    meta.set_str("__index", lib);
                    Value::Table(meta)
                }
                None => Value::Nil,
            },
            _ => Value::Nil,
        }))
    });
    def("assert", |_, args| {
        if arg(args, 0).truthy() {
            return Ok(Ret::Values(args.to_vec()));
        }
        Err(match args.get(1) {
            Some(message) => Error::raised(message.clone(), 1),
            None => Error::runtime("assertion failed!"),
        })
    });
    def("error", |_, args| {
        let level = opt_int(args, 1, "error", 1)?.clamp(0, 8) as u8;
        Err(Error::raised(arg(args, 0).clone(), level))
    });
    def("pcall", |_, args| {
        if args.is_empty() {
            return Err(arg_error(0, "pcall", "value", &Value::Nil));
        }
        Ok(Ret::Protected { func: args[0].clone(), args: args[1..].to_vec(), handler: None })
    });
    def("xpcall", |_, args| {
        if args.len() < 2 {
            return Err(arg_error(1, "xpcall", "function", &Value::Nil));
        }
        Ok(Ret::Protected { func: args[0].clone(), args: args[2..].to_vec(), handler: Some(args[1].clone()) })
    });
    def("unpack", super::table::unpack);
    def("load", |lua, args| {
        let source = match arg(args, 0) {
            Value::Str(s) => s.clone(),
            other => return Err(arg_error(0, "load", "string", other)),
        };
        let chunk = match arg(args, 1) {
            Value::Str(name) => name.to_str().trim_start_matches(['=', '@']).to_string(),
            _ => "chunk".to_string(),
        };
        let env = match arg(args, 3) {
            Value::Table(t) => Some(t.clone()),
            _ => None,
        };
        Ok(match lua.load(&source.to_str(), &chunk, env) {
            Ok(f) => Ret::One(f),
            Err(e) => Ret::Values(vec![Value::Nil, e.value]),
        })
    });

    globals.set_str("loadstring", globals.get_str("load"));
    globals.set_str("_G", globals.clone());
    globals.set_str("_VERSION", "Lua 5.4");

    let debug = lua.new_table();
    debug.set_str(
        "traceback",
        Lua::native("debug.traceback", |lua, args| {
            let mut out = match arg(args, 0) {
                Value::Nil => String::new(),
                Value::Str(s) => s.to_str().into_owned(),
                other => return Ok(Ret::One(other.clone())),
            };
            if !lua.last_traceback.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("stack traceback:");
                for entry in &lua.last_traceback {
                    out.push_str(&format!("\n    {}:{}: in {}", entry.chunk, entry.line, entry.function));
                }
            }
            Ok(Ret::One(Value::from(out)))
        }),
    );
    lua.set_global("debug", debug);
}
