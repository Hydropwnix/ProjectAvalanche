use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        ContentHash(*blake3::hash(bytes).as_bytes())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("bad content hash {} (64 lower-case hex digits)", ssx_core::text::quoted(.0))]
pub struct HashParseError(pub String);

impl FromStr for ContentHash {
    type Err = HashParseError;
    fn from_str(s: &str) -> Result<Self, HashParseError> {
        let bad = || HashParseError(s.chars().take(80).collect());
        if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(bad());
        }
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
        }
        Ok(ContentHash(out))
    }
}

pub fn hash_reader(mut r: impl Read) -> io::Result<(u64, ContentHash)> {
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        size += n as u64;
    }
    Ok((size, ContentHash(*h.finalize().as_bytes())))
}

pub fn hash_file(path: &Path) -> io::Result<(u64, ContentHash)> {
    hash_reader(std::fs::File::open(path)?)
}

pub type Hashed = io::Result<(u64, ContentHash)>;

pub fn hash_files(paths: &[PathBuf], threads: usize) -> Vec<Hashed> {
    let next = AtomicUsize::new(0);
    let out: Vec<Mutex<Option<Hashed>>> = paths.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..threads.clamp(1, paths.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(p) = paths.get(i) else { break };
                    *out[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(hash_file(p));
                }
            });
        }
    });
    out.into_iter()
        .map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()).unwrap_or_else(|| Err(io::ErrorKind::Other.into())))
        .collect()
}

fn check_index_path(p: &str) -> Result<(), String> {
    let bad = p.is_empty()
        || p.starts_with('/')
        || p.chars().any(|c| c == '\\' || c.is_control())
        || p.split('/').any(|s| s.is_empty() || s == "." || s == "..");
    if bad { Err(format!("bad path {}", ssx_core::text::quoted(p))) } else { Ok(()) }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentIndex {
    pub entries: BTreeMap<String, (u64, ContentHash)>,
}

pub const INDEX_HEADER: &str = "path\tsize\tblake3";

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("content index line {line}: {}", ssx_core::text::Text::new(.message))]
pub struct IndexError {
    pub line: usize,
    pub message: String,
}

impl ContentIndex {
    pub fn to_tsv(&self) -> String {
        let mut s = String::from(INDEX_HEADER);
        s.push('\n');
        for (path, (size, hash)) in &self.entries {
            s.push_str(&format!("{path}\t{size}\t{hash}\n"));
        }
        s
    }

    pub fn from_tsv(text: &str) -> Result<Self, IndexError> {
        let err = |line: usize, message: String| IndexError { line, message };
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, h)) if h == INDEX_HEADER => {}
            _ => return Err(err(1, format!("expected the header {INDEX_HEADER:?}"))),
        }
        let mut entries = BTreeMap::new();
        for (i, l) in lines {
            let cols: Vec<&str> = l.split('\t').collect();
            let [path, size, hash] = cols[..] else { return Err(err(i + 1, "expected 3 columns".into())) };
            check_index_path(path).map_err(|m| err(i + 1, m))?;
            let canonical =
                !size.is_empty() && size.bytes().all(|b| b.is_ascii_digit()) && (size == "0" || !size.starts_with('0'));
            let size = canonical.then(|| size.parse::<u64>().ok()).flatten().ok_or_else(|| {
                err(
                    i + 1,
                    format!("bad size {} (decimal digits, no sign or leading zero)", ssx_core::text::quoted(size)),
                )
            })?;
            let hash = hash.parse().map_err(|e: HashParseError| err(i + 1, e.to_string()))?;
            if entries.insert(path.to_string(), (size, hash)).is_some() {
                return Err(err(i + 1, format!("{path} listed twice")));
            }
        }
        Ok(ContentIndex { entries })
    }

    pub fn paths_with(&self, hash: ContentHash) -> impl Iterator<Item = &str> {
        self.entries.iter().filter(move |(_, (_, h))| *h == hash).map(|(p, _)| p.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            ContentHash::of(b"").to_string(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        let (n, h) = hash_reader(&b"abc"[..]).unwrap();
        assert_eq!((n, h), (3, ContentHash::of(b"abc")));
        let big = vec![7u8; (1 << 20) * 2 + 5];
        assert_eq!(hash_reader(&big[..]).unwrap(), (big.len() as u64, ContentHash::of(&big)));
    }

    #[test]
    fn one_spelling_per_hash() {
        let h = ContentHash::of(b"ssx");
        assert_eq!(h.to_string().parse::<ContentHash>().unwrap(), h);
        assert!("00".parse::<ContentHash>().is_err());
        assert!("g".repeat(64).parse::<ContentHash>().is_err());
        assert!(h.to_string().to_uppercase().parse::<ContentHash>().is_err(), "upper case");
        let plus = format!("+{}", &h.to_string()[1..]);
        assert!(plus.parse::<ContentHash>().is_err(), "a sign: u8::from_str_radix(\"+f\", 16) would accept it");
        assert!(format!("{}é", &h.to_string()[..62]).parse::<ContentHash>().is_err());
    }

    #[test]
    fn index_tsv() {
        let mut idx = ContentIndex::default();
        idx.entries.insert("big/b.bin".into(), (3, ContentHash::of(b"abc")));
        idx.entries.insert("big/a.bin".into(), (0, ContentHash::of(b"")));
        let text = idx.to_tsv();
        assert!(text.starts_with("path\tsize\tblake3\nbig/a.bin\t0\taf1349"));
        assert_eq!(ContentIndex::from_tsv(&text).unwrap(), idx);
        assert_eq!(idx.paths_with(ContentHash::of(b"abc")).collect::<Vec<_>>(), ["big/b.bin"]);
        assert_eq!(ContentIndex::from_tsv("x").unwrap_err().line, 1);
        let dup = format!("{INDEX_HEADER}\na\t0\t{0}\na\t0\t{0}\n", ContentHash::of(b""));
        assert_eq!(ContentIndex::from_tsv(&dup).unwrap_err().line, 3);
        let h = ContentHash::of(b"");
        for (path, size) in
            [("a", "+5"), ("a", "05"), ("a", ""), ("a", "-1"), ("a", "1e3"), ("a", "99999999999999999999")]
        {
            let row = format!("{INDEX_HEADER}\n{path}\t{size}\t{h}\n");
            assert!(ContentIndex::from_tsv(&row).is_err(), "size {size:?}");
        }
        for path in ["", "/a", "a//b", "../a", "a/./b", "a\\b", "a\u{1}b"] {
            let row = format!("{INDEX_HEADER}\n{path}\t0\t{h}\n");
            assert!(ContentIndex::from_tsv(&row).is_err(), "path {path:?}");
        }
        assert!(ContentIndex::from_tsv(&format!("{INDEX_HEADER}\nbig/a b/$$$.x\t0\t{h}\n")).is_ok());
    }

    #[test]
    fn parallel_matches_serial() {
        let dir = std::env::temp_dir().join(format!("ssx-content-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths: Vec<PathBuf> = (0..9u8)
            .map(|i| {
                let p = dir.join(format!("f{i}"));
                std::fs::write(&p, vec![i; usize::from(i) * 1000]).unwrap();
                p
            })
            .collect();
        let mut with_missing = paths.clone();
        with_missing.push(dir.join("missing"));
        let got = hash_files(&with_missing, 4);
        for (p, r) in paths.iter().zip(&got) {
            assert_eq!(r.as_ref().unwrap(), &hash_file(p).unwrap());
        }
        assert!(got.last().unwrap().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
