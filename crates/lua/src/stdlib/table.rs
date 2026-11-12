//! The `table` library.

use super::{arg, arg_error, int_arg, lib, opt_int, register, str_arg, table_arg};
use crate::error::{Error, Result};
use crate::value::{Ret, Value};
use crate::vm::Lua;

const MAX_UNPACK: i64 = 100_000;

pub(super) fn unpack(_: &mut Lua, args: &[Value]) -> Result<Ret> {
    let t = table_arg(args, 0, "unpack")?;
    let i = opt_int(args, 1, "unpack", 1)?;
    let j = if arg(args, 2).is_nil() { t.len() } else { int_arg(args, 2, "unpack")? };
    if j < i {
        return Ok(Ret::None);
    }
    if j - i >= MAX_UNPACK {
        return Err(Error::runtime("too many results to unpack"));
    }
    let table = t.borrow();
    Ok(Ret::Values((i..=j).map(|k| table.get_int(k)).collect()))
}

/// Stable merge sort with a fallible comparator. Unlike the standard
/// library's sort it never panics on an inconsistent ordering function.
fn merge_sort(
    items: &mut Vec<Value>,
    less: &mut impl FnMut(&Value, &Value) -> Result<bool>,
) -> Result<()> {
    let n = items.len();
    if n < 2 {
        return Ok(());
    }
    let mut buffer: Vec<Value> = Vec::with_capacity(n);
    let mut width = 1;
    while width < n {
        buffer.clear();
        let mut start = 0;
        while start < n {
            let mid = (start + width).min(n);
            let end = (start + 2 * width).min(n);
            let (mut a, mut b) = (start, mid);
            while a < mid && b < end {
                if less(&items[b], &items[a])? {
                    buffer.push(std::mem::take(&mut items[b]));
                    b += 1;
                } else {
                    buffer.push(std::mem::take(&mut items[a]));
                    a += 1;
                }
            }
            for item in &mut items[a..mid] {
                buffer.push(std::mem::take(item));
            }
            for item in &mut items[b..end] {
                buffer.push(std::mem::take(item));
            }
            start = end;
        }
        std::mem::swap(items, &mut buffer);
        width *= 2;
    }
    Ok(())
}

pub(super) fn open(lua: &mut Lua) {
    register(
        lua,
        "table",
        lib![
            "insert" => |_, args| {
                let t = table_arg(args, 0, "insert")?;
                let mut table = t.borrow_mut();
                let n = table.len();
                match args.len() {
                    2 => table.set_int(n + 1, args[1].clone()),
                    3 => {
                        let pos = int_arg(args, 1, "insert")?;
                        if pos < 1 || pos > n + 1 {
                            return Err(Error::runtime("invalid argument #2 to 'insert' (position out of bounds)"));
                        }
                        if n as usize == table.array().len() {
                            table.array_mut().insert(pos as usize - 1, args[2].clone());
                        } else {
                            let mut i = n;
                            while i >= pos {
                                let v = table.get_int(i);
                                table.set_int(i + 1, v);
                                i -= 1;
                            }
                            table.set_int(pos, args[2].clone());
                        }
                    }
                    _ => return Err(Error::runtime("wrong number of arguments to 'insert'")),
                }
                Ok(Ret::None)
            },
            "remove" => |_, args| {
                let t = table_arg(args, 0, "remove")?;
                let mut table = t.borrow_mut();
                let n = table.len();
                let pos = opt_int(args, 1, "remove", n)?;
                if args.len() > 1 && n + 1 != pos && (pos < 1 || pos > n + 1) && !(n == 0 && pos == 0) {
                    return Err(Error::runtime("invalid argument #2 to 'remove' (position out of bounds)"));
                }
                if n == 0 || pos > n {
                    return Ok(Ret::One(table.get_int(pos)));
                }
                if n as usize == table.array().len() {
                    return Ok(Ret::One(table.array_mut().remove(pos as usize - 1)));
                }
                let removed = table.get_int(pos);
                for i in pos..n {
                    let v = table.get_int(i + 1);
                    table.set_int(i, v);
                }
                table.set_int(n, Value::Nil);
                Ok(Ret::One(removed))
            },
            "concat" => |_, args| {
                let t = table_arg(args, 0, "concat")?;
                let sep = if arg(args, 1).is_nil() { None } else { Some(str_arg(args, 1, "concat")?) };
                let i = opt_int(args, 2, "concat", 1)?;
                let j = if arg(args, 3).is_nil() { t.len() } else { int_arg(args, 3, "concat")? };
                let table = t.borrow();
                let mut out = Vec::new();
                for k in i..=j {
                    match table.get_int(k) {
                        Value::Str(s) => out.extend_from_slice(s.as_bytes()),
                        v @ (Value::Int(_) | Value::Num(_)) => out.extend_from_slice(v.to_display().as_bytes()),
                        other => {
                            return Err(Error::runtime(format!(
                                "invalid value (at index {k}) in table for 'concat' (a {})",
                                other.type_name()
                            )));
                        }
                    }
                    if k < j {
                        if let Some(sep) = &sep {
                            out.extend_from_slice(sep.as_bytes());
                        }
                    }
                }
                Ok(Ret::One(Value::Str(crate::value::LuaStr::from_vec(out))))
            },
            "sort" => |lua, args| {
                let t = table_arg(args, 0, "sort")?;
                let comparator = match arg(args, 1) {
                    Value::Nil => None,
                    f @ (Value::Function(_) | Value::Native(_)) => Some(f.clone()),
                    other => return Err(arg_error(1, "sort", "function", other)),
                };
                let n = t.len();
                let mut items: Vec<Value> = {
                    let table = t.borrow();
                    (1..=n).map(|i| table.get_int(i)).collect()
                };
                merge_sort(&mut items, &mut |a, b| match &comparator {
                    Some(f) => Ok(lua.call1(f, &[a.clone(), b.clone()])?.truthy()),
                    None => lua.less(a, b, false),
                })?;
                let mut table = t.borrow_mut();
                for (i, v) in items.into_iter().enumerate() {
                    table.set_int(i as i64 + 1, v);
                }
                Ok(Ret::None)
            },
            "unpack" => unpack,
            "pack" => |lua, args| {
                let t = lua.new_array(args.iter().cloned());
                t.set_str("n", args.len());
                Ok(Ret::One(Value::Table(t)))
            },
            "move" => |_, args| {
                let source = table_arg(args, 0, "move")?;
                let from = int_arg(args, 1, "move")?;
                let end = int_arg(args, 2, "move")?;
                let to = int_arg(args, 3, "move")?;
                let dest = if arg(args, 4).is_nil() { source.clone() } else { table_arg(args, 4, "move")? };
                if end >= from {
                    let values: Vec<Value> = {
                        let table = source.borrow();
                        (from..=end).map(|i| table.get_int(i)).collect()
                    };
                    let mut table = dest.borrow_mut();
                    for (offset, v) in values.into_iter().enumerate() {
                        table.set_int(to + offset as i64, v);
                    }
                }
                Ok(Ret::One(Value::Table(dest)))
            },
            "find" => |_, args| {
                let t = table_arg(args, 0, "find")?;
                let needle = arg(args, 1);
                let init = opt_int(args, 2, "find", 1)?.max(1);
                let table = t.borrow();
                let n = table.len();
                Ok(Ret::One((init..=n).find(|&i| table.get_int(i).raw_eq(needle)).map_or(Value::Nil, Value::Int)))
            },
            "clear" => |_, args| {
                table_arg(args, 0, "clear")?.borrow_mut().clear();
                Ok(Ret::None)
            },
            "clone" => |lua, args| {
                let source = table_arg(args, 0, "clone")?;
                let copy = lua.new_table();
                {
                    let from = source.borrow();
                    let mut to = copy.borrow_mut();
                    for (k, v) in from.iter() {
                        to.set(k, v)?;
                    }
                    to.set_metatable(from.metatable());
                }
                Ok(Ret::One(Value::Table(copy)))
            },
            "create" => |lua, args| {
                let n = int_arg(args, 0, "create")?.clamp(0, 10_000_000) as usize;
                let fill = arg(args, 1).clone();
                let t = lua.new_table();
                if !fill.is_nil() {
                    t.borrow_mut().array_mut().resize(n, fill);
                } else {
                    t.borrow_mut().array_mut().reserve(n.min(1 << 16));
                }
                Ok(Ret::One(Value::Table(t)))
            },
        ],
    );
}
