use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::id::ContentId;
use crate::manifest::line_col;
use crate::material::Problem;

pub const FORMAT: i64 = 1;
pub const MAX_FILE_BYTES: u64 = 1 << 20;
pub const MAX_ENTRIES: usize = 16_384;
pub const MAX_FIELDS: usize = 64;
pub const MAX_NAME: usize = 64;
pub const MAX_TEXT: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateValue {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModState {
    pub entries: BTreeMap<ContentId, BTreeMap<String, StateValue>>,
}

pub fn state_path(save: &Path) -> PathBuf {
    let mut name = save.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".mods.toml");
    save.with_file_name(name)
}

fn name_ok(n: &str) -> bool {
    (1..=MAX_NAME).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn text_ok(t: &str) -> bool {
    t.len() <= MAX_TEXT && !t.chars().any(char::is_control)
}

impl ModState {
    pub fn get(&self, id: &ContentId, field: &str) -> Option<&StateValue> {
        self.entries.get(id)?.get(field)
    }

    pub fn bool(&self, id: &ContentId, field: &str) -> Option<bool> {
        match self.get(id, field)? {
            StateValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn int(&self, id: &ContentId, field: &str) -> Option<i64> {
        match self.get(id, field)? {
            StateValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn text(&self, id: &ContentId, field: &str) -> Option<&str> {
        match self.get(id, field)? {
            StateValue::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn set(&mut self, id: &ContentId, field: &str, value: StateValue) -> bool {
        if !name_ok(field) || matches!(&value, StateValue::Text(t) if !text_ok(t)) {
            return false;
        }
        if !self.entries.contains_key(id) && self.entries.len() >= MAX_ENTRIES {
            return false;
        }
        let e = self.entries.entry(id.clone()).or_default();
        if !e.contains_key(field) && e.len() >= MAX_FIELDS {
            return false;
        }
        e.insert(field.to_string(), value);
        true
    }

    pub fn parse(bytes: &[u8]) -> Result<ModState, Vec<Problem>> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(vec![(0, 0, format!("{} bytes; at most {MAX_FILE_BYTES}", bytes.len()))]);
        }
        let table = crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
        let text = std::str::from_utf8(bytes).unwrap_or_default();
        let at = |key: &str| {
            text.find(&format!("\"{key}\"")).or_else(|| text.find(key)).map_or((0, 0), |i| line_col(text, i))
        };
        let mut problems = Vec::new();
        let mut out = ModState::default();
        match table.get("format") {
            Some(toml::Value::Integer(FORMAT)) => {}
            Some(v) => problems.push((at("format"), format!("format {v}: this game reads format {FORMAT}"))),
            None => problems.push(((0, 0), format!("no `format = {FORMAT}`"))),
        }
        if table.len() > MAX_ENTRIES + 1 {
            return Err(vec![(0, 0, format!("{} entries; at most {MAX_ENTRIES}", table.len() - 1))]);
        }
        for (key, value) in table.iter().filter(|(k, _)| k.as_str() != "format") {
            let id = match key.parse::<ContentId>() {
                Ok(id) => id,
                Err(e) => {
                    problems.push((at(key), format!("{}: {e}", ssx_core::text::quoted(key))));
                    continue;
                }
            };
            let toml::Value::Table(fields) = value else {
                problems.push((at(key), format!("{id}: a table of fields")));
                continue;
            };
            if fields.len() > MAX_FIELDS {
                problems.push((at(key), format!("{id}: {} fields; at most {MAX_FIELDS}", fields.len())));
                continue;
            }
            let mut entry = BTreeMap::new();
            for (name, v) in fields {
                if !name_ok(name) {
                    problems.push((at(key), format!("{id}: a field name of 1 to {MAX_NAME} characters [a-z0-9_]")));
                    continue;
                }
                let v = match v {
                    toml::Value::Boolean(b) => StateValue::Bool(*b),
                    toml::Value::Integer(i) => StateValue::Int(*i),
                    toml::Value::String(t) if text_ok(t) => StateValue::Text(t.clone()),
                    toml::Value::String(_) => {
                        problems.push((at(key), format!("{id}.{name}: one line of at most {MAX_TEXT} bytes")));
                        continue;
                    }
                    _ => {
                        problems.push((at(key), format!("{id}.{name}: a boolean, an integer or a text")));
                        continue;
                    }
                };
                entry.insert(name.clone(), v);
            }
            out.entries.insert(id, entry);
        }
        if problems.is_empty() { Ok(out) } else { Err(problems.into_iter().map(|((l, c), m)| (l, c, m)).collect()) }
    }

    pub fn write(&self) -> String {
        let mut t = toml::Table::new();
        t.insert("format".into(), toml::Value::Integer(FORMAT));
        for (id, fields) in &self.entries {
            let f: toml::Table = fields
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        StateValue::Bool(b) => toml::Value::Boolean(*b),
                        StateValue::Int(i) => toml::Value::Integer(*i),
                        StateValue::Text(s) => toml::Value::String(s.clone()),
                    };
                    (k.clone(), v)
                })
                .collect();
            t.insert(id.to_string(), toml::Value::Table(f));
        }
        let body = toml::to_string(&t).unwrap_or_default();
        format!("# Project Avalanche: the state of mod content in this profile (one entry per content id)\n{body}")
    }

    pub fn read_file(path: &Path) -> Result<Option<ModState>, String> {
        let meta = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if !meta.is_file() {
            return Err(format!("{}: not a regular file", path.display()));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(format!("{}: {} bytes; at most {MAX_FILE_BYTES}", path.display(), meta.len()));
        }
        let mut bytes = Vec::new();
        let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        std::io::Read::read_to_end(&mut std::io::Read::take(f, MAX_FILE_BYTES + 1), &mut bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        ModState::parse(&bytes).map(Some).map_err(|ps| {
            let lines: Vec<String> =
                ps.into_iter().map(|(l, c, m)| format!("{}:{l}:{c}: {m}", path.display())).collect();
            lines.join("; ")
        })
    }

    pub fn write_file(&self, path: &Path) -> Result<(), String> {
        ssx_files::replace(path, self.write().as_bytes(), &ssx_files::Options::new())
            .map_err(|e| format!("{}: {}", path.display(), e.io()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ContentId {
        s.parse().unwrap()
    }

    #[test]
    fn the_state_round_trips() {
        let mut s = ModState::default();
        assert!(s.set(&id("sam-pack:character/sam"), "owned", StateValue::Bool(true)));
        assert!(s.set(&id("sam-pack:character/sam"), "money_earned", StateValue::Int(12_500)));
        assert!(s.set(&id("ssx:character/zoe"), "last_board", StateValue::Text("neon:board/neon".into())));
        assert!(s.set(&id("tunes:music/one"), "enabled", StateValue::Bool(false)));
        let text = s.write();
        let back = ModState::parse(text.as_bytes()).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.write(), text);
        assert_eq!(back.int(&id("sam-pack:character/sam"), "money_earned"), Some(12_500));
    }

    #[test]
    fn hostile_state_is_refused() {
        let cases: [&[u8]; 7] = [
            b"[\"x:character/y\"]\nowned = true\n",
            b"format = 2\n",
            b"format = 1\n[\"not an id\"]\nowned = true\n",
            b"format = 1\n[\"x:character/y\"]\nOwned = true\n",
            b"format = 1\n[\"x:character/y\"]\nlist = [1, 2]\n",
            b"format = 1\n[\"x:character/y\"]\nname = \"a\\nb\"\n",
            b"format = 1\n\"x:character/y\" = 3\n",
        ];
        for c in cases {
            assert!(ModState::parse(c).is_err(), "{}", String::from_utf8_lossy(c));
        }
        let big = vec![b' '; (MAX_FILE_BYTES + 1) as usize];
        assert!(ModState::parse(&big).unwrap_err()[0].2.contains("bytes"));
        let mut s = ModState::default();
        assert!(!s.set(&id("x:character/y"), "Bad", StateValue::Bool(true)));
        assert!(!s.set(&id("x:character/y"), "t", StateValue::Text("x".repeat(MAX_TEXT + 1))));
        for i in 0..MAX_FIELDS {
            assert!(s.set(&id("x:character/y"), &format!("f{i}"), StateValue::Int(1)));
        }
        assert!(!s.set(&id("x:character/y"), "one_more", StateValue::Int(1)));
    }

    #[test]
    fn the_file_sits_beside_the_save() {
        assert_eq!(state_path(Path::new("/a/Saves/ssxvprofile")), Path::new("/a/Saves/ssxvprofile.mods.toml"));
        assert_eq!(state_path(Path::new("p.bin")), Path::new("p.bin.mods.toml"));
    }
}
