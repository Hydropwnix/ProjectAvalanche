pub mod builtins;
pub mod display;
pub mod interp;
pub mod members;
pub mod natives;
pub mod object;
pub mod runtime;
pub mod value;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use ssx_formats::gfx::avm1::Action;

use crate::display::{InstanceId, Player};
use crate::library::{Library, MovieId};
use object::{Heap, ObjId, Scope};
use value::{Str, Value};

pub use runtime::LoadRequest;

#[derive(Clone, Debug)]
pub enum Fault {
    Throw(Value),
    Abort(&'static str),
}

#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub blocks: u64,
    pub actions: u64,
    pub calls: u64,
    pub frames: u64,
    pub empty_pops: u64,
    pub ops: Vec<u64>,
    pub counts: BTreeMap<(String, String), u64>,
}

impl Diagnostics {
    pub fn count_op(&mut self, op: u8) {
        if self.ops.len() < 256 {
            self.ops.resize(256, 0);
        }
        self.ops[usize::from(op)] += 1;
    }

    pub fn note(&mut self, kind: &str, detail: &str) {
        *self.counts.entry((kind.to_string(), detail.to_string())).or_insert(0) += 1;
    }

    pub fn by_kind(&self) -> BTreeMap<&str, (u64, usize)> {
        let mut out: BTreeMap<&str, (u64, usize)> = BTreeMap::new();
        for ((k, _), n) in &self.counts {
            let e = out.entry(k.as_str()).or_default();
            e.0 += n;
            e.1 += 1;
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct NativeCall {
    pub object: &'static str,
    pub method: &'static str,
    pub args: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum Job {
    Code { clip: InstanceId, code: Arc<[Action]>, movie: MovieId },
    Construct { clip: InstanceId },
    Event { clip: InstanceId, event: ClipEvent },
    Unload { obj: ObjId },
    Call { func: Value, this: Value, args: Vec<Value> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipEvent {
    Load,
    EnterFrame,
    Unload,
}

impl ClipEvent {
    pub fn flag(self) -> u32 {
        match self {
            ClipEvent::Load => 0x01,
            ClipEvent::EnterFrame => 0x02,
            ClipEvent::Unload => 0x04,
        }
    }

    pub fn method(self) -> &'static str {
        match self {
            ClipEvent::Load => "onLoad",
            ClipEvent::EnterFrame => "onEnterFrame",
            ClipEvent::Unload => "onUnload",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Interval {
    pub func: Value,
    pub this: Value,
    pub args: Vec<Value>,
    pub period: f64,
    pub next: f64,
    pub once: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Protos {
    pub object: ObjId,
    pub function: ObjId,
    pub array: ObjId,
    pub string: ObjId,
    pub number: ObjId,
    pub boolean: ObjId,
    pub date: ObjId,
    pub movieclip: ObjId,
    pub textfield: ObjId,
    pub textformat: ObjId,
    pub color: ObjId,
    pub color_transform: ObjId,
    pub transform: ObjId,
    pub matrix: ObjId,
    pub point: ObjId,
    pub rectangle: ObjId,
    pub loader: ObjId,
    pub filter: ObjId,
}

pub struct Vm {
    pub heap: Heap,
    pub global: ObjId,
    pub global_scope: Arc<Scope>,
    pub protos: Protos,
    pub stack: Vec<Value>,
    pub display_objs: HashMap<InstanceId, ObjId>,
    pub classes: HashMap<Str, Value>,
    pub(crate) queue: [VecDeque<Job>; 3],
    pub(crate) intervals: BTreeMap<i32, Interval>,
    pub(crate) next_interval: i32,
    pub trace_log: Vec<String>,
    pub print_trace: bool,
    pub diag: Diagnostics,
    pub native_calls: Vec<NativeCall>,
    pub(crate) loads: Vec<LoadRequest>,
    pub time: f64,
    pub(crate) depth: u32,
    pub(crate) budget: u64,
    pub constructing: bool,
    pub(crate) next_construct: bool,
    rng: u32,
    pub class_names: HashMap<ObjId, &'static str>,
    pub(crate) init_ran: HashSet<(MovieId, u16)>,
    pub loaded: Vec<String>,
    pub(crate) cur_version: u8,
    pub(crate) cur_movie: MovieId,
    pub(crate) cur_op: (u8, u32),
    pub(crate) call_names: Vec<String>,
    pub debug_stacks: bool,
    pub watch: Option<String>,
    pub natives: natives::NativeState,
}

pub const ACTION_BUDGET: u64 = 20_000_000;

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

impl Vm {
    pub fn new() -> Vm {
        let mut heap = Heap::default();
        let (global, protos, class_names) = builtins::install(&mut heap);
        let global_scope = Arc::new(Scope { obj: global, kind: object::ScopeKind::Global, parent: None });
        Vm {
            heap,
            global,
            global_scope,
            protos,
            stack: Vec::new(),
            display_objs: HashMap::new(),
            classes: HashMap::new(),
            queue: Default::default(),
            intervals: BTreeMap::new(),
            next_interval: 1,
            trace_log: Vec::new(),
            print_trace: false,
            diag: Diagnostics::default(),
            native_calls: Vec::new(),
            loads: Vec::new(),
            time: 0.0,
            depth: 0,
            budget: ACTION_BUDGET,
            constructing: false,
            next_construct: false,
            rng: 0x2545_F491,
            class_names,
            init_ran: HashSet::new(),
            loaded: Vec::new(),
            cur_version: 7,
            cur_movie: 0,
            cur_op: (0, 0),
            call_names: Vec::new(),
            debug_stacks: std::env::var_os("SSX_AVM1_STACKS").is_some(),
            watch: std::env::var("SSX_AVM1_WATCH").ok(),
            natives: natives::NativeState::default(),
        }
    }

    pub fn trace(&mut self, s: &str) {
        if self.print_trace {
            println!("trace: {s}");
        }
        self.trace_log.push(s.to_string());
    }

    pub fn timer_ms(&self) -> f64 {
        (self.time * 1000.0).floor()
    }

    pub fn random(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }
}

pub struct Ctx<'a> {
    pub vm: &'a mut Vm,
    pub lib: &'a mut Library,
    pub player: &'a mut Player,
}
