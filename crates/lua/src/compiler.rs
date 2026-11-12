//! Bytecode and the AST → bytecode code generator.
//!
//! The VM is a stack machine. Locals occupy fixed frame-relative slots (a
//! local's slot is the stack depth at its declaration) and temporaries live
//! above them. Locals captured by closures are boxed in heap cells.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::error::{Error, Result};
use crate::value::{LuaStr, Value};

/// Marker for "all available values" in result/argument counts.
pub const MULTI: u8 = 255;
/// Positional table items are flushed to the table in batches of this size.
const LIST_BATCH: u16 = 48;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Const(u32),
    Nil,
    /// Push `n` nils.
    Nils(u32),
    True,
    False,
    Pop(u32),
    Dup2,
    GetLocal(u32),
    SetLocal(u32),
    GetCell(u32),
    SetCell(u32),
    /// Box the value in a slot into a fresh heap cell.
    MakeCell(u32),
    GetUpval(u32),
    SetUpval(u32),
    GetGlobal(u32),
    SetGlobal(u32),
    /// `[t, k] -> [t[k]]`
    GetIndex,
    /// `[t] -> [t[K]]`
    GetField(u32),
    /// `[t, k, v] -> []`
    SetIndex,
    /// `[t, v] -> []`
    SetField(u32),
    /// Table at slot `s`, key at slot `s + 1`; pops the value.
    SetIndexAt(u32),
    /// `[t, v] -> [t]` (table constructors)
    InitField(u32),
    /// `[t, k, v] -> [t]`
    InitIndex,
    /// `[t, v1..vn] -> [t]`, appending to the array part.
    SetList { n: u16, multi: bool },
    NewTable { arr: u16, hash: u16 },
    /// `[obj] -> [obj[K], obj]`
    Method(u32),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    IDiv,
    BAnd,
    BOr,
    BXor,
    Shl,
    Shr,
    Concat(u32),
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Not,
    Neg,
    Len,
    BNot,
    Jump(u32),
    /// Pops the condition.
    JumpIfFalse(u32),
    /// `and`: jump keeping a falsy value, otherwise pop it.
    AndJump(u32),
    /// `or`: jump keeping a truthy value, otherwise pop it.
    OrJump(u32),
    Call { nargs: u16, nres: u8, multi: bool },
    Return { n: u16, multi: bool },
    Closure(u32),
    Vararg(u8),
    /// `[start, limit, step]`: validates the loop; pushes the control
    /// variable or jumps to the exit.
    ForPrep(u32),
    /// Pops the control variable, steps, and either pushes it again and
    /// jumps back or falls through.
    ForLoop(u32),
    /// After the iterator call: exits when the first of `nvars` results is
    /// nil, otherwise stores it as the new control value.
    TForTest { exit: u32, nvars: u8 },
}

/// A compiled function prototype.
pub struct Proto {
    pub name: String,
    pub chunk: Rc<str>,
    pub line: u32,
    pub code: Vec<Op>,
    /// Source line for each instruction.
    pub lines: Vec<u32>,
    pub consts: Vec<Value>,
    pub protos: Vec<Rc<Proto>>,
    pub nparams: u32,
    pub is_vararg: bool,
    /// For each upvalue: (captured from the parent's slot?, slot or parent upvalue index).
    pub upvals: Vec<(bool, u32)>,
    /// Descriptions of call targets for error messages, sorted by pc.
    pub call_names: Vec<(u32, String)>,
}

impl Proto {
    pub fn call_name(&self, pc: usize) -> Option<&str> {
        self.call_names
            .binary_search_by_key(&(pc as u32), |(p, _)| *p)
            .ok()
            .map(|i| self.call_names[i].1.as_str())
    }
}

/// Compiles Lua source text into a main-function prototype.
pub fn compile(source: &str, chunk: &str) -> Result<Rc<Proto>> {
    let body = crate::parser::parse(source, chunk)?;
    let chunk: Rc<str> = Rc::from(chunk);
    let proto = FuncGen::compile(&body, &chunk, None)?;
    Ok(Rc::new(proto))
}

#[derive(Hash, PartialEq, Eq)]
enum ConstKey {
    Int(i64),
    Num(u64),
    Str(Vec<u8>),
}

struct LoopCtx {
    /// Stack depth to restore before leaving the loop.
    break_depth: u32,
    continue_depth: u32,
    break_jumps: Vec<usize>,
    /// Jump instructions to patch to the continue point.
    continue_jumps: Vec<usize>,
    /// `repeat` loops evaluate their condition inside the body's scope, so a
    /// `continue` must pad the stack to the depth the condition expects.
    is_repeat: bool,
    /// `Nils` instructions to patch with that padding (see `repeat_stmt`).
    continue_pads: Vec<(usize, u32)>,
}

struct Label {
    name: String,
    pc: u32,
    depth: u32,
}

struct BlockCtx {
    labels: Vec<Label>,
    /// Labels that end the block; they resolve to the point after the block's
    /// locals have been popped, so `goto continue` may skip local declarations.
    end_labels: Vec<String>,
    first_pending: usize,
}

struct PendingGoto {
    name: String,
    pop_pc: usize,
    jump_pc: usize,
    depth: u32,
    line: u32,
}

struct FuncGen<'a> {
    body: &'a FuncBody,
    chunk: &'a Rc<str>,
    parent_slots: Option<&'a [u32]>,
    code: Vec<Op>,
    lines: Vec<u32>,
    consts: Vec<Value>,
    const_map: HashMap<ConstKey, u32>,
    protos: Vec<Rc<Proto>>,
    call_names: Vec<(u32, String)>,
    /// LocalId → frame slot.
    slots: Vec<u32>,
    depth: u32,
    line: u32,
    loops: Vec<LoopCtx>,
    blocks: Vec<BlockCtx>,
    pending: Vec<PendingGoto>,
}

impl<'a> FuncGen<'a> {
    fn compile(body: &'a FuncBody, chunk: &'a Rc<str>, parent_slots: Option<&'a [u32]>) -> Result<Proto> {
        let mut g = FuncGen {
            body,
            chunk,
            parent_slots,
            code: Vec::new(),
            lines: Vec::new(),
            consts: Vec::new(),
            const_map: HashMap::new(),
            protos: Vec::new(),
            call_names: Vec::new(),
            slots: vec![u32::MAX; body.locals.len()],
            depth: 0,
            line: body.line,
            loops: Vec::new(),
            blocks: Vec::new(),
            pending: Vec::new(),
        };
        for (i, &param) in body.params.iter().enumerate() {
            g.slots[param as usize] = i as u32;
        }
        g.depth = body.params.len() as u32;
        for &param in &body.params {
            if body.locals[param as usize].captured {
                let slot = g.slots[param as usize];
                g.emit(Op::MakeCell(slot));
            }
        }
        g.block(&body.body)?;
        g.emit(Op::Return { n: 0, multi: false });
        if let Some(goto) = g.pending.first() {
            return Err(Error::syntax(chunk, goto.line, format!("no visible label '{}' for goto", goto.name)));
        }
        let upvals = body
            .upvals
            .iter()
            .map(|u| {
                if u.from_parent_local {
                    let slots = g.parent_slots.expect("upvalue requires an enclosing function");
                    (true, slots[u.index as usize])
                } else {
                    (false, u.index)
                }
            })
            .collect();
        Ok(Proto {
            name: body.name.clone(),
            chunk: chunk.clone(),
            line: body.line,
            code: g.code,
            lines: g.lines,
            consts: g.consts,
            protos: g.protos,
            nparams: body.params.len() as u32,
            is_vararg: body.is_vararg,
            upvals,
            call_names: g.call_names,
        })
    }

    // ----------------------------------------------------------- emitting

    fn emit(&mut self, op: Op) -> usize {
        self.code.push(op);
        self.lines.push(self.line);
        self.code.len() - 1
    }

    fn pc(&self) -> u32 {
        self.code.len() as u32
    }

    fn patch_jump(&mut self, at: usize, target: u32) {
        self.code[at] = match self.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::AndJump(_) => Op::AndJump(target),
            Op::OrJump(_) => Op::OrJump(target),
            Op::ForPrep(_) => Op::ForPrep(target),
            Op::TForTest { nvars, .. } => Op::TForTest { exit: target, nvars },
            other => unreachable!("not a jump: {other:?}"),
        };
    }

    fn patch_here(&mut self, at: usize) {
        let target = self.pc();
        self.patch_jump(at, target);
    }

    fn emit_pop(&mut self, n: u32) {
        if n > 0 {
            self.emit(Op::Pop(n));
        }
    }

    fn constant(&mut self, key: ConstKey) -> u32 {
        if let Some(&i) = self.const_map.get(&key) {
            return i;
        }
        let value = match &key {
            ConstKey::Int(i) => Value::Int(*i),
            ConstKey::Num(bits) => Value::Num(f64::from_bits(*bits)),
            ConstKey::Str(s) => Value::Str(LuaStr::new(s)),
        };
        self.consts.push(value);
        let i = self.consts.len() as u32 - 1;
        self.const_map.insert(key, i);
        i
    }

    fn str_const(&mut self, s: &[u8]) -> u32 {
        self.constant(ConstKey::Str(s.to_vec()))
    }

    fn is_captured(&self, id: LocalId) -> bool {
        self.body.locals[id as usize].captured
    }

    fn syntax_error<T>(&self, msg: impl Into<String>) -> Result<T> {
        Err(Error::syntax(self.chunk, self.line, msg))
    }

    /// Binds a freshly pushed stack value as local `id`.
    fn bind_local(&mut self, id: LocalId, slot: u32) {
        self.slots[id as usize] = slot;
        if self.is_captured(id) {
            self.emit(Op::MakeCell(slot));
        }
    }

    // --------------------------------------------------------- statements

    fn block(&mut self, stmts: &[Stmt]) -> Result<()> {
        let start_depth = self.depth;
        self.blocks.push(BlockCtx {
            labels: Vec::new(),
            end_labels: Vec::new(),
            first_pending: self.pending.len(),
        });
        // Trailing labels are "end labels" (see `BlockCtx::end_labels`).
        let trailing = stmts.iter().rev().take_while(|s| matches!(s.kind, StmtKind::Label(_))).count();
        let body_len = stmts.len() - trailing;
        for stmt in &stmts[..body_len] {
            self.statement(stmt)?;
        }
        for stmt in &stmts[body_len..] {
            if let StmtKind::Label(name) = &stmt.kind {
                self.line = stmt.line;
                self.check_duplicate_label(name)?;
                self.blocks.last_mut().expect("block").end_labels.push(name.clone());
            }
        }
        self.emit_pop(self.depth - start_depth);
        self.depth = start_depth;
        let ctx = self.blocks.pop().expect("block");
        let target = self.pc();
        self.resolve_pending(ctx.first_pending, |name| {
            ctx.end_labels.iter().any(|l| l == name).then_some((target, start_depth))
        })?;
        Ok(())
    }

    fn check_duplicate_label(&self, name: &str) -> Result<()> {
        let block = self.blocks.last().expect("block");
        if block.labels.iter().any(|l| l.name == name) || block.end_labels.iter().any(|l| l == name) {
            return self.syntax_error(format!("label '{name}' already defined"));
        }
        Ok(())
    }

    /// Patches pending gotos (from index `first`) whose label `lookup` knows.
    fn resolve_pending(&mut self, first: usize, lookup: impl Fn(&str) -> Option<(u32, u32)>) -> Result<()> {
        let mut i = first;
        while i < self.pending.len() {
            if let Some((pc, depth)) = lookup(&self.pending[i].name) {
                let goto = self.pending.remove(i);
                if goto.depth < depth {
                    return Err(Error::syntax(
                        self.chunk,
                        goto.line,
                        format!("goto '{}' jumps into the scope of a local", goto.name),
                    ));
                }
                self.code[goto.pop_pc] = Op::Pop(goto.depth - depth);
                self.code[goto.jump_pc] = Op::Jump(pc);
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    fn statement(&mut self, stmt: &Stmt) -> Result<()> {
        self.line = stmt.line;
        match &stmt.kind {
            StmtKind::Call(e) => {
                self.call(e, 0)?;
            }
            StmtKind::Local(ids, exprs) => {
                let base = self.depth;
                self.exprs_adjusted(exprs, ids.len() as u32)?;
                for (i, &id) in ids.iter().enumerate() {
                    self.bind_local(id, base + i as u32);
                }
            }
            StmtKind::Assign(targets, exprs) => self.assign(targets, exprs)?,
            StmtKind::CompoundAssign(target, op, value) => self.compound_assign(target, *op, value)?,
            StmtKind::Do(body) => self.block(body)?,
            StmtKind::While(cond, body) => {
                let top = self.pc();
                self.expr(cond)?;
                let exit = self.emit(Op::JumpIfFalse(0));
                self.depth -= 1;
                self.open_loop(self.depth, self.depth);
                self.block(body)?;
                self.emit(Op::Jump(top));
                self.close_loop(top);
                self.patch_here(exit);
            }
            StmtKind::Repeat(body, cond) => self.repeat_stmt(body, cond)?,
            StmtKind::If(arms, otherwise) => {
                let mut end_jumps = Vec::new();
                for (i, (cond, body)) in arms.iter().enumerate() {
                    self.expr(cond)?;
                    let next = self.emit(Op::JumpIfFalse(0));
                    self.depth -= 1;
                    self.block(body)?;
                    if i + 1 < arms.len() || otherwise.is_some() {
                        end_jumps.push(self.emit(Op::Jump(0)));
                    }
                    self.patch_here(next);
                }
                if let Some(body) = otherwise {
                    self.block(body)?;
                }
                for jump in end_jumps {
                    self.patch_here(jump);
                }
            }
            StmtKind::NumericFor { var, start, limit, step, body } => {
                let base = self.depth;
                self.expr(start)?;
                self.expr(limit)?;
                match step {
                    Some(step) => self.expr(step)?,
                    None => {
                        let k = self.constant(ConstKey::Int(1));
                        self.emit(Op::Const(k));
                        self.depth += 1;
                    }
                }
                let prep = self.emit(Op::ForPrep(0));
                self.depth += 1;
                let body_pc = self.pc();
                self.bind_local(*var, base + 3);
                self.open_loop(base + 3, base + 4);
                self.block(body)?;
                let continue_pc = self.pc();
                self.emit(Op::ForLoop(body_pc));
                self.depth -= 1;
                self.patch_here(prep);
                self.close_loop(continue_pc);
                self.emit(Op::Pop(3));
                self.depth = base;
            }
            StmtKind::GenericFor { vars, exprs, body } => {
                let base = self.depth;
                self.exprs_adjusted(exprs, 3)?;
                let top = self.pc();
                self.emit(Op::GetLocal(base));
                self.emit(Op::GetLocal(base + 1));
                self.emit(Op::GetLocal(base + 2));
                let nvars = vars.len() as u8;
                self.emit(Op::Call { nargs: 2, nres: nvars, multi: false });
                let test = self.emit(Op::TForTest { exit: 0, nvars });
                self.depth = base + 3 + u32::from(nvars);
                for (i, &id) in vars.iter().enumerate() {
                    self.bind_local(id, base + 3 + i as u32);
                }
                self.open_loop(base + 3, base + 3 + u32::from(nvars));
                self.block(body)?;
                let continue_pc = self.pc();
                self.emit(Op::Pop(u32::from(nvars)));
                self.emit(Op::Jump(top));
                self.depth = base + 3;
                self.patch_here(test);
                self.close_loop(continue_pc);
                self.emit(Op::Pop(3));
                self.depth = base;
            }
            StmtKind::LocalFunction(id, body) => {
                let slot = self.depth;
                self.emit(Op::Nil);
                self.depth += 1;
                self.bind_local(*id, slot);
                self.closure(body)?;
                self.emit(if self.is_captured(*id) { Op::SetCell(slot) } else { Op::SetLocal(slot) });
                self.depth -= 1;
            }
            StmtKind::Return(exprs) => {
                let base = self.depth;
                let multi = self.expr_list(exprs)?;
                let n = (self.depth - base) as u16;
                self.emit(Op::Return { n, multi });
                self.depth = base;
            }
            StmtKind::Break => {
                let Some(ctx) = self.loops.last() else {
                    return self.syntax_error("break outside a loop");
                };
                let pops = self.depth - ctx.break_depth;
                self.emit_pop(pops);
                let jump = self.emit(Op::Jump(0));
                self.loops.last_mut().expect("loop").break_jumps.push(jump);
            }
            StmtKind::Continue => {
                let Some(ctx) = self.loops.last() else {
                    return self.syntax_error("continue outside a loop");
                };
                let (pops, is_repeat, depth) = (self.depth - ctx.continue_depth, ctx.is_repeat, ctx.continue_depth);
                self.emit_pop(pops);
                if is_repeat {
                    let pad = self.emit(Op::Nils(0));
                    self.loops.last_mut().expect("loop").continue_pads.push((pad, depth));
                }
                let jump = self.emit(Op::Jump(0));
                self.loops.last_mut().expect("loop").continue_jumps.push(jump);
            }
            StmtKind::Goto(name) => {
                let found = self
                    .blocks
                    .iter()
                    .rev()
                    .find_map(|b| b.labels.iter().find(|l| &l.name == name).map(|l| (l.pc, l.depth)));
                match found {
                    Some((pc, depth)) => {
                        self.emit_pop(self.depth - depth);
                        self.emit(Op::Jump(pc));
                    }
                    None => {
                        let pop_pc = self.emit(Op::Pop(0));
                        let jump_pc = self.emit(Op::Jump(0));
                        self.pending.push(PendingGoto {
                            name: name.clone(),
                            pop_pc,
                            jump_pc,
                            depth: self.depth,
                            line: stmt.line,
                        });
                    }
                }
            }
            StmtKind::Label(name) => {
                self.check_duplicate_label(name)?;
                let (pc, depth) = (self.pc(), self.depth);
                let block = self.blocks.last_mut().expect("block");
                block.labels.push(Label { name: name.clone(), pc, depth });
                let first = block.first_pending;
                self.resolve_pending(first, |n| (n == name).then_some((pc, depth)))?;
            }
        }
        Ok(())
    }

    fn open_loop(&mut self, break_depth: u32, continue_depth: u32) {
        self.loops.push(LoopCtx {
            break_depth,
            continue_depth,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_repeat: false,
            continue_pads: Vec::new(),
        });
    }

    /// Ends the innermost loop: `continue` goes to `continue_pc`, `break` to here.
    fn close_loop(&mut self, continue_pc: u32) {
        let ctx = self.loops.pop().expect("loop");
        for jump in ctx.continue_jumps {
            self.patch_jump(jump, continue_pc);
        }
        for jump in ctx.break_jumps {
            self.patch_here(jump);
        }
    }

    fn repeat_stmt(&mut self, body: &[Stmt], cond: &Expr) -> Result<()> {
        let start_depth = self.depth;
        let top = self.pc();
        self.open_loop(start_depth, start_depth);
        self.loops.last_mut().expect("loop").is_repeat = true;
        // The body is compiled inline (not through `block`) because the
        // condition is evaluated inside the body's scope.
        self.blocks.push(BlockCtx {
            labels: Vec::new(),
            end_labels: Vec::new(),
            first_pending: self.pending.len(),
        });
        for stmt in body {
            self.statement(stmt)?;
        }
        self.blocks.pop();
        let cond_pc = self.pc();
        let body_depth = self.depth;
        self.line = cond.line;
        self.expr(cond)?;
        let again = self.emit(Op::JumpIfFalse(0));
        self.depth -= 1;
        self.emit_pop(body_depth - start_depth);
        let exit = self.emit(Op::Jump(0));
        self.patch_here(again);
        self.emit_pop(body_depth - start_depth);
        self.emit(Op::Jump(top));
        self.depth = start_depth;
        let ctx = self.loops.last_mut().expect("loop");
        for (pad, depth) in std::mem::take(&mut ctx.continue_pads) {
            self.code[pad] = Op::Nils(body_depth - depth);
        }
        self.close_loop(cond_pc);
        self.patch_here(exit);
        Ok(())
    }

    fn assign(&mut self, targets: &[Expr], exprs: &[Expr]) -> Result<()> {
        if targets.len() == 1 && exprs.len() == 1 {
            let target = &targets[0];
            match &target.kind {
                ExprKind::Index(object, key) => {
                    self.expr(object)?;
                    if let ExprKind::Str(name) = &key.kind {
                        let k = self.str_const(name);
                        self.expr(&exprs[0])?;
                        self.line = target.line;
                        self.emit(Op::SetField(k));
                        self.depth -= 2;
                    } else {
                        self.expr(key)?;
                        self.expr(&exprs[0])?;
                        self.line = target.line;
                        self.emit(Op::SetIndex);
                        self.depth -= 3;
                    }
                }
                _ => {
                    self.expr(&exprs[0])?;
                    self.store(target)?;
                }
            }
            return Ok(());
        }
        let base = self.depth;
        let mut positions = Vec::with_capacity(targets.len());
        for target in targets {
            if let ExprKind::Index(object, key) = &target.kind {
                positions.push(self.depth);
                self.expr(object)?;
                self.expr(key)?;
            } else {
                positions.push(u32::MAX);
            }
        }
        self.exprs_adjusted(exprs, targets.len() as u32)?;
        for (target, &position) in targets.iter().zip(&positions).rev() {
            if position != u32::MAX {
                self.line = target.line;
                self.emit(Op::SetIndexAt(position));
                self.depth -= 1;
            } else {
                self.store(target)?;
            }
        }
        self.emit_pop(self.depth - base);
        self.depth = base;
        Ok(())
    }

    /// Pops the top of the stack into a local, upvalue or global.
    fn store(&mut self, target: &Expr) -> Result<()> {
        let op = match &target.kind {
            ExprKind::Local(id) => {
                let slot = self.slots[*id as usize];
                if self.is_captured(*id) { Op::SetCell(slot) } else { Op::SetLocal(slot) }
            }
            ExprKind::Upval(i) => Op::SetUpval(*i),
            ExprKind::Global(name) => Op::SetGlobal(self.str_const(name.as_bytes())),
            _ => return self.syntax_error("cannot assign to this expression"),
        };
        self.emit(op);
        self.depth -= 1;
        Ok(())
    }

    fn compound_assign(&mut self, target: &Expr, op: BinOp, value: &Expr) -> Result<()> {
        if let ExprKind::Index(object, key) = &target.kind {
            self.expr(object)?;
            self.expr(key)?;
            self.emit(Op::Dup2);
            self.emit(Op::GetIndex);
            self.depth += 1;
            self.expr(value)?;
            self.line = target.line;
            self.binary_op(op);
            self.emit(Op::SetIndex);
            self.depth -= 3;
        } else {
            self.expr(target)?;
            self.expr(value)?;
            self.line = target.line;
            self.binary_op(op);
            self.store(target)?;
        }
        Ok(())
    }

    // -------------------------------------------------------- expressions

    /// Emits the operator for two operands already on the stack.
    fn binary_op(&mut self, op: BinOp) {
        let instr = match op {
            BinOp::Add => Op::Add,
            BinOp::Sub => Op::Sub,
            BinOp::Mul => Op::Mul,
            BinOp::Div => Op::Div,
            BinOp::IDiv => Op::IDiv,
            BinOp::Mod => Op::Mod,
            BinOp::Pow => Op::Pow,
            BinOp::Concat => Op::Concat(2),
            BinOp::Eq => Op::Eq,
            BinOp::Ne => Op::Ne,
            BinOp::Lt => Op::Lt,
            BinOp::Le => Op::Le,
            BinOp::Gt => Op::Gt,
            BinOp::Ge => Op::Ge,
            BinOp::BAnd => Op::BAnd,
            BinOp::BOr => Op::BOr,
            BinOp::BXor => Op::BXor,
            BinOp::Shl => Op::Shl,
            BinOp::Shr => Op::Shr,
        };
        self.emit(instr);
        self.depth -= 1;
    }

    /// Evaluates a list; all but a trailing multi-valued expression push one
    /// value each. Returns whether the last expression left a variable number
    /// of values (not included in `self.depth`).
    fn expr_list(&mut self, exprs: &[Expr]) -> Result<bool> {
        for (i, e) in exprs.iter().enumerate() {
            if i + 1 == exprs.len() && e.kind.is_multi() {
                self.expr_multi(e)?;
                return Ok(true);
            }
            self.expr(e)?;
        }
        Ok(false)
    }

    /// Evaluates a list to exactly `want` values.
    fn exprs_adjusted(&mut self, exprs: &[Expr], want: u32) -> Result<()> {
        let base = self.depth;
        for (i, e) in exprs.iter().enumerate() {
            let have = i as u32;
            let last = i + 1 == exprs.len();
            if last && e.kind.is_multi() && want > have {
                let need = want - have;
                if need >= u32::from(MULTI) {
                    return self.syntax_error("too many values in assignment");
                }
                match &e.kind {
                    ExprKind::Vararg => {
                        self.line = e.line;
                        self.emit(Op::Vararg(need as u8));
                        self.depth += need;
                    }
                    _ => self.call(e, need as u8)?,
                }
            } else {
                self.expr(e)?;
                if have >= want {
                    // Extra expressions are evaluated for side effects only.
                    self.emit(Op::Pop(1));
                    self.depth -= 1;
                }
            }
        }
        let have = self.depth - base;
        if have < want {
            let missing = want - have;
            self.emit(if missing == 1 { Op::Nil } else { Op::Nils(missing) });
            self.depth += missing;
        }
        Ok(())
    }

    fn expr_multi(&mut self, e: &Expr) -> Result<()> {
        match &e.kind {
            ExprKind::Vararg => {
                self.line = e.line;
                self.emit(Op::Vararg(MULTI));
                Ok(())
            }
            _ => self.call(e, MULTI),
        }
    }

    /// Compiles a call expression leaving `nres` results (or all, for `MULTI`).
    fn call(&mut self, e: &Expr, nres: u8) -> Result<()> {
        let base = self.depth;
        let (args, extra, name) = match &e.kind {
            ExprKind::Call(callee, args) => {
                self.expr(callee)?;
                (args, 0u16, describe_callee(callee, self.body))
            }
            ExprKind::Method(object, name, args) => {
                self.expr(object)?;
                let k = self.str_const(name.as_bytes());
                self.line = e.line;
                self.emit(Op::Method(k));
                self.depth += 1;
                (args, 1u16, Some(format!("method '{name}'")))
            }
            _ => unreachable!("call() requires a call expression"),
        };
        if args.len() > 240 {
            return self.syntax_error("too many arguments in function call");
        }
        let multi = self.expr_list(args)?;
        let nargs = (self.depth - base - 1) as u16;
        debug_assert!(nargs >= extra);
        self.line = e.line;
        let pc = self.emit(Op::Call { nargs, nres, multi });
        if let Some(name) = name {
            self.call_names.push((pc as u32, name));
        }
        self.depth = base + if nres == MULTI { 0 } else { u32::from(nres) };
        Ok(())
    }

    /// Compiles an expression to exactly one value on the stack.
    fn expr(&mut self, e: &Expr) -> Result<()> {
        self.line = e.line;
        match &e.kind {
            ExprKind::Nil => {
                self.emit(Op::Nil);
            }
            ExprKind::True => {
                self.emit(Op::True);
            }
            ExprKind::False => {
                self.emit(Op::False);
            }
            ExprKind::Int(i) => {
                let k = self.constant(ConstKey::Int(*i));
                self.emit(Op::Const(k));
            }
            ExprKind::Num(n) => {
                let k = self.constant(ConstKey::Num(n.to_bits()));
                self.emit(Op::Const(k));
            }
            ExprKind::Str(s) => {
                let k = self.str_const(s);
                self.emit(Op::Const(k));
            }
            ExprKind::Vararg => {
                self.emit(Op::Vararg(1));
            }
            ExprKind::Local(id) => {
                let slot = self.slots[*id as usize];
                debug_assert_ne!(slot, u32::MAX, "local used before declaration");
                self.emit(if self.is_captured(*id) { Op::GetCell(slot) } else { Op::GetLocal(slot) });
            }
            ExprKind::Upval(i) => {
                self.emit(Op::GetUpval(*i));
            }
            ExprKind::Global(name) => {
                let k = self.str_const(name.as_bytes());
                self.emit(Op::GetGlobal(k));
            }
            ExprKind::Index(object, key) => {
                self.expr(object)?;
                if let ExprKind::Str(name) = &key.kind {
                    let k = self.str_const(name);
                    self.line = e.line;
                    self.emit(Op::GetField(k));
                } else {
                    self.expr(key)?;
                    self.line = e.line;
                    self.emit(Op::GetIndex);
                    self.depth -= 1;
                }
                return Ok(());
            }
            ExprKind::Call(..) | ExprKind::Method(..) => return self.call(e, 1),
            ExprKind::Function(body) => {
                self.closure(body)?;
                return Ok(());
            }
            ExprKind::Binary(op, lhs, rhs) => {
                if *op == BinOp::Concat {
                    // Flatten right-associative chains into one instruction.
                    let mut operands = vec![lhs.as_ref()];
                    let mut rest = rhs.as_ref();
                    while let ExprKind::Binary(BinOp::Concat, l, r) = &rest.kind {
                        operands.push(l.as_ref());
                        rest = r.as_ref();
                    }
                    operands.push(rest);
                    for operand in &operands {
                        self.expr(operand)?;
                    }
                    self.line = e.line;
                    self.emit(Op::Concat(operands.len() as u32));
                    self.depth -= operands.len() as u32 - 1;
                } else {
                    self.expr(lhs)?;
                    self.expr(rhs)?;
                    self.line = e.line;
                    self.binary_op(*op);
                }
                return Ok(());
            }
            ExprKind::Unary(op, operand) => {
                self.expr(operand)?;
                self.line = e.line;
                self.emit(match op {
                    UnOp::Neg => Op::Neg,
                    UnOp::Not => Op::Not,
                    UnOp::Len => Op::Len,
                    UnOp::BNot => Op::BNot,
                });
                return Ok(());
            }
            ExprKind::And(lhs, rhs) | ExprKind::Or(lhs, rhs) => {
                self.expr(lhs)?;
                let jump = self.emit(if matches!(e.kind, ExprKind::And(..)) { Op::AndJump(0) } else { Op::OrJump(0) });
                self.depth -= 1;
                self.expr(rhs)?;
                self.patch_here(jump);
                return Ok(());
            }
            ExprKind::Table(items) => return self.table(items),
            ExprKind::Paren(inner) => return self.expr(inner),
        }
        self.depth += 1;
        Ok(())
    }

    fn table(&mut self, items: &[TableItem]) -> Result<()> {
        let positional = items.iter().filter(|i| matches!(i, TableItem::Positional(_))).count();
        let arr = positional.min(u16::MAX as usize) as u16;
        let hash = (items.len() - positional).min(u16::MAX as usize) as u16;
        self.emit(Op::NewTable { arr, hash });
        self.depth += 1;
        let mut pending: u16 = 0;
        for (i, item) in items.iter().enumerate() {
            match item {
                TableItem::Positional(value) => {
                    if i + 1 == items.len() && value.kind.is_multi() {
                        self.expr_multi(value)?;
                        self.emit(Op::SetList { n: pending, multi: true });
                        self.depth -= u32::from(pending);
                        pending = 0;
                    } else {
                        self.expr(value)?;
                        pending += 1;
                        if pending == LIST_BATCH {
                            self.emit(Op::SetList { n: pending, multi: false });
                            self.depth -= u32::from(pending);
                            pending = 0;
                        }
                    }
                }
                TableItem::Field(key, value) => {
                    if pending > 0 {
                        self.emit(Op::SetList { n: pending, multi: false });
                        self.depth -= u32::from(pending);
                        pending = 0;
                    }
                    if let ExprKind::Str(name) = &key.kind {
                        let k = self.str_const(name);
                        self.expr(value)?;
                        self.emit(Op::InitField(k));
                        self.depth -= 1;
                    } else {
                        self.expr(key)?;
                        self.expr(value)?;
                        self.emit(Op::InitIndex);
                        self.depth -= 2;
                    }
                }
            }
        }
        if pending > 0 {
            self.emit(Op::SetList { n: pending, multi: false });
            self.depth -= u32::from(pending);
        }
        Ok(())
    }

    fn closure(&mut self, body: &FuncBody) -> Result<()> {
        let proto = FuncGen::compile(body, self.chunk, Some(&self.slots))?;
        self.protos.push(Rc::new(proto));
        self.emit(Op::Closure(self.protos.len() as u32 - 1));
        self.depth += 1;
        Ok(())
    }
}

fn describe_callee(callee: &Expr, body: &FuncBody) -> Option<String> {
    match &callee.kind {
        ExprKind::Global(name) => Some(format!("global '{name}'")),
        ExprKind::Local(id) => Some(format!("local '{}'", body.locals[*id as usize].name)),
        ExprKind::Upval(i) => Some(format!("upvalue '{}'", body.upvals[*i as usize].name)),
        ExprKind::Index(_, key) => match &key.kind {
            ExprKind::Str(s) => Some(format!("field '{}'", String::from_utf8_lossy(s))),
            _ => None,
        },
        _ => None,
    }
}
