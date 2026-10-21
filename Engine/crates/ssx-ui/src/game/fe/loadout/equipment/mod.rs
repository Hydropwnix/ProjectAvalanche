use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::inventory::{self, DropInfo, Inventory, UiItem, flag};
use crate::game::fe::{ActionParam, FeEnv, FrontEnd, screens};
use crate::game::loc::Localization;
use crate::game::values::UiValues;

mod actions;
mod data;

pub use actions::*;
pub use data::*;

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
    pub pending: Option<(List, usize)>,
    pub gear_info: Option<UiItem>,
    pub purchase_result: bool,
    pub online: i32,
    pub rider: u32,
    pub inv: Option<Box<Inventory>>,
}

impl Equipment {
    pub fn rider_name(&self, fe: &FrontEnd) -> String {
        let forced = fe.career.event(self.drop).and_then(|e| e.forced.get(5).copied()).filter(|&k| k != 0);
        let luri = fe.profile.attribs.get(&u32::from_be_bytes(*b"LURI")).map(|&v| v as u32);
        let key = [Some(self.rider).filter(|&k| k != 0), forced, luri].into_iter().flatten().next().unwrap_or(0);
        fe.career.riders.iter().find(|r| r.key == key).map_or_else(|| "zoe".to_string(), |r| r.name.clone())
    }
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

fn set_category(fe: &mut FrontEnd, env: &mut FeEnv, category: i32) {
    let kind = if category == 0 { inventory::RIDER } else { i64::from(category - 1) };
    let mut inv = inv(fe);
    let drop = drop_info(fe);
    let ctx = inv.store_context(&fe.career, &fe.profile, drop.as_ref());
    inv.build_lists(&fe.career, &fe.profile, env.loc, drop.as_ref(), &ctx, kind);
    inv.build_selected_gear(&fe.career, &fe.profile, env.loc, drop.as_ref());
    env.values.set_int(0x6C, inv.survival);
    fe.equipment.inv = Some(inv);
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
    fe.equipment.inv = Some(inv);
    set_category(fe, env, 0);
}

pub fn activate(fe: &mut FrontEnd, env: &mut FeEnv) {
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
    eq.rider_count = env.values.int(0x60);
    for id in [0x5F, 0x61, 0x62, 0x60] {
        env.values.set_int(id, 0);
    }
    env.values.set_bool(0x6A, false);
    eq.online = env.values.int(0x22F);
    if eq.state == 0 {
        fill_list(fe);
        fe.publish.extend(
            [
                "dp_playable_characters",
                "dp_playable_characters_index",
                "dp_equipment_items_index",
                "dp_store_items_index",
            ]
            .map(String::from),
        );
        fe.publish.extend(["dp_equipment_categories", "dp_equipment_categories_index"].map(String::from));
        if fe.equipment.rider_count == 1 {
            select(fe, env, List::Items, 0);
            return;
        }
        fe.equipment.state = state::RIDER;
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

fn fill_list(fe: &mut FrontEnd) {
    let Some((list, store)) = fe.equipment.inv.as_ref().map(|i| (i.owned_list.clone(), i.store_list.clone())) else {
        return;
    };
    let eq = &mut fe.equipment;
    if eq.category == 0 {
        eq.rider_index = list.iter().position(|u| u.key == eq.last_rider).unwrap_or(0) as i32;
        eq.rider_count = list.len() as i32;
    }
    eq.list = list;
    eq.store = store;
}

fn equip(fe: &mut FrontEnd, env: &mut FeEnv, item: &UiItem) {
    let mut inv = inv(fe);
    let kind = if fe.equipment.category == 0 { inventory::RIDER } else { i64::from(fe.equipment.category - 1) };
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

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv) {
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
    if !env.values.bool(0x9E) && fe.equipment.score < 30 {
        let score = env.loc.percent(i64::from(fe.equipment.score));
        let body = env.loc.format("TXT_EQUIP_LOW_SURVIVAL_BODY", &[&score]);
        env.values.set_str(0x99, body);
        env.nav_events.push("go_to_low_survive_popup".into());
        return;
    }
    advance_confirmed(fe, env);
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

fn highlight(fe: &mut FrontEnd, env: &mut FeEnv, item: &UiItem) {
    let mut inv = inv(fe);
    let kind = if fe.equipment.category == 0 { inventory::RIDER } else { i64::from(fe.equipment.category - 1) };
    if (0..=inventory::KINDS as i64).contains(&kind) {
        inv.new_items[kind as usize].retain(|&e| e != (item.key, item.src));
    }
    let drop = drop_info(fe);
    if kind == inventory::RIDER {
        inv.rider_bio(&fe.career, &fe.profile, env.loc, item.key);
    } else if (0..4).contains(&kind) {
        let k = kind as usize;
        let slot = fe.equipment.slot.max(0) as usize;
        let saved = inv.equipped[k].get(slot).copied();
        if slot < inv.equipped[k].len() {
            inv.equipped[k][slot] = (item.key, item.src);
        }
        let score = inv.survival_score(&fe.career, &fe.profile, drop.as_ref());
        if let Some(s) = saved {
            inv.equipped[k][slot] = s;
        }
        env.values.set_int(0x6C, score);
    }
    fe.equipment.inv = Some(inv);
    fe.equipment.gear_info = Some(item.clone());
}

fn purchase(fe: &mut FrontEnd, env: &mut FeEnv, list: List, index: usize) {
    let entries = if list == List::Store { &fe.equipment.store } else { &fe.equipment.list };
    let Some(item) = entries.get(index).cloned() else { return };
    if item.cost > i64::from(env.values.int(0x228)) {
        env.nav_events.push("go_to_equip_not_enough_money_popup".into());
        return;
    }
    if fe.equipment.state == state::GEAR && fe.profile.generated.len() + fe.profile.owned_items.len() >= 450 {
        env.nav_events.push("equip_open_inventory_full".into());
        fe.equipment.purchase_result = false;
        fe.publish.push("dp_equipment_purchase_result".into());
        return;
    }
    let mut inv = inv(fe);
    let kind = if fe.equipment.category == 0 { inventory::RIDER } else { i64::from(fe.equipment.category - 1) };
    let paid = if item.src == 1 {
        let info = drop_info(fe);
        let ctx = inv.store_context(&fe.career, &fe.profile, info.as_ref());
        let data = inv.data.clone();
        let now = inv.now_ms;
        fe.profile
            .buy_offer(&mut inv.store, &data, &ctx, kind.max(0) as usize, item.key, (fe.time as u32).max(1), now)
            .map(|id| {
                inv.new_items[kind as usize].push((id, 1));
                item.cost
            })
    } else {
        fe.profile.buy_fixed(&inv.catalog, item.key).inspect(|_| {
            if (0..=inventory::KINDS as i64).contains(&kind) {
                inv.new_items[kind as usize].push((item.key, 0));
            }
        })
    };
    fe.equipment.inv = Some(inv);
    match paid {
        Ok(cost) => {
            fe.note(format!("equipment: bought {:#010x}/{} for {cost}", item.key, item.src));
            let adj = env.values.int(0x93);
            env.values.set_int(0x93, adj - cost as i32);
            fe.equipment.purchase_result = true;
            set_category(fe, env, fe.equipment.category);
            fill_list(fe);
            fe.publish.extend(
                ["dp_equipment_purchase_result", "dp_equipment_items", "dp_store_items", "dp_playable_characters"]
                    .map(String::from),
            );
        }
        Err(e) => fe.note(format!("equipment: purchase refused: {e:?}")),
    }
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
                    env.values.set_int(0x93, adj + p as i32);
                    fe.note(format!("equipment: sold {:#010x}/{} for {p}", item.key, item.src));
                }
                set_category(fe, env, fe.equipment.category);
                refresh_inventory_view(fe);
                fe.publish.extend(["dp_inventory_items", "dp_inventory_items_index"].map(String::from));
            }
        }
        "advance_to_game" => advance(fe, env),
        "advance_to_game_confirmed" => advance_confirmed(fe, env),
        "never_show_low_survive_again" => env.values.set_bool(0x9E, true),
        _ => return false,
    }
    true
}
