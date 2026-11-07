use serde::Deserialize;

use super::{At, gear_type, table_text};
use crate::id::{ContentId, Kind};
use crate::material::Problem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Suit,
    Board,
    Gear,
}

impl ItemKind {
    pub fn value(self) -> i64 {
        match self {
            ItemKind::Suit => 0,
            ItemKind::Board => 1,
            ItemKind::Gear => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemDef {
    pub kind: ItemKind,
    pub slot: &'static str,
    pub title: ContentId,
    pub rider: Option<ContentId>,
    pub locked: bool,
    pub default: bool,
    pub level: i64,
    pub quality: i64,
    pub cost: i64,
    pub attributes: Vec<(i32, u8)>,
    pub icon: Option<ContentId>,
    pub unk_b6ef00c6: i64,
    pub materials: std::collections::BTreeMap<String, ContentId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Raw {
    slot: Option<toml::Spanned<String>>,
    title: toml::Spanned<String>,
    rider: Option<toml::Spanned<String>>,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    level: Option<toml::Spanned<i64>>,
    #[serde(default)]
    quality: Option<toml::Spanned<i64>>,
    cost: toml::Spanned<i64>,
    attributes: Option<toml::Spanned<Vec<[i64; 2]>>>,
    icon: Option<toml::Spanned<String>>,
    #[serde(rename = "0xb6ef00c6", default)]
    unk_b6ef00c6: i64,
    #[serde(default)]
    materials: std::collections::BTreeMap<String, toml::Spanned<String>>,
}

pub const MAX_LEVEL: i64 = 9;
pub const MAX_QUALITY: i64 = 4;

pub fn parse_item(bytes: &[u8], board: bool) -> Result<ItemDef, Vec<Problem>> {
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    parse_text(text, text, board)
}

pub fn parse_item_table(t: &toml::Table, board: bool) -> Result<ItemDef, Vec<Problem>> {
    parse_text(&table_text(t)?, "", board)
}

fn parse_text(text: &str, lines: &str, board: bool) -> Result<ItemDef, Vec<Problem>> {
    let raw: Raw = toml::from_str(text).map_err(|e| {
        let (line, column) = match e.span() {
            Some(s) if !lines.is_empty() => crate::manifest::line_col(lines, s.start),
            _ => (0, 0),
        };
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut at = At { text: lines, problems: Vec::new() };
    let (kind, slot) = match (&raw.slot, board) {
        (None, true) => (ItemKind::Board, "board"),
        (Some(s), true) => {
            at.push(s.span(), "a board.toml has no slot (a board's is the board)".into());
            (ItemKind::Board, "board")
        }
        (None, false) => {
            at.push(0..0, "slot: the GearItemType (wingsuit, iceaxe, armour, thermal, oxygen, headlamp, …)".into());
            (ItemKind::Gear, "wingsuit")
        }
        (Some(s), false) => match gear_type(s.get_ref()).filter(|(n, _, _)| *n != "board") {
            Some(("jacket", _, _)) => (ItemKind::Suit, "jacket"),
            Some((n, _, _)) => (ItemKind::Gear, n),
            None => {
                at.push(
                    s.span(),
                    format!("slot {}: one of wingsuit, iceaxe, armour, thermal, oxygen, headlamp, pulsegoggles, jacket, pants (a board is a board.toml)", ssx_core::text::quoted(s.get_ref())),
                );
                (ItemKind::Gear, "wingsuit")
            }
        },
    };
    let title = at.id(&raw.title, Kind::String, false);
    let rider = raw.rider.as_ref().and_then(|r| at.id(r, Kind::Character, false));
    let icon = raw.icon.as_ref().and_then(|i| at.id(i, Kind::Texture, false));
    let range = |at: &mut At, v: &Option<toml::Spanned<i64>>, max: i64, what: &str| match v {
        Some(v) if !(0..=max).contains(v.get_ref()) => {
            at.push(v.span(), format!("{what}: 0 to {max}"));
            0
        }
        Some(v) => *v.get_ref(),
        None => 0,
    };
    let level = range(&mut at, &raw.level, MAX_LEVEL, "level");
    let quality = range(&mut at, &raw.quality, MAX_QUALITY, "quality");
    if !(0..=super::character::MAX_COST).contains(raw.cost.get_ref()) {
        at.push(raw.cost.span(), format!("cost: 0 to {} credits", super::character::MAX_COST));
    }
    let attributes = raw.attributes.as_ref().map(|a| at.stat_pairs(a, "attributes")).unwrap_or_default();
    let materials = super::materials(&mut at, &raw.materials);
    if !at.problems.is_empty() {
        return Err(at.problems);
    }
    Ok(ItemDef {
        kind,
        slot,
        title: title.ok_or_else(|| vec![(0, 0, "title".to_string())])?,
        rider,
        locked: raw.locked,
        default: raw.default,
        level,
        quality,
        cost: *raw.cost.get_ref(),
        attributes,
        icon,
        unk_b6ef00c6: raw.unk_b6ef00c6,
        materials,
    })
}

impl ItemDef {
    pub fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        if self.kind != ItemKind::Board {
            t.insert("slot".into(), toml::Value::String(self.slot.into()));
        }
        t.insert("title".into(), toml::Value::String(self.title.to_string()));
        if let Some(r) = &self.rider {
            t.insert("rider".into(), toml::Value::String(r.to_string()));
        }
        t.insert("locked".into(), toml::Value::Boolean(self.locked));
        t.insert("default".into(), toml::Value::Boolean(self.default));
        t.insert("level".into(), toml::Value::Integer(self.level));
        t.insert("quality".into(), toml::Value::Integer(self.quality));
        t.insert("cost".into(), toml::Value::Integer(self.cost));
        let pairs = self
            .attributes
            .iter()
            .map(|&(s, v)| toml::Value::Array(vec![toml::Value::Integer(s.into()), toml::Value::Integer(v.into())]));
        t.insert("attributes".into(), toml::Value::Array(pairs.collect()));
        if let Some(i) = &self.icon {
            t.insert("icon".into(), toml::Value::String(i.to_string()));
        }
        t.insert("0xb6ef00c6".into(), toml::Value::Integer(self.unk_b6ef00c6));
        if !self.materials.is_empty() {
            let m = self.materials.iter().map(|(k, v)| (k.clone(), toml::Value::String(v.to_string())));
            t.insert("materials".into(), toml::Value::Table(m.collect()));
        }
        t
    }
}
