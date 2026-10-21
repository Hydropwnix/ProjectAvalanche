use crate::reader::Reader;
use crate::{Error, Result};

pub const HEADER_SIZE: usize = 0x28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Event,
    Function,
    Global,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub kind: Kind,
    pub name: String,
    pub id: u32,
    pub default: i32,
}

#[derive(Clone, Debug)]
pub struct CsiFile {
    pub unk_04: [u8; 6],
    pub interface: u16,
    pub entries: Vec<Entry>,
}

impl CsiFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(b"MOIR")?;
        let unk_04: [u8; 6] = r.array()?;
        let counts = [r.u16()? as usize, r.u16()? as usize, r.u16()? as usize];
        let interface = r.u16()?;
        if r.bytes(HEADER_SIZE - 0x12)?.iter().any(|&b| b != 0) {
            return Err(Error::invalid("MOIR: runtime header area not zero"));
        }
        let mut raw = Vec::new();
        for (k, kind) in [Kind::Event, Kind::Function, Kind::Global].into_iter().enumerate() {
            for _ in 0..counts[k] {
                if r.u32()? != 0 {
                    return Err(Error::invalid("MOIR: subscriber word not zero"));
                }
                let default = if kind == Kind::Global { r.i32()? } else { 0 };
                let name_offset = r.u32()? as usize;
                let hash = r.u16()?;
                if r.u16()? != 0 {
                    return Err(Error::invalid("MOIR: entry padding not zero"));
                }
                raw.push((kind, default, name_offset, hash));
            }
        }
        let mut entries = Vec::with_capacity(raw.len());
        for (kind, default, name_offset, hash) in raw {
            if name_offset != r.pos() {
                return Err(Error::invalid(format!("MOIR: name at {name_offset:#x}, expected {:#x}", r.pos())));
            }
            let name = r.cstr()?;
            entries.push(Entry { kind, name, id: u32::from(interface) << 16 | u32::from(hash), default });
        }
        let tail = &data[r.pos()..];
        if tail.len() > 3 || tail.iter().any(|&b| b != 0) {
            return Err(Error::invalid(format!("MOIR: {} trailing bytes", tail.len())));
        }
        Ok(Self { unk_04, interface, entries })
    }

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }
}
