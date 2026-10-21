mod schema;
mod value;

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use crate::reader::Reader;
use crate::{Error, Result};

pub use schema::*;
pub use value::*;

pub const TAG_STREAM: &[u8; 7] = b"GD.STRM";
pub const TAG_REFL: &[u8; 7] = b"GD.REFL";
pub const TAG_DATA: &[u8; 7] = b"GD.DATA";

pub const OBJECT_MAGIC: u64 = 0x030E_6205;

pub const OBJECT_HEADER_SIZE: usize = 0x20;

pub mod builtin {
    pub const BOOL: u32 = 0x01;
    pub const INT8: u32 = 0x02;
    pub const UINT8: u32 = 0x03;
    pub const INT16: u32 = 0x04;
    pub const UINT16: u32 = 0x05;
    pub const INT32: u32 = 0x06;
    pub const UINT32: u32 = 0x07;
    pub const INT64: u32 = 0x08;
    pub const UINT64: u32 = 0x09;
    pub const FLOAT: u32 = 0x0A;
    pub const VECTOR2: u32 = 0x0B;
    pub const VECTOR3: u32 = 0x0C;
    pub const VECTOR4: u32 = 0x0D;
    pub const QUATERNION: u32 = 0x0E;
    pub const MATRIX44: u32 = 0x0F;
    pub const GUID: u32 = 0x10;
    pub const STRING: u32 = 0x11;
    pub const DATA_REF: u32 = 0x12;
    pub const COUNT: u32 = 0x13;
}

const BUILTINS: [(&str, u32, u32, usize, u32); builtin::COUNT as usize] = [
    ("Invalid", 0, 0, 0, 0),
    ("Bool", 1, 1, 0, 0),
    ("Int8", 1, 1, 0, 0),
    ("UInt8", 1, 1, 0, 0),
    ("Int16", 2, 2, 0, 0),
    ("UInt16", 2, 2, 0, 0),
    ("Int32", 4, 4, 0, 0),
    ("UInt32", 4, 4, 0, 0),
    ("Int64", 8, 8, 0, 0),
    ("UInt64", 8, 8, 0, 0),
    ("Float", 4, 4, 0, 0),
    ("Vector2", 16, 16, 2, builtin::FLOAT),
    ("Vector3", 16, 16, 3, builtin::FLOAT),
    ("Vector4", 16, 16, 4, builtin::FLOAT),
    ("Quaternion", 16, 16, 4, builtin::FLOAT),
    ("Matrix44", 64, 16, 4, builtin::VECTOR4),
    ("Guid", 16, 4, 0, 0),
    ("String", 16, 8, 0, 0),
    ("DataRef", 8, 8, 0, 0),
];

const fn crc32_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const CRC32_TABLE: [u32; 256] = crc32_table();

pub fn crc32_msb(data: &[u8], mut crc: u32) -> u32 {
    for &b in data {
        crc = CRC32_TABLE[((crc >> 24) ^ b as u32) as usize] ^ (crc << 8);
    }
    crc
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub fn is_null(&self) -> bool {
        self.0 == [0; 16]
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-",
            u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]])
        )?;
        for x in &b[8..10] {
            write!(f, "{x:02x}")?;
        }
        f.write_str("-")?;
        for x in &b[10..] {
            write!(f, "{x:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({self})")
    }
}

struct Payload<'a> {
    data: &'a [u8],
    base: usize,
    relocs: Vec<usize>,
    reloc_set: BTreeSet<usize>,
    used: BTreeSet<usize>,
}

impl<'a> Payload<'a> {
    fn bad(&self, at: usize, msg: impl fmt::Display) -> Error {
        Error::invalid(format!("GD blob payload at {:#x}, +{at:#x}: {msg}", self.base))
    }

    fn reader(&self, at: usize) -> Result<Reader<'a>> {
        let mut r = Reader::be(self.data);
        r.seek(at)?;
        Ok(r)
    }

    fn slice(&self, at: usize, len: usize) -> Result<&'a [u8]> {
        self.reader(at)?.bytes(len)
    }

    fn ptr(&mut self, at: usize) -> Result<Option<usize>> {
        let v = self.reader(at)?.u64()?;
        let relocated = self.reloc_set.contains(&at);
        match (v, relocated) {
            (0, false) => Ok(None),
            (0, true) => Err(self.bad(at, "relocated null pointer")),
            (_, false) => Err(self.bad(at, format!("pointer {v:#x} without relocation"))),
            (v, true) if v >= self.data.len() as u64 => Err(self.bad(at, format!("pointer {v:#x} out of range"))),
            (v, true) => {
                if !self.used.insert(at) {
                    return Err(self.bad(at, "pointer slot read twice"));
                }
                Ok(Some(v as usize))
            }
        }
    }

    fn all_relocs_used(&self) -> Result<()> {
        if self.used != self.reloc_set {
            let unused = self.reloc_set.difference(&self.used).count();
            return Err(self.bad(0, format!("{unused} relocations not reached by the decoder")));
        }
        Ok(())
    }
}

type RawBlob<'a> = (usize, [u8; 8], Payload<'a>);

fn split_blobs(data: &[u8]) -> Result<(u32, Vec<RawBlob<'_>>)> {
    let mut r = Reader::be(data);
    r.expect(TAG_STREAM)?;
    let order = r.u8()?;
    let total = r.u32()?;
    let arena = r.u32()?;
    if order != b'b' {
        return Err(Error::invalid(format!("GD stream byte order {order:#x}: only big-endian ('b') is known")));
    }
    if total as usize != data.len() {
        return Err(Error::invalid(format!("GD stream size {total:#x} != file size {:#x}", data.len())));
    }
    let mut blobs = Vec::new();
    while r.remaining() > 0 {
        let at = r.pos();
        let tag: [u8; 8] = r.array()?;
        let size = r.u32()? as usize;
        let reloc_off = r.u32()? as usize;
        if !(tag.starts_with(TAG_REFL) || tag.starts_with(TAG_DATA)) || tag[7] != b'b' {
            return Err(Error::invalid(format!("GD blob at {at:#x}: unknown tag {tag:?}")));
        }
        if size < 0x14
            || reloc_off < 0x10
            || !reloc_off.is_multiple_of(4)
            || reloc_off + 4 > size
            || at + size > data.len()
        {
            return Err(Error::invalid(format!("GD blob at {at:#x}: size {size:#x} / reloc offset {reloc_off:#x}")));
        }
        let blob = &data[at..at + size];
        let mut rr = Reader::be(blob);
        rr.seek(reloc_off)?;
        let n = rr.u32()? as usize;
        if reloc_off + 4 + n * 4 != size {
            return Err(Error::invalid(format!("GD blob at {at:#x}: {n} relocations do not end the blob")));
        }
        let payload = &blob[0x10..reloc_off];
        let mut relocs = Vec::with_capacity(n);
        for _ in 0..n {
            let o = rr.u32()? as usize;
            if !o.is_multiple_of(8) || o + 8 > payload.len() {
                return Err(Error::invalid(format!("GD blob at {at:#x}: bad relocation {o:#x}")));
            }
            relocs.push(o);
        }
        let reloc_set: BTreeSet<usize> = relocs.iter().copied().collect();
        if reloc_set.len() != n {
            return Err(Error::invalid(format!("GD blob at {at:#x}: duplicate relocations")));
        }
        let p = Payload { data: payload, base: at + 0x10, relocs, reloc_set, used: BTreeSet::new() };
        blobs.push((at, tag, p));
        r.seek(at + size)?;
    }
    Ok((arena, blobs))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub blob_offset: usize,
    pub object: Object,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub unk_load_arena_size: u32,
    pub schema: Schema,
    pub assets: Vec<Asset>,
}

impl Stream {
    pub fn parse(data: &[u8]) -> Result<Stream> {
        let (unk_load_arena_size, blobs) = split_blobs(data)?;
        let mut blobs = blobs.into_iter();
        let schema = match blobs.next() {
            Some((_, tag, mut p)) if tag.starts_with(TAG_REFL) => parse_schema(&mut p)?,
            _ => return Err(Error::invalid("GD stream does not start with a REFL blob")),
        };
        let mut assets = Vec::new();
        for (blob_offset, tag, p) in blobs {
            if !tag.starts_with(TAG_DATA) {
                return Err(Error::invalid(format!("GD blob at {blob_offset:#x}: second REFL blob")));
            }
            assets.push(Asset { blob_offset, object: parse_data(&schema, p)? });
        }
        Ok(Stream { unk_load_arena_size, schema, assets })
    }

    pub fn assets_of_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = &'a Asset> + 'a {
        self.assets.iter().filter(move |a| a.object.type_desc(&self.schema).name == class)
    }

    pub fn find(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.object.name(&self.schema) == Some(name))
    }

    pub fn guid_index(&self) -> HashMap<Guid, usize> {
        let mut m = HashMap::new();
        for (i, a) in self.assets.iter().enumerate() {
            if let Some(g) = a.object.guid(&self.schema).filter(|g| !g.is_null()) {
                m.entry(g).or_insert(i);
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_posix_check_value() {
        assert_eq!(!crc32_msb(b"123456789", 0), 0x765E_7680);
    }

    #[test]
    fn guid_text_form() {
        let g = Guid([0x66, 0xB2, 0x2D, 0x55, 0x1B, 0x23, 0x35, 0x46, 0x91, 0x24, 0xDC, 0x2C, 0x96, 0x06, 0xB1, 0x5D]);
        assert_eq!(g.to_string(), "552db266-231b-4635-9124-dc2c9606b15d");
        assert!(Guid::default().is_null());
    }

    fn be32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_be_bytes());
    }

    fn be64(v: &mut Vec<u8>, x: u64) {
        v.extend_from_slice(&x.to_be_bytes());
    }

    struct Ty {
        id: u32,
        first: i32,
        size: u32,
        align: u32,
        flags: [u8; 4],
        slots: Vec<RawField>,
        pool: Vec<u8>,
    }

    fn builtin(id: u32) -> Ty {
        let (name, size, align, ..) = BUILTINS[id as usize];
        let pool = [&[0u8][..], name.as_bytes(), &[0]].concat();
        Ty { id, first: 0, size, align, flags: [0, 1, 0, 0], slots: Vec::new(), pool }
    }

    fn test_asset() -> Ty {
        let decl = [("__name", -2, builtin::STRING, 0), ("__guid", -3, builtin::GUID, 0)];
        let decl = decl.into_iter().chain([
            ("__base", -1, builtin::DATA_REF, 0),
            ("Value", 0, builtin::FLOAT, 0),
            ("Items", 1, builtin::FLOAT, FIELD_DYNAMIC_ARRAY),
        ]);
        let mut pool = b"\0TestAsset\0".to_vec();
        let mut fields = Vec::new();
        for (name, index, type_id, flags) in decl {
            let (_, size, align, ..) = BUILTINS[type_id as usize];
            let (size, align) = if flags & 1 != 0 { (0x10, 8) } else { (size, align) };
            let f = FieldDesc {
                index,
                name: name.into(),
                type_id,
                type_index: 0,
                size,
                offset: 0,
                count: 1,
                flags,
                align: align as u16,
                run: 0,
            };
            fields.push((f, pool.len() as u32));
            pool.extend_from_slice(name.as_bytes());
            pool.push(0);
        }
        fields.sort_by_key(|(f, _)| f.index);
        let descs: Vec<FieldDesc> = fields.iter().map(|(f, _)| f.clone()).collect();
        let (offsets, size, align) = compute_layout(&descs, true);
        let mut slots: Vec<RawField> = fields
            .iter()
            .zip(&offsets)
            .map(|((f, name_offset), &offset)| RawField {
                type_id: f.type_id,
                size: f.size,
                offset: offset as u32,
                name_offset: *name_offset,
                count: 1,
                flags: f.flags,
                align: f.align,
                run: 0,
                type_ptr: 0,
            })
            .collect();
        let runs = compute_runs(&slots);
        slots.iter_mut().zip(runs).for_each(|(s, r)| s.run = r);
        let id = type_layout_hash(&slots, &pool, -3, 1, align);
        Ty { id, first: -3, size: size as u32, align, flags: [1, 0, 0, 0], slots, pool }
    }

    fn blob(tag: &[u8; 7], payload: &[u8], relocs: &[u32]) -> Vec<u8> {
        let mut v = tag.to_vec();
        v.push(b'b');
        let reloc_off = 0x10 + payload.len() as u32;
        be32(&mut v, reloc_off + 4 + 4 * relocs.len() as u32);
        be32(&mut v, reloc_off);
        v.extend_from_slice(payload);
        be32(&mut v, relocs.len() as u32);
        relocs.iter().for_each(|&r| be32(&mut v, r));
        v
    }

    fn test_stream() -> (Vec<u8>, u32) {
        let mut types = vec![
            builtin(builtin::FLOAT),
            builtin(builtin::GUID),
            builtin(builtin::STRING),
            builtin(builtin::DATA_REF),
            test_asset(),
        ];
        types.sort_by_key(|t| t.id);
        let id_of = |types: &[Ty]| types.iter().find(|t| t.first == -3).map_or(0, |t| t.id);
        let asset_id = id_of(&types);
        let mut offsets = Vec::new();
        let mut at = 8 + 8 * types.len();
        for t in &types {
            at = at.next_multiple_of(8);
            offsets.push(at);
            at += 0x20 + 0x20 * t.slots.len() + t.pool.len();
        }
        let target = |id: u32| offsets[types.iter().position(|t| t.id == id).unwrap()] as u64;
        let mut p = Vec::new();
        let mut relocs = Vec::new();
        be64(&mut p, types.len() as u64);
        for (i, &o) in offsets.iter().enumerate() {
            relocs.push(8 + 8 * i as u32);
            be64(&mut p, o as u64);
        }
        for (t, &o) in types.iter().zip(&offsets) {
            p.resize(o, 0);
            let last = t.first + t.slots.len() as i32 - 1;
            for x in
                [t.first as u32, last as u32, t.size, t.align, 0x20 + 0x20 * t.slots.len() as u32, t.pool.len() as u32]
            {
                be32(&mut p, x);
            }
            p.extend_from_slice(&t.flags);
            be32(&mut p, t.id);
            for f in &t.slots {
                for x in [f.type_id, f.size, f.offset, f.name_offset] {
                    be32(&mut p, x);
                }
                for x in [f.count, f.flags, f.align, f.run] {
                    p.extend_from_slice(&x.to_be_bytes());
                }
                relocs.push(p.len() as u32);
                be64(&mut p, target(f.type_id));
            }
            p.extend_from_slice(&t.pool);
        }
        p.resize(p.len().next_multiple_of(4), 0);
        let mut file = blob(TAG_REFL, &p, &relocs);
        let t = types.iter().find(|t| t.id == asset_id).unwrap();
        let off = |name: &str| {
            let k = t.pool.windows(name.len() + 1).position(|w| w == [name.as_bytes(), &[0]].concat()).unwrap() as u32;
            t.slots.iter().find(|s| s.name_offset == k).unwrap().offset as usize
        };
        let mut d = Vec::new();
        for x in [OBJECT_MAGIC, 0, asset_id as u64] {
            be64(&mut d, x);
        }
        be32(&mut d, 0);
        d.extend_from_slice(&[0, 0x20, 0, 0]);
        let body = d.len();
        d.resize(body + t.size as usize, 0);
        let end = d.len();
        d[body + off("__guid")..][..16].copy_from_slice(&[7; 16]);
        d[body + off("Value")..][..4].copy_from_slice(&1.5f32.to_be_bytes());
        let (name_at, items_at) = (end, (end + 6).next_multiple_of(4));
        let name = body + off("__name");
        d[name..name + 16].copy_from_slice(&[[0, 0, 0, 6, 0, 0, 0, 6], (name_at as u64).to_be_bytes()].concat());
        let items = body + off("Items");
        d[items..items + 16].copy_from_slice(&[[0, 0, 0, 2, 0, 0, 0, 2], (items_at as u64).to_be_bytes()].concat());
        d.extend_from_slice(b"probe\0");
        d.resize(items_at, 0);
        d.extend_from_slice(&[1.0f32.to_be_bytes(), 2.0f32.to_be_bytes()].concat());
        file.extend(blob(TAG_DATA, &d, &[(name + 8) as u32, (items + 8) as u32]));
        let mut out = TAG_STREAM.to_vec();
        out.push(b'b');
        be32(&mut out, 16 + file.len() as u32);
        be32(&mut out, 0x1234);
        out.extend(file);
        (out, asset_id)
    }

    #[test]
    fn synthetic_stream_round_trip() {
        let (data, id) = test_stream();
        let s = Stream::parse(&data).unwrap();
        assert_eq!(s.unk_load_arena_size, 0x1234);
        assert_eq!(s.schema.types.len(), 5);
        let a = &s.assets[0].object;
        assert_eq!(a.type_desc(&s.schema).id, id);
        assert_eq!(a.name(&s.schema), Some("probe"));
        assert_eq!(a.guid(&s.schema), Some(Guid([7; 16])));
        assert_eq!(a.get(&s.schema, "Value").and_then(Value::as_f32), Some(1.5));
        assert_eq!(a.get(&s.schema, "Items"), Some(&Value::Array(vec![Value::Float(1.0), Value::Float(2.0)])));
        assert_eq!(s.find("probe").map(|x| x.blob_offset), Some(s.assets[0].blob_offset));
        assert_eq!(s.guid_index().len(), 1);
    }

    #[test]
    fn synthetic_stream_corruptions_are_rejected() {
        let (data, _) = test_stream();
        let mut bad = data.clone();
        bad[11] ^= 1;
        assert!(Stream::parse(&bad).is_err());
        assert!(Stream::parse(&data[..data.len() - 4]).is_err());
        let refl_size = u32::from_be_bytes(data[0x18..0x1C].try_into().unwrap()) as usize;
        for i in (0x20 + 8..0x10 + refl_size - 0x10).step_by(7) {
            let mut bad = data.clone();
            bad[i] ^= 0x10;
            assert!(Stream::parse(&bad).is_err(), "flip at {i:#x} accepted");
        }
    }
}
