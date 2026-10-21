use crate::value::{Str, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Nil,
    Bool(bool),
    Const(u32),
    Pop(u32),
    Dup(u32),
    GetLocal(u16),
    SetLocal(u16),
    NewCell(u16),
    GetCell(u16),
    SetCell(u16),
    GetUpval(u16),
    SetUpval(u16),
    GetGlobal(u32),
    SetGlobal(u32),
    GetIndex,
    GetField(u32),
    SetIndex,
    SetField(u32),
    SetIndexDeep(u16),
    NewTable(u32, u32),
    SetList { n: u32, multi: bool, first: u32 },
    SelfOp(u32),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Unm,
    Not,
    Len,
    Concat(u32),
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Jump(i32),
    JumpIfFalse(i32),
    JumpIfTrue(i32),
    AndJump(i32),
    OrJump(i32),
    Call { args: u32, multi: bool, results: i32 },
    TailCall { args: u32, multi: bool },
    Return { n: u32, multi: bool },
    Vararg(i32),
    Closure(u32),
    ForPrep { base: u16, jump: i32 },
    ForLoop { base: u16, jump: i32 },
    TForCall { base: u16, nvars: u16 },
    TForLoop { base: u16, jump: i32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum VarDesc {
    Global(Str),
    Local(Str),
    Field(Str),
    Upvalue(Str),
    Method(Str),
}

impl VarDesc {
    pub fn kind(&self) -> &'static str {
        match self {
            VarDesc::Global(_) => "global",
            VarDesc::Local(_) => "local",
            VarDesc::Field(_) => "field",
            VarDesc::Upvalue(_) => "upvalue",
            VarDesc::Method(_) => "method",
        }
    }
    pub fn name(&self) -> &Str {
        match self {
            VarDesc::Global(s) | VarDesc::Local(s) | VarDesc::Field(s) | VarDesc::Upvalue(s) | VarDesc::Method(s) => s,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Capture {
    Slot(u16),
    Upval(u16),
}

#[derive(Debug, Default)]
pub struct Proto {
    pub code: Vec<Op>,
    pub lines: Vec<u32>,
    pub consts: Vec<Value>,
    pub protos: Vec<std::rc::Rc<Proto>>,
    pub captures: Vec<Capture>,
    pub upval_names: Vec<Str>,
    pub num_params: u16,
    pub captured_params: Vec<u16>,
    pub is_vararg: bool,
    pub arg_slot: Option<u16>,
    pub num_slots: u16,
    pub source: std::rc::Rc<str>,
    pub line: u32,
    pub last_line: u32,
    pub hints: Vec<(u32, Option<VarDesc>, Option<VarDesc>)>,
    pub concat_hints: Vec<(u32, Vec<Option<VarDesc>>)>,
}

impl Proto {
    pub fn hint(&self, pc: usize) -> (Option<&VarDesc>, Option<&VarDesc>) {
        match self.hints.binary_search_by_key(&(pc as u32), |h| h.0) {
            Ok(i) => (self.hints[i].1.as_ref(), self.hints[i].2.as_ref()),
            Err(_) => (None, None),
        }
    }
}
