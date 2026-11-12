//! The interpreter: call frames, the instruction loop, coroutines and the
//! host-facing [`Lua`] state.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::compiler::{self, MULTI, Op, Proto};
use crate::error::{Error, ErrorKind, Result, TraceEntry};
use crate::gc::{Gc, GcStats};
use crate::value::*;

const MAX_FRAMES: usize = 200;
const MAX_NATIVE_DEPTH: u32 = 40;
const MAX_STACK: usize = 400_000;

/// `Some(handler)` marks a frame as a `pcall`/`xpcall` boundary.
pub(crate) type Protect = Option<Option<Value>>;

pub(crate) struct Frame {
    pub(crate) closure: Rc<Closure>,
    pc: usize,
    /// Stack index of local slot 0.
    base: usize,
    /// Stack index results are written to (where the function value was).
    ret_base: usize,
    nres: u8,
    pub(crate) varargs: Vec<Value>,
    pub(crate) protect: Protect,
}

/// A native call that yielded and is waiting for resume values.
struct Pending {
    ret_base: usize,
    nres: u8,
    prefix_true: bool,
}

/// Execution state of one coroutine (or of a host-initiated call).
#[derive(Default)]
pub struct ThreadState {
    pub(crate) stack: Vec<Value>,
    pub(crate) frames: Vec<Frame>,
    /// Number of values produced by the last multi-result instruction.
    multi: usize,
    pending: Option<Pending>,
}

enum CallOutcome {
    /// A Lua frame was pushed; the instruction loop should continue in it.
    Frame,
    /// The call completed; results are on the stack.
    Done,
    Yield(Vec<Value>),
}

enum Exec {
    Done,
    Yield(Vec<Value>),
}

/// Result of resuming a coroutine.
#[derive(Debug)]
pub enum Resumed {
    /// The coroutine yielded these values and can be resumed again.
    Yield(Vec<Value>),
    /// The coroutine's function returned these values; it is now dead.
    Return(Vec<Value>),
}

/// Interned metamethod names.
pub(crate) struct Names {
    pub(crate) index: Value,
    pub(crate) newindex: Value,
    pub(crate) call: Value,
    pub(crate) eq: Value,
    pub(crate) lt: Value,
    pub(crate) le: Value,
    pub(crate) len: Value,
    pub(crate) concat: Value,
    pub(crate) unm: Value,
    pub(crate) tostring: Value,
    pub(crate) metatable: Value,
    arith: [Value; 7],
}

impl Names {
    fn new() -> Self {
        Names {
            index: Value::str("__index"),
            newindex: Value::str("__newindex"),
            call: Value::str("__call"),
            eq: Value::str("__eq"),
            lt: Value::str("__lt"),
            le: Value::str("__le"),
            len: Value::str("__len"),
            concat: Value::str("__concat"),
            unm: Value::str("__unm"),
            tostring: Value::str("__tostring"),
            metatable: Value::str("__metatable"),
            arith: [
                ArithOp::Add,
                ArithOp::Sub,
                ArithOp::Mul,
                ArithOp::Div,
                ArithOp::Mod,
                ArithOp::Pow,
                ArithOp::IDiv,
            ]
            .map(|op| Value::str(op.event())),
        }
    }

    fn arith(&self, op: ArithOp) -> &Value {
        &self.arith[op as usize]
    }
}

/// A Lua state: globals, the heap and the interpreter.
///
/// The state is single-threaded and sandboxed: it has no access to the file
/// system, network or process environment unless the host adds functions that
/// provide it.
pub struct Lua {
    globals: TableRef,
    pub(crate) string_lib: Option<TableRef>,
    /// Handler for indexing vectors beyond `x`/`y`/`z`: a table or a function
    /// called as `handler(vector, key)`.
    vector_index: Option<Value>,
    pub(crate) names: Names,
    budget: i64,
    native_depth: u32,
    /// Coroutines currently being resumed, innermost last.
    running: Vec<ThreadRef>,
    arg_buf: Vec<Value>,
    spare_states: Vec<Box<ThreadState>>,
    gc: Gc,
    app_data: HashMap<TypeId, Rc<dyn Any>>,
    /// Stack trace of the most recent error caught by `pcall`/`xpcall`.
    pub(crate) last_traceback: Vec<TraceEntry>,
    /// Receives `print` output. Defaults to standard output.
    print_hook: Option<Rc<dyn Fn(&str)>>,
}

impl Default for Lua {
    fn default() -> Self {
        Lua::new()
    }
}

impl Lua {
    /// Creates a state with the standard library loaded.
    pub fn new() -> Lua {
        let mut lua = Lua::bare();
        crate::stdlib::open(&mut lua);
        lua
    }

    /// Creates a state with no globals at all.
    pub fn bare() -> Lua {
        let mut gc = Gc::default();
        let globals = TableRef::new_untracked(Table::default());
        gc.track_table(&globals);
        Lua {
            globals,
            string_lib: None,
            vector_index: None,
            names: Names::new(),
            budget: i64::MAX,
            native_depth: 0,
            running: Vec::new(),
            arg_buf: Vec::new(),
            spare_states: Vec::new(),
            gc,
            app_data: HashMap::new(),
            last_traceback: Vec::new(),
            print_hook: None,
        }
    }

    // ------------------------------------------------------------ objects

    pub fn globals(&self) -> TableRef {
        self.globals.clone()
    }

    pub fn set_global(&mut self, name: &str, value: impl Into<Value>) {
        self.globals.set_str(name, value.into());
    }

    pub fn get_global(&self, name: &str) -> Value {
        self.globals.get_str(name)
    }

    pub fn new_table(&mut self) -> TableRef {
        let table = TableRef::new_untracked(Table::default());
        self.gc.track_table(&table);
        if self.gc.should_collect() {
            self.gc.collect();
        }
        table
    }

    /// Creates a table from a sequence of values.
    pub fn new_array(&mut self, values: impl IntoIterator<Item = Value>) -> TableRef {
        let table = self.new_table();
        table.borrow_mut().arr.extend(values);
        table
    }

    /// Wraps a Rust closure as a Lua function.
    pub fn native(name: impl Into<std::borrow::Cow<'static, str>>, f: impl Fn(&mut Lua, &[Value]) -> Result<Ret> + 'static) -> Value {
        Value::Native(Rc::new(Native { name: name.into(), f: Box::new(f) }))
    }

    /// Wraps a host object as a userdata value.
    pub fn userdata<T: UserData>(value: T) -> Value {
        Value::UserData(Rc::new(value))
    }

    pub fn set_string_lib(&mut self, lib: TableRef) {
        self.string_lib = Some(lib);
    }

    pub fn string_lib(&self) -> Option<TableRef> {
        self.string_lib.clone()
    }

    /// Sets the fallback used when indexing vector values (see [`Value::Vector`]).
    pub fn set_vector_index(&mut self, handler: Value) {
        self.vector_index = Some(handler);
    }

    /// Stores a host value retrievable with [`Lua::app_data`].
    pub fn set_app_data<T: 'static>(&mut self, data: Rc<T>) {
        self.app_data.insert(TypeId::of::<T>(), data);
    }

    pub fn app_data<T: 'static>(&self) -> Option<Rc<T>> {
        self.app_data.get(&TypeId::of::<T>()).cloned().and_then(|rc| rc.downcast::<T>().ok())
    }

    pub fn set_print_hook(&mut self, hook: impl Fn(&str) + 'static) {
        self.print_hook = Some(Rc::new(hook));
    }

    pub(crate) fn print(&self, text: &str) {
        match &self.print_hook {
            Some(hook) => hook(text),
            None => println!("{text}"),
        }
    }

    // ------------------------------------------------------------- limits

    /// Sets how many loop iterations and calls may run before a
    /// [`ErrorKind::Budget`] error stops the script.
    pub fn set_budget(&mut self, budget: i64) {
        self.budget = budget;
    }

    pub fn budget(&self) -> i64 {
        self.budget
    }

    /// Runs a cycle collection now.
    pub fn collect_garbage(&mut self) -> GcStats {
        self.gc.collect()
    }

    pub fn gc_stats(&self) -> GcStats {
        self.gc.stats()
    }

    // ------------------------------------------------------------ loading

    /// Compiles a chunk. `env` is the table its global accesses use; `None`
    /// means the shared globals table.
    pub fn load(&mut self, source: &str, chunk: &str, env: Option<TableRef>) -> Result<Value> {
        let proto = compiler::compile(source, chunk)?;
        Ok(self.instantiate(proto, env))
    }

    /// Turns a compiled prototype into a callable main function.
    pub fn instantiate(&mut self, proto: Rc<Proto>, env: Option<TableRef>) -> Value {
        let closure = Rc::new(Closure {
            gc: GcHeader::default(),
            proto,
            upvals: Vec::new(),
            env: env.unwrap_or_else(|| self.globals.clone()),
        });
        self.gc.track_closure(&closure);
        Value::Function(closure)
    }

    /// Compiles and runs a chunk against the shared globals.
    pub fn exec(&mut self, source: &str, chunk: &str) -> Result<Vec<Value>> {
        let f = self.load(source, chunk, None)?;
        self.call(&f, &[])
    }

    // ------------------------------------------------------------ calling

    /// Calls a Lua value from the host. The callee cannot yield.
    pub fn call(&mut self, func: &Value, args: &[Value]) -> Result<Vec<Value>> {
        if self.native_depth >= MAX_NATIVE_DEPTH {
            return Err(Error::runtime("stack overflow (too many nested native calls)"));
        }
        let mut th = self.spare_states.pop().unwrap_or_default();
        th.stack.push(func.clone());
        th.stack.extend_from_slice(args);
        self.native_depth += 1;
        let result = self.call_in(&mut th, 0, args.len());
        self.native_depth -= 1;
        let out = match result {
            Ok(Exec::Done) => Ok(th.stack.drain(..).collect()),
            Ok(Exec::Yield(_)) => Err(Error::runtime("attempt to yield across a native call boundary")),
            Err(e) => Err(e),
        };
        th.stack.clear();
        th.frames.clear();
        th.pending = None;
        self.spare_states.push(th);
        out
    }

    /// Calls a value and returns its first result.
    pub fn call1(&mut self, func: &Value, args: &[Value]) -> Result<Value> {
        Ok(self.call(func, args)?.into_iter().next().unwrap_or(Value::Nil))
    }

    fn call_in(&mut self, th: &mut ThreadState, func_idx: usize, nargs: usize) -> Result<Exec> {
        let stop = th.frames.len();
        match self.call_at(th, func_idx, nargs, MULTI, None)? {
            CallOutcome::Frame => self.execute(th, stop),
            CallOutcome::Done => Ok(Exec::Done),
            CallOutcome::Yield(values) => Ok(Exec::Yield(values)),
        }
    }

    // --------------------------------------------------------- coroutines

    /// Creates a suspended coroutine that will run `func` when first resumed.
    pub fn create_thread(&mut self, func: Value) -> ThreadRef {
        let mut state = Box::<ThreadState>::default();
        state.stack.push(func);
        let thread = ThreadRef(Rc::new(Thread {
            gc: GcHeader::default(),
            state: RefCell::new(Some(state)),
            status: Cell::new(ThreadStatus::Suspended),
        }));
        self.gc.track_thread(&thread);
        thread
    }

    /// The innermost coroutine being resumed, if any.
    pub fn current_thread(&self) -> Option<ThreadRef> {
        self.running.last().cloned()
    }

    /// Resumes a suspended coroutine.
    pub fn resume(&mut self, co: &ThreadRef, args: Vec<Value>) -> Result<Resumed> {
        if co.status() != ThreadStatus::Suspended {
            return Err(Error::runtime(format!("cannot resume {} coroutine", co.status().name())));
        }
        if self.native_depth >= MAX_NATIVE_DEPTH {
            return Err(Error::runtime("stack overflow (too many nested coroutines)"));
        }
        let Some(mut th) = co.0.state.borrow_mut().take() else {
            return Err(Error::runtime("cannot resume dead coroutine"));
        };
        if let Some(previous) = self.running.last() {
            previous.0.status.set(ThreadStatus::Normal);
        }
        co.0.status.set(ThreadStatus::Running);
        self.running.push(co.clone());
        self.native_depth += 1;

        let result = if let Some(pending) = th.pending.take() {
            th.stack.truncate(pending.ret_base);
            if pending.prefix_true {
                th.stack.push(Value::Bool(true));
            }
            th.stack.extend(args);
            finish_results(&mut th, pending.ret_base, pending.nres);
            if th.frames.is_empty() { Ok(Exec::Done) } else { self.execute(&mut th, 0) }
        } else {
            let nargs = args.len();
            th.stack.extend(args);
            self.call_in(&mut th, 0, nargs)
        };

        self.native_depth -= 1;
        self.running.pop();
        if let Some(previous) = self.running.last() {
            previous.0.status.set(ThreadStatus::Running);
        }
        match result {
            Ok(Exec::Yield(values)) => {
                co.0.status.set(ThreadStatus::Suspended);
                *co.0.state.borrow_mut() = Some(th);
                Ok(Resumed::Yield(values))
            }
            Ok(Exec::Done) => {
                co.0.status.set(ThreadStatus::Dead);
                Ok(Resumed::Return(th.stack.drain(..).collect()))
            }
            Err(e) => {
                co.0.status.set(ThreadStatus::Dead);
                Err(e)
            }
        }
    }

    /// Kills a suspended coroutine, releasing everything it holds.
    pub fn close_thread(&mut self, co: &ThreadRef) -> bool {
        match co.status() {
            ThreadStatus::Suspended | ThreadStatus::Dead => {
                co.0.state.borrow_mut().take();
                co.0.status.set(ThreadStatus::Dead);
                true
            }
            _ => false,
        }
    }

    // -------------------------------------------------------- metamethods

    pub fn metatable_of(&self, v: &Value) -> Option<TableRef> {
        match v {
            Value::Table(t) => t.borrow().meta.clone(),
            _ => None,
        }
    }

    fn metamethod(&self, v: &Value, event: &Value) -> Value {
        match v {
            Value::Table(t) => match &t.borrow().meta {
                Some(meta) => meta.get(event),
                None => Value::Nil,
            },
            _ => Value::Nil,
        }
    }

    /// `obj[key]`, honouring `__index`.
    pub fn index(&mut self, obj: &Value, key: &Value) -> Result<Value> {
        let mut current = obj.clone();
        for _ in 0..64 {
            match &current {
                Value::Table(t) => {
                    let handler = {
                        let table = t.borrow();
                        let v = table.get(key);
                        if !v.is_nil() {
                            return Ok(v);
                        }
                        match &table.meta {
                            None => return Ok(Value::Nil),
                            Some(meta) => meta.get(&self.names.index),
                        }
                    };
                    match handler {
                        Value::Nil => return Ok(Value::Nil),
                        Value::Function(_) | Value::Native(_) => {
                            return self.call1(&handler, &[current.clone(), key.clone()]);
                        }
                        other => current = other,
                    }
                }
                Value::Str(_) => {
                    return Ok(match &self.string_lib {
                        Some(lib) => lib.get(key),
                        None => Value::Nil,
                    });
                }
                Value::UserData(u) => {
                    let u = u.clone();
                    return u.index(self, key);
                }
                Value::Vector(x, y, z) => {
                    if let Value::Str(s) = key {
                        match s.as_bytes() {
                            b"X" | b"x" => return Ok(Value::Num(f64::from(*x))),
                            b"Y" | b"y" => return Ok(Value::Num(f64::from(*y))),
                            b"Z" | b"z" => return Ok(Value::Num(f64::from(*z))),
                            _ => {}
                        }
                    }
                    return match self.vector_index.clone() {
                        Some(Value::Table(t)) => Ok(t.get(key)),
                        Some(handler) => self.call1(&handler, &[current.clone(), key.clone()]),
                        None => Ok(Value::Nil),
                    };
                }
                other => {
                    return Err(Error::runtime(format!(
                        "attempt to index {} with {}",
                        describe_type(other),
                        describe_key(key)
                    )));
                }
            }
        }
        Err(Error::runtime("'__index' chain too long; possible loop"))
    }

    /// `obj[key] = value`, honouring `__newindex`.
    pub fn set_index(&mut self, obj: &Value, key: Value, value: Value) -> Result<()> {
        let mut current = obj.clone();
        for _ in 0..64 {
            match &current {
                Value::Table(t) => {
                    let handler = {
                        let mut table = t.borrow_mut();
                        let handler = match &table.meta {
                            None => Value::Nil,
                            Some(meta) => {
                                if table.get(&key).is_nil() {
                                    meta.get(&self.names.newindex)
                                } else {
                                    Value::Nil
                                }
                            }
                        };
                        if handler.is_nil() {
                            return table.set(key, value);
                        }
                        handler
                    };
                    match handler {
                        Value::Function(_) | Value::Native(_) => {
                            self.call(&handler, &[current.clone(), key, value])?;
                            return Ok(());
                        }
                        other => current = other,
                    }
                }
                Value::UserData(u) => {
                    let u = u.clone();
                    return u.new_index(self, &key, value);
                }
                other => {
                    return Err(Error::runtime(format!(
                        "attempt to index {} with {}",
                        describe_type(other),
                        describe_key(&key)
                    )));
                }
            }
        }
        Err(Error::runtime("'__newindex' chain too long; possible loop"))
    }

    /// Converts a value to a string the way `tostring` does.
    pub fn tostring(&mut self, v: &Value) -> Result<String> {
        if let Value::Table(_) = v {
            let handler = self.metamethod(v, &self.names.tostring);
            if !handler.is_nil() {
                return Ok(self.call1(&handler, &[v.clone()])?.to_display());
            }
        }
        Ok(v.to_display())
    }

    /// Equality as the `==` operator sees it.
    pub fn equals(&mut self, a: &Value, b: &Value) -> Result<bool> {
        if a.raw_eq(b) {
            return Ok(true);
        }
        match (a, b) {
            (Value::Table(_), Value::Table(_)) => {
                let mut handler = self.metamethod(a, &self.names.eq);
                if handler.is_nil() {
                    handler = self.metamethod(b, &self.names.eq);
                }
                if handler.is_nil() {
                    return Ok(false);
                }
                Ok(self.call1(&handler, &[a.clone(), b.clone()])?.truthy())
            }
            (Value::UserData(x), Value::UserData(y)) => Ok(x.equals(y.as_ref())),
            _ => Ok(false),
        }
    }

    /// `a < b` (or `a <= b`), honouring `__lt`/`__le`.
    pub fn less(&mut self, a: &Value, b: &Value, or_equal: bool) -> Result<bool> {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => return Ok(if or_equal { x <= y } else { x < y }),
            (Value::Str(x), Value::Str(y)) => {
                let ord = x.as_bytes().cmp(y.as_bytes());
                return Ok(if or_equal { ord.is_le() } else { ord.is_lt() });
            }
            (Value::Int(_) | Value::Num(_), Value::Int(_) | Value::Num(_)) => {
                let (x, y) = (a.to_number().unwrap_or(f64::NAN), b.to_number().unwrap_or(f64::NAN));
                return Ok(if or_equal { x <= y } else { x < y });
            }
            (Value::UserData(x), Value::UserData(y)) => {
                if or_equal {
                    if x.equals(y.as_ref()) {
                        return Ok(true);
                    }
                }
                if let Some(result) = x.less_than(y.as_ref()) {
                    return Ok(result);
                }
            }
            _ => {}
        }
        let event = if or_equal { self.names.le.clone() } else { self.names.lt.clone() };
        let mut handler = self.metamethod(a, &event);
        if handler.is_nil() {
            handler = self.metamethod(b, &event);
        }
        if handler.is_nil() {
            let (ta, tb) = (describe_type(a), describe_type(b));
            return Err(Error::runtime(if ta == tb {
                format!("attempt to compare two {ta} values")
            } else {
                format!("attempt to compare {ta} with {tb}")
            }));
        }
        Ok(self.call1(&handler, &[a.clone(), b.clone()])?.truthy())
    }

    /// Arithmetic on arbitrary values (the slow path of the arithmetic
    /// instructions): string coercion, vectors, userdata and metamethods.
    pub fn arith(&mut self, op: ArithOp, a: &Value, b: &Value) -> Result<Value> {
        if let (Some(x), Some(y)) = (arith_operand(a), arith_operand(b)) {
            return arith_numbers(op, &x, &y);
        }
        if let Some(v) = vector_arith(op, a, b) {
            return Ok(v);
        }
        if let Value::UserData(u) = a {
            let u = u.clone();
            if let Some(v) = u.arith(self, op, a, b)? {
                return Ok(v);
            }
        }
        if let Value::UserData(u) = b {
            let u = u.clone();
            if let Some(v) = u.arith(self, op, a, b)? {
                return Ok(v);
            }
        }
        let event = self.names.arith(op).clone();
        let mut handler = self.metamethod(a, &event);
        if handler.is_nil() {
            handler = self.metamethod(b, &event);
        }
        if !handler.is_nil() {
            return self.call1(&handler, &[a.clone(), b.clone()]);
        }
        let culprit = if arith_operand(a).is_none() { a } else { b };
        Err(Error::runtime(format!(
            "attempt to perform arithmetic ({}) on {}",
            &op.event()[2..],
            describe_value_kind(culprit)
        )))
    }

    fn negate(&mut self, v: &Value) -> Result<Value> {
        match v {
            Value::Int(i) => Ok(Value::Int(i.wrapping_neg())),
            Value::Num(n) => Ok(Value::Num(-n)),
            Value::Vector(x, y, z) => Ok(Value::Vector(-x, -y, -z)),
            Value::Str(_) => match arith_operand(v) {
                Some(Value::Int(i)) => Ok(Value::Int(i.wrapping_neg())),
                Some(Value::Num(n)) => Ok(Value::Num(-n)),
                _ => Err(Error::runtime("attempt to perform arithmetic (unm) on a string value")),
            },
            Value::UserData(u) => {
                let u = u.clone();
                match u.neg(self)? {
                    Some(v) => Ok(v),
                    None => Err(Error::runtime(format!(
                        "attempt to perform arithmetic (unm) on a {} value",
                        u.type_name()
                    ))),
                }
            }
            _ => {
                let handler = self.metamethod(v, &self.names.unm);
                if handler.is_nil() {
                    return Err(Error::runtime(format!(
                        "attempt to perform arithmetic (unm) on {}",
                        describe_value_kind(v)
                    )));
                }
                self.call1(&handler, &[v.clone(), v.clone()])
            }
        }
    }

    /// The `#` operator.
    pub fn length(&mut self, v: &Value) -> Result<Value> {
        match v {
            Value::Str(s) => Ok(Value::Int(s.len() as i64)),
            Value::Table(t) => {
                let handler = self.metamethod(v, &self.names.len);
                if handler.is_nil() {
                    Ok(Value::Int(t.len()))
                } else {
                    self.call1(&handler, &[v.clone()])
                }
            }
            Value::UserData(u) => match u.length() {
                Some(n) => Ok(Value::Int(n)),
                None => Err(Error::runtime(format!("attempt to get length of a {} value", u.type_name()))),
            },
            other => Err(Error::runtime(format!("attempt to get length of {}", describe_value_kind(other)))),
        }
    }

    fn concat_slow(&mut self, values: &[Value]) -> Result<Value> {
        let mut acc = values[values.len() - 1].clone();
        for left in values[..values.len() - 1].iter().rev() {
            let mut buf = Vec::new();
            if concat_piece(left, &mut buf) && concat_piece(&acc, &mut buf) {
                acc = Value::Str(LuaStr::from_vec(buf));
                continue;
            }
            let mut handler = self.metamethod(left, &self.names.concat);
            if handler.is_nil() {
                handler = self.metamethod(&acc, &self.names.concat);
            }
            if handler.is_nil() {
                let mut scratch = Vec::new();
                let culprit = if concat_piece(left, &mut scratch) { &acc } else { left };
                return Err(Error::runtime(format!("attempt to concatenate {}", describe_value_kind(culprit))));
            }
            acc = self.call1(&handler, &[left.clone(), acc.clone()])?;
        }
        Ok(acc)
    }

    // ------------------------------------------------------------- calls

    fn call_at(
        &mut self,
        th: &mut ThreadState,
        func_idx: usize,
        mut nargs: usize,
        nres: u8,
        mut protect: Protect,
    ) -> Result<CallOutcome> {
        loop {
            let callee = th.stack[func_idx].clone();
            let result = match &callee {
                Value::Function(closure) => {
                    if th.frames.len() >= MAX_FRAMES || th.stack.len() >= MAX_STACK {
                        return Err(Error::runtime("stack overflow"));
                    }
                    let nparams = closure.proto.nparams as usize;
                    let base = func_idx + 1;
                    let mut varargs = Vec::new();
                    if nargs > nparams {
                        if closure.proto.is_vararg {
                            varargs = th.stack.drain(base + nparams..).collect();
                        } else {
                            th.stack.truncate(base + nparams);
                        }
                    } else if nargs < nparams {
                        th.stack.resize(base + nparams, Value::Nil);
                    }
                    th.frames.push(Frame {
                        closure: closure.clone(),
                        pc: 0,
                        base,
                        ret_base: func_idx,
                        nres,
                        varargs,
                        protect,
                    });
                    return Ok(CallOutcome::Frame);
                }
                Value::Native(native) => {
                    let mut buf = std::mem::take(&mut self.arg_buf);
                    buf.clear();
                    buf.extend(th.stack.drain(func_idx + 1..));
                    th.stack.truncate(func_idx);
                    let result = (native.f)(self, &buf);
                    buf.clear();
                    self.arg_buf = buf;
                    result
                }
                Value::UserData(u) => {
                    let args: Vec<Value> = th.stack.drain(func_idx + 1..).collect();
                    th.stack.truncate(func_idx);
                    u.call(self, &args)
                }
                Value::Table(_) => {
                    let handler = self.metamethod(&callee, &self.names.call);
                    if handler.is_nil() {
                        Err(Error::runtime("attempt to call a table value"))
                    } else {
                        th.stack.insert(func_idx + 1, callee.clone());
                        th.stack[func_idx] = handler;
                        nargs += 1;
                        continue;
                    }
                }
                other => Err(Error::runtime(format!("attempt to call {}", describe_value_kind(other)))),
            };
            th.stack.truncate(func_idx);
            match result {
                Ok(Ret::Yield(values)) => {
                    th.pending = Some(Pending { ret_base: func_idx, nres, prefix_true: protect.is_some() });
                    return Ok(CallOutcome::Yield(values));
                }
                Ok(Ret::Protected { func, args, handler }) => {
                    th.stack.push(func);
                    nargs = args.len();
                    th.stack.extend(args);
                    protect = Some(handler);
                    continue;
                }
                Ok(Ret::Call { func, args }) => {
                    th.stack.push(func);
                    nargs = args.len();
                    th.stack.extend(args);
                    continue;
                }
                Ok(ret) => {
                    if protect.is_some() {
                        th.stack.push(Value::Bool(true));
                    }
                    match ret {
                        Ret::One(v) => th.stack.push(v),
                        Ret::Values(values) => th.stack.extend(values),
                        _ => {}
                    }
                }
                Err(e) => match protect {
                    Some(handler) if e.kind != ErrorKind::Budget => {
                        let value = self.handle_error_value(e, handler);
                        th.stack.push(Value::Bool(false));
                        th.stack.push(value);
                    }
                    _ => return Err(e),
                },
            }
            finish_results(th, func_idx, nres);
            return Ok(CallOutcome::Done);
        }
    }

    /// Produces the value `pcall`/`xpcall` reports for a caught error.
    fn handle_error_value(&mut self, e: Error, handler: Option<Value>) -> Value {
        self.last_traceback = e.traceback;
        match handler {
            Some(handler) => match self.call1(&handler, &[e.value]) {
                Ok(v) => v,
                Err(inner) => inner.value,
            },
            None => e.value,
        }
    }

    /// Runs `th` until its frame count drops back to `stop`.
    fn execute(&mut self, th: &mut ThreadState, stop: usize) -> Result<Exec> {
        loop {
            match self.run(th, stop) {
                Ok(exec) => return Ok(exec),
                Err(e) => {
                    if self.unwind(th, stop, e)? {
                        return Ok(Exec::Done);
                    }
                }
            }
        }
    }

    /// Handles an error raised inside `run`. Returns `Ok(true)` when a
    /// protected call caught it and the entry frame is gone, `Ok(false)` when
    /// execution should continue in the catching frame's caller.
    fn unwind(&mut self, th: &mut ThreadState, stop: usize, mut e: Error) -> Result<bool> {
        // Attach a source position to interpreter- and native-raised errors.
        if !e.located {
            e.located = true;
            let level = usize::from(e.level.max(1));
            if e.level > 0 && th.frames.len() >= stop + level {
                let frame = &th.frames[th.frames.len() - level];
                let proto = &frame.closure.proto;
                let line = proto.lines.get(frame.pc.saturating_sub(1)).copied().unwrap_or(proto.line);
                e.chunk = Some(proto.chunk.to_string());
                e.line = Some(line);
                if let Value::Str(msg) = &e.value {
                    e.value = Value::from(format!("{}:{}: {}", proto.chunk, line, msg.to_str()));
                }
            }
        }
        let catcher = if e.kind == ErrorKind::Budget {
            None
        } else {
            (stop..th.frames.len()).rev().find(|&i| th.frames[i].protect.is_some())
        };
        let unwound_from = catcher.unwrap_or(stop);
        for frame in th.frames[unwound_from..].iter().rev() {
            let proto = &frame.closure.proto;
            e.traceback.push(TraceEntry {
                chunk: proto.chunk.to_string(),
                line: proto.lines.get(frame.pc.saturating_sub(1)).copied().unwrap_or(proto.line),
                function: if proto.name == "main chunk" || proto.name == "anonymous function" {
                    proto.name.clone()
                } else {
                    format!("function '{}'", proto.name)
                },
            });
        }
        match catcher {
            Some(i) => {
                let frame = th.frames.swap_remove(i);
                th.frames.truncate(i);
                th.stack.truncate(frame.ret_base);
                let value = self.handle_error_value(e, frame.protect.flatten());
                th.stack.push(Value::Bool(false));
                th.stack.push(value);
                finish_results(th, frame.ret_base, frame.nres);
                Ok(th.frames.len() == stop)
            }
            None => {
                let ret_base = th.frames.get(stop).map_or(0, |f| f.ret_base);
                th.frames.truncate(stop);
                th.stack.truncate(ret_base);
                Err(e)
            }
        }
    }

    #[inline]
    fn tick(&mut self) -> Result<()> {
        self.budget -= 1;
        if self.budget < 0 {
            return Err(Error::budget(
                "script exceeded its execution budget (is there a loop without a wait?)",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn run(&mut self, th: &mut ThreadState, stop: usize) -> Result<Exec> {
        'frame: loop {
            let fi = th.frames.len() - 1;
            let cl = th.frames[fi].closure.clone();
            let base = th.frames[fi].base;
            let mut pc = th.frames[fi].pc;
            let proto: &Proto = &cl.proto;
            let code = &proto.code[..];
            let consts = &proto.consts[..];

            macro_rules! tri {
                ($e:expr) => {
                    match $e {
                        Ok(v) => v,
                        Err(e) => {
                            th.frames[fi].pc = pc;
                            return Err(e);
                        }
                    }
                };
            }
            macro_rules! pop {
                () => {
                    th.stack.pop().expect("operand stack underflow")
                };
            }
            // Binary arithmetic with integer and float fast paths.
            macro_rules! arith {
                ($op:expr, $int:expr, $num:expr) => {{
                    let b = pop!();
                    let top = th.stack.len() - 1;
                    let fast = match (&th.stack[top], &b) {
                        (Value::Int(x), Value::Int(y)) => $int(*x, *y),
                        (Value::Num(x), Value::Num(y)) => Some(Value::Num($num(*x, *y))),
                        (Value::Int(x), Value::Num(y)) => Some(Value::Num($num(*x as f64, *y))),
                        (Value::Num(x), Value::Int(y)) => Some(Value::Num($num(*x, *y as f64))),
                        _ => None,
                    };
                    th.stack[top] = match fast {
                        Some(v) => v,
                        None => {
                            let a = th.stack[top].clone();
                            tri!(self.arith($op, &a, &b))
                        }
                    };
                }};
            }
            macro_rules! compare {
                ($int:expr, $num:expr, $swap:expr, $or_equal:expr) => {{
                    let b = pop!();
                    let top = th.stack.len() - 1;
                    let fast = match (&th.stack[top], &b) {
                        (Value::Int(x), Value::Int(y)) => Some($int(x, y)),
                        (Value::Num(x), Value::Num(y)) => Some($num(x, y)),
                        _ => None,
                    };
                    let result = match fast {
                        Some(r) => r,
                        None => {
                            let a = th.stack[top].clone();
                            if $swap { tri!(self.less(&b, &a, $or_equal)) } else { tri!(self.less(&a, &b, $or_equal)) }
                        }
                    };
                    th.stack[top] = Value::Bool(result);
                }};
            }
            macro_rules! bitwise {
                ($f:expr) => {{
                    let b = pop!();
                    let top = th.stack.len() - 1;
                    let x = tri!(bit_operand(&th.stack[top]));
                    let y = tri!(bit_operand(&b));
                    th.stack[top] = Value::Int($f(x, y));
                }};
            }

            loop {
                let op = code[pc];
                pc += 1;
                match op {
                    Op::Const(k) => th.stack.push(consts[k as usize].clone()),
                    Op::Nil => th.stack.push(Value::Nil),
                    Op::Nils(n) => {
                        let len = th.stack.len() + n as usize;
                        th.stack.resize(len, Value::Nil);
                    }
                    Op::True => th.stack.push(Value::Bool(true)),
                    Op::False => th.stack.push(Value::Bool(false)),
                    Op::Pop(n) => {
                        let len = th.stack.len() - n as usize;
                        th.stack.truncate(len);
                    }
                    Op::Dup2 => {
                        let n = th.stack.len();
                        let (a, b) = (th.stack[n - 2].clone(), th.stack[n - 1].clone());
                        th.stack.push(a);
                        th.stack.push(b);
                    }
                    Op::GetLocal(slot) => {
                        let v = th.stack[base + slot as usize].clone();
                        th.stack.push(v);
                    }
                    Op::SetLocal(slot) => {
                        let v = pop!();
                        th.stack[base + slot as usize] = v;
                    }
                    Op::GetCell(slot) => {
                        let v = match &th.stack[base + slot as usize] {
                            Value::Cell(cell) => cell.get(),
                            other => other.clone(),
                        };
                        th.stack.push(v);
                    }
                    Op::SetCell(slot) => {
                        let v = pop!();
                        match &th.stack[base + slot as usize] {
                            Value::Cell(cell) => cell.set(v),
                            _ => th.stack[base + slot as usize] = v,
                        }
                    }
                    Op::MakeCell(slot) => {
                        let v = std::mem::take(&mut th.stack[base + slot as usize]);
                        let cell = CellRef(Rc::new(CellBox { gc: GcHeader::default(), v: RefCell::new(v) }));
                        self.gc.track_cell(&cell);
                        th.stack[base + slot as usize] = Value::Cell(cell);
                    }
                    Op::GetUpval(i) => th.stack.push(cl.upvals[i as usize].get()),
                    Op::SetUpval(i) => {
                        let v = pop!();
                        cl.upvals[i as usize].set(v);
                    }
                    Op::GetGlobal(k) => {
                        let key = &consts[k as usize];
                        let fast = {
                            let env = cl.env.borrow();
                            let v = env.get(key);
                            if v.is_nil() && env.meta.is_some() { None } else { Some(v) }
                        };
                        let v = match fast {
                            Some(v) => v,
                            None => tri!(self.index(&Value::Table(cl.env.clone()), key)),
                        };
                        th.stack.push(v);
                    }
                    Op::SetGlobal(k) => {
                        let v = pop!();
                        let key = consts[k as usize].clone();
                        let has_meta = cl.env.borrow().meta.is_some();
                        if has_meta {
                            tri!(self.set_index(&Value::Table(cl.env.clone()), key, v));
                        } else {
                            tri!(cl.env.borrow_mut().set(key, v));
                        }
                    }
                    Op::GetIndex => {
                        let key = pop!();
                        let top = th.stack.len() - 1;
                        let fast = match &th.stack[top] {
                            Value::Table(t) => {
                                let table = t.borrow();
                                let v = table.get(&key);
                                if !v.is_nil() || table.meta.is_none() { Some(v) } else { None }
                            }
                            _ => None,
                        };
                        th.stack[top] = match fast {
                            Some(v) => v,
                            None => {
                                let obj = th.stack[top].clone();
                                tri!(self.index(&obj, &key))
                            }
                        };
                    }
                    Op::GetField(k) => {
                        let key = &consts[k as usize];
                        let top = th.stack.len() - 1;
                        let fast = match &th.stack[top] {
                            Value::Table(t) => {
                                let table = t.borrow();
                                let v = table.get(key);
                                if !v.is_nil() || table.meta.is_none() { Some(v) } else { None }
                            }
                            _ => None,
                        };
                        th.stack[top] = match fast {
                            Some(v) => v,
                            None => {
                                let obj = th.stack[top].clone();
                                tri!(self.index(&obj, key))
                            }
                        };
                    }
                    Op::SetIndex => {
                        let v = pop!();
                        let key = pop!();
                        let obj = pop!();
                        tri!(self.set_index_fast(&obj, key, v));
                    }
                    Op::SetField(k) => {
                        let v = pop!();
                        let obj = pop!();
                        tri!(self.set_index_fast(&obj, consts[k as usize].clone(), v));
                    }
                    Op::SetIndexAt(slot) => {
                        let v = pop!();
                        let obj = th.stack[base + slot as usize].clone();
                        let key = th.stack[base + slot as usize + 1].clone();
                        tri!(self.set_index_fast(&obj, key, v));
                    }
                    Op::InitField(k) => {
                        let v = pop!();
                        if let Some(Value::Table(t)) = th.stack.last() {
                            tri!(t.borrow_mut().set(consts[k as usize].clone(), v));
                        }
                    }
                    Op::InitIndex => {
                        let v = pop!();
                        let key = pop!();
                        if let Some(Value::Table(t)) = th.stack.last() {
                            tri!(t.borrow_mut().set(key, v));
                        }
                    }
                    Op::SetList { n, multi } => {
                        let count = n as usize + if multi { th.multi } else { 0 };
                        let start = th.stack.len() - count;
                        if let Value::Table(t) = &th.stack[start - 1] {
                            let t = t.clone();
                            let mut table = t.borrow_mut();
                            if table.map.is_empty() {
                                table.arr.extend(th.stack.drain(start..));
                            } else {
                                let mut next = table.arr.len() as i64 + 1;
                                for v in th.stack.drain(start..) {
                                    table.set_int(next, v);
                                    next += 1;
                                }
                            }
                        }
                        th.stack.truncate(start);
                    }
                    Op::NewTable { arr, hash } => {
                        let t = self.new_table();
                        if arr > 0 || hash > 0 {
                            let mut table = t.borrow_mut();
                            table.arr.reserve(arr as usize);
                            table.map.reserve(hash as usize);
                        }
                        th.stack.push(Value::Table(t));
                    }
                    Op::Method(k) => {
                        let key = &consts[k as usize];
                        let top = th.stack.len() - 1;
                        let obj = th.stack[top].clone();
                        let method = tri!(self.index(&obj, key));
                        th.stack[top] = method;
                        th.stack.push(obj);
                    }
                    Op::Add => arith!(ArithOp::Add, |x: i64, y: i64| Some(Value::Int(x.wrapping_add(y))), |x: f64, y: f64| x + y),
                    Op::Sub => arith!(ArithOp::Sub, |x: i64, y: i64| Some(Value::Int(x.wrapping_sub(y))), |x: f64, y: f64| x - y),
                    Op::Mul => arith!(ArithOp::Mul, |x: i64, y: i64| Some(Value::Int(x.wrapping_mul(y))), |x: f64, y: f64| x * y),
                    Op::Div => arith!(ArithOp::Div, |x: i64, y: i64| Some(Value::Num(x as f64 / y as f64)), |x: f64, y: f64| x / y),
                    Op::Pow => arith!(ArithOp::Pow, |x: i64, y: i64| Some(Value::Num((x as f64).powf(y as f64))), |x: f64, y: f64| x.powf(y)),
                    Op::Mod => arith!(
                        ArithOp::Mod,
                        |x: i64, y: i64| if y == 0 { None } else { Some(Value::Int(int_mod(x, y))) },
                        float_mod
                    ),
                    Op::IDiv => arith!(
                        ArithOp::IDiv,
                        |x: i64, y: i64| if y == 0 { None } else { Some(Value::Int(int_floor_div(x, y))) },
                        |x: f64, y: f64| (x / y).floor()
                    ),
                    Op::BAnd => bitwise!(|x: i64, y: i64| x & y),
                    Op::BOr => bitwise!(|x: i64, y: i64| x | y),
                    Op::BXor => bitwise!(|x: i64, y: i64| x ^ y),
                    Op::Shl => bitwise!(shift_left),
                    Op::Shr => bitwise!(|x: i64, y: i64| shift_left(x, y.wrapping_neg())),
                    Op::Concat(n) => {
                        let start = th.stack.len() - n as usize;
                        let mut buf = Vec::new();
                        let simple = th.stack[start..].iter().all(|v| concat_piece(v, &mut buf));
                        let result = if simple {
                            Value::Str(LuaStr::from_vec(buf))
                        } else {
                            let values: Vec<Value> = th.stack[start..].to_vec();
                            tri!(self.concat_slow(&values))
                        };
                        th.stack.truncate(start);
                        th.stack.push(result);
                    }
                    Op::Eq | Op::Ne => {
                        let b = pop!();
                        let top = th.stack.len() - 1;
                        let mut equal = th.stack[top].raw_eq(&b);
                        if !equal && matches!((&th.stack[top], &b), (Value::Table(_), Value::Table(_)) | (Value::UserData(_), Value::UserData(_))) {
                            let a = th.stack[top].clone();
                            equal = tri!(self.equals(&a, &b));
                        }
                        th.stack[top] = Value::Bool(equal == (op == Op::Eq));
                    }
                    Op::Lt => compare!(|x: &i64, y: &i64| x < y, |x: &f64, y: &f64| x < y, false, false),
                    Op::Le => compare!(|x: &i64, y: &i64| x <= y, |x: &f64, y: &f64| x <= y, false, true),
                    Op::Gt => compare!(|x: &i64, y: &i64| x > y, |x: &f64, y: &f64| x > y, true, false),
                    Op::Ge => compare!(|x: &i64, y: &i64| x >= y, |x: &f64, y: &f64| x >= y, true, true),
                    Op::Not => {
                        let top = th.stack.len() - 1;
                        th.stack[top] = Value::Bool(!th.stack[top].truthy());
                    }
                    Op::Neg => {
                        let top = th.stack.len() - 1;
                        th.stack[top] = match &th.stack[top] {
                            Value::Int(i) => Value::Int(i.wrapping_neg()),
                            Value::Num(n) => Value::Num(-n),
                            other => {
                                let v = other.clone();
                                tri!(self.negate(&v))
                            }
                        };
                    }
                    Op::Len => {
                        let top = th.stack.len() - 1;
                        let v = th.stack[top].clone();
                        th.stack[top] = tri!(self.length(&v));
                    }
                    Op::BNot => {
                        let top = th.stack.len() - 1;
                        let x = tri!(bit_operand(&th.stack[top]));
                        th.stack[top] = Value::Int(!x);
                    }
                    Op::Jump(target) => {
                        if (target as usize) < pc {
                            tri!(self.tick());
                        }
                        pc = target as usize;
                    }
                    Op::JumpIfFalse(target) => {
                        if !pop!().truthy() {
                            pc = target as usize;
                        }
                    }
                    Op::AndJump(target) => {
                        if th.stack.last().is_some_and(Value::truthy) {
                            th.stack.pop();
                        } else {
                            pc = target as usize;
                        }
                    }
                    Op::OrJump(target) => {
                        if th.stack.last().is_some_and(Value::truthy) {
                            pc = target as usize;
                        } else {
                            th.stack.pop();
                        }
                    }
                    Op::Call { nargs, nres, multi } => {
                        tri!(self.tick());
                        let total = nargs as usize + if multi { th.multi } else { 0 };
                        let func_idx = th.stack.len() - total - 1;
                        th.frames[fi].pc = pc;
                        if !matches!(
                            th.stack[func_idx],
                            Value::Function(_) | Value::Native(_) | Value::UserData(_) | Value::Table(_)
                        ) {
                            let what = describe_value_kind(&th.stack[func_idx]);
                            return Err(Error::runtime(match proto.call_name(pc - 1) {
                                Some(name) => format!("attempt to call {what} ({name})"),
                                None => format!("attempt to call {what}"),
                            }));
                        }
                        match self.call_at(th, func_idx, total, nres, None)? {
                            CallOutcome::Frame => continue 'frame,
                            CallOutcome::Done => {}
                            CallOutcome::Yield(values) => return Ok(Exec::Yield(values)),
                        }
                    }
                    Op::Return { n, multi } => {
                        let count = n as usize + if multi { th.multi } else { 0 };
                        let frame = th.frames.pop().expect("active frame");
                        let src = th.stack.len() - count;
                        let mut dst = frame.ret_base;
                        if frame.protect.is_some() {
                            th.stack[dst] = Value::Bool(true);
                            dst += 1;
                        }
                        if src != dst {
                            for i in 0..count {
                                th.stack[dst + i] = std::mem::take(&mut th.stack[src + i]);
                            }
                        }
                        th.stack.truncate(dst + count);
                        finish_results(th, frame.ret_base, frame.nres);
                        if th.frames.len() == stop {
                            return Ok(Exec::Done);
                        }
                        continue 'frame;
                    }
                    Op::Closure(i) => {
                        let child = proto.protos[i as usize].clone();
                        let mut upvals = Vec::with_capacity(child.upvals.len());
                        for &(from_local, index) in &child.upvals {
                            if from_local {
                                match &th.stack[base + index as usize] {
                                    Value::Cell(cell) => upvals.push(cell.clone()),
                                    other => {
                                        // A captured slot is always boxed; tolerate the
                                        // impossible case by capturing a copy.
                                        let cell = CellRef(Rc::new(CellBox {
                                            gc: GcHeader::default(),
                                            v: RefCell::new(other.clone()),
                                        }));
                                        self.gc.track_cell(&cell);
                                        upvals.push(cell);
                                    }
                                }
                            } else {
                                upvals.push(cl.upvals[index as usize].clone());
                            }
                        }
                        let closure = Rc::new(Closure {
                            gc: GcHeader::default(),
                            proto: child,
                            upvals,
                            env: cl.env.clone(),
                        });
                        self.gc.track_closure(&closure);
                        th.stack.push(Value::Function(closure));
                    }
                    Op::Vararg(n) => {
                        let start = th.stack.len();
                        th.stack.extend(th.frames[fi].varargs.iter().cloned());
                        finish_results(th, start, n);
                    }
                    Op::ForPrep(exit) => {
                        let top = th.stack.len();
                        match tri!(for_prep(&th.stack[top - 3], &th.stack[top - 2], &th.stack[top - 1])) {
                            Some((start, limit, step)) => {
                                th.stack[top - 3] = start.clone();
                                th.stack[top - 2] = limit;
                                th.stack[top - 1] = step;
                                th.stack.push(start);
                            }
                            None => pc = exit as usize,
                        }
                    }
                    Op::ForLoop(body) => {
                        tri!(self.tick());
                        th.stack.pop();
                        let top = th.stack.len();
                        let next = match (&th.stack[top - 3], &th.stack[top - 2], &th.stack[top - 1]) {
                            (Value::Int(i), Value::Int(limit), Value::Int(step)) => match i.checked_add(*step) {
                                Some(n) if (*step > 0 && n <= *limit) || (*step < 0 && n >= *limit) => {
                                    Some(Value::Int(n))
                                }
                                _ => None,
                            },
                            (Value::Num(i), Value::Num(limit), Value::Num(step)) => {
                                let n = i + step;
                                ((*step > 0.0 && n <= *limit) || (*step < 0.0 && n >= *limit)).then_some(Value::Num(n))
                            }
                            _ => None,
                        };
                        if let Some(v) = next {
                            th.stack[top - 3] = v.clone();
                            th.stack.push(v);
                            pc = body as usize;
                        }
                    }
                    Op::TForTest { exit, nvars } => {
                        let first = th.stack.len() - nvars as usize;
                        if th.stack[first].is_nil() {
                            th.stack.truncate(first);
                            pc = exit as usize;
                        } else {
                            th.stack[first - 1] = th.stack[first].clone();
                        }
                    }
                }
            }
        }
    }

    #[inline]
    fn set_index_fast(&mut self, obj: &Value, key: Value, value: Value) -> Result<()> {
        if let Value::Table(t) = obj {
            let mut table = t.borrow_mut();
            if table.meta.is_none() {
                return table.set(key, value);
            }
        }
        self.set_index(obj, key, value)
    }
}

/// Adjusts the values at `ret_base..` to the number the caller expects.
fn finish_results(th: &mut ThreadState, ret_base: usize, nres: u8) {
    if nres == MULTI {
        th.multi = th.stack.len() - ret_base;
    } else {
        th.stack.resize(ret_base + nres as usize, Value::Nil);
    }
}

fn describe_type(v: &Value) -> &'static str {
    match v {
        Value::UserData(u) => u.type_name(),
        other => other.type_name(),
    }
}

fn describe_value_kind(v: &Value) -> String {
    match v {
        Value::Nil => "a nil value".into(),
        other => format!("a {} value", describe_type(other)),
    }
}

fn describe_key(key: &Value) -> String {
    match key {
        Value::Str(s) => format!("'{}'", s.to_str()),
        other => other.type_name().to_string(),
    }
}

/// Appends the string form of a concatenation operand; false if not allowed.
fn concat_piece(v: &Value, out: &mut Vec<u8>) -> bool {
    match v {
        Value::Str(s) => out.extend_from_slice(s.as_bytes()),
        Value::Int(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Value::Num(n) => out.extend_from_slice(fmt_number(*n).as_bytes()),
        _ => return false,
    }
    true
}

/// A numeric operand, coercing numeric strings.
fn arith_operand(v: &Value) -> Option<Value> {
    match v {
        Value::Int(_) | Value::Num(_) => Some(v.clone()),
        Value::Str(s) => crate::lexer::parse_number(s.to_str().trim()),
        _ => None,
    }
}

fn arith_numbers(op: ArithOp, a: &Value, b: &Value) -> Result<Value> {
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        let (x, y) = (*x, *y);
        return Ok(match op {
            ArithOp::Add => Value::Int(x.wrapping_add(y)),
            ArithOp::Sub => Value::Int(x.wrapping_sub(y)),
            ArithOp::Mul => Value::Int(x.wrapping_mul(y)),
            ArithOp::Div => Value::Num(x as f64 / y as f64),
            ArithOp::Pow => Value::Num((x as f64).powf(y as f64)),
            ArithOp::Mod => {
                if y == 0 {
                    return Err(Error::runtime("attempt to perform 'n%0'"));
                }
                Value::Int(int_mod(x, y))
            }
            ArithOp::IDiv => {
                if y == 0 {
                    return Err(Error::runtime("attempt to perform 'n//0'"));
                }
                Value::Int(int_floor_div(x, y))
            }
        });
    }
    let (x, y) = (a.to_number().unwrap_or(f64::NAN), b.to_number().unwrap_or(f64::NAN));
    Ok(Value::Num(match op {
        ArithOp::Add => x + y,
        ArithOp::Sub => x - y,
        ArithOp::Mul => x * y,
        ArithOp::Div => x / y,
        ArithOp::Pow => x.powf(y),
        ArithOp::Mod => float_mod(x, y),
        ArithOp::IDiv => (x / y).floor(),
    }))
}

fn vector_arith(op: ArithOp, a: &Value, b: &Value) -> Option<Value> {
    let apply = |f: fn(f32, f32) -> f32, a: (f32, f32, f32), b: (f32, f32, f32)| {
        Value::Vector(f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
    };
    let f: fn(f32, f32) -> f32 = match op {
        ArithOp::Add => |x, y| x + y,
        ArithOp::Sub => |x, y| x - y,
        ArithOp::Mul => |x, y| x * y,
        ArithOp::Div => |x, y| x / y,
        ArithOp::IDiv => |x, y| (x / y).floor(),
        ArithOp::Mod => |x, y| x - (x / y).floor() * y,
        ArithOp::Pow => f32::powf,
    };
    match (a, b) {
        (Value::Vector(x, y, z), Value::Vector(u, v, w)) => Some(apply(f, (*x, *y, *z), (*u, *v, *w))),
        (Value::Vector(x, y, z), Value::Int(_) | Value::Num(_)) if !matches!(op, ArithOp::Add | ArithOp::Sub) => {
            let s = b.to_number()? as f32;
            Some(apply(f, (*x, *y, *z), (s, s, s)))
        }
        (Value::Int(_) | Value::Num(_), Value::Vector(x, y, z)) if matches!(op, ArithOp::Mul | ArithOp::Div) => {
            let s = a.to_number()? as f32;
            Some(apply(f, (s, s, s), (*x, *y, *z)))
        }
        _ => None,
    }
}

fn int_mod(x: i64, y: i64) -> i64 {
    let r = x.wrapping_rem(y);
    if r != 0 && (r ^ y) < 0 { r + y } else { r }
}

fn int_floor_div(x: i64, y: i64) -> i64 {
    let q = x.wrapping_div(y);
    if x.wrapping_rem(y) != 0 && ((x < 0) != (y < 0)) { q - 1 } else { q }
}

fn float_mod(x: f64, y: f64) -> f64 {
    if y.is_infinite() && x.is_finite() {
        return if (x >= 0.0) == (y > 0.0) { x } else { y };
    }
    let r = x % y;
    if r != 0.0 && ((r < 0.0) != (y < 0.0)) { r + y } else { r }
}

fn shift_left(x: i64, n: i64) -> i64 {
    if n <= -64 || n >= 64 {
        0
    } else if n >= 0 {
        ((x as u64) << n) as i64
    } else {
        ((x as u64) >> (-n)) as i64
    }
}

fn bit_operand(v: &Value) -> Result<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Num(n) => float_to_int_exact(*n).ok_or_else(|| Error::runtime("number has no integer representation")),
        Value::Str(_) => v.to_integer().ok_or_else(|| Error::runtime("attempt to perform bitwise operation on a string value")),
        other => Err(Error::runtime(format!(
            "attempt to perform bitwise operation on {}",
            describe_value_kind(other)
        ))),
    }
}

/// Normalizes a numeric `for` loop's control values. `None` means the loop
/// body never runs.
fn for_prep(start: &Value, limit: &Value, step: &Value) -> Result<Option<(Value, Value, Value)>> {
    if let (Value::Int(start), Value::Int(step)) = (start, step) {
        let (start, step) = (*start, *step);
        if step == 0 {
            return Err(Error::runtime("'for' step is zero"));
        }
        let limit = match limit {
            Value::Int(l) => *l,
            other => {
                let Some(n) = other.to_number() else {
                    return Err(Error::runtime("'for' limit must be a number"));
                };
                if n.is_nan() {
                    return Ok(None);
                }
                if step > 0 {
                    if n >= i64::MAX as f64 {
                        i64::MAX
                    } else if n < i64::MIN as f64 {
                        return Ok(None);
                    } else {
                        n.floor() as i64
                    }
                } else if n <= i64::MIN as f64 {
                    i64::MIN
                } else if n > i64::MAX as f64 {
                    return Ok(None);
                } else {
                    n.ceil() as i64
                }
            }
        };
        let runs = if step > 0 { start <= limit } else { start >= limit };
        return Ok(runs.then_some((Value::Int(start), Value::Int(limit), Value::Int(step))));
    }
    let number = |v: &Value, what: &str| {
        v.to_number().ok_or_else(|| Error::runtime(format!("'for' {what} must be a number")))
    };
    let (start, limit, step) = (number(start, "initial value")?, number(limit, "limit")?, number(step, "step")?);
    if step == 0.0 {
        return Err(Error::runtime("'for' step is zero"));
    }
    let runs = if step > 0.0 { start <= limit } else { start >= limit };
    Ok(runs.then_some((Value::Num(start), Value::Num(limit), Value::Num(step))))
}
