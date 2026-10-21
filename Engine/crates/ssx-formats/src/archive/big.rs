use crate::reader::Reader;
use crate::{Error, Result};

const HEADER_SIZE: usize = 0x30;

#[derive(Clone, Debug)]
pub struct BigHeader {
    pub version: u16,
    pub file_count: u32,
    pub flags: u16,
    pub alignment_shift: u8,
    pub base_header_size: u32,
    pub name_header_size: u32,
    pub path_entry_size: u8,
    pub dir_entry_size: u8,
    pub dir_count: u16,
    pub archive_size: u64,
}

#[derive(Clone, Debug)]
pub struct BigEntry {
    pub offset: u64,
    pub size: u32,
    pub unk_zsize: u32,
    pub hash: u32,
    pub unk_flag: u8,
    pub dir: String,
    pub name: String,
}

impl BigEntry {
    pub fn path(&self) -> String {
        if self.dir.is_empty() || self.dir == "." { self.name.clone() } else { format!("{}/{}", self.dir, self.name) }
    }

    pub fn extension(&self) -> &str {
        self.name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("")
    }
}

#[derive(Clone, Debug)]
pub struct BigArchive {
    pub header: BigHeader,
    pub entries: Vec<BigEntry>,
}

impl BigArchive {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(b"EB")?;
        let header = BigHeader {
            version: r.u16()?,
            file_count: r.u32()?,
            flags: r.u16()?,
            alignment_shift: r.u8()?,
            base_header_size: {
                let _reserved = r.u8()?;
                r.u32()?
            },
            name_header_size: r.u32()?,
            path_entry_size: r.u8()?,
            dir_entry_size: r.u8()?,
            dir_count: r.u16()?,
            archive_size: r.u64()?,
        };
        if header.version != 3 {
            return Err(Error::invalid(format!("unsupported EB big version {}", header.version)));
        }
        if header.path_entry_size < 3 {
            return Err(Error::invalid("EB big path entries too small"));
        }

        let n = header.file_count as usize;
        r.seek(HEADER_SIZE)?;
        let mut index = Vec::with_capacity(n);
        for _ in 0..n {
            index.push((r.u32()?, r.u32()?, r.u32()?, r.u32()?));
        }
        let flags = r.bytes(n)?;

        r.seek(header.base_header_size as usize)?;
        let mut paths = Vec::with_capacity(n);
        for _ in 0..n {
            let dir = r.u16()? as usize;
            let name = r.fixed_str(header.path_entry_size as usize - 2)?;
            paths.push((dir, name));
        }
        r.align(16)?;
        let mut dirs = Vec::with_capacity(header.dir_count as usize);
        for _ in 0..header.dir_count {
            dirs.push(r.fixed_str(header.dir_entry_size as usize)?);
        }

        let entries = index
            .into_iter()
            .zip(paths)
            .zip(flags)
            .map(|(((offset, zsize, size, hash), (dir, name)), &flag)| {
                let dir = dirs
                    .get(dir)
                    .cloned()
                    .ok_or_else(|| Error::invalid(format!("directory index {dir} out of range for {name}")))?;
                Ok(BigEntry {
                    offset: (offset as u64) << header.alignment_shift,
                    size,
                    unk_zsize: zsize,
                    hash,
                    unk_flag: flag,
                    dir,
                    name,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self { header, entries })
    }

    pub fn table_size(header_bytes: &[u8; HEADER_SIZE]) -> Result<usize> {
        let mut r = Reader::be(header_bytes);
        r.expect(b"EB")?;
        r.seek(0x0c)?;
        let base = r.u32()? as usize;
        let names = r.u32()? as usize;
        Ok(base + names)
    }

    pub fn find(&self, path: &str) -> Option<&BigEntry> {
        let hash = path_hash(path);
        let start = self.entries.partition_point(|e| e.hash < hash);
        self.entries[start..].iter().take_while(|e| e.hash == hash).find(|e| e.path() == path)
    }

    pub fn member<'a>(&self, data: &'a [u8], entry: &BigEntry) -> Result<&'a [u8]> {
        let start = entry.offset as usize;
        let end = start + entry.size as usize;
        data.get(start..end).ok_or(Error::UnexpectedEof {
            offset: start,
            wanted: entry.size as usize,
            available: data.len().saturating_sub(start),
        })
    }
}

pub fn path_hash(path: &str) -> u32 {
    path.bytes().fold(5381u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32))
}

pub const HEADER_LEN: usize = HEADER_SIZE;
