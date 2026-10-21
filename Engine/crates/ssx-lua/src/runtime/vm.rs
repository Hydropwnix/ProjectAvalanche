use std::any::Any;
use std::rc::Rc;

use crate::bytecode::{Proto, VarDesc};
use crate::heap::{Cell, Function, Heap, LuaClosure, NativeClosure, NativeFn, Obj, UserData};
use crate::table::Table;
use crate::value::{GcRef, Value};

pub const MAXCCALLS: u32 = 200;
// TODO(guess): the limit is taken as a frame count — the game doubles its call-info array (0x82A99D40) and fails when the grown size passes 20 000, so its deepest stack may be the last power of two below it; settle: the array's start size and the growth path in 0x82A99D18 / 0x82A99268 against a deep recursion.
pub const MAXCALLS: usize = 20000;
pub const MAXCSTACK: i32 = 0x7FFF;

#[derive(Clone, Debug)]
pub enum LuaError {
    Error(Value),
    Yield(Vec<Value>),
}

impl LuaError {
    pub fn message(&self) -> String {
        match self {
            LuaError::Error(Value::Str(s)) => s.to_str_lossy(),
            LuaError::Error(Value::Num(n)) => crate::number::tostring(*n),
            LuaError::Error(v) => format!("(error object is a {} value)", v.type_name()),
            LuaError::Yield(_) => "attempt to yield across metamethod/C-call boundary".into(),
        }
    }
}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for LuaError {}

pub type LuaResult<T> = Result<T, LuaError>;

pub type FileReader = Rc<dyn Fn(&str) -> Option<Vec<u8>>>;

pub struct Frame {
    pub func: GcRef,
    pub lua: bool,
    pub pc: usize,
    pub base: usize,
    pub varargs: Vec<Value>,
    pub ret: usize,
    pub results: i32,
    pub boundary: bool,
    pub tail: bool,
    pub name: Option<VarDesc>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CoStatus {
    Suspended,
    Running,
    Normal,
    Dead,
}

#[derive(Clone, Copy, Debug)]
pub struct Pending {
    pub func: usize,
    pub results: i32,
    pub tail: bool,
}

#[derive(Default)]
pub struct ThreadState {
    pub stack: Vec<Value>,
    pub cells: Vec<Option<Cell>>,
    pub frames: Vec<Frame>,
    pub status: Option<CoStatus>,
    pub multret: usize,
    pub nny: u32,
    pub pending: Option<Pending>,
    pub start: Option<Value>,
}

impl ThreadState {
    pub fn for_each_ref(&self, f: &mut impl FnMut(&Value)) {
        self.stack.iter().for_each(&mut *f);
        for c in self.cells.iter().flatten() {
            f(&c.borrow());
        }
        for fr in &self.frames {
            f(&Value::Func(fr.func));
            fr.varargs.iter().for_each(&mut *f);
        }
        if let Some(s) = &self.start {
            f(s);
        }
    }
}

pub struct Lua {
    pub heap: Heap,
    pub th: ThreadState,
    pub cur: GcRef,
    pub main: GcRef,
    pub globals: GcRef,
    pub registry: GcRef,
    pub type_meta: Vec<(&'static str, GcRef)>,
    resumers: Vec<GcRef>,
    pub ccalls: u32,
    pinned: Vec<Value>,
    pub host: Option<Box<dyn Any>>,
    pub budget: u64,
    pub executed: u64,
    pub printed: Vec<String>,
    pub file_reader: Option<FileReader>,
    pub gc_stopped: bool,
}

impl Default for Lua {
    fn default() -> Self {
        Self::new()
    }
}

impl Lua {
    pub fn new() -> Lua {
        let mut heap = Heap::default();
        heap.threshold = 10_000;
        let main = heap.alloc(Obj::Thread(Box::default()));
        let globals = heap.alloc(Obj::Table(Table::new(0, 0)));
        let registry = heap.alloc(Obj::Table(Table::new(0, 0)));
        let th = ThreadState { status: Some(CoStatus::Running), ..Default::default() };
        Lua {
            heap,
            th,
            cur: main,
            main,
            globals,
            registry,
            type_meta: Vec::new(),
            resumers: Vec::new(),
            ccalls: 0,
            pinned: Vec::new(),
            host: None,
            budget: 0,
            executed: 0,
            printed: Vec::new(),
            file_reader: None,
            gc_stopped: false,
        }
    }

    pub fn pin(&mut self, v: Value) {
        if v.gc().is_some() {
            self.pinned.push(v);
        }
    }

    pub fn unpin_all(&mut self) {
        self.pinned.clear();
    }

    pub fn collect_garbage(&mut self) -> usize {
        let mut roots = vec![Value::Table(self.globals), Value::Table(self.registry), Value::Thread(self.main)];
        roots.extend(self.type_meta.iter().map(|(_, m)| Value::Table(*m)));
        roots.extend(self.resumers.iter().map(|r| Value::Thread(*r)));
        roots.push(Value::Thread(self.cur));
        roots.extend(self.pinned.iter().cloned());
        self.th.for_each_ref(&mut |v| roots.push(v.clone()));
        self.heap.collect(&roots)
    }

    pub fn maybe_collect(&mut self) {
        if !self.gc_stopped && self.heap.debt > self.heap.threshold {
            self.collect_garbage();
        }
    }

    pub fn new_table(&mut self, narray: usize, nhash: usize) -> GcRef {
        self.heap.alloc(Obj::Table(Table::new(narray, nhash)))
    }

    pub fn create_table(&mut self) -> Value {
        Value::Table(self.new_table(0, 0))
    }

    pub fn create_function(
        &mut self,
        name: &str,
        f: impl Fn(&mut Lua, &[Value]) -> LuaResult<Vec<Value>> + 'static,
    ) -> Value {
        self.create_function_with(name, Rc::new(f), Vec::new())
    }

    pub fn create_function_with(&mut self, name: &str, f: NativeFn, upvals: Vec<Value>) -> Value {
        let c = NativeClosure { name: name.into(), f, upvals, env: self.globals };
        Value::Func(self.heap.alloc(Obj::Func(Function::Native(c))))
    }

    pub fn create_userdata(&mut self, data: Box<dyn Any>, meta: Option<GcRef>) -> Value {
        Value::UserData(self.heap.alloc(Obj::UserData(UserData { data, meta, env: None })))
    }

    pub fn new_closure(&mut self, proto: Rc<Proto>, upvals: Vec<Cell>, env: GcRef) -> Value {
        Value::Func(self.heap.alloc(Obj::Func(Function::Lua(LuaClosure { proto, upvals, env }))))
    }

    pub fn set_global(&mut self, name: &str, v: Value) {
        let g = self.globals;
        let _ = self.heap.table_mut(g).set(Value::str(name), v);
    }

    pub fn get_global(&self, name: &str) -> Value {
        self.heap.table(self.globals).get_str(name)
    }

    pub fn register(&mut self, name: &str, f: impl Fn(&mut Lua, &[Value]) -> LuaResult<Vec<Value>> + 'static) {
        let v = self.create_function(name, f);
        self.set_global(name, v);
    }

    pub fn raw_get(&self, t: GcRef, k: &Value) -> Value {
        self.heap.table(t).get(k)
    }

    pub fn raw_set(&mut self, t: GcRef, k: Value, v: Value) -> LuaResult<()> {
        self.heap.table_mut(t).set(k, v).map_err(|e| match e {
            crate::table::KeyError::Nil => self.rt_error("table index is nil"),
            crate::table::KeyError::NaN => self.rt_error("table index is NaN"),
        })
    }

    pub fn upvalue(&self, i: usize) -> Value {
        match self.th.frames.last() {
            Some(fr) if !fr.lua => match self.heap.func(fr.func) {
                Function::Native(c) => c.upvals.get(i).cloned().unwrap_or_default(),
                Function::Lua(_) => Value::Nil,
            },
            _ => Value::Nil,
        }
    }

    pub fn metatable(&self, v: &Value) -> Option<GcRef> {
        match v {
            Value::Table(t) => self.heap.table(*t).meta,
            Value::UserData(u) => self.heap.userdata(*u).meta,
            _ => {
                let tn = v.type_name();
                self.type_meta.iter().find(|(n, _)| *n == tn).map(|(_, m)| *m)
            }
        }
    }

    pub fn metamethod(&self, v: &Value, event: &str) -> Value {
        match self.metatable(v) {
            Some(m) => self.heap.table(m).get_str(event),
            None => Value::Nil,
        }
    }

    pub fn where_(&self, level: usize) -> String {
        let mut n = 0;
        for fr in self.th.frames.iter().rev() {
            n += 1;
            if n == level {
                if !fr.lua {
                    return String::new();
                }
                if let Function::Lua(c) = self.heap.func(fr.func) {
                    let line = c.proto.lines.get(fr.pc.saturating_sub(1)).copied().unwrap_or(0);
                    return format!("{}:{}: ", crate::parser::chunkid(&c.proto.source), line);
                }
            }
        }
        String::new()
    }

    pub fn rt_error(&self, msg: &str) -> LuaError {
        let level = if self.th.frames.last().is_some_and(|f| !f.lua) { 2 } else { 1 };
        LuaError::Error(Value::str(&format!("{}{}", self.where_(level), msg)))
    }

    fn native_name(&self) -> (String, bool) {
        match self.th.frames.last().and_then(|f| f.name.as_ref()) {
            Some(VarDesc::Method(n)) => (n.to_str_lossy(), true),
            Some(d) => (d.name().to_str_lossy(), false),
            None => ("?".into(), false),
        }
    }

    pub fn arg_error(&self, narg: usize, msg: &str) -> LuaError {
        let (name, method) = self.native_name();
        if method {
            if narg == 1 {
                return self.rt_error(&format!("calling '{name}' on bad self ({msg})"));
            }
            return self.rt_error(&format!("bad argument #{} to '{name}' ({msg})", narg - 1));
        }
        self.rt_error(&format!("bad argument #{narg} to '{name}' ({msg})"))
    }

    pub fn new_thread(&mut self, body: Value) -> GcRef {
        let st = ThreadState { status: Some(CoStatus::Suspended), start: Some(body), ..Default::default() };
        self.heap.alloc(Obj::Thread(Box::new(st)))
    }

    pub fn is_main(&self) -> bool {
        self.cur == self.main
    }

    pub(crate) fn push_resumer(&mut self, r: GcRef) {
        self.resumers.push(r);
    }

    pub(crate) fn pop_resumer(&mut self) -> Option<GcRef> {
        self.resumers.pop()
    }
}
