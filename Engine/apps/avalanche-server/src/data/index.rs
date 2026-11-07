use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use super::manifest::{hex, parse_hash};

pub const INDEX_FILE: &str = "content.tsv";
pub const HEADER: &str = "path\tsize\tblake3";
const MAX_BYTES: u64 = 1 << 20;
const MAX_ROWS: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    pub rows: BTreeMap<String, (u64, [u8; 32])>,
}

impl Index {
    pub fn to_tsv(&self) -> String {
        let mut s = format!("{HEADER}\n");
        for (path, (size, hash)) in &self.rows {
            s.push_str(&format!("{path}\t{size}\t{}\n", hex(hash)));
        }
        s
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("{0}")]
    Io(String),
    #[error("line {line}: {message}")]
    Line { line: usize, message: String },
}

pub fn hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

pub fn hash_file(path: &Path) -> std::io::Result<(u64, [u8; 32])> {
    let mut f = std::fs::File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok((size, *h.finalize().as_bytes()));
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
}

pub fn read(data: &Path) -> Result<Index, IndexError> {
    let path = data.join(INDEX_FILE);
    let meta = std::fs::symlink_metadata(&path).map_err(|e| IndexError::Io(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(IndexError::Io(format!("{}: a link or a folder, not a file", path.display())));
    }
    let mut text = String::new();
    std::fs::File::open(&path)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_string(&mut text))
        .map_err(|e| IndexError::Io(format!("{}: {e}", path.display())))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(IndexError::Io(format!("{}: larger than {MAX_BYTES} bytes", path.display())));
    }
    parse(&text)
}

pub fn parse(text: &str) -> Result<Index, IndexError> {
    let err = |line: usize, message: String| IndexError::Line { line, message };
    let mut lines = text.split_terminator('\n').enumerate();
    match lines.next() {
        Some((_, h)) if h == HEADER => {}
        _ => return Err(err(1, format!("expected the header {HEADER:?}"))),
    }
    let mut index = Index::default();
    let mut last: Option<String> = None;
    for (i, line) in lines {
        let n = i + 1;
        if index.rows.len() >= MAX_ROWS {
            return Err(err(n, format!("more than {MAX_ROWS} rows")));
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let [path, size, h] = cols[..] else { return Err(err(n, "expected 3 columns".into())) };
        ssx_assets::subset::Member::from_data_path(path).map_err(|e| err(n, format!("{e:#}")))?;
        let canonical =
            !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()) && (size == "0" || !size.starts_with('0'));
        let size = canonical
            .then(|| size.parse::<u64>().ok())
            .flatten()
            .ok_or_else(|| err(n, format!("size {:?}", size.chars().take(24).collect::<String>())))?;
        let h = parse_hash(h).ok_or_else(|| err(n, "the hash: 64 lower-case hex digits".into()))?;
        if last.as_deref().is_some_and(|l| l >= path) {
            return Err(err(n, "not sorted by path, or twice".into()));
        }
        last = Some(path.to_string());
        index.rows.insert(path.to_string(), (size, h));
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_round_trips_and_refuses_with_the_line() {
        let mut ix = Index::default();
        ix.rows.insert("big/bootcache/b.bin".into(), (3, hash(b"abc")));
        ix.rows.insert("big/bootcache/a.bin".into(), (0, hash(b"")));
        let text = ix.to_tsv();
        assert!(text.starts_with("path\tsize\tblake3\nbig/bootcache/a.bin\t0\taf1349b9"), "{text}");
        assert_eq!(parse(&text).expect("reads"), ix);
        let line = |e: IndexError| e.to_string();
        let rows: Vec<&str> = text.lines().collect();
        assert!(line(parse("path\tsize\tsha1\n").unwrap_err()).starts_with("line 1"));
        assert!(
            line(parse(&format!("{HEADER}\n{}\n{}\n", rows[2], rows[1])).unwrap_err())
                .starts_with("line 3: not sorted")
        );
        assert!(
            line(parse(&format!("{HEADER}\n{}\n", rows[1].replace("\t0\t", "\t00\t"))).unwrap_err()).contains("size")
        );
        assert!(
            line(parse(&format!("{HEADER}\n{}\n", rows[1].replace("big/", "big/../"))).unwrap_err())
                .starts_with("line 2")
        );
        assert!(line(parse(&format!("{HEADER}\n{}x\n", rows[1])).unwrap_err()).contains("hash"));
        assert!(line(parse(&format!("{HEADER}\n{}\textra\n", rows[1])).unwrap_err()).contains("3 columns"));
    }
}
