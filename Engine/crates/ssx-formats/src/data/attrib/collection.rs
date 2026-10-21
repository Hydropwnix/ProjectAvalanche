use std::collections::HashMap;

use crate::reader::Reader;
use crate::{Error, Result};

use super::container::{Export, FILE_VLT, PtrTarget, Vault};
use super::schema::{Definition, Schema, TypeInfo};
use super::value::{self, Value};
use super::{EXPORT_CLASS, EXPORT_COLLECTION, EXPORT_DATABASE};

pub const ENTRY_ARRAY: u8 = 0x02;
pub const ENTRY_REFERENCE: u8 = 0x08;
pub const ENTRY_INLINE: u8 = 0x40;
const ARRAY_ALIGNED_16: u16 = 0x8000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: u32,
    pub value: [u8; 4],
    pub type_index: u16,
    pub flags: u8,
    pub unk: u8,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct Collection {
    pub key: u32,
    pub export_key: u32,
    pub class: u32,
    pub parent: u32,
    pub table_reserve: u32,
    pub unk_table_param: u32,
    pub types: Vec<u32>,
    pub types_capacity: u16,
    pub layout: Option<PtrTarget>,
    pub entries: Vec<Entry>,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldSource {
    Layout,
    Inline,
    OutOfLine,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldData {
    Single(Value),
    Array(Vec<Value>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub key: u32,
    pub type_hash: u32,
    pub source: FieldSource,
    pub data: FieldData,
}

#[derive(Clone, Debug, Default)]
pub struct DecodedCollection {
    pub fields: Vec<Field>,
    pub pointer_slots: Vec<PtrTarget>,
}

impl DecodedCollection {
    pub fn field(&self, key: u32) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key)
    }
}

pub fn collections(v: &Vault) -> Result<Vec<Collection>> {
    let mut out = Vec::new();
    for e in &v.exports {
        match e.type_hash {
            EXPORT_COLLECTION => out.push(Collection::parse(v, e)?),
            EXPORT_CLASS | EXPORT_DATABASE => {}
            t => return Err(Error::invalid(format!("export {:#010x}: unknown type {t:#010x}", e.key))),
        }
    }
    Ok(out)
}

pub fn index(collections: &[Collection]) -> HashMap<(u32, u32), usize> {
    collections.iter().enumerate().map(|(i, c)| ((c.class, c.key), i)).collect()
}

pub fn with_inherited<'c>(
    collections: &[Collection],
    decoded: &'c [DecodedCollection],
    index: &HashMap<(u32, u32), usize>,
    idx: usize,
) -> Result<Vec<(u32, &'c Field)>> {
    let mut out: Vec<(u32, &Field)> = decoded[idx].fields.iter().map(|f| (collections[idx].key, f)).collect();
    let mut cur = &collections[idx];
    for _ in 0..collections.len() {
        if cur.parent == 0 {
            return Ok(out);
        }
        let &p = index.get(&(cur.class, cur.parent)).ok_or_else(|| {
            Error::invalid(format!("collection {:#010x}: parent {:#010x} not in this vault", cur.key, cur.parent))
        })?;
        for f in decoded[p].fields.iter().filter(|f| f.source != FieldSource::Layout) {
            if !out.iter().any(|(_, o)| o.key == f.key) {
                out.push((collections[p].key, f));
            }
        }
        cur = &collections[p];
    }
    Err(Error::invalid(format!("collection {:#010x}: parent chain loops", collections[idx].key)))
}

impl Collection {
    pub fn parse(v: &Vault, e: &Export) -> Result<Self> {
        let mut r = Reader::be(v.export_bytes(e));
        let (key, class, parent, table_reserve, unk_table_param) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let count = r.u32()?;
        let (num_types, types_capacity) = (r.u16()?, r.u16()?);
        let layout_slot = e.offset + 0x1c;
        let layout = v.pointer(FILE_VLT, layout_slot);
        r.skip(4)?;
        let what = format!("collection {key:#010x} (class {class:#010x})");
        if types_capacity != num_types.next_multiple_of(2) || table_reserve != count {
            return Err(Error::invalid(format!(
                "{what}: types {num_types}/{types_capacity}, table {table_reserve}/{count}"
            )));
        }
        if e.size as u64 != 0x20 + 4 * types_capacity as u64 + 12 * count as u64 {
            return Err(Error::invalid(format!("{what}: export size {} does not match its counts", e.size)));
        }
        let types: Vec<u32> = (0..num_types).map(|_| r.u32()).collect::<Result<_>>()?;
        if (num_types..types_capacity).any(|_| r.u32().ok() != Some(0)) {
            return Err(Error::invalid(format!("{what}: non-zero unused type slot")));
        }
        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let offset = e.offset + r.pos() as u32;
            let entry = Entry {
                key: r.u32()?,
                value: r.array::<4>()?,
                type_index: r.u16()?,
                flags: r.u8()?,
                unk: r.u8()?,
                offset,
            };
            if entry.unk != 0 || entry.type_index >= num_types {
                return Err(Error::invalid(format!("{what}: bad entry {entry:?}")));
            }
            entries.push(entry);
        }
        let export_key = e.key;
        Ok(Collection {
            key,
            export_key,
            class,
            parent,
            table_reserve,
            unk_table_param,
            types,
            types_capacity,
            layout,
            entries,
            offset: e.offset,
        })
    }
}

impl Collection {
    pub fn decode(&self, v: &Vault, schema: &Schema) -> Result<DecodedCollection> {
        let what = format!("collection {:#010x} (class {:#010x})", self.key, self.class);
        let class = schema.class(self.class).ok_or_else(|| Error::invalid(format!("{what}: unknown class")))?;
        let mut out = DecodedCollection::default();
        let ty = |d: &Definition| -> Result<&TypeInfo> {
            schema.type_info(d.type_hash).ok_or_else(|| Error::invalid(format!("{what}: unknown type")))
        };
        match (class.layout_size, self.layout) {
            (0, None) => {}
            (size, Some(base)) if size > 0 => {
                out.pointer_slots.push(PtrTarget { file: FILE_VLT, offset: self.offset + 0x1c });
                let mut layout: Vec<&Definition> = class.definitions.iter().filter(|d| d.in_layout()).collect();
                layout.sort_by_key(|d| d.offset);
                for d in layout {
                    let t = ty(d)?;
                    let at = PtrTarget { file: base.file, offset: base.offset + d.offset as u32 };
                    let value = value::decode(v, t.kind, t.size, at, &mut out.pointer_slots)?;
                    let data = FieldData::Single(value);
                    out.fields.push(Field { key: d.key, type_hash: d.type_hash, source: FieldSource::Layout, data });
                }
            }
            (size, layout) => return Err(Error::invalid(format!("{what}: layout size {size} vs pointer {layout:?}"))),
        }
        for e in &self.entries {
            let d = class
                .definition(e.key)
                .filter(|d| !d.in_layout() && !d.is_static())
                .ok_or_else(|| Error::invalid(format!("{what}: entry {:#010x} is not a table field", e.key)))?;
            let t = ty(d)?;
            if self.types[e.type_index as usize] != d.type_hash {
                return Err(Error::invalid(format!("{what}: entry {:#010x} type differs from its definition", e.key)));
            }
            let mut want = 0;
            if d.is_array() {
                want |= ENTRY_ARRAY;
            }
            if d.is_reference() {
                want |= ENTRY_REFERENCE;
            }
            if want == 0 && d.size <= 4 {
                want = ENTRY_INLINE;
            }
            if e.flags != want {
                return Err(Error::invalid(format!("{what}: entry {e:?} flags, expected {want:#04x}")));
            }
            let slot = PtrTarget { file: FILE_VLT, offset: e.offset + 4 };
            let (source, data) = if want == ENTRY_INLINE {
                if e.value[d.size as usize..].iter().any(|&b| b != 0) {
                    return Err(Error::invalid(format!("{what}: entry {e:?}: bytes after the inline value")));
                }
                let value = value::decode(v, t.kind, t.size, slot, &mut out.pointer_slots)?;
                (FieldSource::Inline, FieldData::Single(value))
            } else {
                let target = v
                    .pointer(slot.file, slot.offset)
                    .ok_or_else(|| Error::invalid(format!("{what}: entry {e:?} has no pointer")))?;
                out.pointer_slots.push(slot);
                let data = if d.is_array() {
                    FieldData::Array(decode_array(v, d, t, target, &mut out.pointer_slots)?)
                } else {
                    FieldData::Single(value::decode(v, t.kind, t.size, target, &mut out.pointer_slots)?)
                };
                (FieldSource::OutOfLine, data)
            };
            out.fields.push(Field { key: e.key, type_hash: d.type_hash, source, data });
        }
        Ok(out)
    }
}

fn decode_array(
    v: &Vault,
    d: &Definition,
    t: &TypeInfo,
    at: PtrTarget,
    slots: &mut Vec<PtrTarget>,
) -> Result<Vec<Value>> {
    let mut r = Reader::be(v.file(at.file));
    r.seek(at.offset as usize)?;
    let (capacity, count, element_size, flags) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
    if capacity != count || element_size != d.size || count > d.max_count {
        return Err(Error::invalid(format!("array at {at:?}: {capacity}/{count} x {element_size} for {d:?}")));
    }
    let start = match flags {
        0 if d.alignment_log2 < 4 => 8,
        ARRAY_ALIGNED_16 if d.alignment_log2 == 4 => {
            if r.u64()? != 0 {
                return Err(Error::invalid(format!("array at {at:?}: non-zero padding")));
            }
            16
        }
        _ => {
            return Err(Error::invalid(format!(
                "array at {at:?}: flags {flags:#06x} for alignment {}",
                d.alignment_log2
            )));
        }
    };
    (0..count as u32)
        .map(|i| {
            let el = PtrTarget { file: at.file, offset: at.offset + start + i * element_size as u32 };
            value::decode(v, t.kind, t.size, el, slots)
        })
        .collect()
}
