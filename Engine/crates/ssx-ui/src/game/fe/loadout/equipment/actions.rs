use super::*;

pub(super) fn refresh_inventory_view(fe: &mut FrontEnd) {
    let all = fe.equipment.inv.as_ref().map(|i| i.inventory_list.clone()).unwrap_or_default();
    let first = fe.equipment.inv_first.max(0) as usize;
    let max = if fe.equipment.inv_max > 0 { fe.equipment.inv_max as usize } else { all.len() };
    fe.equipment.inventory = all.into_iter().skip(first).take(max).collect();
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: &ActionParam) {
    let index = param.number.max(0.0) as usize;
    match action {
        "A_GameAdvanceFlow" => advance(fe, env),
        "A_EquipmentBackCategory" => match fe.equipment.state {
            state::RIDER => {
                fe.note("equipment: back (FE manager +0x88(4), save)");
                save_state_reset(env);
                crate::game::fe::globe::back_from_equipment(fe, env);
            }
            state::GEAR => {
                let (k, slot) = (i64::from(fe.equipment.category - 1), fe.equipment.slot.max(0) as usize);
                let mut inv = inv(fe);
                inv.close_category(&fe.career, &fe.profile, k, slot);
                fe.equipment.inv = Some(inv);
                set_category(fe, env, fe.equipment.category);
                fe.equipment.state = state::READY;
                fe.equipment.score = env.values.int(0x6C);
                fe.publish.extend(
                    [
                        "dp_screen_state",
                        "dp_selected_rider",
                        "dp_equipment_categories",
                        "dp_equipment_categories_index",
                        "dp_equipment_score",
                    ]
                    .map(String::from),
                );
            }
            _ => {}
        },
        "A_EquipmentCategoryHighlight" if matches!(fe.equipment.state, state::READY | state::READY_CATEGORY) => {
            let (c, s) = (param.field("category"), param.field("slot"));
            fe.equipment.category = if c.is_nan() { fe.equipment.category } else { c as i32 + 1 };
            fe.equipment.slot = if s.is_nan() { 0 } else { s as i32 };
            set_category(fe, env, fe.equipment.category);
            fill_list(fe);
            let k = fe.equipment.category - 1;
            let gear = fe.equipment.inv.as_ref().and_then(|i| {
                let before: usize = i.category_slots.iter().take(k.max(0) as usize).map(|&n| n.max(0) as usize).sum();
                i.selected_gear.get(1 + before + fe.equipment.slot.max(0) as usize).cloned()
            });
            fe.equipment.gear_info = gear;
            fe.publish.extend(
                ["dp_equipment_items", "dp_store_items", "dp_selected_gear_info", "dp_selected_rider"]
                    .map(String::from),
            );
        }
        "A_EquipmentCategorySelect" => {
            fe.equipment.state = state::GEAR;
            let (k, slot) = (i64::from(fe.equipment.category - 1), fe.equipment.slot.max(0) as usize);
            let mut inv = inv(fe);
            inv.open_category(&fe.career, &fe.profile, k, slot);
            let survival = inv.survival;
            fe.equipment.inv = Some(inv);
            set_category(fe, env, fe.equipment.category);
            if let Some(inv) = fe.equipment.inv.as_mut() {
                inv.survival = survival;
            }
            env.values.set_int(0x6C, survival);
            fill_list(fe);
            fe.publish.push("dp_screen_state".into());
        }
        "A_EquipmentBackReady" => {
            if fe.equipment.rider_count == 1 {
                fe.note("equipment: back (one rider: save and leave)");
                save_state_reset(env);
                crate::game::fe::globe::back_from_equipment(fe, env);
                return;
            }
            fe.equipment.slot = 0;
            fe.equipment.category = 0;
            set_category(fe, env, 0);
            fill_list(fe);
            fe.equipment.state = state::RIDER;
            fe.publish.extend(
                ["dp_playable_characters", "dp_playable_characters_index", "dp_screen_state"].map(String::from),
            );
        }
        "A_EquipmentItemSelected" => {
            let Some(item) = fe.equipment.list.get(index).cloned() else { return };
            if fe.equipment.category != 0 {
                if item.has(flag::AVAILABLE) {
                    select(fe, env, List::Items, index);
                }
                return;
            }
            fe.equipment.rider_index = index as i32;
            fe.equipment.last_rider = item.key;
            env.values.set_int(0x65, item.key as i32);
            if !item.has(flag::AVAILABLE) {
                fe.equipment.pending = Some((List::Items, index));
                purchase(fe, env, List::Items, index);
                return;
            }
            select(fe, env, List::Items, index);
        }
        "A_EquipmentItemHighlighted" | "A_EquipmentRiderHighlighted" => {
            if action == "A_EquipmentRiderHighlighted" && fe.equipment.category != 0 {
                return;
            }
            let Some(item) = fe.equipment.list.get(index).cloned() else { return };
            highlight(fe, env, &item);
            fe.equipment.score = env.values.int(0x6C);
            fe.publish.extend(
                ["dp_selected_gear_info", "dp_selected_rider_info", "dp_equipment_score", "dp_selected_rider"]
                    .map(String::from),
            );
        }
        "A_EquipmentOptimizeGear" if matches!(fe.equipment.state, state::READY | state::READY_CATEGORY) => {
            let mut inv = inv(fe);
            let drop = drop_info(fe);
            inv.optimize(&fe.career, &fe.profile, env.loc, drop.as_ref());
            inv.save_loadout(&fe.career, &mut fe.profile);
            env.values.set_int(0x6C, inv.survival);
            fe.equipment.inv = Some(inv);
            fe.equipment.score = env.values.int(0x6C);
            fe.publish.extend(["dp_equipment_score", "dp_equipment_categories"].map(String::from));
        }
        "A_EquipmentStoreItemSelected" => {
            fe.equipment.pending = Some((List::Store, index));
            purchase(fe, env, List::Store, index);
        }
        "A_EquipmentStoreItemHighlighted" => {
            let Some(item) = fe.equipment.store.get(index).cloned() else { return };
            highlight(fe, env, &item);
            fe.equipment.score = env.values.int(0x6C);
            fe.publish.extend(["dp_equipment_score", "dp_selected_gear_info", "dp_selected_rider"].map(String::from));
        }
        "A_EquipmentRefreshStore" => fe.publish.extend(["dp_equipment_items", "dp_store_items"].map(String::from)),
        "A_EquipmentViewMoreSelected" => {
            fe.equipment.state = state::INVENTORY;
            save_state(fe, env);
            fe.go_to_screen(env, 0x31);
        }
        "A_EquipmentViewMoreHighlighted" if fe.equipment.state == state::GEAR => {
            fe.equipment.gear_info = None;
            fe.publish.extend(["dp_equipment_score", "dp_selected_gear_info"].map(String::from));
        }
        "A_EquipmentInventorySetup" => {
            fe.equipment.inv_max = param.field("maxData").max(0.0) as i32;
            fe.equipment.inv_unselectable = param.field("unselectableData").max(0.0) as i32;
            fe.equipment.inv_first = 0;
            fe.equipment.inv_focus = 0;
        }
        "A_EquipmentInventoryRequestData" => {
            let d = param.number as i32;
            let eq = &mut fe.equipment;
            if d < 1 {
                if eq.inv_first > 0 {
                    eq.inv_first = (eq.inv_first + d).max(0);
                } else {
                    eq.inv_focus = (eq.inv_focus + d).max(0);
                }
            } else if eq.inv_unselectable <= eq.inv_focus {
                eq.inv_first += d;
            } else {
                eq.inv_focus += d;
            }
            refresh_inventory_view(fe);
            fe.publish.extend(["dp_inventory_items", "dp_inventory_items_index"].map(String::from));
        }
        "A_EquipmentInventoryItemSelected" => select(fe, env, List::Inventory, index),
        "A_EquipmentInventoryItemHighlighted" => {
            let Some(item) = fe.equipment.inventory.get(index).cloned() else { return };
            highlight(fe, env, &item);
            fe.publish.push("dp_selected_gear_info".into());
        }
        "A_EquipmentInventoryItemBack" => {
            fe.equipment.state = state::GEAR;
            save_state(fe, env);
            fe.go_to_screen(env, screens::id::EQUIPMENT);
        }
        "A_EquipmentSellGear" => {
            fe.equipment.pending = Some((List::Inventory, index));
            env.nav_events.push("equip_sell_confirm_popup".into());
        }
        _ => {}
    }
}
