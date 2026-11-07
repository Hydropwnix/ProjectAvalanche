use std::collections::BTreeSet;
use std::fmt;
use std::sync::OnceLock;

use ssx_assets::subset::Member;

const TSV: &str = include_str!("../../../../assets/server/server-files.tsv");
const HEADER: &str = "path\tclass\tsize\tblake3";

pub struct Pressing {
    pub fingerprint: &'static str,
    pub iso_size: u64,
}

pub const KNOWN_PRESSING: Pressing = Pressing { fingerprint: "660dcce05812cf68", iso_size: 7_838_695_424 };

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Rules,
    Collision,
    Ghosts,
    Verify,
    NotServer,
}

impl Class {
    pub const OPTIONAL: [Class; 3] = [Class::Collision, Class::Ghosts, Class::Verify];

    pub fn name(self) -> &'static str {
        match self {
            Class::Rules => "rules",
            Class::Collision => "optional-collision",
            Class::Ghosts => "optional-ghosts",
            Class::Verify => "optional-verify",
            Class::NotServer => "not-server",
        }
    }

    pub fn option(self) -> &'static str {
        match self {
            Class::Rules => "rules",
            Class::Collision => "collision",
            Class::Ghosts => "ghosts",
            Class::Verify => "verify",
            Class::NotServer => "not-server",
        }
    }

    fn parse(s: &str) -> Option<Class> {
        [Class::Rules, Class::Collision, Class::Ghosts, Class::Verify, Class::NotServer]
            .into_iter()
            .find(|c| c.name() == s)
    }

    pub fn from_option(s: &str) -> Option<Class> {
        Class::OPTIONAL.into_iter().find(|c| c.option() == s)
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub member: Member,
    pub class: Class,
    pub size: u64,
    pub blake3: [u8; 32],
}

#[derive(Debug)]
pub struct Manifest {
    pub entries: Vec<Entry>,
}

impl Manifest {
    pub fn of(&self, classes: &BTreeSet<Class>) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.class == Class::Rules || classes.contains(&e.class)).collect()
    }

    pub fn get(&self, data_path: &str) -> Option<&Entry> {
        self.entries.binary_search_by(|e| e.member.data_path().as_str().cmp(data_path)).ok().map(|i| &self.entries[i])
    }
}

pub fn parse_hash(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse(text: &str) -> Result<Manifest, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, h)) if h == HEADER => {}
        _ => return Err(format!("line 1: expected the header {HEADER:?}")),
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (i, line) in lines {
        let at = |m: String| format!("line {}: {m}", i + 1);
        let cols: Vec<&str> = line.split('\t').collect();
        let [path, class, size, hash] = cols[..] else { return Err(at("expected 4 columns".into())) };
        let member = Member::from_data_path(path).map_err(|e| at(e.to_string()))?;
        let class = Class::parse(class).ok_or_else(|| at(format!("unknown class {class:?}")))?;
        let size = size.parse::<u64>().map_err(|_| at(format!("size {size:?}")))?;
        let blake3 = parse_hash(hash).ok_or_else(|| at(format!("hash {hash:?}")))?;
        if entries.last().is_some_and(|l| l.member.data_path().as_str() >= path) {
            return Err(at("not sorted by path, or twice".into()));
        }
        entries.push(Entry { member, class, size, blake3 });
    }
    Ok(Manifest { entries })
}

pub fn manifest() -> Result<&'static Manifest, String> {
    static LIST: OnceLock<Result<Manifest, String>> = OnceLock::new();
    LIST.get_or_init(|| parse(TSV)).as_ref().map_err(|e| format!("the server's member list: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_list_reads_and_has_s04s_counts() {
        let m = manifest().expect("reads");
        let count = |c: Class| m.entries.iter().filter(|e| e.class == c).count();
        assert_eq!(
            [Class::Rules, Class::Collision, Class::Ghosts, Class::Verify, Class::NotServer].map(count),
            [342, 715, 727, 130, 11]
        );
        let rules: u64 = m.entries.iter().filter(|e| e.class == Class::Rules).map(|e| e.size).sum();
        assert_eq!(rules, 21_181_483, "the rules subset's size (multiplayer.md §8.3: 21.2 MB)");
        assert!(m.get("big/bootcache/data/xenon/attribdb/attribdb.vlt").is_some_and(|e| e.class == Class::Rules));
    }

    #[test]
    fn the_list_is_the_manifests() {
        let dev = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Game-Data/Manifests/server-files.tsv");
        let Ok(text) = std::fs::read_to_string(dev) else { return };
        let ours: Vec<(String, &str)> =
            manifest().expect("reads").entries.iter().map(|e| (e.member.data_path(), e.class.name())).collect();
        let theirs: Vec<(String, &str)> = text
            .lines()
            .skip(1)
            .map(|l| {
                let mut c = l.split('\t');
                (c.next().unwrap_or_default().to_string(), c.next().unwrap_or_default())
            })
            .collect();
        assert_eq!(ours, theirs);
    }

    #[test]
    fn a_broken_list_is_refused_with_its_line() {
        let row = "big/bootcache/a.bin\trules\t1\t".to_string() + &"0".repeat(64);
        assert!(parse(&format!("{HEADER}\n{row}\n")).is_ok());
        assert!(parse("path\tclass\n").unwrap_err().starts_with("line 1"));
        assert!(parse(&format!("{HEADER}\n{row}\n{row}\n")).unwrap_err().starts_with("line 3: not sorted"));
        assert!(parse(&format!("{HEADER}\n{}\n", row.replace("rules", "core"))).unwrap_err().contains("class"));
        assert!(parse(&format!("{HEADER}\n{}\n", row.replace("big/", "../"))).is_err());
        assert!(parse(&format!("{HEADER}\n{}\n", row.replace("\t1\t", "\t-1\t"))).unwrap_err().contains("size"));
        assert!(parse(&format!("{HEADER}\n{}A\n", &row[..row.len() - 1])).unwrap_err().contains("hash"));
    }
}
