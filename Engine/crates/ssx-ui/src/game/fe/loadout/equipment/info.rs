use super::*;
use crate::game::loc::{FormatArg, format_positional};

pub const INVENTORY_LIMIT: i32 = 450;

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Num(f64),
    Bool(bool),
    Str(String),
    Array(Vec<Field>),
}

impl Field {
    pub fn to_value(&self, cx: &mut Ctx) -> Value {
        match self {
            Field::Num(n) => Value::Number(*n),
            Field::Bool(b) => Value::Bool(*b),
            Field::Str(s) => Value::str(s),
            Field::Array(a) => {
                let items: Vec<Value> = a.iter().map(|f| f.to_value(cx)).collect();
                Value::Object(cx.new_array(items))
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GearInfo {
    fields: Vec<(&'static str, Field)>,
}

impl GearInfo {
    pub fn set(&mut self, name: &'static str, v: Field) {
        match self.fields.iter_mut().find(|(k, _)| *k == name) {
            Some(f) => f.1 = v,
            None => self.fields.push((name, v)),
        }
    }

    pub fn get(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|(k, _)| *k == name).map(|(_, v)| v)
    }

    pub fn to_value(&self, cx: &mut Ctx) -> Value {
        let o = Value::Object(cx.new_object());
        for (k, v) in &self.fields {
            let v = v.to_value(cx);
            let _ = cx.set_member(&o, k, v);
        }
        o
    }
}

pub(super) fn stat_columns(loc: &mut Localization, u: &UiItem) -> [(&'static str, Field); 5] {
    let mut cols: [Vec<Field>; 5] = Default::default();
    for &(t, v) in &u.stats {
        if t < 0 {
            continue;
        }
        cols[0].push(Field::Num(t as f64));
        cols[1].push(Field::Num(v as f64));
        cols[2].push(Field::Num(v as f64));
        cols[3].push(Field::Str(loc.text(&format!("TXT_EQUIPSCREEN_STAT{t}_LONG"))));
        cols[4].push(Field::Str(loc.text(&format!("TXT_EQUIPSCREEN_STAT{t}_DESCRIPTION"))));
    }
    let [a, b, c, d, e] = cols;
    [
        ("itemStatTypes", Field::Array(a)),
        ("itemStatValues", Field::Array(b)),
        ("itemStatPercs", Field::Array(c)),
        ("itemStatNames", Field::Array(d)),
        ("itemStatDescription", Field::Array(e)),
    ]
}

pub fn fill_entry(info: &mut GearInfo, loc: &mut Localization, values: &mut UiValues, u: &UiItem) {
    let flags = |f: u32| Field::Bool(u.has(f));
    let price = loc.currency(u.cost, true);
    let level = loc.format("TXT_EQUIP_LEVEL", &[&u.level.to_string()]);
    let recommend = u.recommend.clone();
    let slot = u.slot;
    if slot == 8 {
        values.set(crate::game::fe::hint::GEAR_ENTRY, UiValue::Item(Box::new(u.clone())));
    }
    let help = loc.text(&format!("TXT_EQUIPSCREEN_GEAR{slot}_HELP"));
    let tag = loc.format("TXT_PROFILE_GAMERTAG", &[&u.gamertag]);
    for (k, v) in [
        ("state", Field::Num(if u.has(flag::EMPTY) { 2.0 } else { 0.0 })),
        ("strItem", Field::Str(u.name.clone())),
        ("description", Field::Str(u.description.clone())),
        ("assetUrl", Field::Str(u.asset.clone())),
        ("secondaryAssetUrl", Field::Str(u.secondary.clone())),
        ("isAvailable", flags(flag::AVAILABLE)),
        ("isPurchaseable", flags(flag::PURCHASEABLE)),
        ("isLevelPurchaseable", flags(flag::LEVEL_PURCHASEABLE)),
        ("isSellable", flags(flag::SELLABLE)),
        ("price", Field::Str(price)),
        ("level", Field::Num(u.level as f64)),
        ("levelProgress", Field::Num(u.level_progress as f64)),
        ("levelString", Field::Str(level)),
        ("quality", Field::Num(u.quality as f64)),
        ("qualityRating", Field::Num(u.rarity as f64)),
        ("showStats", flags(flag::SHOW_STATS)),
        ("showLevel", flags(flag::SHOW_LEVEL)),
        ("isRecommended", flags(flag::RECOMMENDED)),
        ("recommendText", Field::Str(recommend)),
        ("gearItemType", Field::Num(slot as f64)),
        ("strHelp", Field::Str(help)),
        ("gamerTagCallout", Field::Str(tag)),
        ("isNew", flags(flag::NEW)),
        ("isEquipped", flags(flag::EQUIPPED)),
        ("isMystery", flags(flag::MYSTERY)),
    ] {
        info.set(k, v);
    }
    for (k, v) in stat_columns(loc, u) {
        info.set(k, v);
    }
}

pub fn fill_view_more(info: &mut GearInfo, loc: &mut Localization, values: &UiValues, category: i32) {
    let size = values.int(0x64);
    let title = loc.text("TXT_INV_VIEWMORE");
    let cat = loc.text(&format!("TXT_INV_VIEW_MORE_CAT{category}"));
    let desc = loc.text("TXT_INV_VIEW_MORE_DESC");
    let args = [FormatArg::Str(&cat), FormatArg::Int(i64::from(size)), FormatArg::Int(i64::from(INVENTORY_LIMIT))];
    let desc = format_positional(&desc, &args);
    for (k, v) in [
        ("state", Field::Num(1.0)),
        ("strItem", Field::Str(title)),
        ("description", Field::Str(desc)),
        ("isAvailable", Field::Bool(true)),
        ("isPurchaseable", Field::Bool(false)),
        ("price", Field::Str(String::new())),
        ("isAffordable", Field::Bool(true)),
        ("isNew", Field::Bool(false)),
        ("showStats", Field::Bool(false)),
    ] {
        info.set(k, v);
    }
}
