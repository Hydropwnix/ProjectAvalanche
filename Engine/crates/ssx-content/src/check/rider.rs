use std::collections::BTreeSet;

use super::{Checked, Severity};
use crate::id::{ContentId, Kind};
use crate::manifest::{Entry, MANIFEST_FILE};
use crate::model::{self, Model};
use crate::path::PackageTree;
use crate::rider::{CharacterDef, ItemDef};

#[derive(Clone, Debug)]
pub struct RiderEntryFile {
    pub id: ContentId,
    pub added: bool,
    pub model: Option<(String, std::sync::Arc<Model>)>,
    pub character: Option<(String, CharacterDef)>,
    pub item: Option<(String, ItemDef)>,
}

#[derive(Default)]
pub struct RiderFiles {
    pub entries: Vec<RiderEntryFile>,
}

const CHARACTER_KEYS: [&str; 8] =
    ["stands_in_for", "face", "voice", "voice_lines", "default_board", "outfit", "career", "materials"];
const CAREER_KEYS: [&str; 11] = [
    "title",
    "display_name",
    "description",
    "cost",
    "locked",
    "stat_bonuses",
    "portrait",
    "round_icon",
    "icon",
    "default",
    "0x940b9b22",
];
const ITEM_KEYS: [&str; 12] = [
    "slot",
    "title",
    "rider",
    "locked",
    "default",
    "level",
    "quality",
    "cost",
    "attributes",
    "icon",
    "0xb6ef00c6",
    "materials",
];

fn read(c: &mut Checked, tree: &PackageTree, f: &str, max: u64) -> Option<Vec<u8>> {
    match tree.read(f, max) {
        Ok(b) => Some(b),
        Err(e) => {
            c.push(Severity::Error, f, (0, 0), e.to_string());
            None
        }
    }
}

pub(super) fn check_entry(
    c: &mut Checked,
    tree: &PackageTree,
    id: &ContentId,
    entry: &Entry,
    at: (usize, usize),
    added: bool,
) {
    let kind = id.kind();
    let model_ext = |f: &str| model::FORMATS.iter().any(|e| f.to_ascii_lowercase().ends_with(&format!(".{e}")));
    let toml = |f: &str| f.to_ascii_lowercase().ends_with(".toml");
    let (model_file, data_file) = match entry {
        Entry::File(f) if toml(f) => (None, Some(f.clone())),
        Entry::File(f) if model_ext(f) => (Some(f.clone()), None),
        Entry::File(f) => {
            let message = format!(
                "{id}: {}: a {} is a .glb / .gltf, a .toml, or both by role",
                ssx_core::text::quoted(f),
                kind.as_str()
            );
            return c.push(Severity::Error, MANIFEST_FILE, at, message);
        }
        Entry::Files(files) => {
            if let Some(r) = files.keys().find(|r| !crate::package::RIDER_ROLES.contains(&r.as_str())) {
                let message = format!(
                    "{id}: the role {}; a {}'s roles are model and data",
                    ssx_core::text::quoted(r),
                    kind.as_str()
                );
                return c.push(Severity::Error, MANIFEST_FILE, at, message);
            }
            let (m, d) = (files.get("model").cloned(), files.get("data").cloned());
            if m.as_deref().is_some_and(|f| !model_ext(f)) || d.as_deref().is_some_and(|f| !toml(f)) {
                return c.push(
                    Severity::Error,
                    MANIFEST_FILE,
                    at,
                    format!("{id}: model is a .glb / .gltf, data a .toml"),
                );
            }
            (m, d)
        }
    };
    if added && (model_file.is_none() || data_file.is_none()) {
        let message =
            format!("{id}: an added {} needs its model and its record: {{ model = …, data = … }}", kind.as_str());
        c.push(Severity::Error, MANIFEST_FILE, at, message);
    }
    let mut out = RiderEntryFile { id: id.clone(), added, model: None, character: None, item: None };
    if let Some(f) = &model_file {
        let limits = model::Limits::skinned();
        if let Some(bytes) = read(c, tree, f, limits.max_file_bytes) {
            match model::decode_file(f, &bytes, tree, &limits) {
                Ok(m) => {
                    for n in &m.notes {
                        c.push(Severity::Note, f, (0, 0), n.clone());
                    }
                    let parts: usize = m.lods.iter().map(|l| l.parts.len()).sum();
                    let joints: Vec<usize> = m.skins.iter().map(|s| s.joints.len()).collect();
                    let message = format!(
                        "{} LODs, {parts} parts, {} vertices, {} triangles, {} skins of {joints:?} joints",
                        m.lods.len(),
                        m.vertex_count(),
                        m.triangle_count(),
                        m.skins.len()
                    );
                    c.push(Severity::Note, f, (0, 0), message);
                    out.model = Some((f.clone(), std::sync::Arc::new(m)));
                }
                Err(e) => c.push(Severity::Error, f, (0, 0), e.to_string()),
            }
        }
    }
    if let Some(f) = &data_file
        && let Some(bytes) = read(c, tree, f, crate::limits::MAX_RECORD_BYTES)
    {
        let problems = match kind {
            Kind::Character => match crate::rider::parse_character(&bytes, f) {
                Ok(d) => {
                    out.character = Some((f.clone(), d));
                    Vec::new()
                }
                Err(p) => p,
            },
            _ => match crate::rider::parse_item(&bytes, kind == Kind::Board) {
                Ok(d) => {
                    out.item = Some((f.clone(), d));
                    Vec::new()
                }
                Err(p) => p,
            },
        };
        for (l, col, m) in problems {
            c.push(Severity::Error, f, (l, col), m);
        }
    }
    c.riders.entries.push(out);
}

pub(super) fn check_patch(c: &mut Checked, tree: &PackageTree, id: &ContentId, f: &str) {
    let Some(bytes) = read(c, tree, f, crate::limits::MAX_RECORD_BYTES) else { return };
    let table = match crate::package::parse_record_at(&bytes) {
        Ok(t) => t,
        Err((l, col, m)) => return c.push(Severity::Error, f, (l, col), m),
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut unknown = |key: &str, known: &[&str], within: &str| {
        if !known.contains(&key) {
            let at = super::locate_key(&text, key);
            c.push(
                Severity::Error,
                f,
                at,
                format!("{} is not a key of {within} ({})", ssx_core::text::quoted(key), known.join(", ")),
            );
        }
    };
    let (top, name): (&[&str], &str) = match id.kind() {
        Kind::Character => (&CHARACTER_KEYS, "character.toml"),
        Kind::Board => (&ITEM_KEYS[1..], "board.toml"),
        _ => (&ITEM_KEYS, "gear.toml"),
    };
    let mut career = BTreeSet::new();
    for (k, v) in &table {
        unknown(k, top, name);
        if k == "career"
            && let toml::Value::Table(t) = v
        {
            career.extend(t.keys().cloned());
        }
    }
    for k in career {
        unknown(&k, &CAREER_KEYS, "[career]");
    }
}
