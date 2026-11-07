use ssx_formats::attrib::collection::{Field, FieldData, FieldSource};
use ssx_formats::attrib::value::{RefSpec, Value, ValueKind};
use toml::{Table, Value as Toml};

use super::ant::float;
use super::check::{narrow, number};
use super::schema::{Class, FieldSchema, FieldType, RecordSchema};

#[derive(Clone, Debug)]
pub struct FieldSpec {
    pub key: u32,
    pub name: String,
    pub type_hash: u32,
    pub kind: ValueKind,
    pub array: bool,
    pub ty: FieldType,
    pub class: Class,
    pub source: String,
    pub note: String,
}

pub fn field_name(key: u32, dictionary: Option<&str>) -> String {
    match dictionary {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => n.to_string(),
        _ => format!("0x{key:08x}"),
    }
}

pub fn schema(name: &str, fields: &[FieldSpec]) -> RecordSchema {
    let mut rs = RecordSchema::new(name);
    for f in fields {
        let fs = FieldSchema {
            ty: f.ty.clone(),
            class: f.class,
            source: f.source.clone(),
            note: f.note.clone(),
            optional: false,
        };
        rs.fields.insert(f.name.clone(), fs);
    }
    rs
}

pub fn to_table(fields: &[FieldSpec], values: &[(u32, &Field)], refs: &dyn Fn(&RefSpec) -> String) -> Table {
    let mut t = Table::new();
    for spec in fields {
        let Some((_, f)) = values.iter().find(|(_, f)| f.key == spec.key) else { continue };
        let one = |v: &Value| value(v, refs);
        let v = match &f.data {
            FieldData::Single(v) if !spec.array => one(v),
            FieldData::Single(v) => Some(Toml::Array(one(v).into_iter().collect())),
            FieldData::Array(vs) => Some(Toml::Array(vs.iter().filter_map(one).collect())),
        };
        if let Some(v) = v {
            t.insert(spec.name.clone(), v);
        }
    }
    t
}

fn value(v: &Value, refs: &dyn Fn(&RefSpec) -> String) -> Option<Toml> {
    let fl = |xs: &[f32]| Toml::Array(xs.iter().map(|&x| Toml::Float(float(x))).collect());
    Some(match v {
        Value::Bool(b) => Toml::Boolean(*b),
        Value::Int8(x) => Toml::Integer((*x).into()),
        Value::UInt8(x) => Toml::Integer((*x).into()),
        Value::Int16(x) => Toml::Integer((*x).into()),
        Value::UInt16(x) => Toml::Integer((*x).into()),
        Value::Int32(x) => Toml::Integer((*x).into()),
        Value::UInt32(x) => Toml::Integer((*x).into()),
        Value::Int64(x) => Toml::Integer(*x),
        Value::UInt64(x) => Toml::Integer(i64::try_from(*x).ok()?),
        Value::Float(x) => Toml::Float(float(*x)),
        Value::Vector2(a) => fl(a),
        Value::Vector3(a) => fl(a),
        Value::Vector4(a) => fl(a),
        Value::Text(t) => Toml::String(t.clone().unwrap_or_default()),
        Value::Key(k) => Toml::Integer((*k).into()),
        Value::Ref(r) => Toml::String(refs(r)),
        Value::Raw { bytes, pointers } if pointers.is_empty() && bytes.len() == 4 => {
            Toml::Integer(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).into())
        }
        Value::Raw { .. } => return None,
    })
}

pub fn apply(
    fields: &[FieldSpec],
    own: &mut Vec<Field>,
    table: &Table,
    refs: &dyn Fn(u32, &str) -> Option<RefSpec>,
) -> Result<(), String> {
    for (name, v) in table {
        let spec = fields
            .iter()
            .find(|f| f.name == *name)
            .ok_or_else(|| format!("no field {}", ssx_core::text::quoted(name)))?;
        let data = if spec.array {
            let a = v.as_array().ok_or_else(|| format!("{name}: a list expected"))?;
            let items: Result<Vec<Value>, String> = a.iter().map(|x| set(spec, x, refs)).collect();
            FieldData::Array(items.map_err(|e| format!("{name}: {e}"))?)
        } else {
            FieldData::Single(set(spec, v, refs).map_err(|e| format!("{name}: {e}"))?)
        };
        match own.iter_mut().find(|f| f.key == spec.key) {
            Some(f) => f.data = data,
            None => own.push(Field { key: spec.key, type_hash: spec.type_hash, source: FieldSource::Inline, data }),
        }
    }
    Ok(())
}

fn set(spec: &FieldSpec, v: &Toml, refs: &dyn Fn(u32, &str) -> Option<RefSpec>) -> Result<Value, String> {
    let int = |lo: i64, hi: i64| -> Result<i64, String> {
        v.as_integer().filter(|x| (lo..=hi).contains(x)).ok_or_else(|| format!("an integer {lo} … {hi} expected"))
    };
    let f = |x: &Toml| number(x).filter(|x| x.is_finite()).and_then(narrow).ok_or("a finite number expected");
    let fs = |n: usize| -> Result<Vec<f32>, String> {
        let a = v.as_array().filter(|a| a.len() == n).ok_or_else(|| format!("{n} numbers expected"))?;
        a.iter().map(|x| f(x).map_err(str::to_string)).collect()
    };
    Ok(match spec.kind {
        ValueKind::Bool => Value::Bool(v.as_bool().ok_or("true or false expected")?),
        ValueKind::Int8 => Value::Int8(i8::try_from(int(i8::MIN.into(), i8::MAX.into())?).map_err(|e| e.to_string())?),
        ValueKind::UInt8 => Value::UInt8(u8::try_from(int(0, u8::MAX.into())?).map_err(|e| e.to_string())?),
        ValueKind::Int16 => {
            Value::Int16(i16::try_from(int(i16::MIN.into(), i16::MAX.into())?).map_err(|e| e.to_string())?)
        }
        ValueKind::UInt16 => Value::UInt16(u16::try_from(int(0, u16::MAX.into())?).map_err(|e| e.to_string())?),
        ValueKind::Int32 => {
            Value::Int32(i32::try_from(int(i32::MIN.into(), i32::MAX.into())?).map_err(|e| e.to_string())?)
        }
        ValueKind::UInt32 => Value::UInt32(u32::try_from(int(0, u32::MAX.into())?).map_err(|e| e.to_string())?),
        ValueKind::Int64 => Value::Int64(int(i64::MIN, i64::MAX)?),
        ValueKind::UInt64 => Value::UInt64(u64::try_from(int(0, i64::MAX)?).map_err(|e| e.to_string())?),
        ValueKind::Float => Value::Float(f(v)?),
        ValueKind::Vector2 => Value::Vector2(fs(2)?.try_into().map_err(|_| "2 numbers expected")?),
        ValueKind::Vector3 => Value::Vector3(fs(3)?.try_into().map_err(|_| "3 numbers expected")?),
        ValueKind::Vector4 => Value::Vector4(fs(4)?.try_into().map_err(|_| "4 numbers expected")?),
        ValueKind::Text => Value::Text(Some(v.as_str().ok_or("a text expected")?.to_string())),
        ValueKind::Key => Value::Key(u32::try_from(int(0, u32::MAX.into())?).map_err(|e| e.to_string())?),
        ValueKind::RefSpec | ValueKind::ClassRefSpec(_) => {
            let class = if let ValueKind::ClassRefSpec(c) = spec.kind { c } else { 0 };
            let s = v.as_str().ok_or("a reference (\"<class>/<collection>\") expected")?;
            Value::Ref(refs(class, s).ok_or_else(|| format!("{} names no collection", ssx_core::text::quoted(s)))?)
        }
        ValueKind::Raw => {
            let x = u32::try_from(int(0, u32::MAX.into())?).map_err(|e| e.to_string())?;
            Value::Raw { bytes: x.to_be_bytes().to_vec(), pointers: Vec::new() }
        }
    })
}
