use std::io::Write;

use super::{CENTRAL_SIG, END_SIG, LOCAL_SIG, Method, ZipError, ZipLimits, refused};
use crate::path::check_package_path;

pub struct PackFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

const MADE_BY: u16 = 0x0314;
const NEEDED: u16 = 20;
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;
const FILE_ATTR: u32 = 0o100_644 << 16;
const DEFLATE_LEVEL: u32 = 6;

struct Written {
    name: String,
    method: Method,
    crc: u32,
    stored: u32,
    size: u32,
    offset: u32,
}

fn deflate(bytes: &[u8]) -> Result<Vec<u8>, ZipError> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(DEFLATE_LEVEL));
    e.write_all(bytes)?;
    Ok(e.finish()?)
}

pub fn write_zip<W: Write>(
    mut out: W,
    files: impl IntoIterator<Item = Result<PackFile, ZipError>>,
    limits: &ZipLimits,
) -> Result<u64, ZipError> {
    let mut written: Vec<Written> = Vec::new();
    let (mut at, mut total) = (0u64, 0u64);
    let fits = |n: u64| u32::try_from(n).ok().filter(|_| n <= limits.max_archive_bytes);
    for file in files {
        let PackFile { name, bytes } = file?;
        check_package_path(&name, false).map_err(|e| refused(format!("{name}: {e}")))?;
        if let Some(prev) = written.last()
            && prev.name.as_bytes() >= name.as_bytes()
        {
            return Err(refused(format!("{name}: after {} (names are packed once, in ascending order)", prev.name)));
        }
        if written.len() >= limits.max_entries.min(0xFFFE) {
            return Err(refused(format!("more than {} files", limits.max_entries.min(0xFFFE))));
        }
        let size = bytes.len() as u64;
        total += size;
        if size > limits.max_file_bytes || total > limits.max_total_bytes {
            return Err(refused(format!(
                "{name}: a file over {} or a package over {} bytes",
                limits.max_file_bytes, limits.max_total_bytes
            )));
        }
        let packed = deflate(&bytes)?;
        let (method, data) = if packed.len() as u64 >= size || limits.bomb(size, packed.len() as u64) {
            (Method::Stored, &bytes[..])
        } else {
            (Method::Deflated, &packed[..])
        };
        let mut crc = flate2::Crc::new();
        crc.update(&bytes);
        let offset = fits(at)
            .ok_or_else(|| refused(format!("{name}: the archive passes {} bytes", limits.max_archive_bytes)))?;
        let w = Written { name, method, crc: crc.sum(), stored: data.len() as u32, size: size as u32, offset };
        let mut h = Vec::with_capacity(30 + w.name.len());
        h.extend(LOCAL_SIG.to_le_bytes());
        for v in [NEEDED, 0, method.code(), DOS_TIME, DOS_DATE] {
            h.extend(v.to_le_bytes());
        }
        for v in [w.crc, w.stored, w.size] {
            h.extend(v.to_le_bytes());
        }
        h.extend((w.name.len() as u16).to_le_bytes());
        h.extend(0u16.to_le_bytes());
        h.extend(w.name.as_bytes());
        out.write_all(&h)?;
        out.write_all(data)?;
        at += (h.len() + data.len()) as u64;
        written.push(w);
    }
    if written.is_empty() {
        return Err(refused("no files to pack"));
    }
    let cd_offset =
        fits(at).ok_or_else(|| refused(format!("the archive passes {} bytes", limits.max_archive_bytes)))?;
    let mut cd = Vec::new();
    for w in &written {
        cd.extend(CENTRAL_SIG.to_le_bytes());
        for v in [MADE_BY, NEEDED, 0, w.method.code(), DOS_TIME, DOS_DATE] {
            cd.extend(v.to_le_bytes());
        }
        for v in [w.crc, w.stored, w.size] {
            cd.extend(v.to_le_bytes());
        }
        for v in [w.name.len() as u16, 0, 0, 0, 0] {
            cd.extend(v.to_le_bytes());
        }
        cd.extend(FILE_ATTR.to_le_bytes());
        cd.extend(w.offset.to_le_bytes());
        cd.extend(w.name.as_bytes());
    }
    let end = at + cd.len() as u64 + 22;
    let cd_size = fits(cd.len() as u64)
        .filter(|_| fits(end).is_some())
        .ok_or_else(|| refused("the archive passes its size cap"))?;
    let n = written.len() as u16;
    cd.extend(END_SIG.to_le_bytes());
    for v in [0u16, 0, n, n] {
        cd.extend(v.to_le_bytes());
    }
    cd.extend(cd_size.to_le_bytes());
    cd.extend(cd_offset.to_le_bytes());
    cd.extend(0u16.to_le_bytes());
    out.write_all(&cd)?;
    out.flush()?;
    Ok(end)
}
