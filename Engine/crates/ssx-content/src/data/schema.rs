use std::collections::BTreeMap;
use std::fmt;

use crate::id::Kind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Simulation,
    Presentation,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Simulation => "simulation",
            Class::Presentation => "presentation",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Bool,
    Int { min: i64, max: i64 },
    IntOf(Vec<i64>),
    Float { min: f64, max: f64, f32: bool },
    Text { max: usize, one_of: Vec<String> },
    Guid { classes: Vec<String>, null: bool, one_of: Option<Vec<[u8; 16]>> },
    Hex { max: usize },
    Id(Kind),
    Fixed(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldType {
    Scalar(Scalar),
    Tuple(Vec<FieldType>),
    List { elem: Box<FieldType>, min: usize, max: usize, ascending: bool },
    Table(Box<RecordSchema>),
    Map { key: Scalar, value: Box<FieldType>, max: usize },
    OneOf(Vec<FieldType>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldSchema {
    pub ty: FieldType,
    pub class: Class,
    pub source: String,
    pub note: String,
    pub optional: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordSchema {
    pub name: String,
    pub fields: BTreeMap<String, FieldSchema>,
}

impl RecordSchema {
    pub fn new(name: &str) -> Self {
        RecordSchema { name: name.to_string(), fields: BTreeMap::new() }
    }

    pub fn flat(&self) -> Vec<(String, &FieldSchema)> {
        let mut out = Vec::new();
        self.flat_into("", &mut out);
        out
    }

    fn flat_into<'a>(&'a self, prefix: &str, out: &mut Vec<(String, &'a FieldSchema)>) {
        for (name, f) in &self.fields {
            let path = crate::data::spans::child(prefix, name);
            out.push((path.clone(), f));
            let mut ty = &f.ty;
            let mut p = path;
            while let FieldType::List { elem, .. } = ty {
                p.push_str("[]");
                ty = elem;
            }
            if let FieldType::Table(t) = ty {
                t.flat_into(&p, out);
            }
        }
    }

    pub fn count(&self) -> usize {
        self.flat().len()
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scalar::Bool => f.write_str("true or false"),
            Scalar::Int { min, max } if min == max => write!(f, "the integer {min}"),
            Scalar::Int { min, max } => write!(f, "an integer {min} … {max}"),
            Scalar::IntOf(v) if v.len() == 1 => write!(f, "the integer {}", v[0]),
            Scalar::IntOf(v) if v.len() <= 8 => write!(f, "one of {v:?}"),
            Scalar::IntOf(v) => write!(f, "one of {} integers", v.len()),
            Scalar::Float { min, max, f32: true } if min == max => write!(f, "the number {}", *min as f32),
            Scalar::Float { min, max, f32: true } => write!(f, "a number {} … {}", *min as f32, *max as f32),
            Scalar::Float { min, max, .. } if min == max => write!(f, "the number {min}"),
            Scalar::Float { min, max, .. } => write!(f, "a number {min} … {max}"),
            Scalar::Text { one_of, .. } if one_of.len() == 1 => write!(f, "the text {:?}", one_of[0]),
            Scalar::Text { one_of, .. } if !one_of.is_empty() => write!(f, "one of {} texts", one_of.len()),
            Scalar::Text { max, .. } => write!(f, "a text of at most {max} bytes"),
            Scalar::Guid { classes, null, one_of } => {
                let null = if *null { "null (00000000-0000-0000-0000-000000000000) or " } else { "" };
                match one_of {
                    Some(v) if v.is_empty() => write!(f, "null (00000000-0000-0000-0000-000000000000)"),
                    Some(v) => write!(
                        f,
                        "{null}one of the {} reference(s) the disc has here ({})",
                        v.len(),
                        classes.join(" / ")
                    ),
                    None => write!(f, "{null}a GUID of the bundle's {}", classes.join(" / ")),
                }
            }
            Scalar::Hex { max } => write!(f, "hex digits of at most {max} bytes"),
            Scalar::Id(kind) => write!(f, "a {kind} id"),
            Scalar::Fixed(why) => write!(f, "not settable ({why})"),
        }
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldType::Scalar(s) => s.fmt(f),
            FieldType::Tuple(items) => write!(f, "{} values", items.len()),
            FieldType::List { min, max, .. } if min == max => write!(f, "a list of {min}"),
            FieldType::List { min, max, .. } => write!(f, "a list of {min} … {max}"),
            FieldType::Table(t) => write!(f, "a table ({})", t.name),
            FieldType::Map { key, value, max } => write!(f, "a table of at most {max} entries ({key} = {value})"),
            FieldType::OneOf(alts) => {
                let all: Vec<String> = alts.iter().map(ToString::to_string).collect();
                write!(f, "{}", all.join(" or "))
            }
        }
    }
}
