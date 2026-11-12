//! The `coroutine` library.

use super::{arg, arg_error, func_arg, lib, register};
use crate::error::{Error, ErrorKind, Result};
use crate::value::{Ret, ThreadRef, Value};
use crate::vm::{Lua, Resumed};

fn thread_arg(args: &[Value], i: usize, func: &str) -> Result<ThreadRef> {
    match arg(args, i) {
        Value::Thread(t) => Ok(t.clone()),
        other => Err(arg_error(i, func, "coroutine", other)),
    }
}

pub(super) fn open(lua: &mut Lua) {
    register(
        lua,
        "coroutine",
        lib![
            "create" => |lua, args| {
                let f = func_arg(args, 0, "create")?;
                Ok(Ret::One(Value::Thread(lua.create_thread(f))))
            },
            "resume" => |lua, args| {
                let co = thread_arg(args, 0, "resume")?;
                match lua.resume(&co, args[1..].to_vec()) {
                    Ok(Resumed::Yield(mut values) | Resumed::Return(mut values)) => {
                        values.insert(0, Value::Bool(true));
                        Ok(Ret::Values(values))
                    }
                    // Budget exhaustion must stop the whole script, not just this coroutine.
                    Err(e) if e.kind == ErrorKind::Budget => Err(e),
                    Err(e) => Ok(Ret::Values(vec![Value::Bool(false), e.value])),
                }
            },
            "yield" => |lua, args| {
                if lua.current_thread().is_none() {
                    return Err(Error::runtime("attempt to yield from outside a coroutine"));
                }
                Ok(Ret::Yield(args.to_vec()))
            },
            "status" => |_, args| Ok(Ret::One(Value::str(thread_arg(args, 0, "status")?.status().name()))),
            "wrap" => |lua, args| {
                let f = func_arg(args, 0, "wrap")?;
                let co = lua.create_thread(f);
                Ok(Ret::One(Lua::native("wrapped coroutine", move |lua, args| {
                    match lua.resume(&co, args.to_vec())? {
                        Resumed::Yield(values) | Resumed::Return(values) => Ok(Ret::Values(values)),
                    }
                })))
            },
            "isyieldable" => |lua, _| Ok(Ret::One(Value::Bool(lua.current_thread().is_some()))),
            "running" => |lua, _| Ok(match lua.current_thread() {
                Some(co) => Ret::Values(vec![Value::Thread(co), Value::Bool(false)]),
                None => Ret::Values(vec![Value::Nil, Value::Bool(true)]),
            }),
            "close" => |lua, args| {
                let co = thread_arg(args, 0, "close")?;
                if lua.close_thread(&co) {
                    Ok(Ret::One(Value::Bool(true)))
                } else {
                    Err(Error::runtime(format!("cannot close a {} coroutine", co.status().name())))
                }
            },
        ],
    );
}
