use super::{none, one};
use crate::value::{GcRef, Value};
use crate::vm::{Lua, LuaResult};

const SENTINEL: usize = 0x05E5_71E1;

pub fn open(l: &mut Lua) {
    let p = l.lib_table("package");
    let loaded = l.loaded_table().unwrap_or(p);
    let set = |l: &mut Lua, k: &str, v: Value| {
        let _ = l.heap.table_mut(p).set(Value::str(k), v);
    };
    set(l, "loaded", Value::Table(loaded));
    let preload = l.create_table();
    set(l, "preload", preload);
    set(l, "path", Value::str("?.lua"));
    set(l, "cpath", Value::str("?.dll"));
    set(l, "config", Value::str("\\\n;\n?\n!\n-"));
    let loaders = l.new_table(4, 0);
    let pre = l.create_function("loader_preload", |l, a| {
        let name = l.check_str(a, 1)?;
        let Value::Table(p) = l.get_global("package") else { return none() };
        let Value::Table(pre) = l.heap.table(p).get_str("preload") else {
            return Err(l.rt_error("'package.preload' must be a table"));
        };
        let f = l.heap.table(pre).get(&Value::Str(name.clone()));
        if f.is_nil() {
            return one(Value::str(&format!("\n\tno field package.preload['{}']", name.to_str_lossy())));
        }
        one(f)
    });
    let lua_loader = l.create_function("loader_Lua", |l, a| {
        let name = l.check_str(a, 1)?.to_str_lossy();
        let Value::Table(p) = l.get_global("package") else { return none() };
        let path = l.heap.table(p).get_str("path").to_str_coerce().map(|s| s.to_str_lossy()).unwrap_or_default();
        search_path(l, &name, &path)
    });
    let c_loader = l.create_function("loader_C", |l, a| {
        let name = l.check_str(a, 1)?.to_str_lossy();
        one(Value::str(&format!("\n\tno C loader for '{name}'")))
    });
    for (i, f) in [pre, lua_loader, c_loader].into_iter().enumerate() {
        let _ = l.heap.table_mut(loaders).set(Value::Num((i + 1) as f64), f);
    }
    set(l, "loaders", Value::Table(loaders));
    l.lib_fn(p, "seeall", |l, a| {
        let t = l.check_table(a, 1)?;
        let mt = match l.heap.table(t).meta {
            Some(m) => m,
            None => {
                let m = l.new_table(0, 1);
                l.heap.table_mut(t).meta = Some(m);
                m
            }
        };
        let g = l.globals;
        let _ = l.heap.table_mut(mt).set(Value::str("__index"), Value::Table(g));
        none()
    });
    l.lib_fn(p, "loadlib", |_, _| {
        Ok(vec![Value::Nil, Value::str("dynamic libraries not enabled"), Value::str("absent")])
    });
    l.register("require", require);
    l.register("module", module);
}

pub fn search_path(l: &mut Lua, name: &str, path: &str) -> LuaResult<Vec<Value>> {
    let file = name.replace('.', "/");
    let mut msg = String::new();
    for templ in path.split(';').filter(|s| !s.is_empty()) {
        let fname = templ.replace('?', &file);
        match l.load_file(&fname) {
            Ok(f) => return one(f),
            Err(Value::Str(e)) if e.to_str_lossy().starts_with("cannot open") => {
                msg.push_str(&format!("\n\tno file '{fname}'"));
            }
            Err(e) => {
                let e = e.to_str_coerce().map(|s| s.to_str_lossy()).unwrap_or_default();
                return Err(l.rt_error(&format!("error loading module '{name}' from file '{fname}':\n\t{e}")));
            }
        }
    }
    one(Value::str(&msg))
}

fn require(l: &mut Lua, a: &[Value]) -> LuaResult<Vec<Value>> {
    let name = l.check_str(a, 1)?;
    let loaded = l.loaded_table().ok_or_else(|| l.rt_error("no _LOADED"))?;
    let key = Value::Str(name.clone());
    let cur = l.heap.table(loaded).get(&key);
    if cur.truthy() {
        if matches!(cur, Value::LightUserData(SENTINEL)) {
            return Err(l.rt_error(&format!("loop or previous error loading module '{}'", name.to_str_lossy())));
        }
        return one(cur);
    }
    let Value::Table(p) = l.get_global("package") else { return Err(l.rt_error("'package.loaders' must be a table")) };
    let Value::Table(loaders) = l.heap.table(p).get_str("loaders") else {
        return Err(l.rt_error("'package.loaders' must be a table"));
    };
    let mut msg = String::new();
    let mut i = 1;
    let loader_fn = loop {
        let ld = l.heap.table(loaders).get_int(i);
        if ld.is_nil() {
            return Err(l.rt_error(&format!("module '{}' not found:{msg}", name.to_str_lossy())));
        }
        let r = l.call(ld, &[Value::Str(name.clone())])?.into_iter().next().unwrap_or_default();
        match r {
            Value::Func(_) => break r,
            Value::Str(s) => msg.push_str(&s.to_str_lossy()),
            Value::Num(n) => msg.push_str(&crate::number::tostring(n)),
            _ => {}
        }
        i += 1;
    };
    l.raw_set(loaded, key.clone(), Value::LightUserData(SENTINEL))?;
    let r = l.call(loader_fn, &[Value::Str(name.clone())])?.into_iter().next().unwrap_or_default();
    if !r.is_nil() {
        l.raw_set(loaded, key.clone(), r)?;
    }
    let v = l.heap.table(loaded).get(&key);
    if matches!(v, Value::LightUserData(SENTINEL)) {
        l.raw_set(loaded, key, Value::Bool(true))?;
        return one(true);
    }
    one(v)
}

fn find_table(l: &mut Lua, name: &str) -> Result<GcRef, ()> {
    let mut t = l.globals;
    for part in name.split('.') {
        let k = Value::str(part);
        match l.heap.table(t).get(&k) {
            Value::Nil => {
                let n = l.new_table(0, 0);
                let _ = l.heap.table_mut(t).set(k, Value::Table(n));
                t = n;
            }
            Value::Table(n) => t = n,
            _ => return Err(()),
        }
    }
    Ok(t)
}

fn module(l: &mut Lua, a: &[Value]) -> LuaResult<Vec<Value>> {
    let name = l.check_str(a, 1)?;
    let ns = name.to_str_lossy();
    let loaded = l.loaded_table().ok_or_else(|| l.rt_error("no _LOADED"))?;
    let key = Value::Str(name.clone());
    let m = match l.heap.table(loaded).get(&key) {
        Value::Table(m) => m,
        _ => {
            let Ok(m) = find_table(l, &ns) else {
                return Err(l.rt_error(&format!("name conflict for module '{ns}'")));
            };
            l.raw_set(loaded, key, Value::Table(m))?;
            m
        }
    };
    if l.heap.table(m).get_str("_NAME").is_nil() {
        let pkg = ns.rfind('.').map_or("", |i| &ns[..=i]).to_string();
        let tm = l.heap.table_mut(m);
        let _ = tm.set(Value::str("_M"), Value::Table(m));
        let _ = tm.set(Value::str("_NAME"), Value::Str(name.clone()));
        let _ = tm.set(Value::str("_PACKAGE"), Value::str(&pkg));
    }
    let n = l.th.frames.len();
    let caller = (n >= 2).then(|| l.th.frames[n - 2].func);
    match caller.map(|f| (f, l.heap.func_mut(f))) {
        Some((_, crate::heap::Function::Lua(c))) => c.env = m,
        _ => return Err(l.rt_error("'module' not called from a Lua function")),
    }
    for opt in a.iter().skip(1) {
        l.call(opt.clone(), &[Value::Table(m)])?;
    }
    none()
}
