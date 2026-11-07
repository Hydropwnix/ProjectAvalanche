use ssx_formats::ant::{Guid, Object, Schema, Struct, Value, builtin};
use toml::{Table, Value as Toml};

use super::check::{narrow, number, parse_guid, parse_hex};

pub const HEADER: [&str; 3] = ["__guid", "__name", "__base"];

pub const MAX_NESTING: usize = 16;

pub fn chain<'a>(schema: &Schema, obj: &'a Object) -> Vec<&'a Object> {
    let mut out = vec![obj];
    while let Some(b) = out.last().and_then(|o| o.base(schema)) {
        if out.len() > 64 {
            break;
        }
        out.push(b);
    }
    out
}

pub fn to_table(schema: &Schema, obj: &Object) -> Result<(Table, Vec<String>), String> {
    to_table_at(schema, obj, 0)
}

fn to_table_at(schema: &Schema, obj: &Object, depth: usize) -> Result<(Table, Vec<String>), String> {
    let mut t = Table::new();
    let mut shadowed = Vec::new();
    for o in chain(schema, obj) {
        for (f, v) in o.body.fields(schema) {
            if HEADER.contains(&f.name.as_str()) {
                continue;
            }
            if t.contains_key(&f.name) {
                shadowed.push(format!("{}.{}", o.type_desc(schema).name, f.name));
                continue;
            }
            if let Some(x) = value(schema, v, depth)? {
                t.insert(f.name.clone(), x);
            }
        }
    }
    Ok((t, shadowed))
}

fn struct_table(schema: &Schema, s: &Struct, depth: usize) -> Result<Table, String> {
    let mut t = Table::new();
    for (f, v) in s.fields(schema) {
        if let Some(x) = value(schema, v, depth)? {
            t.insert(f.name.clone(), x);
        }
    }
    Ok(t)
}

pub fn float(x: f32) -> f64 {
    match format!("{x}").parse::<f64>() {
        Ok(d) if d as f32 == x || (x.is_nan() && d.is_nan()) => d,
        _ => f64::from(x),
    }
}

fn floats(xs: &[f32]) -> Toml {
    Toml::Array(xs.iter().map(|&x| Toml::Float(float(x))).collect())
}

fn value(schema: &Schema, v: &Value, depth: usize) -> Result<Option<Toml>, String> {
    if depth > MAX_NESTING {
        return Err(format!("values nested deeper than {MAX_NESTING}"));
    }
    let i = |x: i64| Some(Toml::Integer(x));
    Ok(match v {
        Value::Bool(b) => Some(Toml::Boolean(*b)),
        Value::Int8(x) => i(i64::from(*x)),
        Value::UInt8(x) => i(i64::from(*x)),
        Value::Int16(x) => i(i64::from(*x)),
        Value::UInt16(x) => i(i64::from(*x)),
        Value::Int32(x) => i(i64::from(*x)),
        Value::UInt32(x) => i(i64::from(*x)),
        Value::Int64(x) => i(*x),
        Value::UInt64(x) => match i64::try_from(*x) {
            Ok(x) => i(x),
            Err(_) => Some(Toml::String(format!("{x:016x}"))),
        },
        Value::Float(x) => Some(Toml::Float(float(*x))),
        Value::Vector2(a) => Some(floats(a)),
        Value::Vector3(a) => Some(floats(a)),
        Value::Vector4(a) | Value::Quaternion(a) => Some(floats(a)),
        Value::Matrix44(m) => Some(Toml::Array(m.iter().map(|r| floats(r)).collect())),
        Value::Guid(g) => Some(Toml::String(g.to_string())),
        Value::String(s) => Some(Toml::String(s.clone())),
        Value::DataRef(None) => None,
        Value::DataRef(Some(o)) => Some(Toml::Table(to_table_at(schema, o, depth + 1)?.0)),
        Value::Struct(s) => Some(Toml::Table(struct_table(schema, s, depth + 1)?)),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for x in items {
                out.push(value(schema, x, depth + 1)?.unwrap_or_else(|| Toml::Table(Table::new())));
            }
            Some(Toml::Array(out))
        }
        Value::Bytes(b) => Some(Toml::String(b.iter().map(|x| format!("{x:02x}")).collect())),
    })
}

pub fn apply(schema: &Schema, obj: &mut Object, table: &Table) -> Result<(), String> {
    apply_at(schema, obj, table, 0)
}

fn apply_at(schema: &Schema, obj: &mut Object, table: &Table, depth: usize) -> Result<(), String> {
    if depth > MAX_NESTING {
        return Err(format!("values nested deeper than {MAX_NESTING}"));
    }
    for (name, v) in table {
        if HEADER.contains(&name.as_str()) {
            return Err(format!("{name} is the asset's identity, not a field"));
        }
        let slot = field_mut(schema, obj, name).ok_or_else(|| format!("no field {}", ssx_core::text::quoted(name)))?;
        set(schema, slot, v, depth + 1).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

fn field_mut<'a>(schema: &Schema, obj: &'a mut Object, name: &str) -> Option<&'a mut Value> {
    let ti = obj.body.type_index;
    let own = schema.types.get(ti)?.field(name).map(|(i, _)| i);
    match own {
        Some(i) => obj.body.values.get_mut(i),
        None => {
            let (bi, _) = schema.types.get(ti)?.field("__base")?;
            match obj.body.values.get_mut(bi)? {
                Value::DataRef(Some(b)) => field_mut(schema, b, name),
                _ => None,
            }
        }
    }
}

fn set(schema: &Schema, slot: &mut Value, v: &Toml, depth: usize) -> Result<(), String> {
    if depth > MAX_NESTING {
        return Err(format!("values nested deeper than {MAX_NESTING}"));
    }
    macro_rules! int {
        ($t:ty) => {
            match v {
                Toml::Integer(x) => <$t>::try_from(*x).ok(),
                _ => None,
            }
            .ok_or_else(|| format!("an integer {} … {} expected", <$t>::MIN, <$t>::MAX))?
        };
    }
    let f = || -> Result<f32, String> {
        number(v).filter(|x| x.is_finite()).and_then(narrow).ok_or_else(|| "a finite number expected".to_string())
    };
    let fs = |n: usize| -> Result<Vec<f32>, String> {
        let a = v.as_array().filter(|a| a.len() == n).ok_or_else(|| format!("{n} numbers expected"))?;
        a.iter()
            .map(|x| number(x).filter(|x| x.is_finite()).and_then(narrow).ok_or_else(|| "a finite number".to_string()))
            .collect()
    };
    match slot {
        Value::Bool(b) => *b = v.as_bool().ok_or("true or false expected")?,
        Value::Int8(x) => *x = int!(i8),
        Value::UInt8(x) => *x = int!(u8),
        Value::Int16(x) => *x = int!(i16),
        Value::UInt16(x) => *x = int!(u16),
        Value::Int32(x) => *x = int!(i32),
        Value::UInt32(x) => *x = int!(u32),
        Value::Int64(x) => *x = int!(i64),
        Value::UInt64(x) => {
            *x = match v {
                Toml::Integer(i) if *i >= 0 => u64::try_from(*i).map_err(|e| e.to_string())?,
                Toml::String(s) if s.len() == 16 => {
                    let b = parse_hex(s).ok_or("16 hex digits expected")?;
                    u64::from_be_bytes(b.try_into().map_err(|_| "16 hex digits expected")?)
                }
                _ => return Err("a non-negative integer expected".into()),
            }
        }
        Value::Float(x) => *x = f()?,
        Value::Vector2(a) => a.copy_from_slice(&fs(2)?),
        Value::Vector3(a) => a.copy_from_slice(&fs(3)?),
        Value::Vector4(a) | Value::Quaternion(a) => a.copy_from_slice(&fs(4)?),
        Value::Matrix44(m) => {
            let rows = v.as_array().filter(|r| r.len() == 4).ok_or("4 rows of 4 numbers expected")?;
            for (row, r) in m.iter_mut().zip(rows) {
                let mut one = Value::Vector4([0.0; 4]);
                set(schema, &mut one, r, depth + 1)?;
                if let Value::Vector4(x) = one {
                    *row = x;
                }
            }
        }
        Value::Guid(g) => *g = Guid(v.as_str().and_then(parse_guid).ok_or("a GUID expected")?),
        Value::String(s) => *s = v.as_str().ok_or("a text expected")?.to_string(),
        Value::DataRef(None) => return Err("the disc's embedded object here is null: nothing to set".into()),
        Value::DataRef(Some(o)) => apply_at(schema, o, v.as_table().ok_or("a table expected")?, depth + 1)?,
        Value::Struct(s) => {
            let t = v.as_table().ok_or("a table expected")?;
            for (name, x) in t {
                let i = s
                    .type_desc(schema)
                    .field(name)
                    .map(|(i, _)| i)
                    .ok_or_else(|| format!("no field {}", ssx_core::text::quoted(name)))?;
                let slot =
                    s.values.get_mut(i).ok_or_else(|| format!("no value for {}", ssx_core::text::quoted(name)))?;
                set(schema, slot, x, depth + 1).map_err(|e| format!("{name}: {e}"))?;
            }
        }
        Value::Array(items) => {
            let a = v.as_array().ok_or("a list expected")?;
            set_list(schema, items, a, depth)?;
        }
        Value::Bytes(b) => *b = v.as_str().and_then(parse_hex).ok_or("hex digits expected")?,
    }
    Ok(())
}

fn set_list(schema: &Schema, items: &mut Vec<Value>, a: &[Toml], depth: usize) -> Result<(), String> {
    if a.len() != items.len() {
        let template = items.first().cloned().ok_or("the disc's list here is empty: no item to grow it from")?;
        items.truncate(a.len());
        items.resize(a.len(), template);
    }
    for (k, (slot, x)) in items.iter_mut().zip(a).enumerate() {
        set(schema, slot, x, depth + 1).map_err(|e| format!("[{k}]: {e}"))?;
    }
    Ok(())
}

pub fn guids(schema: &Schema, obj: &Object, out: &mut Vec<Guid>) {
    for o in chain(schema, obj) {
        for (f, v) in o.body.fields(schema) {
            if !HEADER.contains(&f.name.as_str()) {
                value_guids(schema, v, out, 0);
            }
        }
    }
}

fn value_guids(schema: &Schema, v: &Value, out: &mut Vec<Guid>, depth: usize) {
    if depth > MAX_NESTING {
        return;
    }
    match v {
        Value::Guid(g) if !g.is_null() => out.push(*g),
        Value::DataRef(Some(o)) => {
            for (f, x) in o.body.fields(schema) {
                if !HEADER.contains(&f.name.as_str()) {
                    value_guids(schema, x, out, depth + 1);
                }
            }
        }
        Value::Struct(s) => s.values.iter().for_each(|x| value_guids(schema, x, out, depth + 1)),
        Value::Array(items) => items.iter().for_each(|x| value_guids(schema, x, out, depth + 1)),
        _ => {}
    }
}

pub fn is_scalar(id: u32) -> bool {
    id != builtin::DATA_REF && id < builtin::COUNT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_read_back_exactly() {
        for x in [0.15f32, 1.0 / 3.0, f32::MIN_POSITIVE, f32::MAX, -0.0, 1e-45, 123456.79, 0.1 + 0.2] {
            let d = float(x);
            assert_eq!((d as f32).to_bits(), x.to_bits(), "{x}");
        }
        assert_eq!(float(0.15), 0.15, "the shortest decimal, not 0.15000000596046448");
    }
}
