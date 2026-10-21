use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use super::bindings::{BINDINGS, Binding, CONSTANTS, GAME_STATE, GAME_STATE_BINDERS, TYPES};
use super::{Binder, LIBS_FRONTEND, LIBS_WORLD, open_libs};
use crate::stdlib::package::search_path;
use crate::value::Value;
use crate::vm::{FileReader, Lua, LuaResult};

#[derive(Debug, Default)]
pub struct Recorder {
    pub calls: BTreeMap<String, usize>,
    pub log: Vec<String>,
    pub keep_log: bool,
    pub undefined_reads: BTreeMap<String, usize>,
    pub host_calls: BTreeMap<String, usize>,
}

#[derive(Debug)]
pub struct StubObject {
    pub ty: &'static str,
    pub id: u32,
}

pub type Rec = Rc<RefCell<Recorder>>;

fn describe(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("\"{}\"", s.to_str_lossy()),
        Value::Num(n) => crate::number::tostring(*n),
        Value::Bool(b) => b.to_string(),
        Value::Nil => "nil".into(),
        other => other.type_name().into(),
    }
}

fn check_args(l: &Lua, b: &Binding, args: &[Value]) -> LuaResult<()> {
    for (i, letter) in b.args.split(',').enumerate().filter(|(_, s)| !s.is_empty()) {
        let n = i + 1;
        let present = !matches!(args.get(i), None | Some(Value::Nil));
        match letter {
            "S" => {
                l.check_str(args, n)?;
            }
            "N" => {
                l.check_num(args, n)?;
            }
            "I" => {
                l.check_int(args, n)?;
            }
            "T5" => {
                l.check_table(args, n)?;
            }
            "S?" if present => {
                l.check_str(args, n)?;
            }
            "N?" if present => {
                l.check_num(args, n)?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub type Hook = Rc<dyn Fn(&mut Lua, &'static Binding, &[Value]) -> LuaResult<Option<Box<dyn Any>>>>;

fn new_object(l: &mut Lua, ty: &'static str, serial: &Rc<RefCell<u32>>, payload: Option<Box<dyn Any>>) -> Value {
    let mut s = serial.borrow_mut();
    *s += 1;
    let reg = l.registry;
    let key = if ty.is_empty() { "__untyped" } else { ty };
    let meta = match l.heap.table(reg).get_str(key) {
        Value::Table(m) => Some(m),
        _ => match l.heap.table(reg).get_str("__untyped") {
            Value::Table(m) => Some(m),
            _ => None,
        },
    };
    let data = payload.unwrap_or_else(|| Box::new(StubObject { ty, id: *s }));
    l.create_userdata(data, meta)
}

fn stub(
    l: &mut Lua,
    b: &'static Binding,
    rec: &Rec,
    serial: &Rc<RefCell<u32>>,
    checked: bool,
    hook: Option<Hook>,
) -> Value {
    let rec = rec.clone();
    let serial = serial.clone();
    l.create_function(b.name, move |l, args| {
        if checked {
            check_args(l, b, args)?;
        }
        let key = if b.recv.is_empty() { b.name.to_string() } else { format!("{}:{}", b.recv, b.name) };
        {
            let mut r = rec.borrow_mut();
            *r.calls.entry(key.clone()).or_default() += 1;
            if r.keep_log {
                let a: Vec<String> = args.iter().skip(usize::from(!b.recv.is_empty())).map(describe).collect();
                r.log.push(format!("{key}({})", a.join(", ")));
            }
        }
        let payload = match &hook {
            Some(h) => h(l, b, args)?,
            None => None,
        };
        if !b.ret.is_empty() || b.nret > 0 {
            return Ok(vec![new_object(l, b.ret, &serial, payload)]);
        }
        Ok(Vec::new())
    })
}

pub fn world_state(binder: Binder, rec: &Rec, checked: bool) -> Lua {
    world_state_hooked(binder, rec, checked, None)
}

pub fn world_state_hooked(binder: Binder, rec: &Rec, checked: bool, hook: Option<Hook>) -> Lua {
    let mut l = Lua::new();
    open_libs(&mut l, &LIBS_WORLD);
    let serial = Rc::new(RefCell::new(0u32));
    let reg = l.registry;
    let all_methods = l.new_table(0, 0);
    for &(_, ty) in TYPES.iter().filter(|(b, _)| *b == binder) {
        let mt = match l.heap.table(reg).get_str(ty) {
            Value::Table(m) => m,
            _ => l.new_table(0, 0),
        };
        let _ = l.heap.table_mut(mt).set(Value::str("__index"), Value::Table(mt));
        let _ = l.heap.table_mut(reg).set(Value::str(ty), Value::Table(mt));
    }
    for b in BINDINGS.iter().filter(|b| b.binder == binder) {
        let f = stub(&mut l, b, rec, &serial, checked, hook.clone());
        if b.recv.is_empty() {
            l.set_global(b.name, f);
        } else {
            if let Value::Table(mt) = l.heap.table(reg).get_str(b.recv) {
                let _ = l.heap.table_mut(mt).set(Value::str(b.name), f.clone());
            }
            let _ = l.heap.table_mut(all_methods).set(Value::str(b.name), f);
        }
    }
    let untyped = l.new_table(0, 1);
    let _ = l.heap.table_mut(untyped).set(Value::str("__index"), Value::Table(all_methods));
    let _ = l.heap.table_mut(reg).set(Value::str("__untyped"), Value::Table(untyped));
    for &(_, name, v) in CONSTANTS.iter().filter(|(b, _, _)| *b == binder) {
        l.set_global(name, Value::Num(v));
    }
    if GAME_STATE_BINDERS.contains(&binder) {
        for (id, name) in GAME_STATE.iter().enumerate().skip(1) {
            l.set_global(name, Value::Num(id as f64));
        }
    }
    watch_globals(&mut l, rec);
    l
}

fn watch_globals(l: &mut Lua, rec: &Rec) {
    let rec = rec.clone();
    let f = l.create_function("__index", move |_, a| {
        if let Some(Value::Str(k)) = a.get(1) {
            *rec.borrow_mut().undefined_reads.entry(k.to_str_lossy()).or_default() += 1;
        }
        Ok(vec![Value::Nil])
    });
    let mt = l.new_table(0, 1);
    let _ = l.heap.table_mut(mt).set(Value::str("__index"), f);
    let g = l.globals;
    l.heap.table_mut(g).meta = Some(mt);
}

pub fn frontend_state(rec: &Rec, read: FileReader, packages_dir: &str) -> Lua {
    let mut l = Lua::new();
    open_libs(&mut l, &LIBS_FRONTEND);
    // TODO(guess): `UXLua` replaces `loadfile`, `dofile`, `print` and `assert` with host functions (0x8312F5E0, 0x8312F1D8, 0x8312F290, 0x8312F298); the standard ones stand in here; settle: decompile them.
    l.file_reader = Some(read);
    for name in ["loadfileasync", "loadjsonfile", "logtelemetry"] {
        let rec = rec.clone();
        l.register(name, move |_, _| {
            *rec.borrow_mut().host_calls.entry(name.to_string()).or_default() += 1;
            Ok(vec![Value::Nil])
        });
    }
    let rec2 = rec.clone();
    let host = move |name: &'static str| {
        let rec = rec2.clone();
        move |_: &mut Lua, _: &[Value]| -> LuaResult<Vec<Value>> {
            *rec.borrow_mut().host_calls.entry(name.to_string()).or_default() += 1;
            Ok(Vec::new())
        }
    };
    let gt = l.lib_table("luagametalk");
    for name in ["registerMessageHandler", "unregisterMessageHandler", "sendMessage", "poll"] {
        l.lib_fn(gt, name, host(name));
    }
    let sw_mt = l.new_table(0, 8);
    let _ = l.heap.table_mut(sw_mt).set(Value::str("__index"), Value::Table(sw_mt));
    for name in ["start", "stop", "reset", "restart"] {
        l.lib_fn(sw_mt, name, host(name));
    }
    l.lib_fn(sw_mt, "isRunning", |_, _| Ok(vec![Value::Bool(false)]));
    l.lib_fn(sw_mt, "getElapsedTime", |_, _| Ok(vec![Value::Num(0.0)]));
    let sw = l.lib_table("luastopwatch");
    l.lib_fn(sw, "new", move |l, _| Ok(vec![l.create_userdata(Box::new(()), Some(sw_mt))]));
    let path = format!("{packages_dir}/?.lua");
    let loader = l.create_function("loader_host", move |l, a| {
        let name = l.check_str(a, 1)?.to_str_lossy();
        search_path(l, &name, &path)
    });
    if let Value::Table(p) = l.get_global("package")
        && let Value::Table(ld) = l.heap.table(p).get_str("loaders")
    {
        let n = l.heap.table(ld).length();
        let _ = l.heap.table_mut(ld).set(Value::Num((n + 1) as f64), loader);
    }
    watch_globals(&mut l, rec);
    l
}
