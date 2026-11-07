use std::collections::BTreeMap;

use serde::Deserialize;

use super::{At, SLOTS, table_text};
use crate::id::{ContentId, Kind};
use crate::material::Problem;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutfitSlot {
    Gear(ContentId),
    Empty,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiderRecord {
    pub title: ContentId,
    pub display_name: String,
    pub description: Option<ContentId>,
    pub cost: i64,
    pub locked: bool,
    pub stat_bonuses: Vec<(i32, u8)>,
    pub portrait: Option<ContentId>,
    pub round_icon: Option<ContentId>,
    pub icon: Option<ContentId>,
    pub default: bool,
    pub unk_940b9b22: i64,
}

pub const CLEARED_SLOTS: [&str; 3] = ["iceaxe", "oxygentank", "wingsuit"];

#[derive(Clone, Debug, PartialEq, Default)]
pub struct CharacterDef {
    pub stands_in_for: Option<ContentId>,
    pub face: Option<ContentId>,
    pub voice: Option<ContentId>,
    pub voice_lines: BTreeMap<ContentId, ContentId>,
    pub default_board: Option<ContentId>,
    pub outfit: BTreeMap<String, OutfitSlot>,
    pub career: Option<RiderRecord>,
    pub materials: BTreeMap<String, ContentId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Raw {
    stands_in_for: Option<toml::Spanned<String>>,
    face: Option<toml::Spanned<String>>,
    voice: Option<toml::Spanned<String>>,
    #[serde(default)]
    voice_lines: BTreeMap<toml::Spanned<String>, toml::Spanned<String>>,
    default_board: Option<toml::Spanned<String>>,
    #[serde(default)]
    outfit: BTreeMap<toml::Spanned<String>, toml::Spanned<String>>,
    career: Option<toml::Spanned<RawCareer>>,
    #[serde(default)]
    materials: BTreeMap<String, toml::Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCareer {
    title: toml::Spanned<String>,
    display_name: toml::Spanned<String>,
    description: Option<toml::Spanned<String>>,
    cost: toml::Spanned<i64>,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    stat_bonuses: Option<toml::Spanned<Vec<[i64; 2]>>>,
    portrait: Option<toml::Spanned<String>>,
    round_icon: Option<toml::Spanned<String>>,
    icon: Option<toml::Spanned<String>>,
    #[serde(default)]
    default: bool,
    #[serde(rename = "0x940b9b22")]
    unk_940b9b22: Option<toml::Spanned<i64>>,
}

pub const MAX_VOICE_LINES: usize = 1024;
pub const MAX_COST: i64 = 100_000_000;

pub fn display_name_ok(n: &str) -> bool {
    (1..=32).contains(&n.len())
        && n.starts_with(|c: char| c.is_ascii_alphabetic())
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || b" ._-".contains(&b))
}

pub fn parse_character(bytes: &[u8], name: &str) -> Result<CharacterDef, Vec<Problem>> {
    let _ = name;
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    parse_text(text, text)
}

pub fn parse_character_table(t: &toml::Table) -> Result<CharacterDef, Vec<Problem>> {
    parse_text(&table_text(t)?, "")
}

fn parse_text(text: &str, lines: &str) -> Result<CharacterDef, Vec<Problem>> {
    let raw: Raw = toml::from_str(text).map_err(|e| {
        let (line, column) = match e.span() {
            Some(s) if !lines.is_empty() => crate::manifest::line_col(lines, s.start),
            _ => (0, 0),
        };
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut at = At { text: lines, problems: Vec::new() };
    let base =
        |at: &mut At, v: &Option<toml::Spanned<String>>| v.as_ref().and_then(|v| at.id(v, Kind::Character, true));
    let mut def = CharacterDef {
        stands_in_for: base(&mut at, &raw.stands_in_for),
        face: base(&mut at, &raw.face),
        voice: base(&mut at, &raw.voice),
        default_board: raw.default_board.as_ref().and_then(|v| at.id(v, Kind::Board, false)),
        ..CharacterDef::default()
    };
    if raw.voice_lines.len() > MAX_VOICE_LINES {
        at.push(0..0, format!("{} voice lines; at most {MAX_VOICE_LINES}", raw.voice_lines.len()));
    }
    for (k, v) in raw.voice_lines.iter().take(MAX_VOICE_LINES) {
        let (Some(event), Some(sound)) = (at.id(k, Kind::Sound, true), at.id(v, Kind::Sound, false)) else { continue };
        if !event.path().starts_with("speech/") {
            at.push(k.span(), format!("{event}: a speech event (`ssx:sound/speech/<lang>/…`)"));
            continue;
        }
        def.voice_lines.insert(event, sound);
    }
    for (k, v) in &raw.outfit {
        let slot = k.get_ref().as_str();
        if !SLOTS.contains(&slot) || slot == "snowboard" {
            at.push(
                k.span(),
                format!("{} is not a boarder slot ({})", ssx_core::text::quoted(slot), SLOTS[..15].join(", ")),
            );
            continue;
        }
        if CLEARED_SLOTS.contains(&slot) {
            at.push(
                k.span(),
                format!(
                    "{slot}: the game empties this slot without an equipped item (0x8293F298's keep flags: the board, \
                     ice axe, oxygen tank and wingsuit show only as items); a package's item is equipped instead"
                ),
            );
            continue;
        }
        let what = match v.get_ref().as_str() {
            "none" => Some(OutfitSlot::Empty),
            _ => match at.id(v, Kind::Gear, false) {
                Some(id) if id.namespace() == crate::BASE_NAMESPACE => {
                    at.push(v.span(), format!("{id}: a disc item's look is the stand-in's, worn when it is equipped; [outfit] names a package's gear or none"));
                    None
                }
                other => other.map(OutfitSlot::Gear),
            },
        };
        if let Some(w) = what {
            def.outfit.insert(slot.to_string(), w);
        }
    }
    if let Some(c) = &raw.career {
        def.career = career(&mut at, c.get_ref());
    }
    def.materials = super::materials(&mut at, &raw.materials);
    if at.problems.is_empty() { Ok(def) } else { Err(at.problems) }
}

fn career(at: &mut At, c: &RawCareer) -> Option<RiderRecord> {
    let title = at.id(&c.title, Kind::String, false);
    let description = c.description.as_ref().and_then(|v| at.id(v, Kind::String, false));
    let texture =
        |at: &mut At, v: &Option<toml::Spanned<String>>| v.as_ref().and_then(|v| at.id(v, Kind::Texture, false));
    let (portrait, round_icon, icon) = (texture(at, &c.portrait), texture(at, &c.round_icon), texture(at, &c.icon));
    if !display_name_ok(c.display_name.get_ref()) {
        at.push(c.display_name.span(), "display_name: 1 to 32 characters [A-Za-z0-9 ._-], a letter first".into());
    }
    if !(0..=MAX_COST).contains(c.cost.get_ref()) {
        at.push(c.cost.span(), format!("cost: 0 to {MAX_COST} credits"));
    }
    let stat_bonuses = c.stat_bonuses.as_ref().map(|s| at.stat_pairs(s, "stat_bonuses")).unwrap_or_default();
    let unk_940b9b22 = c.unk_940b9b22.as_ref().map_or(0, |v| *v.get_ref());
    if !(i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&unk_940b9b22) {
        at.push(c.unk_940b9b22.as_ref().map_or(0..0, |v| v.span()), "0x940b9b22: a 32-bit integer".into());
    }
    Some(RiderRecord {
        title: title?,
        display_name: c.display_name.get_ref().clone(),
        description,
        cost: *c.cost.get_ref(),
        locked: c.locked,
        stat_bonuses,
        portrait,
        round_icon,
        icon,
        default: c.default,
        unk_940b9b22,
    })
}

impl CharacterDef {
    pub fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        let mut put = |k: &str, v: &Option<ContentId>| {
            if let Some(v) = v {
                t.insert(k.into(), toml::Value::String(v.to_string()));
            }
        };
        put("stands_in_for", &self.stands_in_for);
        put("face", &self.face);
        put("voice", &self.voice);
        put("default_board", &self.default_board);
        if !self.voice_lines.is_empty() {
            let lines = self.voice_lines.iter().map(|(k, v)| (k.to_string(), toml::Value::String(v.to_string())));
            t.insert("voice_lines".into(), toml::Value::Table(lines.collect()));
        }
        if !self.outfit.is_empty() {
            let slots = self.outfit.iter().map(|(k, v)| {
                let v = match v {
                    OutfitSlot::Gear(id) => id.to_string(),
                    OutfitSlot::Empty => "none".into(),
                };
                (k.clone(), toml::Value::String(v))
            });
            t.insert("outfit".into(), toml::Value::Table(slots.collect()));
        }
        if let Some(c) = &self.career {
            t.insert("career".into(), toml::Value::Table(c.to_table()));
        }
        if !self.materials.is_empty() {
            let m = self.materials.iter().map(|(k, v)| (k.clone(), toml::Value::String(v.to_string())));
            t.insert("materials".into(), toml::Value::Table(m.collect()));
        }
        t
    }
}

impl RiderRecord {
    pub fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        t.insert("title".into(), toml::Value::String(self.title.to_string()));
        t.insert("display_name".into(), toml::Value::String(self.display_name.clone()));
        if let Some(d) = &self.description {
            t.insert("description".into(), toml::Value::String(d.to_string()));
        }
        t.insert("cost".into(), toml::Value::Integer(self.cost));
        t.insert("locked".into(), toml::Value::Boolean(self.locked));
        let pairs = self
            .stat_bonuses
            .iter()
            .map(|&(s, v)| toml::Value::Array(vec![toml::Value::Integer(s.into()), toml::Value::Integer(v.into())]));
        t.insert("stat_bonuses".into(), toml::Value::Array(pairs.collect()));
        for (k, v) in [("portrait", &self.portrait), ("round_icon", &self.round_icon), ("icon", &self.icon)] {
            if let Some(v) = v {
                t.insert(k.into(), toml::Value::String(v.to_string()));
            }
        }
        t.insert("default".into(), toml::Value::Boolean(self.default));
        t.insert("0x940b9b22".into(), toml::Value::Integer(self.unk_940b9b22));
        t
    }
}
