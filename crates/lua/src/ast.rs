//! Abstract syntax tree produced by the parser.
//!
//! Variable references are already resolved: every name is a local of the
//! enclosing function, an upvalue, or a global. Locals carry a `captured`
//! flag so the code generator knows which ones must live in heap cells.

/// Index into [`FuncBody::locals`].
pub type LocalId = u32;

#[derive(Debug, Clone)]
pub struct Expr {
    pub line: u32,
    pub kind: ExprKind,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Nil,
    True,
    False,
    Int(i64),
    Num(f64),
    Str(Vec<u8>),
    Vararg,
    Local(LocalId),
    Upval(u32),
    Global(String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Method(Box<Expr>, String, Vec<Expr>),
    Function(Box<FuncBody>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Table(Vec<TableItem>),
    /// A parenthesized expression: truncates multiple results to one.
    Paren(Box<Expr>),
}

impl ExprKind {
    /// Whether the expression can produce a variable number of values.
    pub fn is_multi(&self) -> bool {
        matches!(self, ExprKind::Call(..) | ExprKind::Method(..) | ExprKind::Vararg)
    }
}

#[derive(Debug, Clone)]
pub enum TableItem {
    Positional(Expr),
    Field(Expr, Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BAnd,
    BOr,
    BXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Len,
    BNot,
}

#[derive(Debug, Clone)]
pub struct LocalInfo {
    pub name: String,
    /// Set when an inner function refers to this local.
    pub captured: bool,
}

#[derive(Debug, Clone)]
pub struct UpvalDesc {
    pub name: String,
    /// `true`: captures local `index` of the enclosing function.
    /// `false`: re-exports upvalue `index` of the enclosing function.
    pub from_parent_local: bool,
    pub index: u32,
}

#[derive(Debug, Clone)]
pub struct FuncBody {
    pub name: String,
    pub line: u32,
    pub params: Vec<LocalId>,
    pub is_vararg: bool,
    pub body: Block,
    pub locals: Vec<LocalInfo>,
    pub upvals: Vec<UpvalDesc>,
}

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone)]
pub struct Stmt {
    pub line: u32,
    pub kind: StmtKind,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    /// A function call evaluated for its side effects.
    Call(Expr),
    Local(Vec<LocalId>, Vec<Expr>),
    Assign(Vec<Expr>, Vec<Expr>),
    CompoundAssign(Expr, BinOp, Expr),
    Do(Block),
    While(Expr, Block),
    Repeat(Block, Expr),
    If(Vec<(Expr, Block)>, Option<Block>),
    NumericFor { var: LocalId, start: Expr, limit: Expr, step: Option<Expr>, body: Block },
    GenericFor { vars: Vec<LocalId>, exprs: Vec<Expr>, body: Block },
    LocalFunction(LocalId, Box<FuncBody>),
    Return(Vec<Expr>),
    Break,
    Continue,
    Goto(String),
    Label(String),
}
