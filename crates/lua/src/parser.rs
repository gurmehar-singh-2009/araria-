//! Recursive-descent parser producing a scope-resolved [`crate::ast`].

use crate::ast::*;
use crate::error::{Error, Result};
use crate::lexer::{Lexer, Tok, Token};

const MAX_DEPTH: u32 = 180;
const MAX_LOCALS: usize = 220;

struct FuncState {
    locals: Vec<LocalInfo>,
    /// Locals currently in scope, innermost last.
    active: Vec<(String, LocalId)>,
    block_starts: Vec<usize>,
    upvals: Vec<UpvalDesc>,
    is_vararg: bool,
    loop_depth: u32,
}

enum Resolved {
    Local(LocalId),
    Upval(u32),
    Global,
}

pub struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    chunk: &'a str,
    funcs: Vec<FuncState>,
    depth: u32,
}

/// Parses a chunk into its main function body.
pub fn parse(source: &str, chunk: &str) -> Result<FuncBody> {
    let tokens = Lexer::new(source, chunk).tokenize()?;
    let mut parser = Parser { tokens, pos: 0, chunk, funcs: Vec::new(), depth: 0 };
    parser.open_function(true);
    let body = parser.block()?;
    if *parser.peek() != Tok::Eof {
        return parser.unexpected("<eof>");
    }
    let fs = parser.funcs.pop().expect("main function state");
    Ok(FuncBody {
        name: "main chunk".into(),
        line: 0,
        params: Vec::new(),
        is_vararg: true,
        body,
        locals: fs.locals,
        upvals: fs.upvals,
    })
}

impl<'a> Parser<'a> {
    // ------------------------------------------------------------ tokens

    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_ahead(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.tokens.len() - 1);
        &self.tokens[i].tok
    }

    fn line(&self) -> u32 {
        self.tokens[self.pos].line
    }

    fn advance(&mut self) -> Tok {
        let tok = self.tokens[self.pos].tok.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn check(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn accept(&mut self, tok: &Tok) -> bool {
        if self.check(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn error<T>(&self, msg: impl Into<String>) -> Result<T> {
        Err(Error::syntax(self.chunk, self.line(), msg))
    }

    fn unexpected<T>(&self, expected: &str) -> Result<T> {
        self.error(format!("'{}' expected near '{}'", expected, self.peek().describe()))
    }

    fn expect(&mut self, tok: &Tok) -> Result<()> {
        if self.accept(tok) {
            Ok(())
        } else {
            self.unexpected(&tok.describe())
        }
    }

    /// Like `expect`, but mentions the opening token's line when it differs.
    fn expect_match(&mut self, what: &Tok, opener: &Tok, line: u32) -> Result<()> {
        if self.accept(what) {
            return Ok(());
        }
        if line == self.line() {
            self.unexpected(&what.describe())
        } else {
            self.error(format!(
                "'{}' expected (to close '{}' at line {}) near '{}'",
                what.describe(),
                opener.describe(),
                line,
                self.peek().describe()
            ))
        }
    }

    fn expect_name(&mut self) -> Result<String> {
        match self.peek() {
            Tok::Name(_) => match self.advance() {
                Tok::Name(n) => Ok(n),
                _ => unreachable!(),
            },
            _ => self.unexpected("<name>"),
        }
    }

    // ------------------------------------------------------------- scopes

    fn fs(&mut self) -> &mut FuncState {
        self.funcs.last_mut().expect("function state")
    }

    fn open_function(&mut self, is_vararg: bool) {
        self.funcs.push(FuncState {
            locals: Vec::new(),
            active: Vec::new(),
            block_starts: Vec::new(),
            upvals: Vec::new(),
            is_vararg,
            loop_depth: 0,
        });
    }

    fn open_block(&mut self) {
        let n = self.fs().active.len();
        self.fs().block_starts.push(n);
    }

    fn close_block(&mut self) {
        let n = self.fs().block_starts.pop().expect("open block");
        self.fs().active.truncate(n);
    }

    /// Creates a local without bringing it into scope yet.
    fn new_local(&mut self, name: &str) -> Result<LocalId> {
        if self.fs().active.len() >= MAX_LOCALS {
            return self.error("too many local variables in function");
        }
        let fs = self.fs();
        fs.locals.push(LocalInfo { name: name.to_string(), captured: false });
        Ok(fs.locals.len() as LocalId - 1)
    }

    fn activate(&mut self, id: LocalId) {
        let name = self.fs().locals[id as usize].name.clone();
        self.fs().active.push((name, id));
    }

    fn declare(&mut self, name: &str) -> Result<LocalId> {
        let id = self.new_local(name)?;
        self.activate(id);
        Ok(id)
    }

    fn resolve(&mut self, level: usize, name: &str) -> Resolved {
        if let Some((_, id)) = self.funcs[level].active.iter().rev().find(|(n, _)| n == name) {
            return Resolved::Local(*id);
        }
        if level == 0 {
            return Resolved::Global;
        }
        if let Some(i) = self.funcs[level].upvals.iter().position(|u| u.name == name) {
            return Resolved::Upval(i as u32);
        }
        let desc = match self.resolve(level - 1, name) {
            Resolved::Local(id) => {
                self.funcs[level - 1].locals[id as usize].captured = true;
                UpvalDesc { name: name.to_string(), from_parent_local: true, index: id }
            }
            Resolved::Upval(i) => UpvalDesc { name: name.to_string(), from_parent_local: false, index: i },
            Resolved::Global => return Resolved::Global,
        };
        self.funcs[level].upvals.push(desc);
        Resolved::Upval(self.funcs[level].upvals.len() as u32 - 1)
    }

    fn name_expr(&mut self, name: String, line: u32) -> Expr {
        let level = self.funcs.len() - 1;
        let kind = match self.resolve(level, &name) {
            Resolved::Local(id) => ExprKind::Local(id),
            Resolved::Upval(i) => ExprKind::Upval(i),
            Resolved::Global => ExprKind::Global(name),
        };
        Expr { line, kind }
    }

    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.error("chunk has too many syntax levels");
        }
        Ok(())
    }

    // --------------------------------------------------------- statements

    fn block_ends(&self) -> bool {
        matches!(self.peek(), Tok::Eof | Tok::End | Tok::Else | Tok::ElseIf | Tok::Until)
    }

    /// Parses statements up to a block terminator. Scope is managed by callers.
    fn block(&mut self) -> Result<Block> {
        let mut stmts = Vec::new();
        while !self.block_ends() {
            if self.check(&Tok::Return) {
                stmts.push(self.return_stmt()?);
                break;
            }
            if let Some(stmt) = self.statement()? {
                stmts.push(stmt);
            }
        }
        Ok(stmts)
    }

    fn scoped_block(&mut self) -> Result<Block> {
        self.open_block();
        let block = self.block();
        self.close_block();
        block
    }

    fn return_stmt(&mut self) -> Result<Stmt> {
        let line = self.line();
        self.advance();
        let exprs = if self.block_ends() || self.check(&Tok::Semi) { Vec::new() } else { self.expr_list()? };
        self.accept(&Tok::Semi);
        if !self.block_ends() {
            return self.unexpected("<eof>");
        }
        Ok(Stmt { line, kind: StmtKind::Return(exprs) })
    }

    fn is_continue(&self) -> bool {
        matches!(self.peek(), Tok::Name(n) if n == "continue")
            && !matches!(
                self.peek_ahead(1),
                Tok::Assign
                    | Tok::Comma
                    | Tok::Dot
                    | Tok::LBracket
                    | Tok::LParen
                    | Tok::Colon
                    | Tok::Str(_)
                    | Tok::LBrace
                    | Tok::Compound(_)
            )
    }

    fn statement(&mut self) -> Result<Option<Stmt>> {
        self.enter()?;
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }

    fn statement_inner(&mut self) -> Result<Option<Stmt>> {
        let line = self.line();
        let kind = match self.peek() {
            Tok::Semi => {
                self.advance();
                return Ok(None);
            }
            Tok::If => self.if_stmt()?,
            Tok::While => {
                self.advance();
                let cond = self.expr()?;
                self.expect(&Tok::Do)?;
                let body = self.loop_body()?;
                self.expect_match(&Tok::End, &Tok::While, line)?;
                StmtKind::While(cond, body)
            }
            Tok::Do => {
                self.advance();
                let body = self.scoped_block()?;
                self.expect_match(&Tok::End, &Tok::Do, line)?;
                StmtKind::Do(body)
            }
            Tok::For => self.for_stmt()?,
            Tok::Repeat => {
                self.advance();
                // The condition can see the body's locals, so the block stays
                // open until it has been parsed.
                self.open_block();
                self.fs().loop_depth += 1;
                let body = self.block();
                self.fs().loop_depth -= 1;
                let body = body?;
                self.expect_match(&Tok::Until, &Tok::Repeat, line)?;
                let cond = self.expr();
                self.close_block();
                StmtKind::Repeat(body, cond?)
            }
            Tok::Function => self.function_stmt()?,
            Tok::Local => {
                self.advance();
                if self.accept(&Tok::Function) {
                    let name = self.expect_name()?;
                    let id = self.declare(&name)?;
                    let body = self.func_body(name, false, line)?;
                    StmtKind::LocalFunction(id, Box::new(body))
                } else {
                    self.local_stmt()?
                }
            }
            Tok::DColon => {
                self.advance();
                let name = self.expect_name()?;
                self.expect(&Tok::DColon)?;
                StmtKind::Label(name)
            }
            Tok::Break => {
                self.advance();
                if self.fs().loop_depth == 0 {
                    return self.error("break outside a loop");
                }
                StmtKind::Break
            }
            Tok::Goto => {
                self.advance();
                StmtKind::Goto(self.expect_name()?)
            }
            _ if self.is_continue() => {
                self.advance();
                if self.fs().loop_depth == 0 {
                    return self.error("continue outside a loop");
                }
                StmtKind::Continue
            }
            _ => self.expr_stmt()?,
        };
        Ok(Some(Stmt { line, kind }))
    }

    fn loop_body(&mut self) -> Result<Block> {
        self.fs().loop_depth += 1;
        let body = self.scoped_block();
        self.fs().loop_depth -= 1;
        body
    }

    fn if_stmt(&mut self) -> Result<StmtKind> {
        let line = self.line();
        self.advance();
        let mut arms = Vec::new();
        let cond = self.expr()?;
        self.expect(&Tok::Then)?;
        arms.push((cond, self.scoped_block()?));
        let mut otherwise = None;
        loop {
            if self.accept(&Tok::ElseIf) {
                let cond = self.expr()?;
                self.expect(&Tok::Then)?;
                arms.push((cond, self.scoped_block()?));
            } else if self.accept(&Tok::Else) {
                otherwise = Some(self.scoped_block()?);
                self.expect_match(&Tok::End, &Tok::If, line)?;
                break;
            } else {
                self.expect_match(&Tok::End, &Tok::If, line)?;
                break;
            }
        }
        Ok(StmtKind::If(arms, otherwise))
    }

    fn for_stmt(&mut self) -> Result<StmtKind> {
        let line = self.line();
        self.advance();
        let first = self.expect_name()?;
        if self.accept(&Tok::Assign) {
            let start = self.expr()?;
            self.expect(&Tok::Comma)?;
            let limit = self.expr()?;
            let step = if self.accept(&Tok::Comma) { Some(self.expr()?) } else { None };
            self.expect(&Tok::Do)?;
            self.open_block();
            let var = self.declare(&first)?;
            let body = self.loop_body();
            self.close_block();
            self.expect_match(&Tok::End, &Tok::For, line)?;
            Ok(StmtKind::NumericFor { var, start, limit, step, body: body? })
        } else {
            let mut names = vec![first];
            while self.accept(&Tok::Comma) {
                names.push(self.expect_name()?);
            }
            self.expect(&Tok::In)?;
            let exprs = self.expr_list()?;
            self.expect(&Tok::Do)?;
            self.open_block();
            let mut vars = Vec::new();
            for name in &names {
                vars.push(self.declare(name)?);
            }
            let body = self.loop_body();
            self.close_block();
            self.expect_match(&Tok::End, &Tok::For, line)?;
            Ok(StmtKind::GenericFor { vars, exprs, body: body? })
        }
    }

    fn function_stmt(&mut self) -> Result<StmtKind> {
        let line = self.line();
        self.advance();
        let first = self.expect_name()?;
        let mut full_name = first.clone();
        let mut target = self.name_expr(first, line);
        let mut is_method = false;
        loop {
            if self.accept(&Tok::Dot) {
                let key = self.expect_name()?;
                full_name.push('.');
                full_name.push_str(&key);
                target = index_name(target, key, line);
            } else if self.accept(&Tok::Colon) {
                let key = self.expect_name()?;
                full_name.push(':');
                full_name.push_str(&key);
                target = index_name(target, key, line);
                is_method = true;
                break;
            } else {
                break;
            }
        }
        let body = self.func_body(full_name, is_method, line)?;
        let func = Expr { line, kind: ExprKind::Function(Box::new(body)) };
        Ok(StmtKind::Assign(vec![target], vec![func]))
    }

    fn local_stmt(&mut self) -> Result<StmtKind> {
        let mut names = Vec::new();
        loop {
            names.push(self.expect_name()?);
            if self.accept(&Tok::Lt) {
                let attrib = self.expect_name()?;
                if attrib != "const" {
                    return self.error(format!("unsupported attribute '{attrib}'"));
                }
                self.expect(&Tok::Gt)?;
            }
            if !self.accept(&Tok::Comma) {
                break;
            }
        }
        let mut exprs = if self.accept(&Tok::Assign) { self.expr_list()? } else { Vec::new() };
        // `local f = function ... end` gets the variable's name for tracebacks.
        if names.len() == 1 && exprs.len() == 1 {
            name_function(&mut exprs[0], &names[0]);
        }
        let mut ids = Vec::new();
        for name in &names {
            ids.push(self.declare(name)?);
        }
        Ok(StmtKind::Local(ids, exprs))
    }

    fn expr_stmt(&mut self) -> Result<StmtKind> {
        let first = self.suffixed_expr()?;
        if self.check(&Tok::Assign) || self.check(&Tok::Comma) {
            let mut targets = vec![first];
            while self.accept(&Tok::Comma) {
                targets.push(self.suffixed_expr()?);
            }
            self.expect(&Tok::Assign)?;
            let mut exprs = self.expr_list()?;
            for target in &targets {
                self.check_assignable(target)?;
            }
            if targets.len() == 1 && exprs.len() == 1 {
                if let Some(name) = target_name(&targets[0], self.funcs.last().expect("fs")) {
                    name_function(&mut exprs[0], &name);
                }
            }
            return Ok(StmtKind::Assign(targets, exprs));
        }
        if let Tok::Compound(op) = self.peek().clone() {
            self.advance();
            self.check_assignable(&first)?;
            let op = match *op {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::DSlash => BinOp::IDiv,
                Tok::Percent => BinOp::Mod,
                Tok::Caret => BinOp::Pow,
                _ => BinOp::Concat,
            };
            let value = self.expr()?;
            return Ok(StmtKind::CompoundAssign(first, op, value));
        }
        match first.kind {
            ExprKind::Call(..) | ExprKind::Method(..) => Ok(StmtKind::Call(first)),
            _ => self.error(format!("syntax error near '{}'", self.peek().describe())),
        }
    }

    fn check_assignable(&self, e: &Expr) -> Result<()> {
        match e.kind {
            ExprKind::Local(_) | ExprKind::Upval(_) | ExprKind::Global(_) | ExprKind::Index(..) => Ok(()),
            _ => self.error("cannot assign to this expression"),
        }
    }

    // -------------------------------------------------------- expressions

    fn expr_list(&mut self) -> Result<Vec<Expr>> {
        let mut list = vec![self.expr()?];
        while self.accept(&Tok::Comma) {
            list.push(self.expr()?);
        }
        Ok(list)
    }

    pub fn expr(&mut self) -> Result<Expr> {
        self.sub_expr(0)
    }

    fn sub_expr(&mut self, limit: u8) -> Result<Expr> {
        self.enter()?;
        let line = self.line();
        let unary = match self.peek() {
            Tok::Not => Some(UnOp::Not),
            Tok::Minus => Some(UnOp::Neg),
            Tok::Hash => Some(UnOp::Len),
            Tok::Tilde => Some(UnOp::BNot),
            _ => None,
        };
        let mut left = if let Some(op) = unary {
            self.advance();
            let operand = self.sub_expr(12)?;
            fold_unary(op, operand, line)
        } else {
            self.simple_expr()?
        };
        loop {
            let Some((op, lp, rp)) = binary_op(self.peek()) else { break };
            if lp <= limit {
                break;
            }
            let line = self.line();
            self.advance();
            let right = self.sub_expr(rp)?;
            let kind = match op {
                BinaryKind::And => ExprKind::And(Box::new(left), Box::new(right)),
                BinaryKind::Or => ExprKind::Or(Box::new(left), Box::new(right)),
                BinaryKind::Op(op) => ExprKind::Binary(op, Box::new(left), Box::new(right)),
            };
            left = Expr { line, kind };
        }
        self.depth -= 1;
        Ok(left)
    }

    fn simple_expr(&mut self) -> Result<Expr> {
        let line = self.line();
        let kind = match self.peek() {
            Tok::Nil => ExprKind::Nil,
            Tok::True => ExprKind::True,
            Tok::False => ExprKind::False,
            Tok::Int(i) => ExprKind::Int(*i),
            Tok::Num(n) => ExprKind::Num(*n),
            Tok::Str(_) => match self.advance() {
                Tok::Str(s) => return Ok(Expr { line, kind: ExprKind::Str(s) }),
                _ => unreachable!(),
            },
            Tok::Ellipsis => {
                if !self.fs().is_vararg {
                    return self.error("cannot use '...' outside a vararg function");
                }
                ExprKind::Vararg
            }
            Tok::LBrace => return self.table_constructor(),
            Tok::Function => {
                self.advance();
                let body = self.func_body("anonymous function".into(), false, line)?;
                return Ok(Expr { line, kind: ExprKind::Function(Box::new(body)) });
            }
            _ => return self.suffixed_expr(),
        };
        self.advance();
        Ok(Expr { line, kind })
    }

    fn primary_expr(&mut self) -> Result<Expr> {
        let line = self.line();
        match self.peek() {
            Tok::Name(_) => {
                let name = self.expect_name()?;
                Ok(self.name_expr(name, line))
            }
            Tok::LParen => {
                self.advance();
                let inner = self.expr()?;
                self.expect_match(&Tok::RParen, &Tok::LParen, line)?;
                Ok(Expr { line, kind: ExprKind::Paren(Box::new(inner)) })
            }
            _ => self.error(format!("unexpected symbol near '{}'", self.peek().describe())),
        }
    }

    fn suffixed_expr(&mut self) -> Result<Expr> {
        let mut e = self.primary_expr()?;
        loop {
            let line = self.line();
            match self.peek() {
                Tok::Dot => {
                    self.advance();
                    let key = self.expect_name()?;
                    e = index_name(e, key, line);
                }
                Tok::LBracket => {
                    self.advance();
                    let key = self.expr()?;
                    self.expect(&Tok::RBracket)?;
                    e = Expr { line, kind: ExprKind::Index(Box::new(e), Box::new(key)) };
                }
                Tok::Colon => {
                    self.advance();
                    let name = self.expect_name()?;
                    let args = self.call_args()?;
                    e = Expr { line, kind: ExprKind::Method(Box::new(e), name, args) };
                }
                Tok::LParen | Tok::Str(_) | Tok::LBrace => {
                    let args = self.call_args()?;
                    e = Expr { line, kind: ExprKind::Call(Box::new(e), args) };
                }
                _ => return Ok(e),
            }
        }
    }

    fn call_args(&mut self) -> Result<Vec<Expr>> {
        let line = self.line();
        match self.peek() {
            Tok::Str(_) => match self.advance() {
                Tok::Str(s) => Ok(vec![Expr { line, kind: ExprKind::Str(s) }]),
                _ => unreachable!(),
            },
            Tok::LBrace => Ok(vec![self.table_constructor()?]),
            Tok::LParen => {
                self.advance();
                if self.accept(&Tok::RParen) {
                    return Ok(Vec::new());
                }
                let args = self.expr_list()?;
                self.expect_match(&Tok::RParen, &Tok::LParen, line)?;
                Ok(args)
            }
            _ => self.error("function arguments expected"),
        }
    }

    fn table_constructor(&mut self) -> Result<Expr> {
        let line = self.line();
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        while !self.check(&Tok::RBrace) {
            let item_line = self.line();
            match self.peek() {
                Tok::Name(_) if *self.peek_ahead(1) == Tok::Assign => {
                    let name = self.expect_name()?;
                    self.advance();
                    let mut value = self.expr()?;
                    name_function(&mut value, &name);
                    let key = Expr { line: item_line, kind: ExprKind::Str(name.into_bytes()) };
                    items.push(TableItem::Field(key, value));
                }
                Tok::LBracket => {
                    self.advance();
                    let key = self.expr()?;
                    self.expect(&Tok::RBracket)?;
                    self.expect(&Tok::Assign)?;
                    items.push(TableItem::Field(key, self.expr()?));
                }
                _ => items.push(TableItem::Positional(self.expr()?)),
            }
            if !self.accept(&Tok::Comma) && !self.accept(&Tok::Semi) {
                break;
            }
        }
        self.expect_match(&Tok::RBrace, &Tok::LBrace, line)?;
        Ok(Expr { line, kind: ExprKind::Table(items) })
    }

    fn func_body(&mut self, name: String, is_method: bool, line: u32) -> Result<FuncBody> {
        self.open_function(false);
        self.open_block();
        let mut params = Vec::new();
        if is_method {
            params.push(self.declare("self")?);
        }
        self.expect(&Tok::LParen)?;
        if !self.check(&Tok::RParen) {
            loop {
                if self.accept(&Tok::Ellipsis) {
                    self.fs().is_vararg = true;
                    break;
                }
                let param = self.expect_name()?;
                params.push(self.declare(&param)?);
                if !self.accept(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(&Tok::RParen)?;
        let body = self.block();
        let fs = self.funcs.pop().expect("function state");
        let body = body?;
        self.expect_match(&Tok::End, &Tok::Function, line)?;
        Ok(FuncBody { name, line, params, is_vararg: fs.is_vararg, body, locals: fs.locals, upvals: fs.upvals })
    }
}

fn index_name(object: Expr, key: String, line: u32) -> Expr {
    let key = Expr { line, kind: ExprKind::Str(key.into_bytes()) };
    Expr { line, kind: ExprKind::Index(Box::new(object), Box::new(key)) }
}

/// Gives an anonymous function expression a name for tracebacks.
fn name_function(e: &mut Expr, name: &str) {
    if let ExprKind::Function(body) = &mut e.kind {
        if body.name == "anonymous function" {
            body.name = name.to_string();
        }
    }
}

fn target_name(e: &Expr, fs: &FuncState) -> Option<String> {
    match &e.kind {
        ExprKind::Global(name) => Some(name.clone()),
        ExprKind::Local(id) => Some(fs.locals[*id as usize].name.clone()),
        ExprKind::Index(_, key) => match &key.kind {
            ExprKind::Str(s) => Some(String::from_utf8_lossy(s).into_owned()),
            _ => None,
        },
        _ => None,
    }
}

enum BinaryKind {
    And,
    Or,
    Op(BinOp),
}

/// Operator with its (left, right) binding priorities.
fn binary_op(tok: &Tok) -> Option<(BinaryKind, u8, u8)> {
    use BinaryKind::Op;
    Some(match tok {
        Tok::Or => (BinaryKind::Or, 1, 1),
        Tok::And => (BinaryKind::And, 2, 2),
        Tok::Lt => (Op(BinOp::Lt), 3, 3),
        Tok::Gt => (Op(BinOp::Gt), 3, 3),
        Tok::Le => (Op(BinOp::Le), 3, 3),
        Tok::Ge => (Op(BinOp::Ge), 3, 3),
        Tok::Ne => (Op(BinOp::Ne), 3, 3),
        Tok::Eq => (Op(BinOp::Eq), 3, 3),
        Tok::Pipe => (Op(BinOp::BOr), 4, 4),
        Tok::Tilde => (Op(BinOp::BXor), 5, 5),
        Tok::Amp => (Op(BinOp::BAnd), 6, 6),
        Tok::Shl => (Op(BinOp::Shl), 7, 7),
        Tok::Shr => (Op(BinOp::Shr), 7, 7),
        Tok::Concat => (Op(BinOp::Concat), 9, 8),
        Tok::Plus => (Op(BinOp::Add), 10, 10),
        Tok::Minus => (Op(BinOp::Sub), 10, 10),
        Tok::Star => (Op(BinOp::Mul), 11, 11),
        Tok::Slash => (Op(BinOp::Div), 11, 11),
        Tok::DSlash => (Op(BinOp::IDiv), 11, 11),
        Tok::Percent => (Op(BinOp::Mod), 11, 11),
        Tok::Caret => (Op(BinOp::Pow), 14, 13),
        _ => return None,
    })
}

/// Folds negation of numeric literals so `-1` is a constant.
fn fold_unary(op: UnOp, operand: Expr, line: u32) -> Expr {
    let kind = match (op, &operand.kind) {
        (UnOp::Neg, ExprKind::Int(i)) => ExprKind::Int(i.wrapping_neg()),
        (UnOp::Neg, ExprKind::Num(n)) => ExprKind::Num(-n),
        _ => ExprKind::Unary(op, Box::new(operand)),
    };
    Expr { line, kind }
}
