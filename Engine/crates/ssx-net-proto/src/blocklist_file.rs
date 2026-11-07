use std::path::Path;

use crate::keys::{KEY_SIZE, from_hex, to_hex};
use crate::master::{MAX_BLOCKLIST_BYTES, MAX_BLOCKLIST_ENTRIES};

pub const SERVERS_HEADER: &str = "Project Avalanche blocked servers v1";
pub const RESOURCES_HEADER: &str = "Project Avalanche blocked resources v1";
pub const MAX_REASON_CHARS: usize = 200;

pub fn parse(text: &[u8], header: &str, shown: &str) -> Result<Vec<([u8; KEY_SIZE], String)>, String> {
    if text.len() > MAX_BLOCKLIST_BYTES {
        return Err(format!("{shown}: larger than {MAX_BLOCKLIST_BYTES} bytes"));
    }
    let text = std::str::from_utf8(text).map_err(|_| format!("{shown}: not UTF-8"))?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
    if lines.next() != Some(header) {
        return Err(format!("{shown}:1: the first line must be {header:?}"));
    }
    let mut out: Vec<([u8; KEY_SIZE], String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (i, line) in lines.enumerate() {
        let n = i + 2;
        if out.len() == MAX_BLOCKLIST_ENTRIES {
            return Err(format!("{shown}:{n}: more than {MAX_BLOCKLIST_ENTRIES} entries"));
        }
        let (hex, reason) = line.split_once(' ').unwrap_or((line, ""));
        let key = from_hex(hex).map_err(|e| format!("{shown}:{n}: {e}"))?;
        if reason.chars().count() > MAX_REASON_CHARS || reason.chars().any(char::is_control) {
            return Err(format!(
                "{shown}:{n}: a reason of at most {MAX_REASON_CHARS} characters, no control characters"
            ));
        }
        if !seen.insert(key) {
            return Err(format!("{shown}:{n}: {} twice", to_hex(&key)));
        }
        out.push((key, reason.to_string()));
    }
    Ok(out)
}

pub fn read(path: &Path, header: &str) -> Result<Vec<([u8; KEY_SIZE], String)>, String> {
    use std::io::Read;
    let shown = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|e| format!("{shown}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{shown}: not a file"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_BLOCKLIST_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("{shown}: {e}"))?;
    parse(&bytes, header, &shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_lines() {
        let k1 = "ab".repeat(32);
        let k2 = "CD".repeat(32);
        let good = parse(format!("{RESOURCES_HEADER}\n{k1} a reason\r\n{k2}\n").as_bytes(), RESOURCES_HEADER, "f")
            .expect("good");
        assert_eq!(good, vec![([0xAB; 32], "a reason".to_string()), ([0xCD; 32], String::new())]);
        for bad in [
            String::new(),
            format!("{k1}\n"),
            format!("{RESOURCES_HEADER}\n\n"),
            format!("{RESOURCES_HEADER}\n{k1}\n{k1} again\n"),
            format!("{RESOURCES_HEADER}\n{}\n", &k1[..63]),
            format!("{RESOURCES_HEADER}\n{k1}x\n"),
            format!("{RESOURCES_HEADER}\n{k1} a\u{7}b\n"),
            format!("{RESOURCES_HEADER}\n{k1} {}\n", "r".repeat(201)),
            format!("# comment\n{RESOURCES_HEADER}\n"),
            format!("{SERVERS_HEADER}\n"),
        ] {
            assert!(parse(bad.as_bytes(), RESOURCES_HEADER, "f").is_err(), "{bad:?}");
        }
        assert!(parse(&[0xFF, 0xFE], RESOURCES_HEADER, "f").unwrap_err().contains("UTF-8"));
        assert!(parse(&vec![b'a'; MAX_BLOCKLIST_BYTES + 1], RESOURCES_HEADER, "f").unwrap_err().contains("larger"));
    }
}
