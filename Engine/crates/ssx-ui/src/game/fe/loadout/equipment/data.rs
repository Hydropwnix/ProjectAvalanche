use super::*;

pub(super) fn set(cx: &mut Ctx, o: &Value, k: &str, v: Value) {
    let _ = cx.set_member(o, k, v);
}

pub(super) fn common_fields(cx: &mut Ctx, o: &Value, u: &UiItem) {
    set(cx, o, "assetUrl", Value::str(&u.asset));
    set(cx, o, "secondaryAssetUrl", Value::str(&u.secondary));
    for (k, f) in [
        ("isAvailable", flag::AVAILABLE),
        ("isPurchaseable", flag::PURCHASEABLE),
        ("isLevelPurchaseable", flag::LEVEL_PURCHASEABLE),
        ("isSellable", flag::SELLABLE),
        ("showStats", flag::SHOW_STATS),
        ("isRecommended", flag::RECOMMENDED),
        ("isNew", flag::NEW),
        ("isEquipped", flag::EQUIPPED),
        ("isMystery", flag::MYSTERY),
    ] {
        set(cx, o, k, Value::Bool(u.has(f)));
    }
}

pub(super) fn stat_fields(cx: &mut Ctx, loc: &mut Localization, o: &Value, u: &UiItem) {
    let mut cols: [Vec<Value>; 5] = Default::default();
    for &(t, v) in &u.stats {
        if t < 0 {
            continue;
        }
        cols[0].push(Value::Number(t as f64));
        cols[1].push(Value::Number(v as f64));
        cols[2].push(Value::Number(v as f64));
        cols[3].push(Value::str(&loc.text(&format!("TXT_EQUIPSCREEN_STAT{t}_LONG"))));
        cols[4].push(Value::str(&loc.text(&format!("TXT_EQUIPSCREEN_STAT{t}_DESCRIPTION"))));
    }
    for (k, c) in ["itemStatTypes", "itemStatValues", "itemStatPercs", "itemStatNames", "itemStatDescription"]
        .into_iter()
        .zip(cols)
    {
        let a = Value::Object(cx.new_array(c));
        set(cx, o, k, a);
    }
}

pub(super) fn items_array(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization) -> Value {
    let riders = fe.equipment.category == 0;
    let mut out = Vec::new();
    for (i, u) in fe.equipment.list.iter().enumerate() {
        if !riders && i == 7 {
            break;
        }
        let o = Value::Object(cx.new_object());
        common_fields(cx, &o, u);
        if riders {
            set(cx, &o, "strItem", Value::str(&u.name));
            set(cx, &o, "level", Value::Number(u.level as f64));
            set(cx, &o, "levelProgress", Value::Number(u.level_progress as f64));
        }
        if u.has(flag::EMPTY) {
            set(cx, &o, "state", Value::Number(2.0));
            set(cx, &o, "strItem", Value::str(&u.name));
        } else {
            set(cx, &o, "state", Value::Number(0.0));
        }
        for (k, v) in [
            ("itemId", Value::Number(i as f64)),
            ("bFocusable", Value::Bool(true)),
            ("bEnabled", Value::Bool(true)),
            ("eventAdvance", Value::str("A_EquipmentItemSelected")),
            ("eventAdvanceParam", Value::Number(i as f64)),
            (
                "eventHighlight",
                Value::str(if riders { "A_EquipmentRiderHighlighted" } else { "A_EquipmentItemHighlighted" }),
            ),
            ("eventHighlightParam", Value::Number(i as f64)),
            ("eventBack", Value::str("A_EquipmentBackCategory")),
        ] {
            set(cx, &o, k, v);
        }
        out.push(o);
    }
    if !riders {
        let o = Value::Object(cx.new_object());
        let more = loc.text("TXT_INV_VIEWMORE");
        for (k, v) in [
            ("state", Value::Number(1.0)),
            ("strItem", Value::str(&more)),
            ("isAvailable", Value::Bool(true)),
            ("isPurchaseable", Value::Bool(false)),
            ("isAffordable", Value::Bool(true)),
            ("isNew", Value::Bool(false)),
            ("bFocusable", Value::Bool(true)),
            ("bEnabled", Value::Bool(true)),
            ("eventAdvance", Value::str("A_EquipmentViewMoreSelected")),
            ("eventHighlight", Value::str("A_EquipmentViewMoreHighlighted")),
            ("eventBack", Value::str("A_EquipmentBackCategory")),
        ] {
            set(cx, &o, k, v);
        }
        out.push(o);
    }
    Value::Object(cx.new_array(out))
}

pub(super) fn store_array(cx: &mut Ctx, loc: &mut Localization, list: &[UiItem], store: bool) -> Value {
    let mut out = Vec::new();
    for (i, u) in list.iter().enumerate() {
        if store && i > 3 {
            break;
        }
        let o = Value::Object(cx.new_object());
        common_fields(cx, &o, u);
        let price = loc.currency(u.cost, true);
        let (adv, hi) = if store {
            ("A_EquipmentStoreItemSelected", "A_EquipmentStoreItemHighlighted")
        } else {
            ("A_EquipmentInventoryItemSelected", "A_EquipmentInventoryItemHighlighted")
        };
        for (k, v) in [
            ("price", Value::str(&price)),
            ("state", Value::Number(if u.has(flag::EMPTY) { 2.0 } else { 0.0 })),
            ("strItem", Value::str(&u.name)),
            ("itemId", Value::Number(i as f64)),
            ("bFocusable", Value::Bool(true)),
            ("bEnabled", Value::Bool(true)),
            ("eventAdvance", Value::str(adv)),
            ("eventAdvanceParam", Value::Number(i as f64)),
            ("eventHighlight", Value::str(hi)),
            ("eventHighlightParam", Value::Number(i as f64)),
            ("eventBack", Value::str(if store { "A_EquipmentBackCategory" } else { "A_EquipmentInventoryItemBack" })),
        ] {
            set(cx, &o, k, v);
        }
        out.push(o);
    }
    Value::Object(cx.new_array(out))
}

pub(super) fn gear_info(cx: &mut Ctx, loc: &mut Localization, u: &UiItem) -> Value {
    let o = Value::Object(cx.new_object());
    let price = loc.currency(u.cost, true);
    let level = loc.format("TXT_EQUIP_LEVEL", &[&u.level.to_string()]);
    let help = loc.text(&format!("TXT_EQUIPSCREEN_GEAR{}_HELP", u.slot));
    let tag = loc.format("TXT_PROFILE_GAMERTAG", &[&u.gamertag]);
    for (k, v) in [
        ("state", Value::Number(if u.has(flag::EMPTY) { 2.0 } else { 0.0 })),
        ("strItem", Value::str(&u.name)),
        ("description", Value::str(&u.description)),
        ("assetUrl", Value::str(&u.asset)),
        ("secondaryAssetUrl", Value::str(&u.secondary)),
        ("isAvailable", Value::Bool(u.has(flag::AVAILABLE))),
        ("isPurchaseable", Value::Bool(u.has(flag::PURCHASEABLE))),
        ("isLevelPurchaseable", Value::Bool(u.has(flag::LEVEL_PURCHASEABLE))),
        ("isSellable", Value::Bool(u.has(flag::SELLABLE))),
        ("price", Value::str(&price)),
        ("level", Value::Number(u.level as f64)),
        ("levelProgress", Value::Number(u.level_progress as f64)),
        ("levelString", Value::str(&level)),
        ("quality", Value::Number(u.quality as f64)),
        ("qualityRating", Value::Number(u.rarity as f64)),
        ("showStats", Value::Bool(u.has(flag::SHOW_STATS))),
        ("showLevel", Value::Bool(u.has(flag::SHOW_LEVEL))),
        ("isRecommended", Value::Bool(u.has(flag::RECOMMENDED))),
        ("recommendText", Value::str(&u.recommend)),
        ("gearItemType", Value::Number(u.slot as f64)),
        ("strHelp", Value::str(&help)),
        ("gamerTagCallout", Value::str(&tag)),
        ("isNew", Value::Bool(u.has(flag::NEW))),
        ("isEquipped", Value::Bool(u.has(flag::EQUIPPED))),
        ("isMystery", Value::Bool(u.has(flag::MYSTERY))),
    ] {
        set(cx, &o, k, v);
    }
    stat_fields(cx, loc, &o, u);
    o
}

pub(super) fn offer_clock(ms: i64) -> String {
    let ms = ms.max(0) as u64;
    let (h, m, s) = (ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60);
    if h != 0 { format!("{h:02}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, values: &UiValues, loc: &mut Localization, dp: &str) -> Option<Value> {
    let eq = &fe.equipment;
    match dp {
        "dp_screen_state" => Some(Value::Number(f64::from(eq.state))),
        "dp_online_status" => Some(Value::Number(f64::from(eq.online))),
        "dp_playable_characters_index"
        | "dp_equipment_items_index"
        | "dp_store_items_index"
        | "dp_inventory_items_index" => Some(Value::Number(match eq.state {
            state::RIDER => f64::from(eq.rider_index),
            state::INVENTORY => f64::from(eq.inv_focus),
            _ => 0.0,
        })),
        "dp_equipment_categories_index" => Some(Value::Number(f64::from(eq.category - 1))),
        "dp_playable_characters" | "dp_equipment_items" => Some(items_array(fe, cx, loc)),
        "dp_store_items" => Some(store_array(cx, loc, &eq.store, true)),
        "dp_inventory_items" => Some(store_array(cx, loc, &eq.inventory, false)),
        "dp_selected_rider" => {
            let o = Value::Object(cx.new_object());
            if let Some(u) = eq.inv.as_ref().and_then(|i| i.selected_gear.first()) {
                set(cx, &o, "strItem", Value::str(&u.name));
                set(cx, &o, "assetUrl", Value::str(&u.asset));
                set(cx, &o, "secondaryAssetUrl", Value::str(&u.secondary));
                set(cx, &o, "level", Value::Number(u.level as f64));
                set(cx, &o, "levelProgress", Value::Number(u.level_progress as f64));
            }
            set(cx, &o, "isAvailable", Value::Bool(true));
            Some(o)
        }
        "dp_equipment_categories" => {
            let mut out = Vec::new();
            let Some(inv) = eq.inv.as_ref() else { return Some(Value::Object(cx.new_array(out))) };
            let mut gear = inv.selected_gear.iter().skip(1);
            for (k, &n) in inv.category_slots.iter().enumerate() {
                if n <= 0 {
                    continue;
                }
                let cat = k as i32 + 1;
                let o = Value::Object(cx.new_object());
                set(cx, &o, "isAvailable", Value::Bool(cat != 5 || eq.online != 0));
                set(cx, &o, "numSlots", Value::Number(n as f64));
                if cat == eq.category {
                    set(cx, &o, "selectedSlot", Value::Number(f64::from(eq.slot)));
                }
                let name = loc.text(&format!("TXT_EQUIP_CAT{cat}_NAME"));
                let desc = loc.text(&format!("TXT_EQUIP_CAT{cat}_DESC"));
                set(cx, &o, "strItem", Value::str(&name));
                set(cx, &o, "description", Value::str(&desc));
                let mut list = Vec::new();
                for _ in 0..n {
                    let Some(u) = gear.next() else { break };
                    let g = Value::Object(cx.new_object());
                    common_fields(cx, &g, u);
                    stat_fields(cx, loc, &g, u);
                    list.push(g);
                }
                let a = Value::Object(cx.new_array(list));
                set(cx, &o, "gearList", a);
                out.push(o);
            }
            Some(Value::Object(cx.new_array(out)))
        }
        "dp_equipment_score" => {
            let o = Value::Object(cx.new_object());
            let label = loc.text("TXT_EQUIP_SCORE_LABEL");
            let s = eq.score;
            let text = if s < 0 { loc.text("TXT_N_A") } else { loc.percent(i64::from(s)) };
            set(cx, &o, "scoreLabel", Value::str(&label));
            set(cx, &o, "scoreValue", Value::Number(f64::from(s)));
            set(cx, &o, "scoreStr", Value::str(&text));
            set(cx, &o, "scoreAvailable", Value::Bool(s >= 0));
            set(cx, &o, "scoreWarning", Value::Bool((0..30).contains(&s)));
            Some(o)
        }
        "dp_limited_time_offer" => {
            let special = eq.store.iter().any(|u| u.has(flag::SPECIAL));
            let left = eq.inv.as_ref().and_then(|i| {
                i.store.stock.iter().flatten().filter(|o| o.special).map(|o| o.expires_ms - i.now_ms).max()
            });
            Some(Value::str(&match (special, left) {
                (true, Some(ms)) => offer_clock(ms),
                _ => String::new(),
            }))
        }
        "dp_equipment_purchase_result" => Some(Value::Bool(eq.purchase_result)),
        "dp_selected_gear_info" => Some(match eq.gear_info.as_ref() {
            Some(u) => gear_info(cx, loc, u),
            None => Value::Object(cx.new_object()),
        }),
        "dp_selected_rider_info" => {
            let o = Value::Object(cx.new_object());
            if let Some(inv) = eq.inv.as_ref() {
                let b = &inv.bio;
                set(cx, &o, "gearAsset", Value::str(&b.gear_asset));
                set(cx, &o, "gearName", Value::str(&b.gear_name));
                let title = loc.text("TXT_SPECIAL_GEAR_TITLE");
                set(cx, &o, "gearTitle", Value::str(&title));
                let perks = loc.text("TXT_CHAR_PERKS_TITLE");
                set(cx, &o, "perkTitle", Value::str(&perks));
                let names: Vec<Value> = b.perks.iter().filter(|p| p.1 > 0).map(|p| Value::str(&p.0)).collect();
                let vals: Vec<Value> = b.perks.iter().filter(|p| p.1 > 0).map(|p| Value::Number(p.1 as f64)).collect();
                let n = Value::Object(cx.new_array(names));
                let v = Value::Object(cx.new_array(vals));
                set(cx, &o, "perkNames", n);
                set(cx, &o, "perkValues", v);
            }
            Some(o)
        }
        _ => {
            let _ = values;
            None
        }
    }
}
