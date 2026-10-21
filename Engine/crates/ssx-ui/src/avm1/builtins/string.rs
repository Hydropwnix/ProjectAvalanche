use super::{Builder, R, arg};
use crate::avm1::Ctx;
use crate::avm1::object::{Kind, ObjId, Object};
use crate::avm1::value::{Value, number_to_radix, to_int32};

pub(crate) fn install(b: &mut Builder, global: ObjId) -> (ObjId, ObjId, ObjId, ObjId) {
    let string = b.heap.alloc(Object::new(Value::Object(b.object_proto), Kind::Boxed(Value::str(""))));
    let sc = b.class(global, "String", string_ctor, string);
    b.method(sc, "fromCharCode", from_char_code);
    b.methods(
        string,
        &[
            ("toString", string_value),
            ("valueOf", string_value),
            ("charAt", char_at),
            ("charCodeAt", char_code_at),
            ("indexOf", index_of),
            ("lastIndexOf", last_index_of),
            ("substr", substr),
            ("substring", substring),
            ("slice", slice),
            ("split", split),
            ("toLowerCase", to_lower),
            ("toUpperCase", to_upper),
            ("concat", concat),
        ],
    );
    let number = b.heap.alloc(Object::new(Value::Object(b.object_proto), Kind::Boxed(Value::Number(0.0))));
    let nc = b.class(global, "Number", number_ctor, number);
    for (n, v) in [
        ("MAX_VALUE", f64::MAX),
        ("MIN_VALUE", 5e-324),
        ("NaN", f64::NAN),
        ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
        ("POSITIVE_INFINITY", f64::INFINITY),
    ] {
        b.value(nc, n, Value::Number(v));
    }
    b.methods(number, &[("toString", number_to_string), ("valueOf", boxed_value)]);
    let boolean = b.heap.alloc(Object::new(Value::Object(b.object_proto), Kind::Boxed(Value::Bool(false))));
    b.class(global, "Boolean", boolean_ctor, boolean);
    b.methods(boolean, &[("toString", boolean_to_string), ("valueOf", boxed_value)]);

    let math = b.obj();
    b.value(global, "Math", Value::Object(math));
    for (n, v) in [
        ("PI", std::f64::consts::PI),
        ("E", std::f64::consts::E),
        ("LN2", std::f64::consts::LN_2),
        ("LN10", std::f64::consts::LN_10),
        ("LOG2E", std::f64::consts::LOG2_E),
        ("LOG10E", std::f64::consts::LOG10_E),
        ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
        ("SQRT2", std::f64::consts::SQRT_2),
    ] {
        b.value(math, n, Value::Number(v));
    }
    b.methods(
        math,
        &[
            ("abs", |cx, _, a| m1(cx, a, f64::abs)),
            ("ceil", |cx, _, a| m1(cx, a, f64::ceil)),
            ("floor", |cx, _, a| m1(cx, a, f64::floor)),
            ("round", |cx, _, a| m1(cx, a, |x| (x + 0.5).floor())),
            ("sqrt", |cx, _, a| m1(cx, a, f64::sqrt)),
            ("sin", |cx, _, a| m1(cx, a, f64::sin)),
            ("cos", |cx, _, a| m1(cx, a, f64::cos)),
            ("tan", |cx, _, a| m1(cx, a, f64::tan)),
            ("asin", |cx, _, a| m1(cx, a, f64::asin)),
            ("acos", |cx, _, a| m1(cx, a, f64::acos)),
            ("atan", |cx, _, a| m1(cx, a, f64::atan)),
            ("exp", |cx, _, a| m1(cx, a, f64::exp)),
            ("log", |cx, _, a| m1(cx, a, f64::ln)),
            ("atan2", |cx, _, a| m2(cx, a, f64::atan2)),
            ("pow", |cx, _, a| m2(cx, a, f64::powf)),
            ("min", math_min),
            ("max", math_max),
            ("random", math_random),
        ],
    );

    let date = b.heap.alloc(Object::new(Value::Object(b.object_proto), Kind::Date(0.0)));
    let dc = b.class(global, "Date", date_ctor, date);
    b.method(dc, "UTC", date_utc);
    b.methods(
        date,
        &[("getTime", date_value), ("valueOf", date_value), ("setTime", date_set_time), ("toString", date_to_string)],
    );
    for (name, f) in DATE_FIELDS {
        b.method(date, name, *f);
    }
    (string, number, boolean, date)
}

fn m1(cx: &mut Ctx, a: &[Value], f: fn(f64) -> f64) -> R {
    Ok(Value::Number(f(cx.to_number(&arg(a, 0)))))
}

fn m2(cx: &mut Ctx, a: &[Value], f: fn(f64, f64) -> f64) -> R {
    let x = cx.to_number(&arg(a, 0));
    let y = cx.to_number(&arg(a, 1));
    Ok(Value::Number(f(x, y)))
}

fn math_min(cx: &mut Ctx, _: Value, a: &[Value]) -> R {
    let mut r = f64::INFINITY;
    for v in a {
        let n = cx.to_number(v);
        if n.is_nan() {
            return Ok(Value::Number(f64::NAN));
        }
        r = r.min(n);
    }
    Ok(Value::Number(r))
}

fn math_max(cx: &mut Ctx, _: Value, a: &[Value]) -> R {
    let mut r = f64::NEG_INFINITY;
    for v in a {
        let n = cx.to_number(v);
        if n.is_nan() {
            return Ok(Value::Number(f64::NAN));
        }
        r = r.max(n);
    }
    Ok(Value::Number(r))
}

fn math_random(cx: &mut Ctx, _: Value, _: &[Value]) -> R {
    Ok(Value::Number(f64::from(cx.vm.random() >> 8) / f64::from(1u32 << 24)))
}

pub(crate) fn box_primitive(cx: &mut Ctx, v: Value) -> Value {
    let proto = match v {
        Value::String(_) => cx.vm.protos.string,
        Value::Number(_) => cx.vm.protos.number,
        Value::Bool(_) => cx.vm.protos.boolean,
        _ => return v,
    };
    Value::Object(cx.new_object_with(proto, Kind::Boxed(v)))
}

fn this_primitive(cx: &Ctx, this: &Value) -> Value {
    match cx.kind_of(this) {
        Some(Kind::Boxed(v)) => v.clone(),
        Some(_) => this.clone(),
        None => this.clone(),
    }
}

fn this_string(cx: &mut Ctx, this: &Value) -> Result<Vec<char>, crate::avm1::Fault> {
    let p = this_primitive(cx, this);
    Ok(cx.to_string(&p)?.chars().collect())
}

fn string_ctor(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let s = if args.is_empty() { Value::str("") } else { Value::String(cx.to_string(&args[0])?) };
    if cx.vm.constructing
        && let Some(o) = this.as_object().and_then(|o| cx.vm.heap.get_mut(o))
    {
        o.kind = Kind::Boxed(s);
        return Ok(this);
    }
    Ok(s)
}

fn string_value(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    let p = this_primitive(cx, &this);
    Ok(Value::String(cx.to_string(&p)?))
}

fn boxed_value(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(this_primitive(cx, &this))
}

fn from_char_code(cx: &mut Ctx, _: Value, a: &[Value]) -> R {
    let mut s = String::new();
    for v in a {
        if let Some(c) = char::from_u32(to_int32(cx.to_number(v)) as u32 & 0xFFFF) {
            s.push(c);
        }
    }
    Ok(s.into())
}

fn char_at(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s = this_string(cx, &this)?;
    let i = to_int32(cx.to_number(&arg(a, 0)));
    Ok(usize::try_from(i).ok().and_then(|i| s.get(i)).map_or_else(String::new, |c| c.to_string()).into())
}

fn char_code_at(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s = this_string(cx, &this)?;
    let i = to_int32(cx.to_number(&arg(a, 0)));
    Ok(Value::Number(usize::try_from(i).ok().and_then(|i| s.get(i)).map_or(f64::NAN, |c| f64::from(u32::from(*c)))))
}

fn find(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    (from..hay.len()).find(|&i| hay[i..].starts_with(needle))
}

fn index_of(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s = this_string(cx, &this)?;
    let n: Vec<char> = cx.to_string(&arg(a, 0))?.chars().collect();
    let from = if a.len() > 1 { to_int32(cx.to_number(&a[1])).max(0) as usize } else { 0 };
    Ok(Value::Number(find(&s, &n, from).map_or(-1.0, |i| i as f64)))
}

fn last_index_of(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s = this_string(cx, &this)?;
    let n: Vec<char> = cx.to_string(&arg(a, 0))?.chars().collect();
    let max = if a.len() > 1 { to_int32(cx.to_number(&a[1])).max(0) as usize } else { s.len() };
    let r = (0..=max.min(s.len())).rev().find(|&i| s[i..].starts_with(&n));
    Ok(Value::Number(r.map_or(-1.0, |i| i as f64)))
}

fn sub(s: &[char], a: usize, b: usize) -> Value {
    if a >= b { Value::str("") } else { s[a..b].iter().collect::<String>().into() }
}

fn substr(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s = this_string(cx, &this)?;
    let len = s.len() as i64;
    let mut start = i64::from(to_int32(cx.to_number(&arg(a, 0))));
    if start < 0 {
        start = (len + start).max(0);
    }
    let start = start.min(len);
    let count = if a.len() > 1 && !a[1].is_undefined() { i64::from(to_int32(cx.to_number(&a[1]))) } else { len };
    let end = (start + count.max(0)).min(len);
    Ok(sub(&s, start as usize, end as usize))
}

fn substring(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s = this_string(cx, &this)?;
    let len = s.len() as i64;
    let x = i64::from(to_int32(cx.to_number(&arg(a, 0)))).clamp(0, len);
    let y =
        if a.len() > 1 && !a[1].is_undefined() { i64::from(to_int32(cx.to_number(&a[1]))).clamp(0, len) } else { len };
    Ok(sub(&s, x.min(y) as usize, x.max(y) as usize))
}

fn slice(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s = this_string(cx, &this)?;
    let len = s.len() as i64;
    let r = |v: i64| if v < 0 { (len + v).max(0) } else { v.min(len) };
    let x = r(i64::from(to_int32(cx.to_number(&arg(a, 0)))));
    let y = if a.len() > 1 && !a[1].is_undefined() { r(i64::from(to_int32(cx.to_number(&a[1])))) } else { len };
    Ok(sub(&s, x as usize, y.max(x) as usize))
}

fn split(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let s: String = this_string(cx, &this)?.into_iter().collect();
    let parts: Vec<Value> = match arg(a, 0) {
        Value::Undefined => vec![s.into()],
        d => {
            let d = cx.to_string(&d)?;
            if d.is_empty() {
                s.chars().map(|c| c.to_string().into()).collect()
            } else {
                s.split(&*d).map(Value::from).collect()
            }
        }
    };
    let limit =
        if a.len() > 1 && !a[1].is_undefined() { to_int32(cx.to_number(&a[1])).max(0) as usize } else { usize::MAX };
    Ok(Value::Object(cx.new_array(parts.into_iter().take(limit).collect())))
}

fn to_lower(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(this_string(cx, &this)?.into_iter().collect::<String>().to_lowercase().into())
}

fn to_upper(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(this_string(cx, &this)?.into_iter().collect::<String>().to_uppercase().into())
}

fn concat(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let mut s: String = this_string(cx, &this)?.into_iter().collect();
    for v in a {
        s.push_str(&cx.to_string(v)?);
    }
    Ok(s.into())
}

fn number_ctor(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let n = Value::Number(if args.is_empty() { 0.0 } else { cx.to_number(&args[0]) });
    if cx.vm.constructing
        && let Some(o) = this.as_object().and_then(|o| cx.vm.heap.get_mut(o))
    {
        o.kind = Kind::Boxed(n);
        return Ok(this);
    }
    Ok(n)
}

fn number_to_string(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let p = this_primitive(cx, &this);
    let n = cx.to_number(&p);
    let radix = if a.is_empty() { 10 } else { to_int32(cx.to_number(&a[0])) };
    Ok(number_to_radix(n, radix).into())
}

fn boolean_ctor(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let b = Value::Bool(!args.is_empty() && cx.to_bool(&args[0]));
    if cx.vm.constructing
        && let Some(o) = this.as_object().and_then(|o| cx.vm.heap.get_mut(o))
    {
        o.kind = Kind::Boxed(b);
        return Ok(this);
    }
    Ok(b)
}

fn boolean_to_string(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    let p = this_primitive(cx, &this);
    Ok(Value::String(cx.to_string(&p)?))
}

fn now_ms(cx: &Ctx) -> f64 {
    1_330_387_200_000.0 + cx.vm.timer_ms()
}

fn date_ctor(cx: &mut Ctx, this: Value, args: &[Value]) -> R {
    let t = match args {
        [] => now_ms(cx),
        [v] => cx.to_number(v),
        _ => {
            let mut f = [1970.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
            for (i, v) in args.iter().take(7).enumerate() {
                f[i] = cx.to_number(v);
            }
            civil_to_ms(f)
        }
    };
    if cx.vm.constructing
        && let Some(o) = this.as_object().and_then(|o| cx.vm.heap.get_mut(o))
    {
        o.kind = Kind::Date(t);
        return Ok(this);
    }
    Ok(Value::Number(t))
}

fn date_utc(cx: &mut Ctx, _: Value, args: &[Value]) -> R {
    let mut f = [1970.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    for (i, v) in args.iter().take(7).enumerate() {
        f[i] = cx.to_number(v);
    }
    Ok(Value::Number(civil_to_ms(f)))
}

fn civil_to_ms(f: [f64; 7]) -> f64 {
    let (y, m) = (f[0] as i64 + (f[1] as i64).div_euclid(12), (f[1] as i64).rem_euclid(12) + 1);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days as f64 + f[2] - 1.0) * 24.0 + f[3]) * 3_600_000.0 + f[4] * 60_000.0 + f[5] * 1000.0 + f[6]
}

fn ms_to_civil(t: f64) -> [f64; 8] {
    let days = (t / 86_400_000.0).floor() as i64;
    let ms_day = t - days as f64 * 86_400_000.0;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let wd = (days + 4).rem_euclid(7);
    [
        y as f64,
        (m - 1) as f64,
        d as f64,
        wd as f64,
        (ms_day / 3_600_000.0).floor(),
        (ms_day / 60_000.0).floor() % 60.0,
        (ms_day / 1000.0).floor() % 60.0,
        ms_day % 1000.0,
    ]
}

fn date_of(cx: &Ctx, this: &Value) -> f64 {
    match cx.kind_of(this) {
        Some(Kind::Date(t)) => *t,
        _ => f64::NAN,
    }
}

fn date_value(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    Ok(Value::Number(date_of(cx, &this)))
}

fn date_set_time(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let t = cx.to_number(&arg(a, 0));
    if let Some(o) = this.as_object().and_then(|o| cx.vm.heap.get_mut(o))
        && let Kind::Date(d) = &mut o.kind
    {
        *d = t;
    }
    Ok(Value::Number(t))
}

fn date_to_string(cx: &mut Ctx, this: Value, _: &[Value]) -> R {
    let c = ms_to_civil(date_of(cx, &this));
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    Ok(format!(
        "{} {} {} {:02}:{:02}:{:02} GMT+0000 {}",
        DAYS[c[3] as usize % 7],
        MONTHS[c[1] as usize % 12],
        c[2],
        c[4],
        c[5],
        c[6],
        c[0]
    )
    .into())
}

type DateField = (&'static str, crate::avm1::object::NativeFn);

const DATE_FIELDS: &[DateField] = &[
    ("getFullYear", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[0]))),
    ("getUTCFullYear", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[0]))),
    ("getYear", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[0] - 1900.0))),
    ("getMonth", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[1]))),
    ("getUTCMonth", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[1]))),
    ("getDate", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[2]))),
    ("getUTCDate", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[2]))),
    ("getDay", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[3]))),
    ("getUTCDay", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[3]))),
    ("getHours", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[4]))),
    ("getUTCHours", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[4]))),
    ("getMinutes", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[5]))),
    ("getUTCMinutes", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[5]))),
    ("getSeconds", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[6]))),
    ("getUTCSeconds", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[6]))),
    ("getMilliseconds", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[7]))),
    ("getUTCMilliseconds", |cx, t, _| Ok(Value::Number(ms_to_civil(date_of(cx, &t))[7]))),
    ("getTimezoneOffset", |_, _, _| Ok(Value::Number(0.0))),
];

pub(crate) fn escape(cx: &mut Ctx, _: Value, a: &[Value]) -> R {
    let s = cx.to_string(&arg(a, 0))?;
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(out.into())
}

pub(crate) fn unescape(cx: &mut Ctx, _: Value, a: &[Value]) -> R {
    let s = cx.to_string(&arg(a, 0))?;
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("x"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    Ok(String::from_utf8_lossy(&out).into_owned().into())
}
