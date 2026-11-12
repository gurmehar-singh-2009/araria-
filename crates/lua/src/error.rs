//! Error type shared by the compiler and the interpreter.

use std::fmt;

use crate::value::Value;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The source text failed to compile.
    Syntax,
    /// An error raised while running (including `error(...)` calls).
    Runtime,
    /// The script exhausted its execution budget. Not catchable by `pcall`,
    /// so a runaway script cannot shield itself from being stopped.
    Budget,
}

/// One line of a stack trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    pub chunk: String,
    pub line: u32,
    pub function: String,
}

/// A Lua error: the raised value plus where it happened.
#[derive(Clone, Debug)]
pub struct Error {
    pub kind: ErrorKind,
    /// The value passed to `error` (a string for interpreter-raised errors).
    pub value: Value,
    /// Chunk and line of the innermost Lua frame, when known.
    pub chunk: Option<String>,
    pub line: Option<u32>,
    /// Innermost frame first.
    pub traceback: Vec<TraceEntry>,
    /// Whether the position has already been folded into the message.
    pub(crate) located: bool,
    /// Which stack level the position should come from (`error(msg, level)`).
    pub(crate) level: u8,
}

impl Error {
    pub fn runtime(msg: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::Runtime,
            value: Value::from(msg.into()),
            chunk: None,
            line: None,
            traceback: Vec::new(),
            located: false,
            level: 1,
        }
    }

    pub fn syntax(chunk: &str, line: u32, msg: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::Syntax,
            value: Value::from(format!("{chunk}:{line}: {}", msg.into())),
            chunk: Some(chunk.to_string()),
            line: Some(line),
            traceback: Vec::new(),
            located: true,
            level: 1,
        }
    }

    pub fn budget(msg: impl Into<String>) -> Self {
        Error { kind: ErrorKind::Budget, ..Error::runtime(msg) }
    }

    /// An error carrying an arbitrary Lua value (`error({code = 1})`).
    pub fn from_value(value: Value) -> Self {
        Error {
            kind: ErrorKind::Runtime,
            value,
            chunk: None,
            line: None,
            traceback: Vec::new(),
            located: true,
            level: 1,
        }
    }

    /// An error raised by `error(value, level)`: string messages get the
    /// position of the function `level` frames up prepended.
    pub fn raised(value: Value, level: u8) -> Self {
        let located = level == 0 || !matches!(value, Value::Str(_));
        Error { located, level, ..Error::from_value(value) }
    }

    /// The error message without the stack trace.
    pub fn message(&self) -> String {
        self.value.to_display()
    }

    /// The message followed by a formatted stack trace.
    pub fn report(&self) -> String {
        let mut out = self.message();
        if !self.traceback.is_empty() {
            out.push_str("\nstack traceback:");
            for entry in &self.traceback {
                out.push_str(&format!("\n    {}:{}: in {}", entry.chunk, entry.line, entry.function));
            }
        }
        out
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for Error {}
