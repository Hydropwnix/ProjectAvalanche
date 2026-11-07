use super::{LevelId, MAX_CLASS, MAX_ROW_ID, MAX_ROWS, Row, files_hash};
use crate::data::{SimContentId, SimRow};

pub const FILE_CLASS: &str = "file";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Difference {
    Files,
    Changed { id: String, ours: String },
    OnlyOurs { id: String, ours: String },
    OnlyTheirs { id: String },
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ssx_core::text::quoted;
        match self {
            Difference::Files => f.write_str("the disc files differ from the server's"),
            Difference::Changed { id, ours } => write!(f, "{} differs (yours from {})", quoted(id), quoted(ours)),
            Difference::OnlyOurs { id, ours } => {
                write!(f, "{} is changed only on your side (by {})", quoted(id), quoted(ours))
            }
            Difference::OnlyTheirs { id } => write!(f, "{} is changed only on the server's side", quoted(id)),
        }
    }
}

impl LevelId {
    pub fn from_rows(level: &str, id: SimContentId, rows: &[SimRow]) -> Result<LevelId, String> {
        let files: Vec<(&str, crate::hash::ContentHash)> =
            rows.iter().filter(|r| r.class == FILE_CLASS).map(|r| (r.id.as_str(), r.hash)).collect();
        let other: Vec<Row> = rows
            .iter()
            .filter(|r| r.class != FILE_CLASS)
            .map(|r| Row { id: r.id.clone(), class: r.class.clone(), hash: r.hash })
            .collect();
        if other.len() > MAX_ROWS {
            return Err(format!(
                "{level}: {} rows besides the disc files, a manifest states at most {MAX_ROWS}",
                other.len()
            ));
        }
        if let Some(r) = other.iter().find(|r| r.id.len() > MAX_ROW_ID || r.class.len() > MAX_CLASS) {
            return Err(format!("{level}: the row {} is longer than a manifest states", ssx_core::text::quoted(&r.id)));
        }
        Ok(LevelId { level: level.to_string(), id, files: files_hash(&files), rows: other })
    }

    pub fn differences(&self, rows: &[SimRow]) -> Vec<Difference> {
        let ours = LevelId::from_rows(&self.level, SimContentId([0; 32]), rows);
        let mut out = Vec::new();
        let files: Vec<(&str, crate::hash::ContentHash)> =
            rows.iter().filter(|r| r.class == FILE_CLASS).map(|r| (r.id.as_str(), r.hash)).collect();
        if files_hash(&files) != self.files {
            out.push(Difference::Files);
        }
        let source = |id: &str| rows.iter().find(|r| r.id == id).map_or_else(String::new, |r| r.layer.clone());
        let theirs: std::collections::BTreeMap<(&str, &str), _> =
            self.rows.iter().map(|r| ((r.id.as_str(), r.class.as_str()), r.hash)).collect();
        let mine: Vec<Row> = ours.map(|l| l.rows).unwrap_or_default();
        let mine_keys: std::collections::BTreeMap<(&str, &str), _> =
            mine.iter().map(|r| ((r.id.as_str(), r.class.as_str()), r.hash)).collect();
        for (&(id, class), hash) in &mine_keys {
            match theirs.get(&(id, class)) {
                Some(h) if h == hash => {}
                Some(_) => out.push(Difference::Changed { id: id.to_string(), ours: source(id) }),
                None => out.push(Difference::OnlyOurs { id: id.to_string(), ours: source(id) }),
            }
        }
        for &(id, class) in theirs.keys() {
            if !mine_keys.contains_key(&(id, class)) {
                out.push(Difference::OnlyTheirs { id: id.to_string() });
            }
        }
        out
    }
}
