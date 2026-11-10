use std::collections::HashSet;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use ssx_content::hash::{ContentHash, ContentIndex};

use super::setup::{ARCHIVES, MOVIE_DIR, big_root, marker, safe_relative};

pub const INDEX_FILE: &str = "content.tsv";
// TODO(guess): 32 MiB, 17 times the index of the one pressing measured (12 928 rows, 1 879 517 bytes; its longest
// path 91 bytes); settle: the indexes of the disc's other pressings (regions).
pub const MAX_INDEX_BYTES: u64 = 32 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Complete { marked: bool, files: usize },
    Missing,
    Unfinished,
    NoMovies,
    NoIndex,
    IndexDamaged(String),
    MarkerDamaged(String),
}

impl State {
    pub fn playable(&self) -> bool {
        matches!(self, State::Complete { .. } | State::NoIndex | State::IndexDamaged(_))
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, State::Complete { .. })
    }

    pub fn needs_repair(&self) -> bool {
        matches!(self, State::NoMovies | State::NoIndex | State::IndexDamaged(_) | State::MarkerDamaged(_))
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            State::Complete { marked: true, files } => write!(f, "complete ({files} files in its content index)"),
            State::Complete { marked: false, files } => {
                write!(f, "an archive tree without a setup marker ({files} files in its content index)")
            }
            State::Missing => f.write_str("not set up"),
            State::Unfinished => f.write_str("its setup has not finished"),
            State::NoMovies => f.write_str("set up by an earlier version: no movies and no content index"),
            State::NoIndex => f.write_str("set up by an earlier version: no content index"),
            State::IndexDamaged(why) => write!(f, "its content index is damaged ({why})"),
            State::MarkerDamaged(why) => write!(f, "its setup marker is damaged ({why})"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incomplete {
    pub data: PathBuf,
    pub state: State,
}

impl fmt::Display for Incomplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let todo = match self.state {
            State::Missing => "set it up in the launcher (Game Data, Extract game data)",
            State::Unfinished => "finish it in the launcher (Game Data, Extract game data)",
            _ => "repair it in the launcher (Game Data, Repair game data)",
        };
        write!(f, "the game data folder {} is {}: {todo}", self.data.display(), self.state)
    }
}

impl std::error::Error for Incomplete {}

#[derive(Clone, Debug)]
pub struct Folder {
    pub state: State,
    pub index: Option<ContentIndex>,
}

pub fn check(data: &Path) -> Folder {
    let only = |state| Folder { state, index: None };
    let marker = match marker(data) {
        Ok(m) => m,
        Err(why) => return only(State::MarkerDamaged(why)),
    };
    let Some(marker) = marker else {
        if !is_tree(data) {
            return only(State::Missing);
        }
        return match read_index(data) {
            Ok(None) => only(State::NoIndex),
            Ok(Some(text)) => verified(&text, None),
            Err(why) => only(State::IndexDamaged(why)),
        };
    };
    match marker["format"].as_u64() {
        Some(1 | 2) => {}
        Some(n) if n > super::setup::MARKER_FORMAT => {
            return only(State::MarkerDamaged(format!("format {n}, of a newer version")));
        }
        _ => return only(State::MarkerDamaged("no known format".into())),
    }
    if marker["complete"] != Value::Bool(true) {
        return only(State::Unfinished);
    }
    if marker["format"].as_u64() == Some(1) {
        return only(State::NoMovies);
    }
    if let Some(missing) = ARCHIVES.iter().find(|a| !marker["finished"][**a].is_object()) {
        return only(State::MarkerDamaged(format!("complete, without {missing} among the finished archives")));
    }
    if !marker["movies"].is_object() {
        return only(State::MarkerDamaged("complete, without the movies".into()));
    }
    let recorded = &marker["content_index"];
    if recorded.is_null() {
        return only(State::NoIndex);
    }
    let recorded = match Recorded::of(recorded) {
        Ok(r) => r,
        Err(why) => return only(State::MarkerDamaged(why)),
    };
    match read_index(data) {
        Ok(Some(text)) => verified(&text, Some(&recorded)),
        Ok(None) => only(State::IndexDamaged(format!("{INDEX_FILE} is missing"))),
        Err(why) => only(State::IndexDamaged(why)),
    }
}

pub fn state(data: &Path) -> State {
    check(data).state
}

pub fn content_index(data: &Path) -> Result<ContentIndex, Incomplete> {
    let folder = check(data);
    folder.index.ok_or_else(|| Incomplete { data: data.to_path_buf(), state: folder.state })
}

fn is_tree(data: &Path) -> bool {
    let big = big_root(data);
    big.join("cache1/data/xenon/attribdb").is_dir() && big.join("data0").is_dir()
}

fn verified(text: &str, recorded: Option<&Recorded>) -> Folder {
    match verify_index(text, recorded) {
        Ok(index) => {
            let state = State::Complete { marked: recorded.is_some(), files: index.entries.len() };
            Folder { state, index: Some(index) }
        }
        Err(why) => Folder { state: State::IndexDamaged(why), index: None },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub files: u64,
    pub bytes: u64,
    pub blake3: Option<ContentHash>,
}

impl Recorded {
    pub fn of(v: &Value) -> Result<Self, String> {
        let bad = || "its content_index is malformed".to_string();
        let files = v["files"].as_u64().ok_or_else(bad)?;
        let bytes = v["bytes"].as_u64().ok_or_else(bad)?;
        let blake3 = match &v["blake3"] {
            Value::Null => None,
            Value::String(s) => Some(s.parse().map_err(|_| bad())?),
            _ => return Err(bad()),
        };
        Ok(Recorded { files, bytes, blake3 })
    }
}

pub fn read_index(data: &Path) -> Result<Option<String>, String> {
    read_text(data, INDEX_FILE)
}

pub fn read_text(dir: &Path, name: &str) -> Result<Option<String>, String> {
    let path = dir.join(name);
    let file = match ssx_files::open_plain(&path) {
        Ok(Some(file)) => file,
        Ok(None) => return Err(format!("{name} is a link or a folder, not a file")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(format!("{name} is larger than {MAX_INDEX_BYTES} bytes"));
    }
    String::from_utf8(bytes).map(Some).map_err(|_| format!("{name} is not UTF-8 text"))
}

pub fn verify_index(text: &str, recorded: Option<&Recorded>) -> Result<ContentIndex, String> {
    let index = ContentIndex::from_tsv(text).map_err(|e| format!("{INDEX_FILE}: {e}"))?;
    if index.to_tsv() != text {
        let line = index.to_tsv().lines().zip(text.lines()).position(|(a, b)| a != b);
        let at = line.map_or_else(|| "at its end".to_string(), |l| format!("at line {}", l + 1));
        return Err(format!("{INDEX_FILE} is not in its one form (sorted rows, each ended by a line feed) {at}"));
    }
    let mut folded = HashSet::with_capacity(index.entries.len());
    let mut bytes = 0u64;
    for (path, (size, _)) in &index.entries {
        let shown = ssx_core::text::quoted(path);
        if !setup_path(path) {
            return Err(format!("{INDEX_FILE} lists {shown}, which is no file of the setup's"));
        }
        if !folded.insert(path.to_ascii_lowercase()) {
            return Err(format!("{INDEX_FILE} lists {shown} twice but for case"));
        }
        bytes = bytes.checked_add(*size).ok_or_else(|| format!("{INDEX_FILE}'s sizes overflow"))?;
    }
    if let Some(r) = recorded {
        if r.files != index.entries.len() as u64 || r.bytes != bytes {
            return Err(format!(
                "{INDEX_FILE} lists {} files of {bytes} bytes, the marker records {} of {}",
                index.entries.len(),
                r.files,
                r.bytes
            ));
        }
        if r.blake3.is_some_and(|h| h != ContentHash::of(text.as_bytes())) {
            return Err(format!("{INDEX_FILE} is not the one the setup wrote (its BLAKE3 differs from the marker's)"));
        }
    }
    Ok(index)
}

pub fn setup_path(path: &str) -> bool {
    if let Some(rest) = path.strip_prefix("big/") {
        let Some((archive, member)) = rest.split_once('/') else { return false };
        return ARCHIVES.contains(&archive) && safe_relative(member).is_some();
    }
    if let Some(rest) = path.strip_prefix("disc/") {
        let lower = rest.to_ascii_lowercase();
        let name = lower.strip_prefix(MOVIE_DIR).and_then(|r| r.strip_prefix('/'));
        return name.is_some_and(|n| !n.contains('/') && n.ends_with(".vp6")) && safe_relative(rest).is_some();
    }
    false
}

#[cfg(test)]
pub(crate) mod tests;
