use super::*;

#[derive(Clone)]
pub struct Raw {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
    pub method: u16,
    pub crc: u32,
    pub size: u32,
    pub flags: u16,
    pub made_by: u16,
    pub attr: u32,
    pub local_name: Option<Vec<u8>>,
    pub extra: Vec<u8>,
    pub after: Vec<u8>,
    pub offset: Option<u32>,
    pub local: bool,
}

pub fn crc(b: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(b);
    c.sum()
}

pub fn stored(name: &str, content: &[u8]) -> Raw {
    Raw {
        name: name.as_bytes().to_vec(),
        data: content.to_vec(),
        method: 0,
        crc: crc(content),
        size: content.len() as u32,
        flags: 0,
        made_by: 0x0314,
        attr: 0o100_644 << 16,
        local_name: None,
        extra: Vec::new(),
        after: Vec::new(),
        offset: None,
        local: true,
    }
}

pub fn deflated(name: &str, content: &[u8]) -> Raw {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
    std::io::Write::write_all(&mut e, content).unwrap();
    Raw { data: e.finish().unwrap(), method: 8, ..stored(name, content) }
}

pub fn assemble(prefix: &[u8], entries: &[Raw], comment: &[u8]) -> Vec<u8> {
    let mut out = prefix.to_vec();
    let mut offsets = Vec::new();
    for r in entries {
        offsets.push(out.len() as u32);
        if !r.local {
            continue;
        }
        let name = r.local_name.as_ref().unwrap_or(&r.name);
        out.extend(LOCAL_SIG.to_le_bytes());
        for v in [20u16, r.flags, r.method, 0, 0x21] {
            out.extend(v.to_le_bytes());
        }
        let (c, s, z) = if r.flags & FLAG_DESCRIPTOR != 0 { (0, 0, 0) } else { (r.crc, r.data.len() as u32, r.size) };
        for v in [c, s, z] {
            out.extend(v.to_le_bytes());
        }
        out.extend((name.len() as u16).to_le_bytes());
        out.extend((r.extra.len() as u16).to_le_bytes());
        out.extend(name);
        out.extend(&r.extra);
        out.extend(&r.data);
        out.extend(&r.after);
    }
    let cd_at = out.len() as u32;
    for (r, &at) in entries.iter().zip(&offsets) {
        out.extend(CENTRAL_SIG.to_le_bytes());
        for v in [r.made_by, 20, r.flags, r.method, 0, 0x21] {
            out.extend(v.to_le_bytes());
        }
        for v in [r.crc, r.data.len() as u32, r.size] {
            out.extend(v.to_le_bytes());
        }
        for v in [r.name.len() as u16, r.extra.len() as u16, 0, 0, 0] {
            out.extend(v.to_le_bytes());
        }
        out.extend(r.attr.to_le_bytes());
        out.extend(r.offset.unwrap_or(at).to_le_bytes());
        out.extend(&r.name);
        out.extend(&r.extra);
    }
    let cd_len = out.len() as u32 - cd_at;
    out.extend(END_SIG.to_le_bytes());
    let n = entries.len() as u16;
    for v in [0u16, 0, n, n] {
        out.extend(v.to_le_bytes());
    }
    out.extend(cd_len.to_le_bytes());
    out.extend(cd_at.to_le_bytes());
    out.extend((comment.len() as u16).to_le_bytes());
    out.extend(comment);
    out
}
