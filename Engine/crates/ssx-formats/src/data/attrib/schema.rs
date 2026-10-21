use std::collections::HashMap;

use crate::reader::Reader;
use crate::{Error, Result};

use super::container::{FILE_BIN, PtrTarget, Vault};
use super::value::{self, Value, ValueKind};
use super::{EXPORT_CLASS, EXPORT_DATABASE};

pub const DEF_ARRAY: u8 = 0x01;
pub const DEF_LAYOUT: u8 = 0x02;
pub const DEF_UNK_04: u8 = 0x04;
pub const DEF_STATIC: u8 = 0x10;
pub const DEF_REFERENCE: u8 = 0x20;
const DEF_KNOWN: u8 = DEF_ARRAY | DEF_LAYOUT | DEF_UNK_04 | DEF_STATIC | DEF_REFERENCE;

#[derive(Clone, Debug)]
pub struct TypeInfo {
    pub hash: u32,
    pub name: String,
    pub size: u32,
    pub kind: ValueKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Definition {
    pub key: u32,
    pub type_hash: u32,
    pub offset: u16,
    pub size: u16,
    pub max_count: u16,
    pub flags: u8,
    pub alignment_log2: u8,
}

impl Definition {
    pub fn is_array(&self) -> bool {
        self.flags & DEF_ARRAY != 0
    }
    pub fn in_layout(&self) -> bool {
        self.flags & DEF_LAYOUT != 0
    }
    pub fn is_static(&self) -> bool {
        self.flags & DEF_STATIC != 0
    }
    pub fn is_reference(&self) -> bool {
        self.flags & DEF_REFERENCE != 0
    }
}

#[derive(Clone, Debug)]
pub struct Class {
    pub key: u32,
    pub unk_collection_capacity: u32,
    pub definitions: Vec<Definition>,
    pub static_size: u32,
    pub static_values: Vec<(u32, Value)>,
    pub layout_size: u32,
    pub unk_layout_a: u16,
    pub unk_layout_b: u16,
}

impl Class {
    pub fn definition(&self, key: u32) -> Option<&Definition> {
        self.definitions.iter().find(|d| d.key == key)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Schema {
    pub unk_database: [u32; 2],
    pub types: Vec<TypeInfo>,
    pub classes: Vec<Class>,
    pub pointer_slots: Vec<PtrTarget>,
    type_index: HashMap<u32, usize>,
    class_index: HashMap<u32, usize>,
}

impl Schema {
    pub fn from_vault(v: &Vault) -> Result<Self> {
        let mut s = Schema::default();
        let mut dbs = v.exports.iter().filter(|e| e.type_hash == EXPORT_DATABASE);
        let (Some(db), None) = (dbs.next(), dbs.next()) else {
            return Err(Error::invalid("schema vault must hold exactly one Attrib::DatabaseLoadData"));
        };
        s.parse_database(v, db.offset, db.size)?;
        for e in &v.exports {
            match e.type_hash {
                EXPORT_DATABASE => {}
                EXPORT_CLASS => {
                    let class = s.parse_class(v, e.offset, e.size)?;
                    if class.key != e.key || s.class_index.insert(class.key, s.classes.len()).is_some() {
                        return Err(Error::invalid(format!("class {:#010x}: key mismatch or duplicate", e.key)));
                    }
                    s.classes.push(class);
                }
                t => return Err(Error::invalid(format!("schema vault: unexpected export type {t:#010x}"))),
            }
        }
        Ok(s)
    }

    pub fn type_info(&self, hash: u32) -> Option<&TypeInfo> {
        self.type_index.get(&hash).map(|&i| &self.types[i])
    }

    pub fn class(&self, key: u32) -> Option<&Class> {
        self.class_index.get(&key).map(|&i| &self.classes[i])
    }

    fn header_pointer(&mut self, v: &Vault, offset: u32, what: &str) -> Result<PtrTarget> {
        let at = PtrTarget { file: super::container::FILE_VLT, offset };
        let target = v.pointer(at.file, offset).ok_or_else(|| Error::invalid(format!("{what}: no pointer")))?;
        self.pointer_slots.push(at);
        Ok(target)
    }

    fn parse_database(&mut self, v: &Vault, offset: u32, size: u32) -> Result<()> {
        let mut r = Reader::be(v.vlt);
        r.seek(offset as usize)?;
        self.unk_database = [r.u32()?, r.u32()?];
        let count = r.u32()?;
        if size as u64 != 16 + 4 * count as u64 {
            return Err(Error::invalid(format!("DatabaseLoadData: size {size} for {count} types")));
        }
        let names_at = self.header_pointer(v, offset + 12, "DatabaseLoadData type names")?;
        r.skip(4)?;
        let mut names = Reader::be(v.file(names_at.file));
        names.seek(names_at.offset as usize)?;
        for i in 0..count as usize {
            let name = names.cstr()?;
            let size = r.u32()?;
            let hash = super::fnv1(name.as_bytes());
            let kind = ValueKind::from_type(&name, size)?;
            if self.type_index.insert(hash, i).is_some() {
                return Err(Error::invalid(format!("duplicate type hash {hash:#010x} ({name})")));
            }
            self.types.push(TypeInfo { hash, name, size, kind });
        }
        Ok(())
    }
}

impl Schema {
    fn parse_class(&mut self, v: &Vault, offset: u32, size: u32) -> Result<Class> {
        if size != 32 {
            return Err(Error::invalid(format!("ClassLoadData at {offset:#x}: size {size}, expected 32")));
        }
        let mut r = Reader::be(v.vlt);
        r.seek(offset as usize)?;
        let key = r.u32()?;
        let unk_collection_capacity = r.u32()?;
        let count = r.u32()? as usize;
        r.skip(4)?;
        let static_size = r.u32()?;
        r.skip(4)?;
        let layout_size = r.u32()?;
        let (unk_layout_a, unk_layout_b) = (r.u16()?, r.u16()?);
        let what = format!("class {key:#010x}");
        if (layout_size == 0) != (unk_layout_a == 0 && unk_layout_b == 0) {
            return Err(Error::invalid(format!("{what}: layout size {layout_size} vs {unk_layout_a}/{unk_layout_b}")));
        }
        let defs_at = self.header_pointer(v, offset + 12, &what)?;
        let mut d = Reader::be(v.file(defs_at.file));
        d.seek(defs_at.offset as usize)?;
        let mut definitions: Vec<Definition> = Vec::with_capacity(count);
        for _ in 0..count {
            let def = Definition {
                key: d.u32()?,
                type_hash: d.u32()?,
                offset: d.u16()?,
                size: d.u16()?,
                max_count: d.u16()?,
                flags: d.u8()?,
                alignment_log2: d.u8()?,
            };
            if definitions.iter().any(|o| o.key == def.key) {
                return Err(Error::invalid(format!("{what}: duplicate definition {:#010x}", def.key)));
            }
            self.check_definition(&def, layout_size, static_size)
                .map_err(|e| Error::invalid(format!("{what}, definition {:#010x}: {e}", def.key)))?;
            definitions.push(def);
        }
        let mut layout: Vec<&Definition> = definitions.iter().filter(|d| d.in_layout()).collect();
        layout.sort_by_key(|d| d.offset);
        if layout.windows(2).any(|w| w[0].offset + w[0].size > w[1].offset) {
            return Err(Error::invalid(format!("{what}: overlapping layout fields")));
        }
        let static_slot = v.pointer(super::container::FILE_VLT, offset + 0x14);
        let mut static_values = Vec::new();
        match (static_size, static_slot) {
            (0, None) => {}
            (_, Some(base)) if static_size > 0 && base.file == FILE_BIN => {
                self.pointer_slots.push(PtrTarget { file: super::container::FILE_VLT, offset: offset + 0x14 });
                for def in definitions.iter().filter(|d| d.is_static()) {
                    let ty = &self.types[self.type_index[&def.type_hash]];
                    let at = PtrTarget { file: base.file, offset: base.offset + def.offset as u32 };
                    let value = value::decode(v, ty.kind, ty.size, at, &mut self.pointer_slots)?;
                    static_values.push((def.key, value));
                }
            }
            _ => return Err(Error::invalid(format!("{what}: static size {static_size} vs pointer {static_slot:?}"))),
        }
        Ok(Class {
            key,
            unk_collection_capacity,
            definitions,
            static_size,
            static_values,
            layout_size,
            unk_layout_a,
            unk_layout_b,
        })
    }

    fn check_definition(&self, d: &Definition, layout_size: u32, static_size: u32) -> Result<()> {
        let ty = self.type_info(d.type_hash).ok_or_else(|| Error::invalid("unknown type"))?;
        let fail = |m: &str| Err(Error::invalid(format!("{m} ({d:?}, type {})", ty.name)));
        let end = d.offset as u32 + d.size as u32;
        if d.flags & !DEF_KNOWN != 0 || d.flags & DEF_UNK_04 == 0 {
            return fail("unexpected flags");
        }
        if d.size as u32 != ty.size || d.alignment_log2 > 4 {
            return fail("size differs from the type table or alignment > 16");
        }
        if d.is_array() != (d.max_count != 1) || d.max_count == 0 {
            return fail("array flag and max_count disagree");
        }
        if d.is_reference() != ty.kind.is_reference() {
            return fail("reference flag and type disagree");
        }
        match (d.in_layout(), d.is_static()) {
            (true, true) => fail("both layout and static"),
            (true, false) if d.is_array() || end > layout_size || !d.offset.is_multiple_of(1 << d.alignment_log2) => {
                fail("layout field is an array, outside the layout or misaligned")
            }
            (false, true) if end > static_size => fail("static field outside the static data"),
            (false, false) if d.offset != 0 => fail("table field with a non-zero offset"),
            _ => Ok(()),
        }
    }
}
