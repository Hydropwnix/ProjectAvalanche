use std::sync::Arc;

use super::object::ObjId;

pub type Str = Arc<str>;

#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Str),
    Object(ObjId),
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::String(Arc::from(s))
    }

    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Undefined)
    }

    pub fn is_nullish(&self) -> bool {
        matches!(self, Value::Undefined | Value::Null)
    }

    pub fn as_object(&self) -> Option<ObjId> {
        match self {
            Value::Object(o) => Some(*o),
            _ => None,
        }
    }

    pub fn same(&self, o: &Value) -> bool {
        match (self, o) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Value {
        Value::Number(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Value {
        Value::Number(f64::from(v))
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Value {
        Value::Bool(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Value {
        Value::str(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Value {
        Value::String(Arc::from(v))
    }
}

impl From<ObjId> for Value {
    fn from(v: ObjId) -> Value {
        Value::Object(v)
    }
}

fn c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

fn strtod(s: &[u8]) -> (f64, usize) {
    let mut i = 0;
    while i < s.len() && c_space(s[i]) {
        i += 1;
    }
    let start = i;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let digits0 = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut mantissa = i - digits0;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        let f0 = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        mantissa += i - f0;
    }
    if mantissa == 0 {
        return (0.0, 0);
    }
    if i < s.len() && (s[i] == b'e' || s[i] == b'E') {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let e0 = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j > e0 {
            i = j;
        }
    }
    let text = std::str::from_utf8(&s[start..i]).unwrap_or("0");
    (text.parse::<f64>().unwrap_or(0.0), i)
}

fn strtol(s: &[u8], base: u32) -> (i32, usize) {
    let mut i = 0;
    while i < s.len() && c_space(s[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut base = base;
    let has_0x = i + 1 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x';
    if (base == 0 || base == 16) && has_0x {
        base = 16;
        i += 2;
    } else if base == 0 {
        base = if i < s.len() && s[i] == b'0' { 8 } else { 10 };
    }
    let d0 = i;
    let mut acc: i64 = 0;
    while i < s.len() {
        let Some(d) = (s[i] as char).to_digit(base) else { break };
        acc = (acc * i64::from(base) + i64::from(d)).min(1 << 40);
        i += 1;
    }
    if i == d0 {
        return (0, 0);
    }
    let v = if neg { -acc } else { acc };
    (v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32, i)
}

pub fn strtod_whole(s: &str) -> Option<f64> {
    let (v, end) = strtod(s.as_bytes());
    (end != 0 && end == s.len()).then_some(v)
}

pub fn parse_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'0' && (b[1] | 0x20) == b'x' {
        let (v, end) = strtol(b, 0);
        return (end != 0 && end == b.len()).then_some(f64::from(v));
    }
    if b.iter().any(|c| matches!(c, b'.' | b'E' | b'e')) {
        let (v, end) = strtod(b);
        return (end != 0 && end == b.len()).then_some(v);
    }
    let (sign, rest) = match b.first() {
        Some(b'-') => (-1, &b[1..]),
        Some(b'+') => (1, &b[1..]),
        _ => (1, b),
    };
    let skipped = b.len() - rest.len();
    if rest.first() == Some(&b'0') && rest.iter().all(|c| (b'0'..=b'7').contains(c)) {
        let (v, _) = strtol(rest, 8);
        return Some(f64::from(v.wrapping_mul(sign)));
    }
    let (v, end) = strtod(rest);
    let end = skipped + end;
    (end != 0 && end == b.len()).then_some(v * f64::from(sign))
}

pub fn number_to_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let i = v as i32;
    if f64::from(i) == v {
        return i.to_string();
    }
    format_g(v, 14)
}

pub fn number_to_radix(v: f64, radix: i32) -> String {
    if !matches!(radix, 2 | 8 | 16) || v.is_nan() || v.is_infinite() {
        return number_to_string(v);
    }
    let u = (v as i32) as u32;
    match radix {
        2 => format!("{u:b}"),
        8 => format!("{u:o}"),
        _ => format!("{u:x}"),
    }
}

pub fn format_g(v: f64, precision: usize) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let p = precision.max(1);
    let e = format!("{:.*e}", p - 1, v);
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if exp < -4 || exp >= p as i32 {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:03}", trim_fraction(mant), exp.abs())
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        trim_fraction(&format!("{v:.decimals$}")).to_string()
    }
}

fn trim_fraction(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}

pub fn to_int32(v: f64) -> i32 {
    if !v.is_finite() || v == 0.0 {
        return 0;
    }
    if (-2147483648.0..=2147483647.0).contains(&v) {
        return v as i32;
    }
    let m = v.abs().floor() % 4294967296.0;
    let u = m as u64 as u32;
    if v < 0.0 { (u as i32).wrapping_neg() } else { u as i32 }
}

pub fn primitive_to_bool(v: &Value, version: u8) -> bool {
    match v {
        Value::Undefined | Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => {
            if s.is_empty() {
                false
            } else if version > 6 {
                true
            } else {
                parse_number(s).is_some_and(|n| n != 0.0 && !n.is_nan())
            }
        }
        Value::Object(_) => true,
    }
}

pub fn primitive_to_number(v: &Value, version: u8) -> f64 {
    match v {
        Value::Undefined | Value::Null => {
            if version < 7 {
                0.0
            } else {
                f64::NAN
            }
        }
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => *n,
        Value::String(s) => parse_number(s).unwrap_or(f64::NAN),
        Value::Object(_) => f64::NAN,
    }
}
