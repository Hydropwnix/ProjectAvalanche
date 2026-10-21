use ssx_assets::career::CareerDb;
use ssx_assets::career::profile::Notice;
use ssx_formats::attrib::fnv1;

use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{FeEnv, FrontEnd, screens};
use crate::game::loc::Localization;

pub const CAMPAIGN_JUST_FINISHED: u16 = 0x57;
const SLIDES: &str = "assets/unlockable_slides/";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnlockItem {
    pub item_name: String,
    pub category: i32,
    pub priority: i32,
    pub message: String,
    pub image: String,
}

pub const RANGE_SLIDES: [(&str, &str); 9] = [
    ("rockies", "unlock_rockies.dds"),
    ("patagonia", "unlock_patagonia.dds"),
    ("africa", "unlock_africa.dds"),
    ("siberia", "unlock_siberia.dds"),
    ("alaska", "unlock_alaska.dds"),
    ("himalayas", "unlock_himalayas.dds"),
    ("alps", "unlock_alps.dds"),
    ("antarctica", "unlock_antarctica.dds"),
    ("new_zealand", "unlock_newzealand.dds"),
];

pub const CHAPTERS: [(&str, &str); 12] = [
    ("You have unlocked ?", "unlock_rockies.dds"),
    ("You have unlocked ??", "unlock_rockies.dds"),
    ("TXT_NM_RNG_ROCKIES", "unlock_rockies.dds"),
    ("TXT_NM_RNG_PATAGONIA", "unlock_patagonia.dds"),
    ("TXT_NM_RNG_SIBERIA", "unlock_siberia.dds"),
    ("TXT_NM_RNG_ALASKA", "unlock_alaska.dds"),
    ("TXT_NM_RNG_ANTARCTICA", "unlock_antarctica.dds"),
    ("TXT_NM_RNG_HIMALAYAS", "unlock_himalayas.dds"),
    ("TXT_NM_RNG_ALPS", "unlock_alps.dds"),
    ("TXT_NM_RNG_AFRICA", "unlock_africa.dds"),
    ("TXT_NM_RNG_NEW_ZEALAND", "unlock_newzealand.dds"),
    ("You have unlocked ???", "unlock_himalayas.dds"),
];

pub const RIDER_SLIDES: [(&str, &str); 10] = [
    ("alex", "unlock_alex.dds"),
    ("elise", "unlock_elise.dds"),
    ("griff", "unlock_griff.dds"),
    ("kaori", "unlock_kaori.dds"),
    ("mac", "unlock_mac.dds"),
    ("moby", "unlock_moby.dds"),
    ("psymon", "unlock_psymon.dds"),
    ("tane", "unlock_tane.dds"),
    ("tyler", "unlock_ty.dds"),
    ("zoe", "unlock_zoe.dds"),
];

fn text_mode1(loc: &mut Localization, key: &str) -> String {
    loc.text_capitals(key)
}

pub fn items_of(notice: &Notice, db: &CareerDb, loc: &mut Localization) -> Vec<UnlockItem> {
    let slide = |f: &str| format!("{SLIDES}{f}");
    match notice {
        Notice::RangeOpened(name) => {
            let key = fnv1(name.as_bytes());
            let title = db.range(key).map(|r| r.title.clone()).unwrap_or_else(|| "undefined".into());
            let image = RANGE_SLIDES
                .iter()
                .find(|(n, _)| fnv1(n.as_bytes()) == key)
                .map_or_else(|| "undefined".into(), |(_, f)| slide(f));
            vec![UnlockItem {
                item_name: text_mode1(loc, &title),
                category: 4,
                priority: 0,
                message: loc.text("TXT_RANGE_UNLOCKED"),
                image,
            }]
        }
        Notice::ChapterUnlocked(n) => {
            let Some(&(name, file)) = CHAPTERS.get(*n as usize) else { return Vec::new() };
            let item_name = if name.starts_with("TXT_") { text_mode1(loc, name) } else { name.to_string() };
            vec![UnlockItem {
                item_name,
                category: 3,
                priority: 0,
                message: loc.text("TXT_CAMPAIGN_CHAPTER"),
                image: slide(file),
            }]
        }
        Notice::RiderUnlocked(name) => {
            let key = fnv1(name.as_bytes());
            let title = db.riders.iter().find(|r| r.key == key).map(|r| r.title.clone()).unwrap_or_default();
            let image = RIDER_SLIDES
                .iter()
                .find(|(n, _)| fnv1(n.as_bytes()) == key)
                .map_or_else(|| "assets/riderPortraitsLarge/elise.dds".into(), |(_, f)| slide(f));
            let message = format!("{} {}", loc.text("TXT_YOU_HAVE_UNLOCKED"), loc.text(&title));
            vec![UnlockItem { item_name: text_mode1(loc, "TXT_CHARACTER"), category: 6, priority: 2, message, image }]
        }
        Notice::GearUnlocked(name) => {
            let key = fnv1(name.as_bytes());
            let title = db.items.iter().find(|i| i.key == key).map(|i| i.title.clone()).unwrap_or_default();
            let message = format!("{} {}", loc.text("TXT_YOU_HAVE_UNLOCKED"), loc.text(&title));
            vec![UnlockItem {
                item_name: text_mode1(loc, "TXT_GEAR"),
                category: 2,
                priority: 1,
                message,
                image: slide("unlock_gear.dds"),
            }]
        }
        Notice::ModesUnlocked => {
            let message = loc.text("TXT_UNLOCK_GAME_MODES");
            vec![
                UnlockItem {
                    item_name: text_mode1(loc, "TXT_EXPLORE"),
                    category: 0,
                    priority: 1,
                    message: message.clone(),
                    image: slide("unlock_explore.dds"),
                },
                UnlockItem {
                    item_name: text_mode1(loc, "TXT_LIVE_EVENTS"),
                    category: 0,
                    priority: 1,
                    message,
                    image: slide("unlock_global_events.dds"),
                },
            ]
        }
        _ => Vec::new(),
    }
}

pub fn activate(fe: &mut FrontEnd, _env: &mut FeEnv) {
    fe.unlocks_hint_wait = false;
    fe.publish.push("dp_unlocks_data".into());
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str) {
    if action != "A_GameAdvanceScreen" {
        return;
    }
    if fe.game_mode == 1 {
        if env.values.bool(CAMPAIGN_JUST_FINISHED) {
            end_of_campaign_hint(fe, env);
            fe.unlocks_hint_wait = true;
        } else {
            fe.go_to_screen(env, screens::id::CONTINUE_STAGE);
        }
    } else {
        fe.go_to_screen(env, screens::id::EXPLORE_POST_MENU);
    }
    fe.unlocks.clear();
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv) {
    if fe.unlocks_hint_wait && env.values.bool(crate::game::fe::hint::HINT_DONE) {
        fe.unlocks_hint_wait = false;
        fe.note("the end-of-campaign hint is done: screen 0x41");
        fe.go_to_screen(env, screens::id::CONTINUE_STAGE);
    }
}

pub fn end_of_campaign_hint(fe: &mut FrontEnd, env: &mut FeEnv) {
    env.values.set_bool(crate::game::fe::hint::HINT_DONE, false);
    env.values.set_int(crate::game::fe::hint::HINT_ID, END_HINT as i32);
    fe.note("the end-of-campaign hint 0xD9CEE023: hint_enable");
    env.nav_events.push("hint_enable".into());
}

pub const END_HINT: u32 = 0xD9CE_E023;

pub fn build(fe: &FrontEnd, cx: &mut Ctx) -> Value {
    let mut items = fe.unlocks.clone();
    items.sort_by_key(|i| (i.category, i.priority));
    let o = Value::Object(cx.new_object());
    let mut list = Vec::new();
    for i in &items {
        let e = Value::Object(cx.new_object());
        let _ = cx.set_member(&e, "unlockMessage", Value::str(&i.message));
        let _ = cx.set_member(&e, "category", Value::Number(f64::from(i.category)));
        let _ = cx.set_member(&e, "priority", Value::Number(f64::from(i.priority)));
        let _ = cx.set_member(&e, "itemName", Value::str(&i.item_name));
        let _ = cx.set_member(&e, "imageUrl", Value::str(&i.image));
        list.push(e);
    }
    let arr = Value::Object(cx.new_array(list));
    let _ = cx.set_member(&o, "unlockItemList", arr);
    o
}
