use crate::value::Str;

pub type LocalId = u32;

#[derive(Clone, Debug)]
pub struct LocalDecl {
    pub name: Str,
    pub slot: u16,
    pub captured: bool,
}

#[derive(Clone, Debug)]
pub enum UpvalSource {
    Local(LocalId),
    Upval(u16),
}

#[derive(Clone, Debug)]
pub struct FuncBody {
    pub params: Vec<LocalId>,
    pub is_vararg: bool,
    pub arg_local: Option<LocalId>,
    pub needs_arg: bool,
    pub body: Block,
    pub locals: Vec<LocalDecl>,
    pub upvals: Vec<(Str, UpvalSource)>,
    pub num_slots: u16,
    pub line: u32,
    pub last_line: u32,
}

pub type Block = Vec<Stat>;

#[derive(Clone, Debug)]
pub enum LValue {
    Local(LocalId),
    Upval(u16),
    Global(Str),
    Index(Box<Expr>, Box<Expr>, u32),
}

#[derive(Clone, Debug)]
pub enum Stat {
    Call(Expr),
    Local(Vec<LocalId>, Vec<Expr>),
    LocalFunction(LocalId, Box<FuncBody>),
    Assign(Vec<LValue>, Vec<Expr>),
    Do(Block),
    While(Expr, Block),
    Repeat(Block, Expr),
    If(Vec<(Expr, Block)>, Option<Block>),
    NumFor { base: u16, var: LocalId, start: Expr, limit: Expr, step: Option<Expr>, body: Block, line: u32 },
    GenFor { base: u16, vars: Vec<LocalId>, exprs: Vec<Expr>, body: Block, line: u32 },
    Return(Vec<Expr>, u32),
    Break(u32),
    Line(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    Len,
}

#[derive(Clone, Debug)]
pub enum TableItem {
    Pos(Expr),
    Named(Str, Expr),
    Keyed(Expr, Expr),
}

#[derive(Clone, Debug)]
pub enum Expr {
    Nil,
    True,
    False,
    Num(f64),
    Str(Str),
    Vararg,
    Func(Box<FuncBody>),
    Local(LocalId),
    Upval(u16),
    Global(Str),
    Index(Box<Expr>, Box<Expr>, u32),
    Call(Box<Expr>, Vec<Expr>, u32),
    Method(Box<Expr>, Str, Vec<Expr>, u32),
    Bin(BinOp, Box<Expr>, Box<Expr>, u32),
    Un(UnOp, Box<Expr>, u32),
    Table(Vec<TableItem>, u32),
    Paren(Box<Expr>),
}

impl Expr {
    pub fn is_multi(&self) -> bool {
        matches!(self, Expr::Call(..) | Expr::Method(..) | Expr::Vararg)
    }
}
