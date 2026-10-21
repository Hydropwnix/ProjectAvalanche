use super::{arg, one};
use crate::heap::Function;
use crate::value::{GcRef, Value};
use crate::vm::{CoStatus, Lua, LuaError, LuaResult, MAXCCALLS};

pub fn open(l: &mut Lua) {
    let t = l.lib_table("coroutine");
    l.lib_fn(t, "create", |l, a| {
        let f = match arg(a, 1) {
            Some(f @ Value::Func(r)) if matches!(l.heap.func(*r), Function::Lua(_)) => f.clone(),
            _ => return Err(l.arg_error(1, "Lua function expected")),
        };
        one(Value::Thread(l.new_thread(f)))
    });
    l.lib_fn(t, "resume", |l, a| {
        let co = match arg(a, 1) {
            Some(Value::Thread(r)) => *r,
            v => return Err(l.tag_error(1, "coroutine", v)),
        };
        match resume(l, co, a[1..].to_vec()) {
            Ok(mut v) => {
                v.insert(0, Value::Bool(true));
                Ok(v)
            }
            Err(e) => Ok(vec![Value::Bool(false), e]),
        }
    });
    l.lib_fn(t, "running", |l, _| if l.is_main() { one(Value::Nil) } else { one(Value::Thread(l.cur)) });
    l.lib_fn(t, "status", |l, a| {
        let co = match arg(a, 1) {
            Some(Value::Thread(r)) => *r,
            v => return Err(l.tag_error(1, "coroutine", v)),
        };
        one(status(l, co))
    });
    l.lib_fn(t, "wrap", |l, a| {
        let f = match arg(a, 1) {
            Some(f @ Value::Func(r)) if matches!(l.heap.func(*r), Function::Lua(_)) => f.clone(),
            _ => return Err(l.arg_error(1, "Lua function expected")),
        };
        let co = Value::Thread(l.new_thread(f));
        let w = l.create_function_with(
            "wrap",
            std::rc::Rc::new(|l: &mut Lua, a: &[Value]| {
                let Value::Thread(co) = l.upvalue(0) else { return Err(l.rt_error("wrap: no thread")) };
                match resume(l, co, a.to_vec()) {
                    Ok(v) => Ok(v),
                    Err(e) => {
                        let e = match e {
                            Value::Str(s) => Value::str(&format!("{}{}", l.where_(2), s.to_str_lossy())),
                            Value::Num(n) => Value::str(&format!("{}{}", l.where_(2), crate::number::tostring(n))),
                            other => other,
                        };
                        Err(LuaError::Error(e))
                    }
                }
            }),
            vec![co],
        );
        one(w)
    });
    l.lib_fn(t, "yield", |l, a| {
        if l.th.nny > 0 || l.is_main() {
            return Err(l.rt_error("attempt to yield across metamethod/C-call boundary"));
        }
        Err(LuaError::Yield(a.to_vec()))
    });
}

pub fn status(l: &Lua, co: GcRef) -> &'static str {
    if co == l.cur {
        return "running";
    }
    match l.heap.thread(co).status {
        Some(CoStatus::Suspended) => "suspended",
        Some(CoStatus::Normal) | Some(CoStatus::Running) => "normal",
        _ => "dead",
    }
}

pub fn resume(l: &mut Lua, co: GcRef, args: Vec<Value>) -> Result<Vec<Value>, Value> {
    match status(l, co) {
        "suspended" => {}
        "dead" => return Err(Value::str("cannot resume dead coroutine")),
        _ => return Err(Value::str("cannot resume non-suspended coroutine")),
    }
    if l.ccalls >= MAXCCALLS {
        return Err(Value::str("C stack overflow"));
    }
    let mut co_state = std::mem::take(l.heap.thread_mut(co));
    co_state.status = Some(CoStatus::Running);
    let mut prev = std::mem::replace(&mut l.th, co_state);
    prev.status = Some(CoStatus::Normal);
    let prev_ref = l.cur;
    *l.heap.thread_mut(prev_ref) = prev;
    l.push_resumer(prev_ref);
    l.cur = co;
    l.ccalls += 1;
    let r = run_coroutine(l, args);
    l.ccalls -= 1;
    let prev_ref = l.pop_resumer().unwrap_or(l.main);
    let mut prev = std::mem::take(l.heap.thread_mut(prev_ref));
    prev.status = Some(CoStatus::Running);
    let mut co_state = std::mem::replace(&mut l.th, prev);
    l.cur = prev_ref;
    let out = match r {
        Ok(v) => {
            co_state.status = Some(CoStatus::Dead);
            Ok(v)
        }
        Err(LuaError::Yield(v)) => {
            co_state.status = Some(CoStatus::Suspended);
            Ok(v)
        }
        Err(LuaError::Error(e)) => {
            co_state.status = Some(CoStatus::Dead);
            co_state.frames.clear();
            co_state.stack.clear();
            co_state.cells.clear();
            Err(e)
        }
    };
    *l.heap.thread_mut(co) = co_state;
    out
}

fn run_coroutine(l: &mut Lua, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    if let Some(f) = l.th.start.take() {
        let n = args.len();
        l.th.stack.push(f);
        l.th.stack.extend(args);
        if l.precall(0, n, -1, None, true)? {
            l.run()?;
        }
        return Ok(l.th.stack.split_off(0));
    }
    let Some(p) = l.th.pending.take() else {
        return Err(LuaError::Error(Value::str("cannot resume non-suspended coroutine")));
    };
    l.th.stack.truncate(p.func);
    l.th.cells.truncate(p.func);
    l.push_results(args, p.results);
    l.run()?;
    Ok(l.th.stack.split_off(0))
}
