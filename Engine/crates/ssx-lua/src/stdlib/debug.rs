use super::{arg, none, one};
use crate::heap::Function;
use crate::value::Value;
use crate::vm::Lua;

pub fn open(l: &mut Lua) {
    let t = l.lib_table("debug");
    l.lib_fn(t, "debug", |_, _| none());
    l.lib_fn(t, "getregistry", |l, _| one(Value::Table(l.registry)));
    l.lib_fn(t, "getmetatable", |l, a| {
        let v = l.check_any(a, 1)?.clone();
        one(l.metatable(&v).map_or(Value::Nil, Value::Table))
    });
    l.lib_fn(t, "setmetatable", |l, a| {
        let v = arg(a, 1).cloned().unwrap_or_default();
        let m = match arg(a, 2) {
            Some(Value::Table(m)) => Some(*m),
            None | Some(Value::Nil) => None,
            _ => return Err(l.arg_error(2, "nil or table expected")),
        };
        match &v {
            Value::Table(t) => l.heap.table_mut(*t).meta = m,
            Value::UserData(u) => l.heap.userdata_mut(*u).meta = m,
            other => {
                let tn = other.type_name();
                l.type_meta.retain(|(n, _)| *n != tn);
                if let Some(m) = m {
                    l.type_meta.push((tn, m));
                }
            }
        }
        one(true)
    });
    l.lib_fn(t, "getfenv", |l, a| match arg(a, 1) {
        Some(Value::Func(f)) => match l.heap.func(*f) {
            Function::Lua(c) => one(Value::Table(c.env)),
            Function::Native(c) => one(Value::Table(c.env)),
        },
        _ => one(Value::Nil),
    });
    l.lib_fn(t, "setfenv", |l, a| {
        let env = l.check_table(a, 2)?;
        if let Some(Value::Func(f)) = arg(a, 1) {
            match l.heap.func_mut(*f) {
                Function::Lua(c) => c.env = env,
                Function::Native(c) => c.env = env,
            }
            return one(arg(a, 1).cloned().unwrap_or_default());
        }
        Err(l.rt_error("'setfenv' cannot change environment of given object"))
    });
    l.lib_fn(t, "getupvalue", |l, a| {
        let n = l.check_int(a, 2)?;
        let Some(Value::Func(f)) = arg(a, 1) else { return Err(l.tag_error(1, "function", arg(a, 1))) };
        if let Function::Lua(c) = l.heap.func(*f)
            && n >= 1
            && (n as usize) <= c.upvals.len()
        {
            let i = n as usize - 1;
            let name = c.proto.upval_names.get(i).map_or(Value::str(""), |s| Value::Str(s.clone()));
            return Ok(vec![name, c.upvals[i].borrow().clone()]);
        }
        none()
    });
    l.lib_fn(t, "setupvalue", |l, a| {
        let n = l.check_int(a, 2)?;
        let v = l.check_any(a, 3)?.clone();
        let Some(Value::Func(f)) = arg(a, 1) else { return Err(l.tag_error(1, "function", arg(a, 1))) };
        if let Function::Lua(c) = l.heap.func(*f)
            && n >= 1
            && (n as usize) <= c.upvals.len()
        {
            let i = n as usize - 1;
            *c.upvals[i].borrow_mut() = v;
            return one(c.proto.upval_names.get(i).map_or(Value::str(""), |s| Value::Str(s.clone())));
        }
        none()
    });
    l.lib_fn(t, "gethook", |_, _| none());
    l.lib_fn(t, "sethook", |_, _| none());
    l.lib_fn(t, "getlocal", |_, _| none());
    l.lib_fn(t, "setlocal", |_, _| none());
    l.lib_fn(t, "getinfo", |l, a| {
        let f = match arg(a, 1) {
            Some(Value::Func(f)) => Some((*f, None)),
            Some(Value::Num(n)) => {
                let level = *n as usize;
                let frames = &l.th.frames;
                frames.len().checked_sub(level + 1).map(|i| (frames[i].func, Some(frames[i].pc)))
            }
            _ => return Err(l.arg_error(1, "function or level expected")),
        };
        let Some((f, pc)) = f else { return one(Value::Nil) };
        let t = l.new_table(0, 8);
        let set = |l: &mut Lua, k: &str, v: Value| {
            let _ = l.heap.table_mut(t).set(Value::str(k), v);
        };
        set(l, "func", Value::Func(f));
        let info = match l.heap.func(f) {
            Function::Lua(c) => Some((
                c.proto.source.to_string(),
                c.proto.line,
                c.proto.last_line,
                c.upvals.len(),
                pc.and_then(|pc| c.proto.lines.get(pc.saturating_sub(1)).copied()),
            )),
            Function::Native(_) => None,
        };
        match info {
            Some((src, line, last, nups, cur)) => {
                set(l, "source", Value::str(&src));
                set(l, "short_src", Value::str(&crate::parser::chunkid(&src)));
                set(l, "linedefined", Value::Num(f64::from(line)));
                set(l, "lastlinedefined", Value::Num(f64::from(last)));
                set(l, "what", Value::str(if line == 0 { "main" } else { "Lua" }));
                set(l, "nups", Value::Num(nups as f64));
                set(l, "currentline", Value::Num(cur.map_or(-1.0, f64::from)));
            }
            None => {
                set(l, "source", Value::str("=[C]"));
                set(l, "short_src", Value::str("[C]"));
                set(l, "what", Value::str("C"));
                set(l, "currentline", Value::Num(-1.0));
            }
        }
        one(Value::Table(t))
    });
    l.lib_fn(t, "traceback", |l, a| {
        let msg = l.opt_str(a, 1, "")?.to_str_lossy();
        let mut out = if msg.is_empty() { String::new() } else { format!("{msg}\n") };
        out.push_str("stack traceback:");
        for fr in l.th.frames.iter().rev().skip(1) {
            match l.heap.func(fr.func) {
                Function::Lua(c) => {
                    let line = c.proto.lines.get(fr.pc.saturating_sub(1)).copied().unwrap_or(0);
                    out.push_str(&format!(
                        "\n\t{}:{}: in function <{}:{}>",
                        crate::parser::chunkid(&c.proto.source),
                        line,
                        crate::parser::chunkid(&c.proto.source),
                        c.proto.line
                    ));
                }
                Function::Native(c) => out.push_str(&format!("\n\t[C]: in function '{}'", c.name)),
            }
        }
        one(out.as_str())
    });
}
