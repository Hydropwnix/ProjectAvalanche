use super::{Builder, R, arg};
use crate::avm1::object::{Kind, ObjId};
use crate::avm1::value::{Value, to_int32};
use crate::avm1::{Ctx, Fault};

const CASEINSENSITIVE: i32 = 1;
const DESCENDING: i32 = 2;
const UNIQUESORT: i32 = 4;
const RETURNINDEXEDARRAY: i32 = 8;
const NUMERIC: i32 = 16;

pub(crate) fn install(b: &mut Builder, global: ObjId) -> ObjId {
    let proto = b.heap.alloc(crate::avm1::object::Object::new(Value::Object(b.object_proto), Kind::Array(Vec::new())));
    let ctor = b.class(global, "Array", array_ctor, proto);
    for (n, v) in [
        ("CASEINSENSITIVE", CASEINSENSITIVE),
        ("DESCENDING", DESCENDING),
        ("UNIQUESORT", UNIQUESORT),
        ("RETURNINDEXEDARRAY", RETURNINDEXEDARRAY),
        ("NUMERIC", NUMERIC),
    ] {
        b.value(ctor, n, Value::from(v));
    }
    b.methods(
        proto,
        &[
            ("push", push),
            ("pop", pop),
            ("shift", shift),
            ("unshift", unshift),
            ("splice", splice),
            ("slice", slice),
            ("concat", concat),
            ("join", join),
            ("toString", to_string),
            ("reverse", reverse),
            ("sort", sort),
            ("sortOn", sort_on),
        ],
    );
    proto
}

fn array_ctor(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let items = match args {
        [Value::Number(n)] => {
            let n = if n.is_finite() && *n >= 0.0 { n.min(f64::from(1u32 << 24)) as usize } else { 0 };
            cx.grew_array(n, n)?;
            vec![Value::Undefined; n]
        }
        _ => args.to_vec(),
    };
    if cx.vm.constructing
        && let Some(o) = this.as_object().and_then(|o| cx.vm.heap.get_mut(o))
    {
        o.kind = Kind::Array(items);
        return Ok(this);
    }
    Ok(Value::Object(cx.new_array(items)))
}

fn elems(cx: &mut Ctx, this: &Value) -> Option<Vec<Value>> {
    let a = match cx.kind_of(this) {
        Some(Kind::Array(a)) => a.clone(),
        _ => return None,
    };
    let _ = cx.walked(a.len() / 4);
    Some(a)
}

fn with_elems<T>(cx: &mut Ctx, this: &Value, f: impl FnOnce(&mut Vec<Value>) -> T) -> Option<T> {
    match this.as_object().and_then(|o| cx.vm.heap.get_mut(o)).map(|o| &mut o.kind) {
        Some(Kind::Array(a)) => Some(f(a)),
        _ => None,
    }
}

fn push(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let len = with_elems(cx, &this, |a| a.len()).unwrap_or(0);
    cx.grew_array(len + args.len(), args.len())?;
    let n = with_elems(cx, &this, |a| {
        a.extend_from_slice(args);
        a.len()
    });
    Ok(n.map_or(Value::Undefined, |n| Value::Number(n as f64)))
}

fn pop(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(with_elems(cx, &this, |a| a.pop()).flatten().unwrap_or_default())
}

fn shift(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(with_elems(cx, &this, |a| if a.is_empty() { None } else { Some(a.remove(0)) }).flatten().unwrap_or_default())
}

fn unshift(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let len = with_elems(cx, &this, |a| a.len()).unwrap_or(0);
    cx.grew_array(len + args.len(), args.len() + len / 4)?;
    let n = with_elems(cx, &this, |a| {
        a.splice(0..0, args.iter().cloned());
        a.len()
    });
    Ok(n.map_or(Value::Undefined, |n| Value::Number(n as f64)))
}

fn rel(cx: &mut Ctx, v: &Value, len: usize, default: usize) -> usize {
    if v.is_undefined() {
        return default;
    }
    let n = to_int32(cx.to_number(v)) as i64;
    let len = len as i64;
    (if n < 0 { (len + n).max(0) } else { n.min(len) }) as usize
}

fn splice(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let Some(a) = elems(cx, &this) else { return Ok(Value::Undefined) };
    let start = rel(cx, &arg(args, 0), a.len(), 0);
    let count = if args.len() > 1 {
        (to_int32(cx.to_number(&args[1])).max(0) as usize).min(a.len() - start)
    } else {
        a.len() - start
    };
    let insert = args.get(2..).unwrap_or(&[]).to_vec();
    cx.grew_array(a.len() - count + insert.len(), insert.len())?;
    let removed =
        with_elems(cx, &this, |a| a.splice(start..start + count, insert).collect::<Vec<_>>()).unwrap_or_default();
    Ok(Value::Object(cx.new_array(removed)))
}

fn slice(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let Some(a) = elems(cx, &this) else { return Ok(Value::Undefined) };
    let s = rel(cx, &arg(args, 0), a.len(), 0);
    let e = rel(cx, &arg(args, 1), a.len(), a.len());
    let part = if s < e { a[s..e].to_vec() } else { Vec::new() };
    Ok(Value::Object(cx.new_array(part)))
}

fn concat(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let mut out = elems(cx, &this).unwrap_or_default();
    for v in args {
        let more = match elems(cx, v) {
            Some(x) => x,
            None => vec![v.clone()],
        };
        cx.grew_array(out.len() + more.len(), more.len())?;
        out.extend(more);
    }
    Ok(Value::Object(cx.new_array(out)))
}

pub(crate) fn join_values(cx: &mut Ctx, a: &[Value], sep: &str) -> Result<String, Fault> {
    let mut s = String::new();
    for (i, v) in a.iter().enumerate() {
        let x = cx.to_string(v)?;
        cx.made_string(s.len() + sep.len() + x.len())?;
        if i > 0 {
            s.push_str(sep);
        }
        s.push_str(&x);
    }
    Ok(s)
}

fn join(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let Some(a) = elems(cx, &this) else { return Ok(Value::Undefined) };
    let sep = match arg(args, 0) {
        Value::Undefined => ",".into(),
        v => cx.to_string(&v)?.to_string(),
    };
    Ok(join_values(cx, &a, &sep)?.into())
}

fn to_string(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    let Some(a) = elems(cx, &this) else { return Ok(Value::str("[object Object]")) };
    Ok(join_values(cx, &a, ",")?.into())
}

fn reverse(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    with_elems(cx, &this, |a| a.reverse());
    Ok(this)
}

use std::cmp::Ordering;

type CmpFn<'a> = dyn FnMut(&mut Ctx, &Value, &Value) -> Result<Ordering, Fault> + 'a;

fn merge_sort(cx: &mut Ctx, v: Vec<(usize, Value)>, cmp: &mut CmpFn) -> Result<Vec<(usize, Value)>, Fault> {
    if v.len() <= 1 {
        return Ok(v);
    }
    let mut right = v;
    let left = right.drain(..right.len() / 2).collect();
    let left = merge_sort(cx, left, cmp)?;
    let right = merge_sort(cx, right, cmp)?;
    let mut out = Vec::with_capacity(left.len() + right.len());
    let (mut l, mut r) = (left.into_iter().peekable(), right.into_iter().peekable());
    while let (Some(a), Some(b)) = (l.peek(), r.peek()) {
        if cmp(cx, &b.1, &a.1)? == Ordering::Less {
            out.extend(r.next());
        } else {
            out.extend(l.next());
        }
    }
    out.extend(l);
    out.extend(r);
    Ok(out)
}

fn compare_values(cx: &mut Ctx, a: &Value, b: &Value, opts: i32) -> Result<Ordering, Fault> {
    let o = if opts & NUMERIC != 0 {
        let (x, y) = (cx.to_number(a), cx.to_number(b));
        x.partial_cmp(&y).unwrap_or(Ordering::Equal)
    } else {
        let (mut x, mut y) = (cx.to_string(a)?.to_string(), cx.to_string(b)?.to_string());
        if opts & CASEINSENSITIVE != 0 {
            x = x.to_lowercase();
            y = y.to_lowercase();
        }
        x.cmp(&y)
    };
    Ok(if opts & DESCENDING != 0 { o.reverse() } else { o })
}

fn finish_sort(cx: &mut Ctx, this: &Value, opts: i32, cmp: &mut CmpFn) -> R {
    let Some(a) = elems(cx, this) else { return Ok(Value::Undefined) };
    let sorted = merge_sort(cx, a.into_iter().enumerate().collect(), cmp)?;
    if opts & UNIQUESORT != 0 {
        for w in sorted.windows(2) {
            if cmp(cx, &w[0].1, &w[1].1)? == Ordering::Equal {
                return Ok(Value::Number(0.0));
            }
        }
    }
    if opts & RETURNINDEXEDARRAY != 0 {
        let idx = sorted.iter().map(|(i, _)| Value::Number(*i as f64)).collect();
        return Ok(Value::Object(cx.new_array(idx)));
    }
    with_elems(cx, this, |a| *a = sorted.into_iter().map(|(_, v)| v).collect());
    Ok(this.clone())
}

fn sort(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let f = arg(args, 0);
    if cx.is_function(&f) {
        let opts = if args.len() > 1 { to_int32(cx.to_number(&args[1])) } else { 0 };
        let mut cmp = |cx: &mut Ctx, a: &Value, b: &Value| -> Result<Ordering, Fault> {
            let r = cx.call(&f, Value::Undefined, &[a.clone(), b.clone()], Value::Undefined)?;
            let n = cx.to_number(&r);
            let o = if n < 0.0 {
                Ordering::Less
            } else if n > 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            };
            Ok(if opts & DESCENDING != 0 { o.reverse() } else { o })
        };
        return finish_sort(cx, &this, opts, &mut cmp);
    }
    let opts = if f.is_undefined() { 0 } else { to_int32(cx.to_number(&f)) };
    let mut cmp = |cx: &mut Ctx, a: &Value, b: &Value| compare_values(cx, a, b, opts);
    finish_sort(cx, &this, opts, &mut cmp)
}

fn sort_on(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let names: Vec<Value> = match elems(cx, &arg(args, 0)) {
        Some(a) => a,
        None => vec![arg(args, 0)],
    };
    let mut fields = Vec::new();
    for n in &names {
        fields.push(cx.to_string(n)?);
    }
    let opt_list: Vec<i32> = match elems(cx, &arg(args, 1)) {
        Some(a) => a.iter().map(|v| to_int32(cx.to_number(v))).collect(),
        None => vec![to_int32(cx.to_number(&arg(args, 1)))],
    };
    let first = opt_list.first().copied().unwrap_or(0);
    let mut cmp = |cx: &mut Ctx, a: &Value, b: &Value| -> Result<Ordering, Fault> {
        for (i, f) in fields.iter().enumerate() {
            let o = *opt_list.get(i).unwrap_or(&first);
            let (x, y) = (cx.get_member(a, f)?, cx.get_member(b, f)?);
            let r = compare_values(cx, &x, &y, o)?;
            if r != Ordering::Equal {
                return Ok(r);
            }
        }
        Ok(Ordering::Equal)
    };
    finish_sort(cx, &this, first, &mut cmp)
}
