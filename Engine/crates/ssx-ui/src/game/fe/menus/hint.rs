use std::collections::BTreeMap;

use anyhow::Result;
use ssx_formats::attrib::Value as AttribValue;
use ssx_formats::attrib::collection::FieldData;
use ssx_formats::attrib::fnv1;

use crate::game::fe::loading::AttribDb;
use crate::game::fe::{ActionParam, FeEnv, FrontEnd};

pub const HINT_ID: u16 = 0x214;
pub const HINT_NO_INPUT: u16 = 0x215;
pub const HINT_DONE: u16 = 0x216;
pub const INPUT_LOCK_COUNT: u16 = 0xBA;
pub const GEAR_HINT: u32 = 0xED62_6A73;
pub const GEOTAG_HINT: u32 = 0xEC87_95A7;
pub const BOOST_HINT: u32 = 0xA4AC_9A6F;
pub const CAMPAIGN_HINT: u32 = 0x23E9_4763;
pub const LEVEL_UP_HINT: u32 = 0x6948_3FCF;
pub const CAMPAIGN_COMPLETE_HINT: u32 = 0xD9CE_E023;
const CLASS: u32 = 0xE04A_24A3;
const F_BODY: u32 = 0x6DFE_676D;
const F_TEXTS: [u32; 4] = [0x203A_E619, 0x203A_E61A, 0x203A_E61B, 0x203A_E61C];
const F_BUTTON: u32 = 0xCA6F_54A8;
const F_IMAGES: [u32; 2] = [0xD7F6_0B12, 0x66FE_82A9];
const F_SCREEN_TYPE: u32 = 0x8250_9184;
const F_TITLE: u32 = 0xDCE0_E62D;
const F_SUSPEND_INPUT: u32 = 0x1A6F_6B46;
const F_TO_ACTIVITY: u32 = 0x8B8C_B70D;
const F_SECONDS: u32 = 0x7D4E_7BBB;
const F_NARROW_ALT: u32 = 0x8ADD_6E04;
const F_PANEL: [u32; 2] = [0xEACA_D681, 0xEACA_D680];
const F_PANEL_ALT: [u32; 2] = [0x3CDC_9C11, 0xE054_763E];
const F_GRADIENT: [u32; 2] = [0xB706_136C, 0xB706_136D];
const F_GRADIENT_ALT: [u32; 2] = [0x553B_B6D2, 0xD1C4_0F55];
const RESET_FLAGS: [&str; 3] =
    ["tutorial_global_wipeout", "TXT_TUTORIAL_HINT_AVALANCHE_TITLE", "tutorial_global_signature"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub body: String,
    pub texts: [String; 4],
    pub button: String,
    pub images: [String; 2],
    pub screen_type: i16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub key: u32,
    pub title: String,
    pub pages: Vec<Page>,
    pub suspend_input: bool,
    pub to_activity: bool,
    pub seconds: f32,
    pub narrow_alt: bool,
    pub panel: [f32; 2],
    pub panel_alt: [f32; 2],
    pub gradient: [f32; 2],
    pub gradient_alt: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub struct Records(pub BTreeMap<u32, Record>);

fn texts(d: &FieldData) -> Vec<String> {
    let one =
        |v: &AttribValue| if let AttribValue::Text(t) = v { t.clone().unwrap_or_default() } else { String::new() };
    match d {
        FieldData::Single(v) => vec![one(v)],
        FieldData::Array(vs) => vs.iter().map(one).collect(),
    }
}

fn first(d: &FieldData) -> Option<&AttribValue> {
    match d {
        FieldData::Single(v) => Some(v),
        FieldData::Array(vs) => vs.first(),
    }
}

impl Records {
    pub fn from_db(db: &AttribDb) -> Result<Self> {
        let mut out = BTreeMap::new();
        for (i, c) in db.cols.iter().enumerate() {
            if c.class != CLASS {
                continue;
            }
            let fields = db.fields(i)?;
            let get = |k: u32| fields.iter().find(|(_, f)| f.key == k).map(|(_, f)| &f.data);
            let list = |k: u32| get(k).map(texts).unwrap_or_default();
            let at = |v: &[String], i: usize| v.get(i).cloned().unwrap_or_default();
            let float = |k: u32| match get(k).and_then(first) {
                Some(AttribValue::Float(x)) => *x,
                _ => 0.0,
            };
            let flag = |k: u32| matches!(get(k).and_then(first), Some(AttribValue::Bool(true)));
            let pair = |k: [u32; 2]| [float(k[0]), float(k[1])];
            let body = list(F_BODY);
            let page_texts = F_TEXTS.map(list);
            let (button, images) = (list(F_BUTTON), F_IMAGES.map(list));
            let types: Vec<i16> = match get(F_SCREEN_TYPE) {
                Some(FieldData::Array(vs)) => {
                    vs.iter().map(|v| if let AttribValue::Int16(x) = v { *x } else { 0 }).collect()
                }
                Some(FieldData::Single(AttribValue::Int16(x))) => vec![*x],
                _ => Vec::new(),
            };
            let pages = (0..body.len())
                .map(|p| Page {
                    body: at(&body, p),
                    texts: [0, 1, 2, 3].map(|k| at(&page_texts[k], p)),
                    button: at(&button, p),
                    images: [at(&images[0], p), at(&images[1], p)],
                    screen_type: types.get(p).copied().unwrap_or(0),
                })
                .collect();
            let title = list(F_TITLE).into_iter().next().unwrap_or_default();
            let record = Record {
                key: c.key,
                title,
                pages,
                suspend_input: flag(F_SUSPEND_INPUT),
                to_activity: flag(F_TO_ACTIVITY),
                seconds: float(F_SECONDS),
                narrow_alt: flag(F_NARROW_ALT),
                panel: pair(F_PANEL),
                panel_alt: pair(F_PANEL_ALT),
                gradient: pair(F_GRADIENT),
                gradient_alt: pair(F_GRADIENT_ALT),
            };
            out.insert(c.key, record);
        }
        Ok(Records(out))
    }

    pub fn get(&self, key: u32) -> Option<&Record> {
        self.0.get(&key)
    }
}

pub fn image_name(name: &str, language_code: &str) -> String {
    let mut out = String::new();
    let mut args = [language_code, "xbox"].into_iter();
    let mut rest = name;
    while let Some(i) = rest.find("%s") {
        out.push_str(&rest[..i]);
        out.push_str(args.next().unwrap_or(""));
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

pub fn language_code(l: crate::game::loc::Language) -> &'static str {
    use crate::game::loc::Language::*;
    match l {
        English => "eng",
        French => "fr",
        German => "ger",
        Spanish => "sp",
        Italian => "it",
        Japanese => "jpn",
    }
}

#[derive(Clone, Debug, Default)]
pub struct Hints {
    pub records: Records,
}

pub fn show(fe: &mut FrontEnd, env: &mut FeEnv, key: u32, force: bool) -> bool {
    let Some(rec) = fe.hints.records.get(key) else { return false };
    if rec.title.is_empty() {
        return false;
    }
    let (title, to_activity) = (rec.title.clone(), rec.to_activity);
    let seen = fe.profile.named_flag(&title);
    if !seen {
        env.values.set_int(HINT_ID, key as i32);
        fe.profile.set_named_flag(&title, true);
        if to_activity {
            let rec = fe.hints.records.get(key).cloned().unwrap_or_default();
            crate::game::fe::notify::post_hint(fe, env.loc, &rec);
            return true;
        }
    } else if force {
        env.values.set_int(HINT_ID, key as i32);
    } else {
        return false;
    }
    fe.note(format!("tutorial hint {key:#010x} ({title}): message 0xFCCD4EB8 {{1}}"));
    fe.popup_message(1);
    !seen
}

pub fn request(fe: &mut FrontEnd, env: &mut FeEnv, key: u32) {
    let mut shown = show(fe, env, key, false);
    if key == crate::game::fe::equipment::EQUIPMENT_HINT {
        let slots = fe.equipment.inv.as_ref().map(|i| i.category_slots.clone()).unwrap_or_default();
        let has = |k: usize| slots.get(k).copied().unwrap_or(0) != 0;
        for (kind, hint) in [(2, GEAR_HINT), (4, GEOTAG_HINT), (3, BOOST_HINT)] {
            if !shown && has(kind) {
                shown = show(fe, env, hint, false);
            }
        }
    }
    if !shown {
        env.values.set_bool(HINT_DONE, true);
    }
}

pub fn campaign_hint(fe: &mut FrontEnd, env: &mut FeEnv, key: u32) {
    if !show(fe, env, key, false) {
        env.values.set_bool(HINT_DONE, true);
    }
}

pub fn reset(fe: &mut FrontEnd) {
    let titles: Vec<String> =
        fe.hints.records.0.values().filter(|r| !r.title.is_empty()).map(|r| r.title.clone()).collect();
    for t in titles.iter().map(String::as_str).chain(RESET_FLAGS) {
        fe.profile.set_named_flag(t, false);
    }
}

pub fn tutorial_action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: &ActionParam) {
    match action {
        "A_GameHintTutorial" if param.number as i32 == 1 => env.nav_events.push("hint_enable".into()),
        "A_GamePopupObjective" => fe.go_to_screen(env, 0x3E),
        "A_GamePopupNone" => fe.go_to_screen(env, 0x37),
        "A_GameLevelUp" => {
            fe.note("message 0x563736F1 (the levelling hint)");
            show(fe, env, LEVEL_UP_HINT, false);
        }
        _ => {}
    }
}

pub fn flag_key(title: &str) -> u32 {
    fnv1(title.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_takes_the_language_code() {
        assert_eq!(
            image_name("assets/tutorial_images/equipment1_%s.dds", "eng"),
            "assets/tutorial_images/equipment1_eng.dds"
        );
        assert_eq!(image_name("assets/tutorial_images/worldtour.dds", "fr"), "assets/tutorial_images/worldtour.dds");
    }
}
