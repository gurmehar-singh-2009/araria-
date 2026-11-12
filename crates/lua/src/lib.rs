//! # Araria Lua
//!
//! A Lua 5.4-style virtual machine written in safe Rust with no native
//! dependencies, so it runs identically on desktop and on
//! `wasm32-unknown-unknown` (where C-based Lua bindings cannot be built).
//!
//! Highlights:
//!
//! * Full language: closures, metatables, varargs, integers and floats,
//!   `goto`, coroutines (yieldable across `pcall`), plus Luau's `continue`
//!   and compound assignment (`+=`, `..=`, ...).
//! * Sandboxed standard library (`string` with patterns, `table`, `math`,
//!   `os` time queries, `coroutine`, `utf8`); no file or process access.
//! * Execution budgets that stop runaway scripts and cannot be caught by
//!   `pcall`.
//! * Line-accurate error messages and stack traces.
//! * A cycle-collecting garbage collector on top of reference counting.
//! * A native 3-component vector value for allocation-free game math.
//!
//! ```
//! use araria_lua::{Lua, Value};
//!
//! let mut lua = Lua::new();
//! let result = lua.exec("local t = {1, 2, 3} return #t * 2", "example").unwrap();
//! assert_eq!(result[0], Value::Int(6));
//! ```
//!
//! Known deviations from reference Lua 5.4: no `<close>` variables, no
//! weak tables, no `__gc`, `__pairs` or integer `for`-loop overflow
//! pre-computation, and metamethods (other than `__call`) cannot yield.

pub mod ast;
pub mod compiler;
mod error;
mod gc;
pub mod lexer;
pub mod parser;
pub mod stdlib;
mod value;
mod vm;

pub use error::{Error, ErrorKind, Result, TraceEntry};
pub use gc::GcStats;
pub use stdlib::{arg_error, func_arg, int_arg, num_arg, opt_int, opt_num, str_arg, table_arg};
pub use value::{
    ArithOp, Closure, LuaStr, Native, NativeFn, Ret, Table, TableRef, ThreadRef, ThreadStatus, UserData, Value,
    float_to_int_exact, fmt_number,
};
pub use vm::{Lua, Resumed, ThreadState};
