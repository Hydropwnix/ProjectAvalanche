use super::{arg, none, one};
use crate::value::{GcRef, Value};
use crate::vm::{Lua, LuaResult};

fn getn(l: &Lua, t: GcRef) -> i64 {
    l.heap.table(t).length()
}

fn geti(l: &Lua, t: GcRef, i: i64) -> Value {
    l.heap.table(t).get_int(i)
}

fn seti(l: &mut Lua, t: GcRef, i: i64, v: Value) -> LuaResult<()> {
    l.raw_set(t, Value::Num(i as f64), v)
}

pub fn open(l: &mut Lua) {
    let t = l.lib_table("table");
    l.lib_fn(t, "concat", |l, a| {
        let sep = l.opt_str(a, 2, "")?;
        let t = l.check_table(a, 1)?;
        let i = l.opt_int(a, 3, 1)?;
        let last = match arg(a, 4) {
            None | Some(Value::Nil) => getn(l, t) as i32,
            _ => l.check_int(a, 4)?,
        };
        let mut out = Vec::new();
        let mut k = i;
        while k <= last {
            match geti(l, t, i64::from(k)).to_str_coerce() {
                Some(s) => out.extend_from_slice(s.bytes()),
                None => return Err(l.arg_error(1, "table contains non-strings")),
            }
            if k != last {
                out.extend_from_slice(sep.bytes());
            }
            k += 1;
        }
        one(Value::bytes(&out))
    });
    l.lib_fn(t, "foreach", |l, a| {
        let t = l.check_table(a, 1)?;
        let f = l.check_func(a, 2)?;
        let mut k = Value::Nil;
        loop {
            let Ok(Some((nk, v))) = l.heap.table(t).next(&k) else { return none() };
            let r = l.call(f.clone(), &[nk.clone(), v])?.into_iter().next().unwrap_or_default();
            if !r.is_nil() {
                return one(r);
            }
            k = nk;
        }
    });
    l.lib_fn(t, "foreachi", |l, a| {
        let t = l.check_table(a, 1)?;
        let f = l.check_func(a, 2)?;
        let n = getn(l, t);
        for i in 1..=n {
            let v = geti(l, t, i);
            let r = l.call(f.clone(), &[Value::Num(i as f64), v])?.into_iter().next().unwrap_or_default();
            if !r.is_nil() {
                return one(r);
            }
        }
        none()
    });
    l.lib_fn(t, "getn", |l, a| {
        let t = l.check_table(a, 1)?;
        one(getn(l, t) as f64)
    });
    l.lib_fn(t, "maxn", |l, a| {
        let t = l.check_table(a, 1)?;
        let mut max = 0.0f64;
        let mut k = Value::Nil;
        while let Ok(Some((nk, _))) = l.heap.table(t).next(&k) {
            if let Value::Num(n) = nk
                && n > max
            {
                max = n;
            }
            k = nk;
        }
        one(max)
    });
    l.lib_fn(t, "insert", |l, a| {
        let t = l.check_table(a, 1)?;
        let mut e = getn(l, t) + 1;
        let pos = match a.len() {
            2 => e,
            3 => {
                let pos = i64::from(l.check_int(a, 2)?);
                if pos > e {
                    e = pos;
                }
                let mut i = e;
                while i > pos {
                    let v = geti(l, t, i - 1);
                    seti(l, t, i, v)?;
                    i -= 1;
                }
                pos
            }
            _ => return Err(l.rt_error("wrong number of arguments to 'insert'")),
        };
        let v = a.last().cloned().unwrap_or_default();
        seti(l, t, pos, v)?;
        none()
    });
    l.lib_fn(t, "remove", |l, a| {
        let t = l.check_table(a, 1)?;
        let e = getn(l, t);
        let mut pos = i64::from(l.opt_int(a, 2, e as i32)?);
        if e == 0 {
            return none();
        }
        let r = geti(l, t, pos);
        while pos < e {
            let v = geti(l, t, pos + 1);
            seti(l, t, pos, v)?;
            pos += 1;
        }
        seti(l, t, e, Value::Nil)?;
        one(r)
    });
    l.lib_fn(t, "setn", |l, a| {
        l.check_table(a, 1)?;
        Err(l.rt_error("'setn' is obsolete"))
    });
    l.lib_fn(t, "sort", |l, a| {
        let t = l.check_table(a, 1)?;
        let n = getn(l, t);
        let cmp = match arg(a, 2) {
            None | Some(Value::Nil) => None,
            Some(f @ Value::Func(_)) => Some(f.clone()),
            v => return Err(l.tag_error(2, "function", v)),
        };
        auxsort(l, t, cmp.as_ref(), 1, n)?;
        none()
    });
}

fn less(l: &mut Lua, cmp: Option<&Value>, a: &Value, b: &Value) -> LuaResult<bool> {
    match cmp {
        Some(f) => Ok(l.call(f.clone(), &[a.clone(), b.clone()])?.first().is_some_and(Value::truthy)),
        None => l.less_than(a, b),
    }
}

fn swap(l: &mut Lua, t: GcRef, i: i64, j: i64) -> LuaResult<()> {
    let (a, b) = (geti(l, t, i), geti(l, t, j));
    seti(l, t, i, b)?;
    seti(l, t, j, a)
}

fn auxsort(l: &mut Lua, t: GcRef, cmp: Option<&Value>, lo: i64, up: i64) -> LuaResult<()> {
    let (mut lo, mut up) = (lo, up);
    while lo < up {
        let (al, au) = (geti(l, t, lo), geti(l, t, up));
        if less(l, cmp, &au, &al)? {
            swap(l, t, lo, up)?;
        }
        if up - lo == 1 {
            break;
        }
        let mut i = (lo + up) / 2;
        let (ai, al) = (geti(l, t, i), geti(l, t, lo));
        if less(l, cmp, &ai, &al)? {
            swap(l, t, i, lo)?;
        } else {
            let au = geti(l, t, up);
            if less(l, cmp, &au, &ai)? {
                swap(l, t, i, up)?;
            }
        }
        if up - lo == 2 {
            break;
        }
        let p = geti(l, t, i);
        l.th.stack.push(p.clone());
        swap(l, t, i, up - 1)?;
        i = lo;
        let mut j = up - 1;
        loop {
            loop {
                i += 1;
                let ai = geti(l, t, i);
                if !less(l, cmp, &ai, &p)? {
                    break;
                }
                if i > up {
                    return Err(l.rt_error("invalid order function for sorting"));
                }
            }
            loop {
                j -= 1;
                let aj = geti(l, t, j);
                if !less(l, cmp, &p, &aj)? {
                    break;
                }
                if j < lo {
                    return Err(l.rt_error("invalid order function for sorting"));
                }
            }
            if j < i {
                break;
            }
            swap(l, t, i, j)?;
        }
        l.th.stack.pop();
        swap(l, t, up - 1, i)?;
        let (sj, si);
        if i - lo < up - i {
            sj = lo;
            si = i - 1;
            lo = i + 1;
        } else {
            sj = i + 1;
            si = up;
            up = i - 1;
        }
        auxsort(l, t, cmp, sj, si)?;
    }
    Ok(())
}
