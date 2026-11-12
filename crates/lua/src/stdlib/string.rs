//! The `string` library.

use std::cell::Cell;

use super::pattern::{self, Matcher};
use super::{arg, arg_error, int_arg, lib, num_arg, opt_int, register, str_arg};
use crate::error::{Error, Result};
use crate::value::{LuaStr, Ret, Value, fmt_g};
use crate::vm::Lua;

const MAX_STRING: usize = 64 * 1024 * 1024;

/// Converts a Lua string position (1-based, negative from the end) to a
/// 0-based offset clamped to `0..=len`.
fn start_pos(i: i64, len: usize) -> usize {
    if i > 0 {
        ((i - 1) as usize).min(len)
    } else if i == 0 {
        0
    } else {
        len.saturating_sub(i.unsigned_abs() as usize)
    }
}

/// Converts an inclusive Lua end position to an exclusive 0-based offset.
fn end_pos(j: i64, len: usize) -> usize {
    if j >= 0 { (j as usize).min(len) } else { (len + 1).saturating_sub(j.unsigned_abs() as usize) }
}

fn ret_str(bytes: Vec<u8>) -> Result<Ret> {
    Ok(Ret::One(Value::Str(LuaStr::from_vec(bytes))))
}

pub(super) fn open(lua: &mut Lua) {
    let string = register(
        lua,
        "string",
        lib![
            "len" => |_, args| Ok(Ret::One(Value::Int(str_arg(args, 0, "len")?.len() as i64))),
            "sub" => |_, args| {
                let s = str_arg(args, 0, "sub")?;
                let bytes = s.as_bytes();
                let start = start_pos(opt_int(args, 1, "sub", 1)?, bytes.len());
                let end = end_pos(opt_int(args, 2, "sub", -1)?, bytes.len());
                ret_str(if start < end { bytes[start..end].to_vec() } else { Vec::new() })
            },
            "upper" => |_, args| ret_str(str_arg(args, 0, "upper")?.as_bytes().to_ascii_uppercase()),
            "lower" => |_, args| ret_str(str_arg(args, 0, "lower")?.as_bytes().to_ascii_lowercase()),
            "reverse" => |_, args| {
                let mut bytes = str_arg(args, 0, "reverse")?.as_bytes().to_vec();
                bytes.reverse();
                ret_str(bytes)
            },
            "rep" => |_, args| {
                let s = str_arg(args, 0, "rep")?;
                let n = int_arg(args, 1, "rep")?.max(0) as usize;
                let sep = if arg(args, 2).is_nil() { None } else { Some(str_arg(args, 2, "rep")?) };
                let sep_len = sep.as_ref().map_or(0, LuaStr::len);
                if n.saturating_mul(s.len() + sep_len) > MAX_STRING {
                    return Err(Error::runtime("resulting string too large"));
                }
                let mut out = Vec::with_capacity(n * (s.len() + sep_len));
                for i in 0..n {
                    if i > 0 {
                        if let Some(sep) = &sep {
                            out.extend_from_slice(sep.as_bytes());
                        }
                    }
                    out.extend_from_slice(s.as_bytes());
                }
                ret_str(out)
            },
            "byte" => |_, args| {
                let s = str_arg(args, 0, "byte")?;
                let i = opt_int(args, 1, "byte", 1)?;
                let start = start_pos(i, s.len());
                let end = end_pos(opt_int(args, 2, "byte", i)?, s.len());
                let bytes = if start < end { &s.as_bytes()[start..end] } else { &[][..] };
                Ok(Ret::Values(bytes.iter().map(|&b| Value::Int(i64::from(b))).collect()))
            },
            "char" => |_, args| {
                let mut out = Vec::with_capacity(args.len());
                for i in 0..args.len() {
                    let c = int_arg(args, i, "char")?;
                    if !(0..=255).contains(&c) {
                        return Err(Error::runtime(format!("invalid argument #{} to 'char' (value out of range)", i + 1)));
                    }
                    out.push(c as u8);
                }
                ret_str(out)
            },
            "format" => format,
            "find" => |_, args| find(args, true),
            "match" => |_, args| find(args, false),
            "gmatch" => gmatch,
            "gsub" => gsub,
            "split" => |lua, args| {
                let s = str_arg(args, 0, "split")?;
                let sep = if arg(args, 1).is_nil() { LuaStr::from(",") } else { str_arg(args, 1, "split")? };
                let (bytes, sep) = (s.as_bytes(), sep.as_bytes());
                let mut parts = Vec::new();
                if sep.is_empty() {
                    parts.extend(bytes.iter().map(|b| Value::Str(LuaStr::new(&[*b]))));
                } else {
                    let mut start = 0;
                    while let Some(i) = pattern::find_plain(bytes, sep, start) {
                        parts.push(Value::Str(LuaStr::new(&bytes[start..i])));
                        start = i + sep.len();
                    }
                    parts.push(Value::Str(LuaStr::new(&bytes[start..])));
                }
                Ok(Ret::One(Value::Table(lua.new_array(parts))))
            },
        ],
    );
    lua.set_string_lib(string);

    register(
        lua,
        "utf8",
        lib![
            "char" => |_, args| {
                let mut out = String::new();
                for i in 0..args.len() {
                    let code = int_arg(args, i, "char")?;
                    match u32::try_from(code).ok().and_then(char::from_u32) {
                        Some(c) => out.push(c),
                        None => return Err(Error::runtime(format!("invalid argument #{} to 'char' (value out of range)", i + 1))),
                    }
                }
                Ok(Ret::One(Value::from(out)))
            },
            "len" => |_, args| {
                let s = str_arg(args, 0, "len")?;
                Ok(match std::str::from_utf8(s.as_bytes()) {
                    Ok(text) => Ret::One(Value::Int(text.chars().count() as i64)),
                    Err(e) => Ret::Values(vec![Value::Nil, Value::Int(e.valid_up_to() as i64 + 1)]),
                })
            },
            "codepoint" => |_, args| {
                let s = str_arg(args, 0, "codepoint")?;
                let i = opt_int(args, 1, "codepoint", 1)?;
                let start = start_pos(i, s.len());
                let end = end_pos(opt_int(args, 2, "codepoint", i)?, s.len());
                let text = String::from_utf8_lossy(&s.as_bytes()[start.min(s.len())..]).into_owned();
                let mut out = Vec::new();
                let mut offset = start;
                for c in text.chars() {
                    if offset >= end {
                        break;
                    }
                    out.push(Value::Int(i64::from(u32::from(c))));
                    offset += c.len_utf8();
                }
                Ok(Ret::Values(out))
            },
        ],
    )
    .set_str("charpattern", Value::Str(LuaStr::new(b"[\x00-\x7F\xC2-\xFD][\x80-\xBF]*")));
}

fn find(args: &[Value], is_find: bool) -> Result<Ret> {
    let name = if is_find { "find" } else { "match" };
    let s = str_arg(args, 0, name)?;
    let pat = str_arg(args, 1, name)?;
    let (src, pat) = (s.as_bytes(), pat.as_bytes());
    let init_arg = opt_int(args, 2, name, 1)?;
    let init = if init_arg > 0 {
        (init_arg - 1) as usize
    } else if init_arg == 0 {
        0
    } else {
        src.len().saturating_sub(init_arg.unsigned_abs() as usize)
    };
    if init > src.len() {
        return Ok(Ret::One(Value::Nil));
    }
    if is_find && (arg(args, 3).truthy() || pattern::is_plain(pat)) {
        return Ok(match pattern::find_plain(src, pat, init) {
            Some(i) => Ret::Values(vec![Value::Int(i as i64 + 1), Value::Int((i + pat.len()) as i64)]),
            None => Ret::One(Value::Nil),
        });
    }
    match pattern::find(src, pat, init)? {
        None => Ok(Ret::One(Value::Nil)),
        Some((start, end, captures)) => {
            if is_find {
                let mut out = vec![Value::Int(start as i64 + 1), Value::Int(end as i64)];
                out.extend(captures);
                Ok(Ret::Values(out))
            } else if captures.is_empty() {
                Ok(Ret::One(Value::Str(LuaStr::new(&src[start..end]))))
            } else {
                Ok(Ret::Values(captures))
            }
        }
    }
}

fn gmatch(_: &mut Lua, args: &[Value]) -> Result<Ret> {
    let s = str_arg(args, 0, "gmatch")?;
    let pat = str_arg(args, 1, "gmatch")?;
    let position = Cell::new(0usize);
    let last_match = Cell::new(usize::MAX);
    Ok(Ret::One(Lua::native("gmatch_iterator", move |_, _| {
        let (src, pattern) = (s.as_bytes(), pat.as_bytes());
        let mut matcher = Matcher::new(src, pattern);
        let mut at = position.get();
        while at <= src.len() {
            matcher.reset();
            if let Some(end) = matcher.do_match(at, 0)? {
                if end != last_match.get() {
                    position.set(end);
                    last_match.set(end);
                    return Ok(Ret::Values(matcher.captures(at, end, true)?));
                }
            }
            at += 1;
        }
        position.set(src.len() + 1);
        Ok(Ret::One(Value::Nil))
    })))
}

fn gsub(lua: &mut Lua, args: &[Value]) -> Result<Ret> {
    let s = str_arg(args, 0, "gsub")?;
    let pat = str_arg(args, 1, "gsub")?;
    let repl = arg(args, 2).clone();
    if !matches!(repl, Value::Str(_) | Value::Int(_) | Value::Num(_) | Value::Table(_) | Value::Function(_) | Value::Native(_)) {
        return Err(arg_error(2, "gsub", "string/function/table", &repl));
    }
    let repl_text = match &repl {
        Value::Int(_) | Value::Num(_) => Some(LuaStr::from(repl.to_display())),
        Value::Str(text) => Some(text.clone()),
        _ => None,
    };
    let max = if arg(args, 3).is_nil() { usize::MAX } else { int_arg(args, 3, "gsub")?.max(0) as usize };
    let src = s.as_bytes();
    let (anchored, pattern) = match pat.as_bytes().first() {
        Some(b'^') => (true, &pat.as_bytes()[1..]),
        _ => (false, pat.as_bytes()),
    };
    let mut matcher = Matcher::new(src, pattern);
    let mut out = Vec::with_capacity(src.len());
    let mut at = 0usize;
    let mut last_match = usize::MAX;
    let mut count = 0usize;
    while count < max {
        matcher.reset();
        match matcher.do_match(at, 0)? {
            Some(end) if end != last_match => {
                count += 1;
                let whole = &src[at..end];
                let replacement = match (&repl_text, &repl) {
                    (Some(text), _) => {
                        let text = text.as_bytes();
                        let mut i = 0;
                        while i < text.len() {
                            if text[i] == b'%' {
                                i += 1;
                                match text.get(i) {
                                    Some(b'%') => out.push(b'%'),
                                    Some(d) if d.is_ascii_digit() => {
                                        let value = if *d == b'0' {
                                            Value::Str(LuaStr::new(whole))
                                        } else {
                                            matcher.capture((*d - b'1') as usize, at, end)?
                                        };
                                        out.extend_from_slice(value.to_display().as_bytes());
                                    }
                                    _ => return Err(Error::runtime("invalid use of '%' in replacement string")),
                                }
                            } else {
                                out.push(text[i]);
                            }
                            i += 1;
                        }
                        None
                    }
                    (None, Value::Table(t)) => Some(lua.index(&Value::Table(t.clone()), &matcher.capture(0, at, end)?)?),
                    (None, f) => {
                        let captures = matcher.captures(at, end, true)?;
                        Some(lua.call1(f, &captures)?)
                    }
                };
                match replacement {
                    None => {}
                    Some(Value::Nil | Value::Bool(false)) => out.extend_from_slice(whole),
                    Some(Value::Str(text)) => out.extend_from_slice(text.as_bytes()),
                    Some(v @ (Value::Int(_) | Value::Num(_))) => out.extend_from_slice(v.to_display().as_bytes()),
                    Some(other) => {
                        return Err(Error::runtime(format!(
                            "invalid replacement value (a {})",
                            other.type_name()
                        )));
                    }
                }
                at = end;
                last_match = end;
            }
            _ if at < src.len() => {
                out.push(src[at]);
                at += 1;
            }
            _ => break,
        }
        if anchored {
            break;
        }
    }
    if at < src.len() {
        out.extend_from_slice(&src[at..]);
    }
    Ok(Ret::Values(vec![Value::Str(LuaStr::from_vec(out)), Value::Int(count as i64)]))
}

#[derive(Default)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

/// Applies width, justification and zero padding to a formatted number.
fn pad(spec: &Spec, body: String, numeric: bool) -> String {
    if body.len() >= spec.width {
        return body;
    }
    let fill = spec.width - body.len();
    if spec.left {
        format!("{body}{}", " ".repeat(fill))
    } else if spec.zero && numeric {
        let sign_len = usize::from(body.starts_with(['-', '+', ' ']));
        let (sign, digits) = body.split_at(sign_len);
        format!("{sign}{}{digits}", "0".repeat(fill))
    } else {
        format!("{}{body}", " ".repeat(fill))
    }
}

fn signed(spec: &Spec, negative: bool, digits: String) -> String {
    if negative {
        format!("-{digits}")
    } else if spec.plus {
        format!("+{digits}")
    } else if spec.space {
        format!(" {digits}")
    } else {
        digits
    }
}

/// `%e` with C-style exponent (`1.5e+02`).
fn exponent(x: f64, precision: usize, upper: bool) -> String {
    let text = format!("{x:.precision$e}");
    let (mantissa, exp) = text.split_once('e').unwrap_or((&text, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let e = if upper { 'E' } else { 'e' };
    format!("{mantissa}{e}{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
}

fn format(lua: &mut Lua, args: &[Value]) -> Result<Ret> {
    let fmt = str_arg(args, 0, "format")?;
    let fmt = fmt.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(fmt.len() + 16);
    let mut next_arg = 1;
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' {
            out.push(c);
            continue;
        }
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        let mut spec = Spec::default();
        while let Some(flag) = fmt.get(i) {
            match flag {
                b'-' => spec.left = true,
                b'+' => spec.plus = true,
                b' ' => spec.space = true,
                b'#' => spec.alt = true,
                b'0' => spec.zero = true,
                _ => break,
            }
            i += 1;
        }
        while let Some(d) = fmt.get(i).filter(|d| d.is_ascii_digit()) {
            spec.width = (spec.width * 10 + usize::from(d - b'0')).min(512);
            i += 1;
        }
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            let mut precision = 0usize;
            while let Some(d) = fmt.get(i).filter(|d| d.is_ascii_digit()) {
                precision = (precision * 10 + usize::from(d - b'0')).min(512);
                i += 1;
            }
            spec.precision = Some(precision);
        }
        let Some(&conv) = fmt.get(i) else {
            return Err(Error::runtime("invalid conversion '%' to 'format'"));
        };
        i += 1;
        let index = next_arg;
        next_arg += 1;
        if index >= args.len() {
            return Err(Error::runtime(format!("invalid argument #{} to 'format' (no value)", index + 1)));
        }
        let piece = match conv {
            b'd' | b'i' => {
                let n = int_arg(args, index, "format")?;
                let mut digits = n.unsigned_abs().to_string();
                if let Some(precision) = spec.precision {
                    if digits.len() < precision {
                        digits = format!("{}{digits}", "0".repeat(precision - digits.len()));
                    }
                }
                pad(&spec, signed(&spec, n < 0, digits), spec.precision.is_none())
            }
            b'u' => pad(&spec, (int_arg(args, index, "format")? as u64).to_string(), true),
            b'c' => {
                out.push(int_arg(args, index, "format")? as u8);
                continue;
            }
            b'x' | b'X' | b'o' => {
                let n = int_arg(args, index, "format")? as u64;
                let body = match conv {
                    b'x' => format!("{n:x}"),
                    b'X' => format!("{n:X}"),
                    _ => format!("{n:o}"),
                };
                let prefix = match (spec.alt, conv) {
                    (true, b'x') => "0x",
                    (true, b'X') => "0X",
                    _ => "",
                };
                pad(&spec, format!("{prefix}{body}"), true)
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' | b'a' | b'A' => {
                let x = num_arg(args, index, "format")?;
                let precision = spec.precision.unwrap_or(6);
                let magnitude = x.abs();
                let body = if !x.is_finite() {
                    if x.is_nan() { "nan".to_string() } else { "inf".to_string() }
                } else {
                    match conv {
                        b'e' | b'E' => exponent(magnitude, precision, conv == b'E'),
                        b'f' | b'F' => format!("{magnitude:.precision$}"),
                        b'a' | b'A' => format!("{magnitude:e}"),
                        _ => {
                            let text = fmt_g(magnitude, precision.max(1));
                            if conv == b'G' { text.to_uppercase() } else { text }
                        }
                    }
                };
                pad(&spec, signed(&spec, x.is_sign_negative() && !x.is_nan(), body), x.is_finite())
            }
            b's' => {
                let text = lua.tostring(&args[index])?;
                let text = match spec.precision {
                    Some(precision) => text.chars().take(precision).collect(),
                    None => text,
                };
                // Write raw bytes for real strings so non-UTF-8 data survives.
                if let (Value::Str(raw), None, 0) = (&args[index], spec.precision, spec.width) {
                    out.extend_from_slice(raw.as_bytes());
                    continue;
                }
                pad(&spec, text, false)
            }
            b'q' => match &args[index] {
                Value::Str(text) => {
                    let mut quoted = String::from("\"");
                    for &b in text.as_bytes() {
                        match b {
                            b'"' => quoted.push_str("\\\""),
                            b'\\' => quoted.push_str("\\\\"),
                            b'\n' => quoted.push_str("\\n"),
                            b'\r' => quoted.push_str("\\r"),
                            0 => quoted.push_str("\\0"),
                            b if b.is_ascii_control() => quoted.push_str(&format!("\\{b}")),
                            b => quoted.push(b as char),
                        }
                    }
                    quoted.push('"');
                    quoted
                }
                other => other.to_display(),
            },
            other => {
                return Err(Error::runtime(format!(
                    "invalid conversion '%{}' to 'format'",
                    other as char
                )));
            }
        };
        out.extend_from_slice(piece.as_bytes());
    }
    ret_str(out)
}
