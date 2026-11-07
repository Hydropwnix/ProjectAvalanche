use std::collections::HashMap;
use std::sync::Arc;

use ssx_formats::gfx::avm1::Action;

use super::value::{Str, Value};
use crate::display::InstanceId;
use crate::library::MovieId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjId(pub u32);

pub mod flags {
    pub const DONT_ENUM: u8 = 1;
    pub const DONT_DELETE: u8 = 2;
    pub const READ_ONLY: u8 = 4;
}

#[derive(Clone, Debug)]
pub struct Property {
    pub value: Value,
    pub flags: u8,
    pub accessor: Option<Box<(Value, Value)>>,
}

impl Property {
    pub fn new(value: Value, flags: u8) -> Self {
        Property { value, flags, accessor: None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Props {
    entries: Vec<(Str, Property)>,
    index: HashMap<Str, usize>,
}

impl Props {
    pub fn get(&self, k: &str) -> Option<&Property> {
        self.index.get(k).map(|&i| &self.entries[i].1)
    }

    pub fn get_mut(&mut self, k: &str) -> Option<&mut Property> {
        self.index.get(k).map(|&i| &mut self.entries[i].1)
    }

    pub fn insert(&mut self, k: Str, p: Property) {
        match self.index.get(&k) {
            Some(&i) => self.entries[i].1 = p,
            None => {
                self.index.insert(k.clone(), self.entries.len());
                self.entries.push((k, p));
            }
        }
    }

    pub fn remove(&mut self, k: &str) -> Option<Property> {
        let i = self.index.remove(k)?;
        let (_, p) = self.entries.remove(i);
        for (j, (key, _)) in self.entries.iter().enumerate().skip(i) {
            self.index.insert(key.clone(), j);
        }
        Some(p)
    }

    pub fn contains(&self, k: &str) -> bool {
        self.index.contains_key(k)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Str, &Property)> {
        self.entries.iter().map(|(k, p)| (k, p))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Str, &mut Property)> {
        self.entries.iter_mut().map(|(k, p)| (&*k, p))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }
}

#[derive(Debug)]
pub struct Scope {
    pub obj: ObjId,
    pub kind: ScopeKind,
    pub parent: Option<Arc<Scope>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    Global,
    Target,
    With,
    Local,
}

pub type NativeFn = fn(&mut super::Ctx, Value, &[Value]) -> Result<Value, super::Fault>;

#[derive(Debug)]
pub struct AvmFunction {
    pub name: Str,
    pub code: Arc<[Action]>,
    pub start: usize,
    pub end: usize,
    pub v2: Option<(u8, u16)>,
    pub params: Vec<(u8, Str)>,
    pub pool: Arc<Vec<Str>>,
    pub version: u8,
    pub movie: MovieId,
}

#[derive(Clone, Debug)]
pub enum FuncCode {
    Native(NativeFn),
    Avm(Arc<AvmFunction>),
}

#[derive(Clone, Debug)]
pub struct Func {
    pub code: FuncCode,
    pub scope: Option<Arc<Scope>>,
    pub base: Option<ObjId>,
}

#[derive(Clone, Debug)]
pub struct DisplayRef {
    pub id: InstanceId,
    pub path: Str,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Plain,
    Function(Box<Func>),
    Array(Vec<Value>),
    Boxed(Value),
    Date(f64),
    Display(DisplayRef),
    Super { this: Value, base: Value, ctor: Value },
    Activation,
    Tagged { tag: &'static str, target: Value },
}

#[derive(Clone, Debug)]
pub struct Object {
    pub proto: Value,
    pub props: Props,
    pub kind: Kind,
    pub interfaces: Vec<ObjId>,
    mark: bool,
}

impl Object {
    pub fn new(proto: Value, kind: Kind) -> Self {
        Object { proto, props: Props::default(), kind, interfaces: Vec::new(), mark: false }
    }

    pub fn func(&self) -> Option<&Func> {
        match &self.kind {
            Kind::Function(f) => Some(f),
            _ => None,
        }
    }

    pub fn display(&self) -> Option<&DisplayRef> {
        match &self.kind {
            Kind::Display(d) => Some(d),
            _ => None,
        }
    }

    fn children(&self, out: &mut Vec<ObjId>) {
        let mut v = |x: &Value| {
            if let Value::Object(o) = x {
                out.push(*o);
            }
        };
        v(&self.proto);
        for (_, p) in self.props.iter() {
            v(&p.value);
            if let Some(a) = &p.accessor {
                v(&a.0);
                v(&a.1);
            }
        }
        match &self.kind {
            Kind::Function(f) => {
                if let Some(b) = f.base {
                    out.push(b);
                }
                let mut s = f.scope.as_deref();
                while let Some(sc) = s {
                    out.push(sc.obj);
                    s = sc.parent.as_deref();
                }
            }
            Kind::Array(a) => {
                for x in a {
                    if let Value::Object(o) = x {
                        out.push(*o);
                    }
                }
            }
            Kind::Boxed(Value::Object(o)) => out.push(*o),
            Kind::Super { this, base, ctor } => {
                for x in [this, base, ctor] {
                    if let Value::Object(o) = x {
                        out.push(*o);
                    }
                }
            }
            Kind::Tagged { target: Value::Object(o), .. } => out.push(*o),
            _ => {}
        }
        out.extend_from_slice(&self.interfaces);
    }
}

#[derive(Default)]
pub struct Heap {
    slots: Vec<Option<Object>>,
    free: Vec<u32>,
    pub since_gc: usize,
}

impl Heap {
    pub fn alloc(&mut self, o: Object) -> ObjId {
        self.since_gc += 1;
        if let Some(i) = self.free.pop() {
            self.slots[i as usize] = Some(o);
            ObjId(i)
        } else {
            self.slots.push(Some(o));
            ObjId(self.slots.len() as u32 - 1)
        }
    }

    pub fn get(&self, id: ObjId) -> Option<&Object> {
        self.slots.get(id.0 as usize).and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, id: ObjId) -> Option<&mut Object> {
        self.slots.get_mut(id.0 as usize).and_then(Option::as_mut)
    }

    pub fn live(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    pub fn script_bytes(&self) -> u64 {
        use super::sandbox::{ELEMENT_BYTES, OBJECT_BYTES, PROPERTY_BYTES};
        let string = |v: &Value| if let Value::String(s) = v { s.len() as u64 } else { 0 };
        let mut total = 0u64;
        for o in self.slots.iter().flatten() {
            total += OBJECT_BYTES;
            for (k, p) in o.props.iter() {
                total += PROPERTY_BYTES + k.len() as u64 + string(&p.value);
            }
            if let Kind::Array(a) = &o.kind {
                total += a.iter().map(|v| ELEMENT_BYTES + string(v)).sum::<u64>();
            }
        }
        total
    }

    pub fn collect(&mut self, roots: impl IntoIterator<Item = ObjId>) -> usize {
        let mut stack: Vec<ObjId> = roots.into_iter().collect();
        let mut children = Vec::new();
        while let Some(id) = stack.pop() {
            let Some(o) = self.slots.get_mut(id.0 as usize).and_then(Option::as_mut) else { continue };
            if o.mark {
                continue;
            }
            o.mark = true;
            children.clear();
            o.children(&mut children);
            stack.extend_from_slice(&children);
        }
        let mut freed = 0;
        for (i, s) in self.slots.iter_mut().enumerate() {
            match s {
                Some(o) if o.mark => o.mark = false,
                Some(_) => {
                    *s = None;
                    self.free.push(i as u32);
                    freed += 1;
                }
                None => {}
            }
        }
        self.since_gc = 0;
        freed
    }
}
