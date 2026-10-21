use std::cell::RefCell;
use std::rc::Rc;

use super::Binder;
use super::bindings::Binding;
use super::record::{Hook, Rec, Recorder, world_state_hooked};
use crate::value::Value;
use crate::vm::Lua;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Obj(ObjId),
    Other(&'static str),
}

impl Arg {
    pub fn str(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Arg::Num(n) => Some(*n),
            Arg::Str(s) => crate::number::str2d(s.as_bytes()),
            _ => None,
        }
    }

    pub fn obj(&self) -> Option<ObjId> {
        match self {
            Arg::Obj(o) => Some(*o),
            _ => None,
        }
    }

    pub fn truthy(&self) -> bool {
        !matches!(self, Arg::Nil | Arg::Bool(false))
    }
}

#[derive(Clone, Debug)]
pub struct Call {
    pub binding: &'static Binding,
    pub args: Vec<Arg>,
    pub ret: Option<ObjId>,
    pub script: usize,
}

impl Call {
    pub fn name(&self) -> &'static str {
        self.binding.name
    }

    pub fn arg(&self, i: usize) -> &Arg {
        self.args.get(i - 1).unwrap_or(&Arg::Nil)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub calls: Vec<Call>,
    pub scripts: Vec<(String, Option<String>)>,
}

impl Graph {
    pub fn methods(&self, o: ObjId) -> impl Iterator<Item = &Call> {
        self.calls.iter().filter(move |c| !c.binding.recv.is_empty() && c.args.first() == Some(&Arg::Obj(o)))
    }

    pub fn creator(&self, o: ObjId) -> Option<&Call> {
        self.calls.iter().find(|c| c.ret == Some(o))
    }
}

fn arg(l: &Lua, v: &Value) -> Arg {
    match v {
        Value::Nil => Arg::Nil,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Num(n) => Arg::Num(*n),
        Value::Str(s) => Arg::Str(s.to_str_lossy()),
        Value::UserData(r) => match crate::heap::downcast::<ObjId>(l.heap.userdata(*r)) {
            Some(o) => Arg::Obj(*o),
            None => Arg::Other("userdata"),
        },
        other => Arg::Other(other.type_name()),
    }
}

pub fn run_batch(binder: Binder, scripts: &[(String, Vec<u8>)], rec: Option<&Rec>) -> Graph {
    run_batch_with(binder, scripts, rec, &|_| {})
}

pub fn run_batch_with(
    binder: Binder,
    scripts: &[(String, Vec<u8>)],
    rec: Option<&Rec>,
    setup: &dyn Fn(&mut Lua),
) -> Graph {
    let g = Rc::new(RefCell::new(Graph::default()));
    let next = Rc::new(RefCell::new(0u32));
    let (hg, hn) = (g.clone(), next.clone());
    let hook: Hook = Rc::new(move |l, b: &'static Binding, a| {
        let args = a.iter().map(|v| arg(l, v)).collect();
        let ret = (!b.ret.is_empty() || b.nret > 0).then(|| {
            let mut n = hn.borrow_mut();
            *n += 1;
            ObjId(*n)
        });
        let mut g = hg.borrow_mut();
        let script = g.scripts.len();
        g.calls.push(Call { binding: b, args, ret, script });
        Ok(ret.map(|o| Box::new(o) as Box<dyn std::any::Any>))
    });
    let own: Rec = Rc::new(RefCell::new(Recorder::default()));
    let mut l = world_state_hooked(binder, rec.unwrap_or(&own), true, Some(hook));
    setup(&mut l);
    for (name, src) in scripts {
        let r = l.exec(src, name).err().map(|e| e.message());
        g.borrow_mut().scripts.push((name.clone(), r));
    }
    drop(l);
    let Ok(cell) = Rc::try_unwrap(g) else { unreachable!("the Lua state that held the graph is gone") };
    cell.into_inner()
}
