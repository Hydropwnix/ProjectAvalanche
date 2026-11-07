use toml::{Table, Value};

use super::schema::{FieldType, RecordSchema, Scalar};
use super::spans::{Spans, child, item};
use crate::id::ContentId;
use crate::package::parse_record_at;
use ssx_core::text::quoted;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Whole,
    Patch,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Problem {
    pub line: usize,
    pub column: usize,
    pub path: String,
    pub message: String,
}

pub const MAX_PROBLEMS: usize = 200;

const MAX_SHOWN_PATH: usize = 160;

pub fn check_text(
    schema: &RecordSchema,
    bytes: &[u8],
    mode: Mode,
    guids: Option<&GuidClass<'_>>,
) -> Result<Table, Vec<Problem>> {
    let table = parse_record_at(bytes)
        .map_err(|(line, column, message)| vec![Problem { line, column, path: String::new(), message }])?;
    let text = std::str::from_utf8(bytes).unwrap_or_default();
    let problems = check_table_with(schema, &table, &Spans::of(text), mode, guids);
    if problems.is_empty() { Ok(table) } else { Err(problems) }
}

pub type GuidClass<'a> = dyn Fn(&[u8; 16]) -> Option<String> + 'a;

pub fn check_table(schema: &RecordSchema, table: &Table, spans: &Spans, mode: Mode) -> Vec<Problem> {
    check_table_with(schema, table, spans, mode, None)
}

pub fn check_table_with(
    schema: &RecordSchema,
    table: &Table,
    spans: &Spans,
    mode: Mode,
    guids: Option<&GuidClass<'_>>,
) -> Vec<Problem> {
    let mut c = Checker { spans, guids, problems: Vec::new(), dropped: 0 };
    c.table(schema, table, "", mode);
    c.problems.sort();
    if c.dropped > 0 {
        let message = format!("{} more problems not listed", c.dropped);
        c.problems.push(Problem { line: 0, column: 0, path: String::new(), message });
    }
    c.problems
}

struct Checker<'a> {
    spans: &'a Spans,
    guids: Option<&'a GuidClass<'a>>,
    problems: Vec<Problem>,
    dropped: usize,
}

impl Checker<'_> {
    fn push(&mut self, path: &str, message: String) {
        if self.problems.len() >= MAX_PROBLEMS {
            self.dropped += 1;
            return;
        }
        let (line, column) = self.spans.find(path);
        self.problems.push(Problem {
            line,
            column,
            path: ssx_core::text::shown_max(path, MAX_SHOWN_PATH).to_string(),
            message,
        });
    }

    fn table(&mut self, schema: &RecordSchema, t: &Table, path: &str, mode: Mode) {
        for (k, v) in t {
            let p = child(path, k);
            match schema.fields.get(k) {
                Some(f) => self.value(&f.ty, v, &p, mode),
                None => {
                    let near = nearest(k, schema.fields.keys().map(String::as_str));
                    let hint = near.map(|n| format!(" (did you mean {n:?}?)")).unwrap_or_default();
                    self.push(&p, format!("{} is no field of {}{hint}", quoted(k), schema.name));
                }
            }
        }
        if mode == Mode::Whole {
            let missing: Vec<&str> = schema
                .fields
                .iter()
                .filter(|(k, f)| {
                    !f.optional && !matches!(f.ty, FieldType::Scalar(Scalar::Fixed(_))) && !t.contains_key(*k)
                })
                .map(|(k, _)| k.as_str())
                .collect();
            if !missing.is_empty() {
                let list: Vec<&str> = missing.iter().take(8).copied().collect();
                let more = if missing.len() > 8 { format!(" and {} more", missing.len() - 8) } else { String::new() };
                let at = if path.is_empty() { "the record" } else { path };
                self.push(path, format!("{at} lacks {} field(s): {}{more}", missing.len(), list.join(", ")));
            }
        }
    }

    fn value(&mut self, ty: &FieldType, v: &Value, path: &str, mode: Mode) {
        match ty {
            FieldType::Scalar(s) => {
                if let Err(e) = scalar_in(s, v, self.guids) {
                    self.push(path, e);
                }
            }
            FieldType::Tuple(items) => match v.as_array() {
                Some(a) if a.len() == items.len() => {
                    for (i, (t, e)) in items.iter().zip(a).enumerate() {
                        self.value(t, e, &item(path, i), Mode::Whole);
                    }
                }
                _ => self.push(path, format!("{} values expected", items.len())),
            },
            FieldType::List { elem, min, max, ascending } => match v.as_array() {
                Some(a) if (*min..=*max).contains(&a.len()) => {
                    for (i, e) in a.iter().enumerate() {
                        self.value(elem, e, &item(path, i), Mode::Whole);
                    }
                    let first = |e: &Value| e.as_array().and_then(|x| x.first()).and_then(number);
                    if *ascending
                        && a.windows(2).any(|w| matches!((first(&w[0]), first(&w[1])), (Some(x), Some(y)) if y < x))
                    {
                        self.push(path, "the points' first values must not decrease (the curve's x)".into());
                    }
                }
                Some(a) => self.push(path, format!("{} items; {ty}", a.len())),
                None => self.push(path, format!("{} expected, not {}", ty, type_name(v))),
            },
            FieldType::Table(s) => match v.as_table() {
                Some(t) => self.table(s, t, path, mode),
                None => self.push(path, format!("a table ({}) expected, not {}", s.name, type_name(v))),
            },
            FieldType::Map { key, value, max } => match v.as_table() {
                Some(t) if t.len() <= *max => {
                    for (k, x) in t {
                        let p = child(path, k);
                        if let Err(e) = scalar_in(key, &Value::String(k.clone()), self.guids) {
                            self.push(&p, format!("the key: {e}"));
                        }
                        self.value(value, x, &p, Mode::Whole);
                    }
                }
                Some(t) => self.push(path, format!("{} entries; at most {max}", t.len())),
                None => self.push(path, format!("{ty} expected, not {}", type_name(v))),
            },
            FieldType::OneOf(alts) => {
                let fits = |t: &FieldType| {
                    let mut sub = Checker { spans: self.spans, guids: self.guids, problems: Vec::new(), dropped: 0 };
                    sub.value(t, v, path, mode);
                    sub.problems.is_empty() && sub.dropped == 0
                };
                if !alts.iter().any(fits) {
                    self.push(path, format!("{ty} expected"));
                }
            }
        }
    }
}

pub fn scalar(s: &Scalar, v: &Value) -> Result<(), String> {
    scalar_in(s, v, None)
}

fn scalar_in(s: &Scalar, v: &Value, guids: Option<&GuidClass<'_>>) -> Result<(), String> {
    let bad = || format!("{s} expected, not {}", type_name(v));
    match (s, v) {
        (Scalar::IntOf(set), Value::Integer(i)) => {
            if set.contains(i) {
                Ok(())
            } else {
                Err(format!("{i} is not {s}"))
            }
        }
        (Scalar::Guid { classes, null, one_of }, Value::String(t)) => {
            let g = parse_guid(t)
                .ok_or_else(|| format!("{} is no GUID (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)", quoted(t)))?;
            if g == [0; 16] {
                return if *null { Ok(()) } else { Err(format!("a null reference here: {s}")) };
            }
            if let Some(set) = one_of
                && !set.contains(&g)
            {
                return Err(format!("{t}: not {s}"));
            }
            let Some(guids) = guids else { return Ok(()) };
            match guids(&g) {
                None => Err(format!("{t}: the bundle holds no asset with this GUID")),
                Some(c) if classes.contains(&c) => Ok(()),
                Some(c) => Err(format!("{t} is a {c}; {s}")),
            }
        }
        (Scalar::Bool, Value::Boolean(_)) => Ok(()),
        (Scalar::Int { min, max }, Value::Integer(i)) => {
            if (*min..=*max).contains(i) {
                Ok(())
            } else {
                Err(format!("{i} is out of range: {s}"))
            }
        }
        (Scalar::Float { min, max, f32 }, Value::Float(_) | Value::Integer(_)) => {
            let x = number(v).ok_or_else(bad)?;
            if !x.is_finite() {
                return Err(format!("{x} is not a finite number"));
            }
            let x =
                if *f32 { f64::from(narrow(x).ok_or_else(|| format!("{x} does not fit a 32-bit float"))?) } else { x };
            if x >= *min && x <= *max { Ok(()) } else { Err(format!("{x} is out of range: {s}")) }
        }
        (Scalar::Text { max, one_of }, Value::String(t)) => {
            if t.len() > *max {
                Err(format!("{} bytes of text; at most {max}", t.len()))
            } else if t.chars().any(char::is_control) {
                Err(format!("{} holds a control character", quoted(t)))
            } else if !one_of.is_empty() && !one_of.iter().any(|o| o == t) {
                Err(format!("{} is not {s}", quoted(t)))
            } else {
                Ok(())
            }
        }
        (Scalar::Hex { max }, Value::String(t)) => match parse_hex(t) {
            Some(b) if b.len() <= *max => Ok(()),
            Some(b) => Err(format!("{} bytes; at most {max}", b.len())),
            None => Err(format!("{}: {s}", quoted(t))),
        },
        (Scalar::Id(kind), Value::String(t)) => match t.parse::<ContentId>() {
            Ok(id) if id.kind() == *kind => Ok(()),
            Ok(id) => Err(format!("{id} is not a {kind} id")),
            Err(e) => Err(format!("{}: {e}", quoted(t))),
        },
        (Scalar::Fixed(why), _) => Err(format!("not settable: {why}")),
        _ => Err(bad()),
    }
}

pub fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Float(x) => Some(*x),
        Value::Integer(i) if i.unsigned_abs() <= 1 << 53 => Some(*i as f64),
        _ => None,
    }
}

pub fn narrow(x: f64) -> Option<f32> {
    let y = x as f32;
    y.is_finite().then_some(y)
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::String(_) => "a text",
        Value::Integer(_) => "an integer",
        Value::Float(_) => "a number",
        Value::Boolean(_) => "true / false",
        Value::Datetime(_) => "a date",
        Value::Array(_) => "a list",
        Value::Table(_) => "a table",
    }
}

pub fn parse_guid(s: &str) -> Option<[u8; 16]> {
    let groups: Vec<&str> = s.split('-').collect();
    let lens = [8, 4, 4, 4, 12];
    if groups.len() != 5 || groups.iter().zip(lens).any(|(g, n)| g.len() != n) {
        return None;
    }
    let hex = groups.concat();
    let b = parse_hex(&hex)?;
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&[b[3], b[2], b[1], b[0]]);
    out[4..6].copy_from_slice(&[b[5], b[4]]);
    out[6..8].copy_from_slice(&[b[7], b[6]]);
    out[8..].copy_from_slice(&b[8..]);
    Some(out)
}

pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn nearest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    if name.len() > 64 {
        return None;
    }
    candidates
        .filter(|c| c.len() <= 64)
        .map(|c| (distance(&name.to_ascii_lowercase(), &c.to_ascii_lowercase()), c))
        .filter(|(d, _)| *d <= 3)
        .min()
        .map(|(_, c)| c)
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != *cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests;
