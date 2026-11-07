pub(crate) mod character;
pub(crate) mod item;
#[cfg(test)]
mod tests;

use std::ops::Range;

pub use character::{CharacterDef, OutfitSlot, RiderRecord, parse_character, parse_character_table};
pub use item::{ItemDef, ItemKind, parse_item, parse_item_table};

use crate::id::{BASE_NAMESPACE, ContentId, Kind};
use crate::manifest::line_col;
use crate::material::Problem;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RiderEntry {
    pub data: Option<(String, toml::Table)>,
    pub model: Option<String>,
}

pub const SLOTS: [&str; 16] = [
    "head",
    "hair",
    "eyes",
    "headgear",
    "jackets",
    "pants",
    "gloves",
    "boots",
    "goggles",
    "headlamp",
    "protection",
    "thermal",
    "oxygentank",
    "iceaxe",
    "wingsuit",
    "snowboard",
];

pub const GEAR_TYPES: [(&str, i64, &[&str]); 10] = [
    ("board", 0, &["snowboard"]),
    ("wingsuit", 1, &["wingsuit"]),
    ("iceaxe", 2, &["iceaxe"]),
    ("armour", 3, &["protection"]),
    ("thermal", 4, &["thermal"]),
    ("oxygen", 5, &["oxygentank"]),
    ("headlamp", 6, &["headlamp"]),
    ("pulsegoggles", 7, &["goggles"]),
    ("jacket", 12, &["jackets", "pants"]),
    ("pants", 13, &["pants"]),
];

pub const GEAR_STATS: i32 = 37;

pub const MAX_STAT_PAIRS: usize = 16;

pub fn slot_of(ava_slot: Option<&str>, node_name: &str) -> Option<&'static str> {
    if let Some(s) = ava_slot {
        return SLOTS.iter().copied().find(|x| *x == s);
    }
    let name = match node_name.rsplit_once('.') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => node_name,
    };
    let name = name.to_ascii_lowercase();
    SLOTS.iter().copied().find(|x| *x == name)
}

pub fn gear_type(name: &str) -> Option<(&'static str, i64, &'static [&'static str])> {
    GEAR_TYPES.iter().copied().find(|(n, _, _)| *n == name)
}

pub fn gear_type_name(value: i64) -> Option<&'static str> {
    GEAR_TYPES.iter().find(|(_, v, _)| *v == value).map(|(n, _, _)| *n)
}

pub(crate) struct At<'a> {
    pub(crate) text: &'a str,
    pub(crate) problems: Vec<Problem>,
}

impl At<'_> {
    pub(crate) fn push(&mut self, span: Range<usize>, m: String) {
        let (line, column) = if self.text.is_empty() { (0, 0) } else { line_col(self.text, span.start) };
        self.problems.push((line, column, m));
    }

    pub(crate) fn id(&mut self, v: &toml::Spanned<String>, kind: Kind, base_only: bool) -> Option<ContentId> {
        match v.get_ref().parse::<ContentId>() {
            Ok(id) if id.kind() != kind => {
                self.push(v.span(), format!("{id} is not a {} id", kind.as_str()));
                None
            }
            Ok(id) if base_only && id.namespace() != BASE_NAMESPACE => {
                self.push(v.span(), format!("{id}: one of the disc's (`{BASE_NAMESPACE}:`) {}s", kind.as_str()));
                None
            }
            Ok(id) => Some(id),
            Err(e) => {
                self.push(v.span(), e.to_string());
                None
            }
        }
    }

    pub(crate) fn stat_pairs(&mut self, v: &toml::Spanned<Vec<[i64; 2]>>, what: &str) -> Vec<(i32, u8)> {
        let pairs = v.get_ref();
        if pairs.len() > MAX_STAT_PAIRS {
            self.push(v.span(), format!("{what}: {} pairs; at most {MAX_STAT_PAIRS}", pairs.len()));
            return Vec::new();
        }
        let mut out = Vec::new();
        for &[stat, value] in pairs {
            if !(-1..i64::from(GEAR_STATS)).contains(&stat) || !(0..=255).contains(&value) {
                self.push(
                    v.span(),
                    format!(
                        "{what}: [{stat}, {value}]: a GearStat (0 to {}, −1 empty) and a value 0 to 255",
                        GEAR_STATS - 1
                    ),
                );
                continue;
            }
            out.push((stat as i32, value as u8));
        }
        out
    }
}

pub const MAX_MATERIAL_BINDINGS: usize = 256;

pub(crate) fn materials(
    at: &mut At,
    raw: &std::collections::BTreeMap<String, toml::Spanned<String>>,
) -> std::collections::BTreeMap<String, ContentId> {
    let mut out = std::collections::BTreeMap::new();
    if raw.len() > MAX_MATERIAL_BINDINGS {
        at.push(0..0, format!("[materials]: {} bindings; at most {MAX_MATERIAL_BINDINGS}", raw.len()));
        return out;
    }
    for (name, v) in raw {
        if name.is_empty() || name.len() > 128 {
            at.push(v.span(), "[materials]: a glTF material name of 1 to 128 bytes".into());
            continue;
        }
        if let Some(id) = at.id(v, Kind::Material, false) {
            out.insert(name.clone(), id);
        }
    }
    out
}

pub(crate) fn table_text(t: &toml::Table) -> Result<String, Vec<Problem>> {
    toml::to_string(t).map_err(|e| vec![(0, 0, format!("the merged record cannot be written: {e}"))])
}
