use ssx_assets::career::random::GameRandom;
use ssx_assets::career::store::purchase::{OfferPurchase, PurchaseError};

use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::inventory::{self, DropInfo, Inventory, UiItem, flag};
use crate::game::fe::{ActionParam, FeEnv, FrontEnd, screens};
use crate::game::loc::Localization;
use crate::game::values::id::EQUIPMENT_LIMITED_TIME_OFFER as LIMITED_TIME_OFFER;
use crate::game::values::{UiValue, UiValues};

mod actions;
mod data;
mod info;
pub mod showcase;
#[cfg(test)]
mod tests;

pub use actions::*;
pub use data::*;
pub use info::*;

pub mod state {
    pub const HIDDEN: i32 = 0;
    pub const RIDER: i32 = 1;
    pub const GEAR: i32 = 2;
    pub const INVENTORY: i32 = 3;
    pub const READY: i32 = 4;
    pub const ADVANCE: i32 = 5;
    pub const READY_CATEGORY: i32 = 6;
    pub const MOVIES: i32 = 7;
    pub const HINT: i32 = 8;
}

pub const TUTORIAL_ITEMS: [u32; 2] = [0x8650_5C81, 0xE318_0443];
pub const EQUIPMENT_HINT: u32 = 0xF47D_E822;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Items,
    Store,
    Inventory,
}

#[derive(Clone, Debug, Default)]
pub struct Equipment {
    pub drop: u32,
    pub state: i32,
    pub category: i32,
    pub slot: i32,
    pub score: i32,
    pub rider_index: i32,
    pub last_rider: u32,
    pub rider_count: i32,
    pub list: Vec<UiItem>,
    pub store: Vec<UiItem>,
    pub inventory: Vec<UiItem>,
    pub inv_max: i32,
    pub inv_unselectable: i32,
    pub inv_first: i32,
    pub inv_focus: i32,
    pub inv_step: i32,
    pub inv_index: i32,
    pub pending: Option<(List, usize)>,
    pub gear_info: GearInfo,
    pub purchase_result: bool,
    pub online: i32,
    pub rider: u32,
    pub inv: Option<Box<Inventory>>,
    pub offer_ms: i32,
    offer_shown: Option<i64>,
    /// A port addition (the menus' 3-D rider showcase): the rider the list has highlighted (None: the chosen one,
    /// `rider_index`, until the list highlights another).
    pub highlight: Option<usize>,
    /// The 3D Rider Select mod: the gear list's hovered item (its list, its place in the full list), the suit or board
    /// the 3-D rider wears meanwhile (the hovered one: its kind, slot and item), and Y pressed on a suit or board
    /// (marking it favourite, or not) for the next update.
    pub hovered: Option<(List, usize)>,
    pub preview: Option<(usize, usize, (u32, u8))>,
    pub favourite_press: bool,
}

impl Equipment {
    fn new_view(&mut self) {
        self.list.clear();
        self.store.clear();
        self.inventory.clear();
        (self.inv_max, self.inv_unselectable, self.inv_first, self.inv_focus) = (0, 0, 0, 0);
        (self.inv_step, self.inv_index) = (0, 0);
        self.pending = None;
        self.gear_info = GearInfo::default();
        self.offer_ms = 0;
        self.offer_shown = None;
    }

    pub fn rider_name(&self, fe: &FrontEnd) -> String {
        let forced = fe.career.event(self.drop).and_then(|e| e.forced.get(5).copied()).filter(|&k| k != 0);
        let luri = fe.profile.attribs.get(&u32::from_be_bytes(*b"LURI")).map(|&v| v as u32);
        let key = [Some(self.rider).filter(|&k| k != 0), forced, luri].into_iter().flatten().next().unwrap_or(0);
        fe.career.riders.iter().find(|r| r.key == key).map_or_else(|| "zoe".to_string(), |r| r.name.clone())
    }
}

fn kind_of(category: i32) -> i64 {
    if category == 0 { inventory::RIDER } else { i64::from(category) - 1 }
}

fn drop_info(fe: &FrontEnd) -> Option<DropInfo> {
    DropInfo::of(&fe.career, &fe.profile, fe.equipment.drop)
}

fn inv(fe: &mut FrontEnd) -> Box<Inventory> {
    fe.equipment.inv.take().unwrap_or_else(|| Box::new(Inventory::new(&fe.career)))
}

pub fn flow_equip(fe: &mut FrontEnd, key: u32, slot: i64) {
    let mut inv = inv(fe);
    let taken = inv.equip_message(&fe.career, &mut fe.profile, fe.current_drop, key, 0, slot);
    fe.equipment.inv = Some(inv);
    fe.note(format!("equip {key:#010x} in slot {slot} (0x82577AB0){}", if taken { "" } else { ": not taken" }));
}

pub fn launch_loadout(fe: &mut FrontEnd, env: &mut FeEnv, drop: u32) {
    fe.current_drop = drop;
    fe.messages.push(crate::game::fe::FeMessage::LoadoutRefresh);
    let mut inv = inv(fe);
    inv.now_ms = (fe.time * 1000.0) as i64;
    let rider = inv.equip_for_launch(&fe.career, &mut fe.profile, drop);
    let info = DropInfo::of(&fe.career, &fe.profile, drop);
    let before = inv.survival;
    let gear = inv.equip_required_for_launch(&fe.career, &fe.profile, env.loc, drop);
    let after = inv.survival;
    if let Some(score) = inv.survival_on(&fe.career, &fe.profile, drop) {
        env.values.set_int(SURVIVAL_SCORE, score);
    }
    fe.equipment.inv = Some(inv);
    fe.note(match rider {
        Some(r) => format!("0x17D251BE; the launch's rider {r:#010x} equipped with their saved loadout (0x82577AB0)"),
        None => format!("0x17D251BE; WARNING no rider for the launch on {drop:#010x} (none listed): nothing equipped"),
    });
    let Some(info) = info.filter(|i| !i.required.is_empty()) else { return };
    let put_on: Vec<String> =
        gear.equipped.iter().map(|&((k, s), slot)| format!("{k:#010x}/{s} in gear slot {slot}")).collect();
    fe.note(format!(
        "the drop's required equipment {:?}: put on [{}], survival score {before} -> {after}",
        info.required,
        put_on.join(", ")
    ));
    if !gear.unmet.is_empty() {
        fe.note(format!(
            "WARNING the launch's rider owns no item for the required equipment {:?}: riding without it, as past the \
             console's low-survival popup",
            gear.unmet
        ));
    }
}

// PORT(choice): a network session's launch (no equipment screen; the round runs on the server's clock, so nothing may
// wait for an answer) shows the console's low-survival warning — TXT_EQUIP_LOW_SURVIVAL_BODY with the launch's survival
// score, under the console's test (UI 0x6C below 30 without FE_POPUP_LOW_SURVIVE_NeverShowAgain, UI 0x9E) — as a notice
// row of the session's chat layer once the level shows it (its 10 s, nothing held, no answer taken). The console asks
// it at Start Run (0x825FB798 → go_to_low_survive_popup → equip_low_surival.pop: TXT_EQUIP_LOW_SURVIVAL_TITLE, Yes / No
// / Yes and never ask again, No the default): Yes launches without the gear (0x825FB788 → 0x825FB660), No stays on the
// equipment screen, the never-again's confirm sets 0x9E (0x825F7368 → 0x825F7308, saved in the profile's chunk 9).
#[cfg(feature = "net")]
pub fn session_low_survival(fe: &mut FrontEnd, env: &mut FeEnv, drop: u32) {
    if DropInfo::of(&fe.career, &fe.profile, drop).is_none() {
        return;
    }
    let score = env.values.int(SURVIVAL_SCORE);
    match low_survival_body(env.loc, env.values, score) {
        Some(body) => {
            fe.note(format!(
                "the console's low-survival popup (0x825FB798: survival score {score} < 30, never-show flag clear): \
                 its body kept as the session's launch notice"
            ));
            fe.ava.session.launch_notice = Some(body);
        }
        None => {
            let why =
                if score >= 30 { "the survival score is 30 or more" } else { "the never-show flag (0x9E) is set" };
            fe.note(format!("no low-survival popup for the launch (survival score {score}): {why}"));
            fe.ava.session.launch_notice = None;
        }
    }
}

fn set_category(fe: &mut FrontEnd, env: &mut FeEnv, category: i32) {
    let kind = kind_of(category);
    let mut inv = inv(fe);
    inv.now_ms = (fe.time * 1000.0) as i64;
    let drop = drop_info(fe);
    let ctx = inv.store_context(&fe.career, &fe.profile, drop.as_ref());
    inv.build_lists(&fe.career, &fe.profile, env.loc, drop.as_ref(), &ctx, kind);
    inv.build_selected_gear(&fe.career, &fe.profile, env.loc, drop.as_ref());
    env.values.set_int(0x6C, inv.survival);
    env.values.set_int(0x64, inv.inventory_size);
    let notes = [store_note(&inv, "built"), owned_note(&inv, "built")];
    fe.equipment.inv = Some(inv);
    for note in notes {
        fe.note(note);
    }
}

fn owned_note(inv: &Inventory, what: &str) -> String {
    let list: Vec<String> = inv.owned_list.iter().map(|u| format!("{:#x}/{}", u.key, u.src)).collect();
    format!("equipment: owned list {what} [{}]", list.join(", "))
}

fn store_note(inv: &Inventory, what: &str) -> String {
    let list: Vec<String> = inv.store_list.iter().map(|u| format!("{:#x}:{}", u.key, u.cost)).collect();
    let specials: Vec<Vec<String>> = (inv.store.stock.iter())
        .map(|l| l.iter().filter(|i| i.special).map(|i| format!("{:#x}@{}", i.id, i.expires_ms)).collect())
        .collect();
    format!(
        "equipment: store list {what} [{}], special shown {:x?}, the stock's specials by kind {specials:?}",
        list.join(", "),
        inv.store.shown_special
    )
}

fn loadout_refresh(fe: &mut FrontEnd, env: &mut FeEnv) {
    let mut inv = inv(fe);
    let rider = fe.profile.attribs.get(&ssx_assets::career::profile::ATTRIB_LAST_RIDER).map_or(0, |&v| v as u32);
    inv.equip_rider(&fe.career, &fe.profile, fe.equipment.drop, rider);
    inv.now_ms = (fe.time * 1000.0) as i64;
    let drop = fe.equipment.drop;
    let force = fe.career.event(drop).is_some() && drop != inv.last_drop;
    if fe.career.event(drop).is_some() {
        inv.last_drop = drop;
    }
    let info = drop_info(fe);
    let ctx = inv.store_context(&fe.career, &fe.profile, info.as_ref());
    let data = inv.data.clone();
    inv.store.update(&data, &ctx, force, inv.now_ms);
    if force {
        inv.recommend_first = inv.store.rng.below(100) >= 50;
    }
    fe.equipment.inv = Some(inv);
    set_category(fe, env, 0);
}

pub fn activate(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.equipment.new_view();
    bought_levels_reached(fe);
    env.values.set_bool(0xEF, true);
    let eq = &mut fe.equipment;
    eq.state = env.values.int(0x62);
    if eq.state == 0 {
        fe.note("equipment: message 0x17D251BE (loadout refresh)");
        fe.messages.push(crate::game::fe::FeMessage::LoadoutRefresh);
        loadout_refresh(fe, env);
        env.values.set_int(0x6C, 0);
    }
    let eq = &mut fe.equipment;
    eq.score = env.values.int(0x6C);
    eq.category = env.values.int(0x5F);
    eq.slot = env.values.int(0x61);
    eq.last_rider = env.values.int(0x65) as u32;
    // A port fix: with no rider picked yet this session (the value starts at 0 at every launch), the character select
    // opens on the profile's last rider (its `LURI`, saved with the profile) rather than the first in the list.
    // (the rider last chosen as kept on disk first: the profile's is only as new as its last save)
    let kept = inventory::event_loadouts::last_rider()
        .filter(|r| fe.career.riders.iter().any(|x| x.key == *r))
        .map(i64::from);
    if eq.last_rider == 0
        && let Some(last) = kept.or_else(|| fe.profile.attribs.get(&ssx_assets::career::profile::ATTRIB_LAST_RIDER).copied())
        && let Ok(last) = u32::try_from(last)
        && last != 0
    {
        eq.last_rider = last;
        env.values.set_int(0x65, last as i32);
    }
    eq.rider_count = env.values.int(0x60);
    for id in [0x5F, 0x61, 0x62, 0x60] {
        env.values.set_int(id, 0);
    }
    env.values.set_bool(0x6A, false);
    eq.online = env.values.int(0x22F);
    if eq.state == 0 {
        publish_list(fe, "dp_playable_characters");
        fe.publish.extend(
            [
                "dp_playable_characters_index",
                "dp_equipment_items_index",
                "dp_store_items_index",
                "dp_equipment_categories",
                "dp_equipment_categories_index",
            ]
            .map(String::from),
        );
        if fe.equipment.rider_count == 1 {
            select(fe, env, List::Items, 0);
            return;
        }
        fe.equipment.state = state::RIDER;
        fe.equipment.highlight = None;
        fe.publish.push("dp_screen_state".into());
        return;
    }
    fe.publish.extend(
        [
            "dp_equipment_items_index",
            "dp_store_items_index",
            "dp_selected_rider",
            "dp_equipment_categories",
            "dp_equipment_categories_index",
            "dp_screen_state",
        ]
        .map(String::from),
    );
}

fn equip(fe: &mut FrontEnd, env: &mut FeEnv, item: &UiItem) {
    let mut inv = inv(fe);
    let kind = kind_of(fe.equipment.category);
    if kind == inventory::RIDER {
        inv.now_ms = (fe.time * 1000.0) as i64;
        inv.select_rider(&fe.career, &fe.profile, fe.equipment.drop, item.key);
        fe.equipment.rider = item.key;
    } else {
        inv.equip(&fe.career, &fe.profile, kind, item.key, item.src, fe.equipment.slot.max(0) as usize);
    }
    let (save_kind, save_slot) = if kind == inventory::RIDER { (kind, 0) } else { (kind, fe.equipment.slot.max(0)) };
    if kind == inventory::RIDER
        || inv.equipped.get(kind as usize).and_then(|l| l.get(save_slot as usize)) == Some(&(item.key, item.src))
    {
        inv.save_slot(&fe.career, &mut fe.profile, save_kind, save_slot as usize, (item.key, item.src));
    }
    let drop = drop_info(fe);
    inv.survival = inv.survival_score(&fe.career, &fe.profile, drop.as_ref());
    inv.build_selected_gear(&fe.career, &fe.profile, env.loc, drop.as_ref());
    env.values.set_int(0x6C, inv.survival);
    fe.equipment.inv = Some(inv);
    fe.note(format!("equipment: equip {:#010x}/{} in kind {kind} slot {}", item.key, item.src, fe.equipment.slot));
}

pub fn select(fe: &mut FrontEnd, env: &mut FeEnv, list: List, index: usize) {
    let entries = match list {
        List::Items => &fe.equipment.list,
        List::Store => &fe.equipment.store,
        List::Inventory => &fe.equipment.inventory,
    };
    let Some(item) = entries.get(index).cloned() else { return };
    equip(fe, env, &item);
    fe.equipment.score = env.values.int(0x6C);
    if fe.equipment.category == 0 {
        if fe.equipment.drop == crate::game::fe::globe::TUTORIAL_DROPS[0] {
            let mut inv = inv(fe);
            for (kind, key) in [(1, TUTORIAL_ITEMS[0]), (2, TUTORIAL_ITEMS[1])] {
                inv.equip(&fe.career, &fe.profile, kind, key, 0, 0);
            }
            fe.equipment.inv = Some(inv);
            fe.equipment.slot = 0;
            fe.equipment.state = state::ADVANCE;
            fe.equipment.category = 6;
            advance(fe, env);
            return;
        }
        fe.equipment.slot = 0;
        fe.equipment.state = state::HINT;
        fe.equipment.category = 1;
        env.values.set_bool(0x216, false);
        fe.note("equipment: message 0x93D1A7F2 (first-time equipment hint)");
        crate::game::fe::hint::request(fe, env, EQUIPMENT_HINT);
        return;
    }
    if fe.equipment.state == state::INVENTORY {
        fe.equipment.state = state::READY_CATEGORY;
        save_state(fe, env);
        fe.go_to_screen(env, screens::id::EQUIPMENT);
        return;
    }
    fe.equipment.state = state::READY;
    fe.publish.extend(
        ["dp_screen_state", "dp_equipment_score", "dp_selected_rider", "dp_equipment_categories"].map(String::from),
    );
}

fn save_state(fe: &FrontEnd, env: &mut FeEnv) {
    let eq = &fe.equipment;
    env.values.set_int(0x5F, eq.category);
    env.values.set_int(0x61, eq.slot);
    env.values.set_int(0x62, eq.state);
    env.values.set_int(0x60, eq.rider_count);
}

/// The 3D Rider Select mod: the suits' and boards' lists (their categories).
pub(crate) fn favourite_kinds(category: i32) -> bool {
    matches!(category, 1 | 2)
}

/// The 3D Rider Select mod: the lists whose hovered item the rider wears meanwhile on the 3D model (suits, boards and
/// gear).
pub(crate) fn preview_kinds(category: i32) -> bool {
    matches!(category, 1..=3)
}

/// The 3D Rider Select mod: Y on an owned suit or board (the gear list's, or the full list's) marks it favourite (or
/// not), listing it first; taken (not for the screen's script) for the next update.
pub fn take_suit_press(fe: &mut FrontEnd, button: crate::game::input::Button) -> bool {
    if !ssx_core::rider_showcase::enabled()
        || !favourite_kinds(fe.equipment.category)
        || button != crate::game::input::Button::DigitalThree
    {
        return false;
    }
    let gear = fe.screen == screens::id::EQUIPMENT && fe.equipment.state == state::GEAR;
    let full_list = fe.screen == 0x31 && fe.equipment.state == state::INVENTORY;
    if !gear && !full_list {
        return false;
    }
    fe.equipment.favourite_press = true;
    true
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv) {
    if let Some(left) = fe.equipment.inv.as_ref().and_then(|i| i.store.shown_special_left((fe.time * 1000.0) as i64)) {
        env.values.set_int(LIMITED_TIME_OFFER, i32::try_from(left).unwrap_or(i32::MAX));
    }
    if fe.equipment.state == state::GEAR {
        if env.values.is_dirty(LIMITED_TIME_OFFER) {
            fe.equipment.offer_ms = env.values.int(LIMITED_TIME_OFFER);
        }
        let eq = &mut fe.equipment;
        let shown = eq.store.iter().any(|u| u.has(flag::SPECIAL)).then(|| i64::from(eq.offer_ms.max(0)) / 1000);
        if shown != eq.offer_shown {
            eq.offer_shown = shown;
            fe.publish.push("dp_limited_time_offer".into());
        }
    }
    if std::mem::take(&mut fe.equipment.favourite_press) {
        actions::toggle_favourite(fe, env);
    }
    let lists = matches!(fe.equipment.state, state::GEAR | state::INVENTORY);
    let suits = favourite_kinds(fe.equipment.category) && lists;
    if !(preview_kinds(fe.equipment.category) && lists) {
        fe.equipment.preview = None;
    }
    if ssx_core::rider_showcase::enabled() {
        // (the highlighted rider's level for sale: its prompt and price placed by the highlighted card)
        let at = fe.equipment.highlight.or_else(|| usize::try_from(fe.equipment.rider_index).ok());
        let offer = showcase::active(fe)
            && at.and_then(|i| fe.equipment.list.get(i)).is_some_and(|u| u.has(flag::LEVEL_PURCHASEABLE));
        ssx_core::rider_showcase::set_level_offer(offer);
        ssx_core::rider_showcase::set_full_list_view(fe.screen == 0x31 && fe.equipment.state == state::INVENTORY);
        // (the favourite suits' stars: which of the gear list's items, and of the full list's (with its highlighted one))
        let stars = |l: &[UiItem]| -> Vec<bool> {
            if suits { l.iter().map(inventory::favorites::is_favourite).collect() } else { Vec::new() }
        };
        let full = fe.equipment.inv.as_ref().map(|i| stars(&i.inventory_list)).unwrap_or_default();
        let highlighted = fe.equipment.hovered.filter(|h| h.0 == List::Inventory).map(|h| h.1);
        ssx_core::rider_showcase::set_favourite_cells(stars(&fe.equipment.list), full, highlighted);
    }
    if fe.equipment.state == state::HINT && env.values.bool(0x216) {
        fe.equipment.state = state::READY;
        fe.publish.extend(
            ["dp_screen_state", "dp_equipment_score", "dp_selected_rider", "dp_equipment_categories"].map(String::from),
        );
    }
    if fe.equipment.state == state::MOVIES && fe.movie.is_none() {
        fe.equipment.state = state::ADVANCE;
        fe.publish.push("dp_screen_state".into());
        launch(fe, env);
    }
    env.values.set_bool(0x67, fe.equipment.state == state::ADVANCE);
}

fn restricted(fe: &FrontEnd) -> bool {
    fe.equipment.inv.as_ref().is_some_and(|i| i.selected_gear.iter().skip(1).any(|u| !u.has(flag::AVAILABLE)))
}

pub fn advance(fe: &mut FrontEnd, env: &mut FeEnv) {
    if restricted(fe) {
        let bad: Vec<String> = fe
            .equipment
            .inv
            .as_ref()
            .map(|i| {
                i.selected_gear
                    .iter()
                    .skip(1)
                    .filter(|u| !u.has(flag::AVAILABLE))
                    .map(|u| format!("{:#010x}/{} flags {:#x}", u.key, u.src, u.flags))
                    .collect()
            })
            .unwrap_or_default();
        fe.note(format!("equipment: restricted gear {bad:?}"));
        env.nav_events.push("equipment_open_gear_restricted".into());
        return;
    }
    if let Some(body) = low_survival_body(env.loc, env.values, fe.equipment.score) {
        env.values.set_str(0x99, body);
        env.nav_events.push("go_to_low_survive_popup".into());
        return;
    }
    advance_confirmed(fe, env);
}

pub const LOW_SURVIVE_NEVER_SHOW: u16 = 0x9E;
pub const SURVIVAL_SCORE: u16 = 0x6C;

pub fn low_survival_body(loc: &mut Localization, values: &UiValues, score: i32) -> Option<String> {
    if values.bool(LOW_SURVIVE_NEVER_SHOW) || score >= 30 {
        return None;
    }
    let percent = loc.percent(i64::from(score));
    Some(loc.format("TXT_EQUIP_LOW_SURVIVAL_BODY", &[&percent]))
}

pub fn advance_confirmed(fe: &mut FrontEnd, env: &mut FeEnv) {
    let drop = fe.equipment.drop;
    if crate::game::fe::progression::pre_launch_movies(fe, drop) {
        fe.note("equipment: pre-launch movies (0x8257D1A8)");
        fe.equipment.state = state::MOVIES;
        return;
    }
    fe.equipment.state = state::ADVANCE;
    fe.publish.push("dp_screen_state".into());
    launch(fe, env);
}

fn launch(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.note("equipment: game setup, save (FE manager +0xC4)");
    fe.requests.push(crate::game::fe::pause::GameRequest::Save);
    env.values.set_int(0x231, 0);
    let drop = fe.equipment.drop;
    save_state_reset(env);
    crate::game::fe::flow::launch(fe, env, drop, false);
}

fn save_state_reset(env: &mut FeEnv) {
    for id in [0x5F, 0x61, 0x62, 0x60] {
        env.values.set_int(id, 0);
    }
    env.values.set_bool(0x6A, false);
}

fn back_out(fe: &mut FrontEnd, env: &mut FeEnv) {
    save_state_reset(env);
    refund_reserved_fee(fe, env);
    fe.note("equipment: back out, save (FE manager +0xC4)");
    fe.requests.push(crate::game::fe::pause::GameRequest::Save);
    crate::game::fe::globe::back_from_equipment(fe, env);
}

fn refund_reserved_fee(fe: &mut FrontEnd, env: &mut FeEnv) {
    let fee = env.values.int(0x231);
    if fee >= 0 {
        fe.profile.add_credits(i64::from(fee), false);
        let bank = env.values.int(0x93);
        env.values.set_int(0x93, bank.wrapping_add(fee));
    }
    env.values.set_int(0x231, 0);
}

fn preview(fe: &mut FrontEnd, env: &mut FeEnv, item: Option<(u32, u8)>) {
    let mut inv = inv(fe);
    let drop = drop_info(fe);
    let kind = kind_of(fe.equipment.category);
    match item {
        None => {
            let score = inv.survival_score(&fe.career, &fe.profile, drop.as_ref());
            env.values.set_int(0x6C, score);
        }
        Some(entry) => {
            inv.seen(kind, entry);
            if kind == inventory::RIDER {
                inv.rider_bio(&fe.career, &fe.profile, env.loc, entry.0);
            } else if let Some(k) = usize::try_from(kind).ok().filter(|&k| k < 4) {
                let slot = usize::try_from(fe.equipment.slot).unwrap_or(0);
                let saved = inv.equipped[k].get(slot).copied();
                if let Some(e) = inv.equipped[k].get_mut(slot) {
                    *e = entry;
                }
                let score = inv.survival_score(&fe.career, &fe.profile, drop.as_ref());
                if let (Some(s), Some(e)) = (saved, inv.equipped[k].get_mut(slot)) {
                    *e = s;
                }
                env.values.set_int(0x6C, score);
            }
        }
    }
    fe.equipment.inv = Some(inv);
}

fn purchase(fe: &mut FrontEnd, env: &mut FeEnv, list: List, index: usize) {
    let entries = if list == List::Store { &fe.equipment.store } else { &fe.equipment.list };
    let Some(item) = entries.get(index).cloned() else { return };
    let available = item.has(flag::AVAILABLE);
    if (!available || item.has(flag::LEVEL_PURCHASEABLE)) && item.cost > i64::from(env.values.int(0x228)) {
        env.nav_events.push("go_to_equip_not_enough_money_popup".into());
        return;
    }
    let state = fe.equipment.state;
    if available || !matches!(state, state::GEAR | state::RIDER) {
        return;
    }
    if state == state::GEAR && env.values.int(0x64) >= INVENTORY_LIMIT {
        env.nav_events.push("equip_open_inventory_full".into());
        fe.equipment.purchase_result = false;
        fe.publish.push("dp_equipment_purchase_result".into());
        return;
    }
    let mut inv = inv(fe);
    let kind = kind_of(fe.equipment.category);
    let new_list = usize::try_from(kind).ok().filter(|&k| k <= inventory::KINDS);
    inv.now_ms = (fe.time * 1000.0) as i64;
    let paid = match (item.src, new_list) {
        (1, Some(k)) if k < inventory::KINDS => {
            let info = drop_info(fe);
            let ctx = inv.store_context(&fe.career, &fe.profile, info.as_ref());
            let data = inv.data.clone();
            let now = inv.now_ms;
            fe.profile
                .buy_offer(
                    &mut inv.store,
                    &data,
                    &ctx,
                    OfferPurchase { kind: k, id: item.key, clock_s: (fe.time as u32).max(1), now_ms: now },
                )
                .map(|id| {
                    inv.new_items[k].push((id, 1));
                    item.cost
                })
        }
        (1, _) => Err(PurchaseError::Unknown),
        _ => fe.profile.buy_fixed(&inv.catalog, item.key).inspect(|_| {
            if let Some(k) = new_list {
                inv.new_items[k].push((item.key, 0));
            }
        }),
    };
    fe.equipment.inv = Some(inv);
    match paid {
        Ok(cost) => {
            fe.note(format!("equipment: bought {:#010x}/{} for {cost}", item.key, item.src));
            let adj = env.values.int(0x93);
            env.values.set_int(0x93, adj.wrapping_sub(i32::try_from(cost).unwrap_or(i32::MAX)));
            after_purchase(fe, env, kind, (item.key, item.src));
        }
        Err(e) => fe.note(format!("equipment: purchase refused: {e:?}")),
    }
    if state == state::GEAR {
        fe.equipment.purchase_result = true;
        fe.publish.push("dp_equipment_purchase_result".into());
    } else {
        publish_list(fe, "dp_playable_characters");
    }
}

fn after_purchase(fe: &mut FrontEnd, env: &mut FeEnv, kind: i64, bought: (u32, u8)) {
    let mut inv = inv(fe);
    let drop = drop_info(fe);
    let ctx = inv.store_context(&fe.career, &fe.profile, drop.as_ref());
    let before = store_note(&inv, "before the purchase");
    if kind != inventory::RIDER {
        inv.put_bought_first(&fe.career, &fe.profile, env.loc, drop.as_ref(), kind, bought);
    }
    inv.refresh_lists(&fe.career, &fe.profile, env.loc, drop.as_ref(), &ctx, kind);
    let after = [store_note(&inv, "refreshed"), owned_note(&inv, "refreshed")];
    inv.build_selected_gear(&fe.career, &fe.profile, env.loc, drop.as_ref());
    env.values.set_int(0x6C, inv.survival);
    env.values.set_int(0x64, inv.inventory_size);
    let price = inv.record(&fe.career, &fe.profile, env.loc, bought.0, bought.1).map(|r| r.cost);
    fe.equipment.inv = Some(inv);
    fe.note(before);
    for note in after {
        fe.note(note);
    }
    let stat = match kind {
        1 => Some("LifetimeSpentOnBoards"),
        2 => Some("LifetimeSpentOnGear"),
        3 => Some("LifetimeSpentOnBoost"),
        _ => None,
    };
    if let (Some(stat), Some(price)) = (stat, price) {
        let n = u32::try_from(price.max(0)).unwrap_or(u32::MAX);
        fe.profile.add_persistent_stat(stat, n);
        fe.note(format!("equipment: stat {stat} += {n} (0x82491280)"));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LevelPurchase {
    NoEntry,
    NotEnoughMoney,
    NotOffered,
    Buy { rider: u32, level: i64 },
}

pub(super) fn level_purchase(list: &[UiItem], index: usize, bank: i32) -> LevelPurchase {
    let Some(item) = list.get(index) else { return LevelPurchase::NoEntry };
    let buys_level = item.has(flag::LEVEL_PURCHASEABLE);
    if (!item.has(flag::AVAILABLE) || buys_level) && item.cost > i64::from(bank) {
        return LevelPurchase::NotEnoughMoney;
    }
    if !buys_level {
        return LevelPurchase::NotOffered;
    }
    LevelPurchase::Buy { rider: item.key, level: item.level }
}

fn purchase_rider_level(fe: &mut FrontEnd, env: &mut FeEnv, argument: i32) {
    let index = usize::try_from(argument).unwrap_or(usize::MAX);
    fe.equipment.rider_index = argument;
    fe.equipment.pending = Some((List::Items, index));
    match level_purchase(&fe.equipment.list, index, env.values.int(0x228)) {
        LevelPurchase::NoEntry => return,
        LevelPurchase::NotEnoughMoney => {
            env.nav_events.push("go_to_equip_not_enough_money_popup".into());
            return;
        }
        LevelPurchase::NotOffered => {}
        LevelPurchase::Buy { rider, level } => rider_purchase_level(fe, env, rider, level),
    }
    publish_list(fe, "dp_playable_characters");
}

/// The Custom Event Menus mod: a rider with a level bought but not yet reached (bought as the game sells them, its XP
/// still to earn) put at that level, as a level bought with the mod on is.
fn bought_levels_reached(fe: &mut FrontEnd) {
    if !ssx_core::rider_showcase::enabled() {
        return;
    }
    let (thresholds, prices) = (&fe.rules.level_thresholds, &fe.rules.level_prices);
    let mut notes = Vec::new();
    for (key, stats) in fe.profile.riders.iter_mut() {
        let bought = stats.max_purchased_level;
        let paid = usize::try_from(bought).ok().and_then(|i| prices.get(i)).is_some_and(|&p| p > 0);
        let now = ssx_assets::career::profile::update_rider_level(&mut stats.clone(), thresholds, prices).0;
        if paid && now < bought {
            let reached = ssx_assets::career::profile::set_rider_level(stats, bought, thresholds, prices);
            notes.push(format!("equipment: rider {key:#010x} put at its bought level {}", reached + 1));
        }
    }
    for n in notes {
        fe.note(n);
    }
}

fn rider_purchase_level(fe: &mut FrontEnd, env: &mut FeEnv, rider: u32, level: i64) {
    match fe.profile.purchase_rider_level(&fe.rules, rider, level) {
        Ok(price) => {
            if let Some(stats) = fe.profile.riders.get_mut(&rider) {
                ssx_assets::career::profile::update_rider_level(
                    stats,
                    &fe.rules.level_thresholds,
                    &fe.rules.level_prices,
                );
            }
            // (the Custom Event Menus mod: a bought level is reached at once (its XP given), so the next can be
            // bought straight after, up to the last)
            if ssx_core::rider_showcase::enabled()
                && let Some(stats) = fe.profile.riders.get_mut(&rider)
            {
                ssx_assets::career::profile::set_rider_level(
                    stats,
                    level,
                    &fe.rules.level_thresholds,
                    &fe.rules.level_prices,
                );
            }
            let bank = env.values.int(0x93).wrapping_sub(i32::try_from(price).unwrap_or(i32::MAX));
            env.values.set_int(0x93, bank);
            fe.note(format!("equipment: rider {rider:#010x} bought level {} for {price}", level.saturating_add(1)));
        }
        Err(e) => fe.note(format!("equipment: rider {rider:#010x} level {level} refused: {e:?}")),
    }
    let mut inv = inv(fe);
    inv.now_ms = (fe.time * 1000.0) as i64;
    let drop = drop_info(fe);
    let ctx = inv.store_context(&fe.career, &fe.profile, drop.as_ref());
    inv.build_lists(&fe.career, &fe.profile, env.loc, drop.as_ref(), &ctx, inventory::RIDER);
    env.values.set_int(0x64, inv.inventory_size);
    fe.equipment.inv = Some(inv);
}

pub fn nav_action(fe: &mut FrontEnd, env: &mut FeEnv, name: &str) -> bool {
    match name {
        "sell_confirm" => {
            if let Some((List::Inventory, i)) = fe.equipment.pending.take()
                && let Some(item) = fe.equipment.inventory.get(i).cloned()
            {
                let inv = inv(fe);
                let price = if item.src == 1 {
                    fe.profile.sell_generated(&inv.data, item.key)
                } else {
                    fe.profile.sell_fixed(&inv.catalog, &inv.data, item.key)
                };
                fe.equipment.inv = Some(inv);
                if let Some(p) = price {
                    let adj = env.values.int(0x93);
                    env.values.set_int(0x93, adj.wrapping_add(i32::try_from(p).unwrap_or(i32::MAX)));
                    fe.note(format!("equipment: sold {:#010x}/{} for {p}", item.key, item.src));
                }
                set_category(fe, env, fe.equipment.category);
                fe.equipment.inventory_sold();
                publish_list(fe, "dp_inventory_items");
                fe.publish.push("dp_inventory_items_index".into());
            }
        }
        "advance_to_game" => advance(fe, env),
        "advance_to_game_confirmed" => advance_confirmed(fe, env),
        "never_show_low_survive_again" => env.values.set_bool(LOW_SURVIVE_NEVER_SHOW, true),
        _ => return false,
    }
    true
}
