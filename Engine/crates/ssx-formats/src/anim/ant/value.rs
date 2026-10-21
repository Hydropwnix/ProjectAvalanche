use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int8(i8),
    UInt8(u8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Vector2([f32; 2]),
    Vector3([f32; 3]),
    Vector4([f32; 4]),
    Quaternion([f32; 4]),
    Matrix44(Box<[[f32; 4]; 4]>),
    Guid(Guid),
    String(String),
    DataRef(Option<Box<Object>>),
    Struct(Struct),
    Array(Vec<Value>),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Struct {
    pub type_index: usize,
    pub values: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub data_offset: u16,
    pub unk_1e: u16,
    pub body: Struct,
}

const MAX_DEPTH: usize = 64;

struct Decoder<'s, 'a> {
    schema: &'s Schema,
    p: Payload<'a>,
    blocks: HashMap<usize, (usize, usize, usize)>,
}

impl Decoder<'_, '_> {
    pub(super) fn object(&mut self, at: usize, depth: usize) -> Result<Object> {
        if depth > MAX_DEPTH {
            return Err(self.p.bad(at, "objects nested too deeply"));
        }
        let mut r = self.p.reader(at)?;
        let (magic, owner, id) = (r.u64()?, r.u64()?, r.u64()?);
        let (refs, data_offset, unk_1e) = (r.u32()?, r.u16()?, r.u16()?);
        if magic != OBJECT_MAGIC || owner != 0 || refs != 0 || id > u32::MAX as u64 {
            return Err(self.p.bad(at, format!("object header {magic:#x} {owner:#x} {id:#x} {refs:#x}")));
        }
        let ti = self.schema.index_of_id(id as u32).ok_or_else(|| self.p.bad(at, format!("unknown type {id:#x}")))?;
        let t = &self.schema.types[ti];
        let start = (at + OBJECT_HEADER_SIZE).next_multiple_of(t.align.max(1) as usize);
        if !at.is_multiple_of(8)
            || at + data_offset as usize != start
            || self.p.slice(at + 0x20, start - at - 0x20)?.iter().any(|&b| b != 0)
        {
            return Err(self.p.bad(at, format!("object data offset {data_offset:#x}")));
        }
        self.p.slice(start, t.size as usize)?;
        let body = self.structure(ti, start, depth)?;
        Ok(Object { data_offset, unk_1e, body })
    }

    fn structure(&mut self, ti: usize, at: usize, depth: usize) -> Result<Struct> {
        let schema = self.schema;
        let t = &schema.types[ti];
        let mut values = Vec::with_capacity(t.fields.len());
        for f in &t.fields {
            let fat = at + f.offset as usize;
            let et = f.type_index;
            let v = if f.is_dynamic_array() {
                self.array(et, fat, depth)?
            } else if f.count > 1 {
                let step = schema.types[et].size.next_multiple_of(schema.types[et].align.max(1)) as usize;
                let items = (0..f.count as usize).map(|i| self.value(et, fat + i * step, depth + 1));
                Value::Array(items.collect::<Result<_>>()?)
            } else {
                self.value(et, fat, depth + 1)?
            };
            values.push(v);
        }
        Ok(Struct { type_index: ti, values })
    }
}

impl Decoder<'_, '_> {
    fn zeros(&self, at: usize, len: usize, what: &str) -> Result<()> {
        if self.p.slice(at, len)?.iter().any(|&b| b != 0) {
            return Err(self.p.bad(at, format!("non-zero {what} padding")));
        }
        Ok(())
    }

    fn floats<const N: usize>(&self, at: usize) -> Result<[f32; N]> {
        let mut r = self.p.reader(at)?;
        let mut out = [0.0; N];
        for x in &mut out {
            *x = r.f32()?;
        }
        Ok(out)
    }

    pub(super) fn value(&mut self, ti: usize, at: usize, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(self.p.bad(at, "values nested too deeply"));
        }
        let t = &self.schema.types[ti];
        let mut r = self.p.reader(at)?;
        Ok(match t.id {
            builtin::BOOL => match r.u8()? {
                0 => Value::Bool(false),
                0xFF => Value::Bool(true),
                b => return Err(self.p.bad(at, format!("Bool byte {b:#x}"))),
            },
            builtin::INT8 => Value::Int8(r.i8()?),
            builtin::UINT8 => Value::UInt8(r.u8()?),
            builtin::INT16 => Value::Int16(r.i16()?),
            builtin::UINT16 => Value::UInt16(r.u16()?),
            builtin::INT32 => Value::Int32(r.i32()?),
            builtin::UINT32 => Value::UInt32(r.u32()?),
            builtin::INT64 => Value::Int64(r.u64()? as i64),
            builtin::UINT64 => Value::UInt64(r.u64()?),
            builtin::FLOAT => Value::Float(r.f32()?),
            builtin::VECTOR2 => {
                self.zeros(at + 8, 8, "Vector2")?;
                Value::Vector2(self.floats(at)?)
            }
            builtin::VECTOR3 => {
                self.zeros(at + 12, 4, "Vector3")?;
                Value::Vector3(self.floats(at)?)
            }
            builtin::VECTOR4 => Value::Vector4(self.floats(at)?),
            builtin::QUATERNION => Value::Quaternion(self.floats(at)?),
            builtin::MATRIX44 => Value::Matrix44(Box::new([
                self.floats(at)?,
                self.floats(at + 16)?,
                self.floats(at + 32)?,
                self.floats(at + 48)?,
            ])),
            builtin::GUID => Value::Guid(Guid(r.array()?)),
            builtin::STRING => {
                let (count, o) = self.array_body(at, 1, 1, 1)?;
                let bytes = self.p.slice(o, count)?;
                let text = match bytes.split_last() {
                    None => String::new(),
                    Some((0, s)) if !s.contains(&0) => String::from_utf8_lossy(s).into_owned(),
                    _ => return Err(self.p.bad(at, "String not NUL-terminated or with inner NUL")),
                };
                Value::String(text)
            }
            builtin::DATA_REF => match self.p.ptr(at)? {
                None => Value::DataRef(None),
                Some(o) => {
                    let obj = self.object(o, depth + 1)?;
                    let size = self.schema.types[obj.body.type_index].size as usize;
                    self.blocks.insert(at, (o, o + obj.data_offset as usize + size, 8));
                    Value::DataRef(Some(Box::new(obj)))
                }
            },
            _ => {
                self.zero_gaps(ti, at)?;
                Value::Struct(self.structure(ti, at, depth)?)
            }
        })
    }

    fn zero_gaps(&self, ti: usize, at: usize) -> Result<()> {
        let t = &self.schema.types[ti];
        let mut spans: Vec<(u64, u64)> =
            t.fields.iter().map(|f| (f.offset as u64, f.offset as u64 + f.footprint())).collect();
        spans.sort_unstable();
        let mut pos = 0;
        let size = t.size as u64;
        for (a, e) in spans.into_iter().chain([(size, size)]) {
            if a > pos {
                self.zeros(at + pos as usize, (a - pos) as usize, "struct")?;
            }
            pos = pos.max(e);
        }
        Ok(())
    }

    fn array_body(&mut self, at: usize, size: usize, step: usize, align: usize) -> Result<(usize, usize)> {
        let mut r = self.p.reader(at)?;
        let (cap, count) = (r.u32()? as usize, r.u32()? as usize);
        let ptr = self.p.ptr(at + 8)?;
        match ptr {
            _ if cap != count => Err(self.p.bad(at, format!("array capacity {cap} != count {count}"))),
            None if count == 0 => Ok((0, 0)),
            Some(o) if count > 0 && o.is_multiple_of(align) => {
                let len = step.checked_mul(count - 1).and_then(|x| x.checked_add(size));
                let len = len.ok_or_else(|| self.p.bad(at, "array too large"))?;
                self.p.slice(o, len)?;
                self.blocks.insert(at + 8, (o, o + len, align));
                Ok((count, o))
            }
            _ => Err(self.p.bad(at, format!("array count {count} with pointer {ptr:?}"))),
        }
    }

    pub(super) fn array(&mut self, et: usize, at: usize, depth: usize) -> Result<Value> {
        let t = &self.schema.types[et];
        let (size, align) = (t.size as usize, t.align.max(1) as usize);
        let step = size.next_multiple_of(align);
        let (count, base) = self.array_body(at, size, step, align)?;
        if t.id == builtin::UINT8 {
            return Ok(Value::Bytes(self.p.slice(base, count)?.to_vec()));
        }
        let mut items = Vec::with_capacity(count);
        for i in 0..count {
            let o = base + i * step;
            if i + 1 < count && step > size {
                self.zeros(o + size, step - size, "array element")?;
            }
            items.push(self.value(et, o, depth + 1)?);
        }
        Ok(Value::Array(items))
    }
}

pub(super) fn parse_data(schema: &Schema, p: Payload<'_>) -> Result<Object> {
    let mut d = Decoder { schema, p, blocks: HashMap::new() };
    let root = d.object(0, 0)?;
    d.p.all_relocs_used()?;
    let mut cur = root.data_offset as usize + schema.types[root.body.type_index].size as usize;
    for slot in &d.p.relocs {
        let (start, end, align) = *d.blocks.get(slot).ok_or_else(|| d.p.bad(*slot, "relocation without a block"))?;
        if start != cur.next_multiple_of(align) {
            return Err(d.p.bad(start, format!("block of pointer slot {slot:#x} not placed after {cur:#x}")));
        }
        d.zeros(cur, start - cur, "block")?;
        cur = end;
    }
    let len = d.p.data.len();
    if cur.next_multiple_of(4) != len {
        return Err(d.p.bad(cur, format!("payload ends at {len:#x}, data at {cur:#x}")));
    }
    d.zeros(cur, len - cur, "tail")?;
    Ok(root)
}

impl Struct {
    pub fn type_desc<'s>(&self, schema: &'s Schema) -> &'s TypeDesc {
        &schema.types[self.type_index]
    }

    pub fn fields<'a>(&'a self, schema: &'a Schema) -> impl Iterator<Item = (&'a FieldDesc, &'a Value)> + 'a {
        self.type_desc(schema).fields.iter().zip(&self.values)
    }

    pub fn own(&self, schema: &Schema, name: &str) -> Option<&Value> {
        let (i, _) = self.type_desc(schema).field(name)?;
        self.values.get(i)
    }
}

impl Object {
    pub fn type_desc<'s>(&self, schema: &'s Schema) -> &'s TypeDesc {
        self.body.type_desc(schema)
    }

    pub fn base(&self, schema: &Schema) -> Option<&Object> {
        match self.body.own(schema, "__base")? {
            Value::DataRef(Some(o)) => Some(o),
            _ => None,
        }
    }

    pub fn get(&self, schema: &Schema, name: &str) -> Option<&Value> {
        let mut o = Some(self);
        let mut depth = 0;
        while let Some(obj) = o {
            if let Some(v) = obj.body.own(schema, name) {
                return Some(v);
            }
            depth += 1;
            o = if depth > MAX_DEPTH { None } else { obj.base(schema) };
        }
        None
    }

    pub fn name(&self, schema: &Schema) -> Option<&str> {
        self.body.own(schema, "__name")?.as_str()
    }

    pub fn guid(&self, schema: &Schema) -> Option<Guid> {
        self.body.own(schema, "__guid")?.as_guid()
    }

    pub fn class_chain<'s>(&self, schema: &'s Schema) -> Vec<&'s str> {
        let mut out = vec![self.type_desc(schema).name.as_str()];
        let mut o = self.base(schema);
        while let Some(b) = o {
            if out.len() > MAX_DEPTH {
                break;
            }
            out.push(b.type_desc(schema).name.as_str());
            o = b.base(schema);
        }
        out
    }
}

impl Value {
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Value::Float(x) => Some(x),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        Some(match *self {
            Value::Bool(b) => b as i64,
            Value::Int8(x) => x as i64,
            Value::UInt8(x) => x as i64,
            Value::Int16(x) => x as i64,
            Value::UInt16(x) => x as i64,
            Value::Int32(x) => x as i64,
            Value::UInt32(x) => x as i64,
            Value::Int64(x) => x,
            Value::UInt64(x) => x as i64,
            _ => return None,
        })
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<[f32; 3]> {
        match *self {
            Value::Vector3(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_guid(&self) -> Option<Guid> {
        match *self {
            Value::Guid(g) => Some(g),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_struct(&self) -> Option<&Struct> {
        match self {
            Value::Struct(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Value::DataRef(Some(o)) => Some(o),
            _ => None,
        }
    }
}
