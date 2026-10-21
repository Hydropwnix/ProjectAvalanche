use super::{Coverage, cstr_at, fnv1, header, i32_at, u8_at, u32_at, zero_pad};
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"SPC0";

#[derive(Clone, Debug)]
pub struct SentenceData {
    pub version: (u16, u16),
    pub buckets: u32,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub sentences: Vec<Sentence>,
}

#[derive(Clone, Debug)]
pub struct Sentence {
    pub unk_00: u32,
    pub probability: u32,
    pub rank: u32,
    pub phrases: Vec<Phrase>,
}

#[derive(Clone, Debug)]
pub struct Phrase {
    pub pool: u8,
    pub kind: u8,
    pub extra: u8,
    pub tags: Vec<Tag>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    pub kind: u8,
    pub first: i8,
    pub end: i8,
    pub cur: i8,
    pub id: u32,
    pub value: i32,
}

impl SentenceData {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let version = header(d, MAGIC)?;
        let (rel, nb, ptr) = (u32_at(d, 0xC)?, u32_at(d, 0x10)?, u32_at(d, 0x14)?);
        if rel != 0 || ptr != 0 || nb == 0 || nb > 0x10000 {
            return Err(Error::invalid(format!("SPC0 header {rel:#x} {nb} {ptr:#x}")));
        }
        let mut cov = Coverage::default();
        cov.claim(0, 0x18 + 8 * nb as usize);
        let mut groups = Vec::new();
        for b in 0..nb as usize {
            let (n, off) = (u32_at(d, 0x18 + 8 * b)? as usize, i32_at(d, 0x1C + 8 * b)?);
            if n == 0 {
                if off != -1 {
                    return Err(Error::invalid(format!("SPC0 empty bucket {b} with offset {off:#x}")));
                }
                continue;
            }
            let o = usize::try_from(off).map_err(|_| Error::invalid("SPC0 bucket offset"))?;
            cov.claim(o, o + 8 * n);
            for k in 0..n {
                let (key, g) = (u32_at(d, o + 8 * k)?, u32_at(d, o + 8 * k + 4)? as usize);
                let (ns, z) = (u32_at(d, g)? as usize, u32_at(d, g + 4)?);
                if key != 0 || z != 0 {
                    return Err(Error::invalid(format!("SPC0 group at {g:#x}: runtime words {key:#x} {z:#x}")));
                }
                let (name, end) = cstr_at(d, g + 8 + 4 * ns)?;
                zero_pad(d, end, "SPC0 group name")?;
                cov.claim_padded(g, end);
                if fnv1(name.as_bytes()) % nb != b as u32 {
                    return Err(Error::invalid(format!("SPC0 group {name} in bucket {b}")));
                }
                let mut sentences = Vec::with_capacity(ns);
                for j in 0..ns {
                    let s = u32_at(d, g + 8 + 4 * j)? as usize;
                    sentences.push(Self::sentence(d, s, &mut cov)?);
                }
                groups.push(Group { name, sentences });
            }
        }
        cov.check(d.len(), "SPC0")?;
        Ok(Self { version, buckets: nb, groups })
    }

    fn sentence(d: &[u8], s: usize, cov: &mut Coverage) -> Result<Sentence> {
        let (unk_00, probability, rank, np) =
            (u32_at(d, s)?, u32_at(d, s + 4)?, u32_at(d, s + 8)?, u32_at(d, s + 12)? as usize);
        cov.claim(s, s + 16 + 4 * np);
        let mut phrases = Vec::with_capacity(np);
        for i in 0..np {
            let p = u32_at(d, s + 16 + 4 * i)? as usize;
            let (pool, kind, nt, extra) = (u8_at(d, p)?, u8_at(d, p + 1)?, u8_at(d, p + 2)? as usize, u8_at(d, p + 3)?);
            if extra != 0 {
                return Err(Error::invalid(format!("SPC0 phrase at {p:#x} with extra data (size unknown)")));
            }
            let mut tags = Vec::with_capacity(nt);
            for t in 0..nt {
                let q = p + 4 + 12 * t;
                let tag = Tag {
                    kind: u8_at(d, q)?,
                    first: u8_at(d, q + 1)? as i8,
                    end: u8_at(d, q + 2)? as i8,
                    cur: u8_at(d, q + 3)? as i8,
                    id: u32_at(d, q + 4)?,
                    value: i32_at(d, q + 8)?,
                };
                let ok = match tag.kind {
                    1 => tag.first >= 0 && tag.first < tag.end && tag.cur == tag.first,
                    2 => (tag.first, tag.end, tag.cur) == (-1, -1, -1),
                    _ => false,
                };
                if !ok {
                    return Err(Error::invalid(format!("SPC0 tag at {q:#x}: {tag:?}")));
                }
                tags.push(tag);
            }
            cov.claim(p, p + 4 + 12 * nt);
            phrases.push(Phrase { pool, kind, extra, tags });
        }
        Ok(Sentence { unk_00, probability, rank, phrases })
    }

    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }
}
