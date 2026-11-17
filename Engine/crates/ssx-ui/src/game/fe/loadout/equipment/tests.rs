use ssx_assets::career::profile::GeneratedItem;
use ssx_assets::career::{CareerDb, Profile, Rider};

use super::*;
use crate::game::loc::{Language, Localization};

fn item(key: u32, flags: u32) -> UiItem {
    UiItem { key, flags, name: format!("item {key:#x}"), asset: format!("img://{key:x}"), ..Default::default() }
}

fn provider(n: u32) -> Equipment {
    let mut inv = Inventory::new(&CareerDb::default());
    let list: Vec<UiItem> = (0..n).map(|i| item(0x100 + i, flag::AVAILABLE)).collect();
    (inv.owned_list, inv.store_list, inv.inventory_list) = (list.clone(), list.clone(), list);
    Equipment { inv: Some(Box::new(inv)), ..Default::default() }
}

fn keys(list: &[UiItem]) -> Vec<u32> {
    list.iter().map(|u| u.key).collect()
}

#[test]
fn the_inventory_window_is_built_when_published_without_a_request() {
    let mut eq = provider(40);
    eq.state = state::INVENTORY;
    eq.inventory_setup(25.0, 20.0);
    eq.rebuild("dp_inventory_items");
    assert_eq!(keys(&eq.inventory), (0x100..0x119).collect::<Vec<_>>(), "the window's first 25");
    assert_eq!(eq.inv_index, 0);
    let mut eq = provider(3);
    eq.state = state::INVENTORY;
    eq.inventory_setup(25.0, 20.0);
    eq.rebuild("dp_inventory_items");
    assert_eq!(keys(&eq.inventory), [0x100, 0x101, 0x102]);
}

#[test]
fn the_lists_are_built_only_in_their_states() {
    let mut eq = provider(10);
    eq.state = state::INVENTORY;
    eq.category = 2;
    eq.rebuild("dp_equipment_items");
    eq.rebuild("dp_store_items");
    assert!(eq.list.is_empty() && eq.store.is_empty(), "0x825F9480 / 0x825F9520 build nothing in state 3");
    eq.state = state::GEAR;
    eq.inventory_setup(25.0, 20.0);
    eq.rebuild("dp_inventory_items");
    assert!(eq.inventory.is_empty(), "0x825F95C0 builds only in state 3");
    eq.rebuild("dp_equipment_items");
    eq.rebuild("dp_store_items");
    assert_eq!(eq.list.len(), 7, "a category's first 7");
    assert_eq!(eq.store.len(), 4, "the store's first 4");
}

#[test]
fn the_rider_list_is_whole_with_the_last_riders_place() {
    let mut eq = provider(10);
    (eq.state, eq.category, eq.last_rider) = (state::RIDER, 0, 0x105);
    eq.rebuild("dp_playable_characters");
    assert_eq!((eq.list.len(), eq.rider_count, eq.rider_index), (10, 10, 5));
    eq.last_rider = 0xDEAD;
    eq.rebuild("dp_playable_characters");
    assert_eq!(eq.rider_index, 0, "a rider not listed: 0");
    if let Some(inv) = eq.inv.as_mut() {
        inv.owned_list.truncate(2);
    }
    eq.rebuild("dp_playable_characters");
    assert_eq!((keys(&eq.list), eq.rider_count), (vec![0x100, 0x101], 2));
}

#[test]
fn the_requests_move_the_focus_then_the_window() {
    let mut eq = provider(60);
    eq.state = state::INVENTORY;
    eq.inventory_setup(25.0, 10.0);
    for _ in 0..2 {
        eq.inventory_step(5.0);
    }
    assert_eq!((eq.inv_first, eq.inv_focus, eq.inv_index, eq.inv_step), (0, 10, 10, 5), "the focus to the band");
    eq.inventory_step(5.0);
    assert_eq!((eq.inv_first, eq.inv_focus), (5, 10), "then the window");
    eq.rebuild("dp_inventory_items");
    assert_eq!(eq.inventory.first().map(|u| u.key), Some(0x105));
    eq.inventory_step(-5.0);
    assert_eq!((eq.inv_first, eq.inv_focus, eq.inv_step), (0, 10, 5), "up: the window while not at the top");
    eq.inventory_step(-5.0);
    assert_eq!((eq.inv_first, eq.inv_focus), (0, 5), "then the focus");
}

#[test]
fn hostile_requests_build_no_entry_past_the_list() {
    assert_eq!((actions::fctiwz(f64::NAN), actions::fctiwz(1e300), actions::fctiwz(-2.7)), (i32::MIN, i32::MAX, -2));
    let category = |c: f64| actions::fctiwz(c).wrapping_add(1);
    assert_eq!([category(1.0), category(f64::NAN), category(1e300)], [2, i32::MIN + 1, i32::MIN]);
    assert_eq!([kind_of(0), kind_of(2), kind_of(i32::MIN)], [5, 1, i64::from(i32::MIN) - 1]);
    for (max, steps) in [
        (25.0, vec![f64::NAN, 1.0]),
        (f64::INFINITY, vec![1e300, 1e300]),
        (25.0, vec![-1e300, 3.0, 3.0]),
        (-4.0, vec![1.0]),
        (25.0, vec![2.0, 2.0, 1e300, 1e300]),
    ] {
        let mut eq = provider(30);
        eq.state = state::INVENTORY;
        eq.inventory_setup(max, 0.0);
        for s in steps {
            eq.inventory_step(s);
        }
        eq.rebuild("dp_inventory_items");
        let first = usize::try_from(eq.inv_first).ok();
        assert!(eq.inventory.len() <= 30, "{max}: {} entries", eq.inventory.len());
        if let (Some(f), Some(u)) = (first, eq.inventory.first()) {
            assert_eq!(u.key, 0x100 + u32::try_from(f).unwrap_or(u32::MAX), "the window starts at its first item");
        }
        eq.inventory_sold();
    }
}

#[test]
fn a_sale_near_the_end_moves_the_window_back_by_the_last_step() {
    let mut eq = provider(30);
    eq.state = state::INVENTORY;
    eq.inventory_setup(25.0, 0.0);
    eq.inventory_step(5.0);
    eq.rebuild("dp_inventory_items");
    assert_eq!((eq.inv_first, eq.inventory.len()), (5, 25));
    eq.inventory_sold();
    assert_eq!(eq.inv_first, 0, "25 shown ≤ focus 0 + 25: back by 5");
    eq.inventory_setup(25.0, 9.0);
    eq.inventory_step(3.0);
    eq.rebuild("dp_inventory_items");
    eq.inventory_sold();
    assert_eq!((eq.inv_first, eq.inv_focus, eq.inv_index), (0, 0, 0));
}

#[test]
fn the_view_more_info_keeps_the_entrys_other_fields() {
    let mut loc = Localization::empty(Language::English);
    for (k, t) in [("TXT_INV_VIEW_MORE_DESC", "%1s: %2d of %3d"), ("TXT_INV_VIEW_MORE_CAT2", "Suits")] {
        assert!(loc.add_port_string(k, t).is_ok());
    }
    let mut values = UiValues::new();
    values.set_int(0x64, 37);
    let mut info = GearInfo::default();
    assert!(info.get("description").is_none(), "a new view's object is empty");
    let mut u = item(0x200, flag::AVAILABLE | flag::PURCHASEABLE | flag::SHOW_STATS | flag::NEW);
    u.level = 3;
    fill_entry(&mut info, &mut loc, &mut values, &u);
    fill_view_more(&mut info, &mut loc, &values, 2);
    let s = |k: &str| match info.get(k) {
        Some(Field::Str(s)) => s.clone(),
        other => format!("{other:?}"),
    };
    assert_eq!(info.get("state"), Some(&Field::Num(1.0)));
    assert_eq!(
        (s("strItem"), s("description"), s("price")),
        ("TXT_INV_VIEWMORE".into(), "Suits: 37 of 450".into(), "".into())
    );
    for (k, v) in [("isAvailable", true), ("isPurchaseable", false), ("isAffordable", true), ("isNew", false)] {
        assert_eq!(info.get(k), Some(&Field::Bool(v)), "{k}");
    }
    assert_eq!(info.get("showStats"), Some(&Field::Bool(false)));
    assert_eq!((s("assetUrl"), info.get("level")), ("img://200".into(), Some(&Field::Num(3.0))));
}

fn rider(key: u32, locked: bool) -> Rider {
    Rider {
        key,
        name: String::new(),
        title: String::new(),
        display_name: String::new(),
        cost: 0,
        locked,
        default: false,
        unk_940b9b22: 0,
        stat_bonuses: Vec::new(),
        portrait: String::new(),
        description: String::new(),
        round_icon: String::new(),
        icon: String::new(),
        content: None,
    }
}

#[test]
fn the_inventory_size_counts_the_items_without_the_owned_riders() {
    let mut db = CareerDb { riders: vec![rider(1, false), rider(2, true), rider(3, true)], ..Default::default() };
    let mut profile = Profile::default();
    profile.owned_items.extend([1, 2, 0x10, 0x11, 0x12]);
    profile.generated.insert(0x900, GeneratedItem::default());
    assert_eq!(Inventory::inventory_size(&db, &profile), 4);
    profile.owned_items.clear();
    profile.generated.clear();
    db.riders.push(rider(4, false));
    assert_eq!(Inventory::inventory_size(&db, &profile), 0, "never below 0");
}

#[test]
fn a_looked_at_item_is_no_longer_new_in_both_lists() {
    let mut inv = Inventory::new(&CareerDb::default());
    let new = item(0x300, flag::NEW | flag::AVAILABLE);
    (inv.owned_list, inv.inventory_list) = (vec![new.clone(), new.clone()], vec![item(0x301, flag::NEW), new]);
    inv.new_items[1] = vec![(0x300, 0), (0x301, 0)];
    inv.seen(1, (0x300, 0));
    assert_eq!(inv.new_items[1], [(0x301, 0)]);
    assert!(!inv.owned_list[0].has(flag::NEW) && inv.owned_list[1].has(flag::NEW), "the first match only");
    assert!(inv.inventory_list[1].flags == flag::AVAILABLE && inv.inventory_list[0].has(flag::NEW));
    inv.seen(-1, (0x301, 0));
    inv.seen(i64::MAX, (0x301, 0));
    assert!(inv.inventory_list[0].has(flag::NEW), "no kind: nothing cleared");
}

fn gear_template(key: u32, slot: i64) -> ssx_assets::career::data::StoreTemplate {
    ssx_assets::career::data::StoreTemplate {
        key,
        title: String::new(),
        slot,
        kind: 2,
        stats: vec![5 + 3 * (slot - 1)],
        price_scale: 1.0,
        charlock_chance: 0,
        bonus_chance: 0,
        min_level: 0,
        max_level: 100,
        min_rarity: 0,
        max_rarity: 5,
        fixed_points: -1,
        bonus_stats: Vec::new(),
        descriptions: Vec::new(),
        members: Vec::new(),
    }
}

#[test]
fn the_gear_store_offers_the_drops_main_gear_even_when_worn() {
    use ssx_assets::career::store::{Store, StoreContext, StoreData};
    let db = CareerDb::default();
    let profile = Profile::default();
    let mut loc = Localization::empty(Language::English);
    let mut group = gear_template(100, -1);
    group.members = vec![1, 3];
    let mut data = StoreData {
        templates: [(1, gear_template(1, 1)), (3, gear_template(3, 3)), (100, group)].into(),
        ..Default::default()
    };
    data.tables.groups = vec![0, 0, 100, 0, 0];
    let mut inv = Inventory::new(&db);
    (inv.data, inv.store, inv.worn_gear) = (data, Store::new(7), 1 << 1);
    let drop = DropInfo { required: vec![0, 7], weights: vec![80, 10], ..Default::default() };
    assert_eq!(drop.main_gear(), Some(1), "requirement 0 is the wingsuit, slot 1");
    let ctx = StoreContext { rider: 9, gear_types: 1 << 3, ..inv.store_context(&db, &profile, Some(&drop)) };
    assert_eq!(ctx.required_gear, 1 << 1);
    inv.store.refresh(&inv.data.clone(), &ctx, 0);
    assert!(inv.store.stock[2].iter().all(|o| o.slot == 3), "the worn wingsuit's slot is never drawn");
    inv.build_lists(&db, &profile, &mut loc, Some(&drop), &ctx, 2);
    assert_eq!(inv.store_list.len(), 4);
    assert_eq!(inv.store_list[0].slot, 1, "0x82493AB8 adds one of the main requirement's gear in front");
    assert!(inv.store_list[1..].iter().all(|u| u.slot == 3));
    let generated = inv.store.stock[2].len();
    inv.worn_gear = 0;
    inv.build_lists(&db, &profile, &mut loc, Some(&drop), &ctx, 2);
    assert_eq!(
        (inv.store_list[0].slot, inv.store.stock[2].len()),
        (1, generated),
        "an offer of it is moved to the front"
    );
    let none = DropInfo { required: vec![7], weights: vec![10], ..Default::default() };
    assert_eq!(none.main_gear(), None, "requirement 7 is no gear");
    assert_eq!(DropInfo { required: vec![0], weights: vec![0], ..Default::default() }.main_gear(), None, "no weight");
    assert_eq!(DropInfo { required: vec![0, 5], weights: vec![60, 20], ..Default::default() }.main_gear(), Some(1));
    assert_eq!(DropInfo { required: vec![6, 4], weights: vec![30, 60], ..Default::default() }.main_gear(), Some(3));
}

#[test]
fn a_purchase_keeps_the_store_order_and_the_front_special() {
    use ssx_assets::career::store::{Request, Store, StoreContext, StoreData};
    let db = CareerDb::default();
    let mut profile = Profile::default();
    profile.add_credits(1_000_000, true);
    let mut loc = Localization::empty(Language::English);
    let mut group = gear_template(100, -1);
    group.members = vec![1, 3];
    let mut data = StoreData {
        templates: [(1, gear_template(1, 1)), (3, gear_template(3, 3)), (100, group)].into(),
        ..Default::default()
    };
    data.tables.groups = vec![0, 0, 100, 0, 0];
    data.tables.durations = vec![10, 30, 90];
    let mut inv = Inventory::new(&db);
    (inv.data, inv.store) = (data.clone(), Store::new(11));
    let ctx = StoreContext { rider: 9, gear_types: (1 << 1) | (1 << 3), ..inv.store_context(&db, &profile, None) };
    let special = inv.store.generate(&data, &ctx, 2, &Request { special: true, ..Request::NORMAL }, 0).unwrap();
    inv.store.offered[2].push(special);
    inv.store.refresh(&data, &ctx, 0);
    inv.build_lists(&db, &profile, &mut loc, None, &ctx, 2);
    let keys = |l: &[UiItem]| l.iter().map(|u| u.key).collect::<Vec<_>>();
    let before = keys(&inv.store_list);
    assert_eq!((before.len(), before[0]), (4, special), "the special leads the page");
    assert_eq!(inv.store.shown_special, Some((2, special)), "0x824943F0 records the front special");
    let due = inv.store.find(2, special).unwrap().expires_ms;
    assert_eq!(inv.store.shown_special_left(5_000), Some(due - 5_000));

    let bought = before[2];
    let equip_id = profile
        .buy_offer(&mut inv.store, &data, &ctx, OfferPurchase { kind: 2, id: bought, clock_s: 1, now_ms: 0 })
        .unwrap();
    inv.put_bought_first(&db, &profile, &mut loc, None, 2, (bought, 1));
    inv.refresh_lists(&db, &profile, &mut loc, None, &ctx, 2);
    let after = keys(&inv.store_list);
    assert_eq!(&after[..3], [before[0], before[1], before[3]], "the order stays, the bought entry left");
    assert_eq!(after.len(), 4);
    assert!(!before.contains(&after[3]), "the fill generated one new offer at the end");
    assert_eq!(inv.owned_list[0].key, equip_id, "the bought item leads the owned list");
    assert!(inv.inventory_list.iter().any(|u| u.key == equip_id));
    assert_eq!(inv.store.shown_special, Some((2, special)), "still in front: recorded again");

    let equip_special = profile
        .buy_offer(&mut inv.store, &data, &ctx, OfferPurchase { kind: 2, id: special, clock_s: 1, now_ms: 0 })
        .unwrap();
    inv.put_bought_first(&db, &profile, &mut loc, None, 2, (special, 1));
    inv.refresh_lists(&db, &profile, &mut loc, None, &ctx, 2);
    assert_eq!(inv.store_list[0].key, before[1]);
    assert_eq!(inv.store.shown_special, None);
    assert_eq!(keys(&inv.owned_list)[..2], [equip_special, equip_id]);
}

#[test]
fn a_bought_item_is_owned_once_after_the_rider_is_chosen_again() {
    use ssx_assets::career::store::{Store, StoreData};
    let db = CareerDb::default();
    let mut profile = Profile::default();
    profile.add_credits(1_000_000, true);
    for slot in [1, 3] {
        profile.add_gear_type(slot);
    }
    let mut loc = Localization::empty(Language::English);
    let mut group = gear_template(100, -1);
    group.members = vec![1, 3];
    let mut data = StoreData {
        templates: [(1, gear_template(1, 1)), (3, gear_template(3, 3)), (100, group)].into(),
        ..Default::default()
    };
    data.tables.groups = vec![0, 0, 100, 0, 0];
    data.tables.durations = vec![10, 30, 90];
    let mut inv = Inventory::new(&db);
    (inv.data, inv.store) = (data.clone(), Store::new(11));
    inv.select_rider(&db, &profile, 0, 9);
    let ctx = inv.store_context(&db, &profile, None);
    inv.build_lists(&db, &profile, &mut loc, None, &ctx, 2);
    let bought = inv.store_list.iter().map(|u| u.key).min().expect("offers");
    assert!(inv.store.stock[2].iter().any(|o| o.id > bought), "an offer after the bought one gets renumbered");
    let equip_id = profile
        .buy_offer(&mut inv.store, &data, &ctx, OfferPurchase { kind: 2, id: bought, clock_s: 1, now_ms: 0 })
        .unwrap();
    inv.put_bought_first(&db, &profile, &mut loc, None, 2, (bought, 1));
    inv.refresh_lists(&db, &profile, &mut loc, None, &ctx, 2);
    assert_eq!(keys(&inv.owned_list), [equip_id]);

    inv.select_rider(&db, &profile, 0, 9);
    let ctx = inv.store_context(&db, &profile, None);
    inv.build_lists(&db, &profile, &mut loc, None, &ctx, 2);
    assert_eq!(keys(&inv.owned_list), [equip_id], "owned once, not twice");
    let reused = inv.store_list.iter().find(|u| u.key == bought).expect("the offer with the bought id is on sale");
    assert!(!reused.has(flag::AVAILABLE), "an offer, not owned");
}

fn tuner_progression() -> (Vec<i64>, Vec<i64>) {
    let thresholds = [
        6_000, 12_000, 24_000, 100_000, 200_000, 350_000, 650_000, 1_000_000, 2_500_000, 5_000_000, 7_500_000,
        10_000_000, 12_500_000, 15_000_000, 17_500_000,
    ];
    let mut prices = vec![0; 15];
    prices[10..].copy_from_slice(&[1_000_000, 2_500_000, 5_000_000, 7_500_000, 10_000_000]);
    (thresholds.to_vec(), prices)
}

#[test]
fn x_buys_the_highlighted_riders_next_level_not_the_equipped_ones() {
    use ssx_assets::career::profile::{RiderStats, update_rider_level};
    let (thresholds, prices) = tuner_progression();
    let mut db = CareerDb { riders: (1..=14).map(|k| rider(k, false)).collect(), ..Default::default() };
    (db.progression.level_thresholds, db.progression.level_prices) = (thresholds.clone(), prices.clone());
    let rules = ssx_assets::career::Rules {
        level_thresholds: thresholds.clone(),
        level_prices: prices.clone(),
        ..Default::default()
    };
    let mut profile = Profile::default();
    profile.add_credits(50_000_000, true);
    for r in &db.riders {
        profile.riders.insert(r.key, RiderStats { money_earned: 6_000_000, ..Default::default() });
    }
    let mut loc = Localization::empty(Language::English);
    let mut inv = Inventory::new(&db);
    inv.select_rider(&db, &profile, 0, 1);
    let ctx = inv.store_context(&db, &profile, None);
    inv.build_lists(&db, &profile, &mut loc, None, &ctx, inventory::RIDER);
    let list = inv.owned_list.clone();
    assert_eq!(list.len(), 14);
    let at = list.iter().position(|u| u.key == 9).expect("rider 9 listed");
    assert!(list[at].has(flag::LEVEL_PURCHASEABLE) && list[at].level == 10 && list[at].cost == 1_000_000);
    assert_eq!(level_purchase(&list, at, 50_000_000), LevelPurchase::Buy { rider: 9, level: 10 });
    for (i, u) in list.iter().enumerate() {
        assert_eq!(level_purchase(&list, i, 50_000_000), LevelPurchase::Buy { rider: u.key, level: 10 });
    }
    assert_eq!(profile.purchase_rider_level(&rules, 9, 10), Ok(1_000_000));
    assert_eq!(profile.credits(), 49_000_000);
    let bought = profile.riders.get_mut(&9).expect("rider 9");
    assert_eq!(bought.max_purchased_level, 10);
    assert_eq!(update_rider_level(bought, &thresholds, &prices), (9, 0), "access to level 11: its bar at 0 %");
    assert_eq!(profile.riders[&1].max_purchased_level, 0, "the equipped rider keeps its levels");
    inv.build_lists(&db, &profile, &mut loc, None, &ctx, inventory::RIDER);
    let list = inv.owned_list.clone();
    let at = list.iter().position(|u| u.key == 9).expect("rider 9 listed");
    assert!(!list[at].has(flag::LEVEL_PURCHASEABLE));
    assert_eq!(level_purchase(&list, at, 50_000_000), LevelPurchase::NotOffered);
    let other = list.iter().position(|u| u.key == 2).expect("rider 2 listed");
    assert_eq!(level_purchase(&list, other, 999_999), LevelPurchase::NotEnoughMoney);
    assert_eq!(level_purchase(&list, 14, i32::MAX), LevelPurchase::NoEntry);
    assert_eq!(level_purchase(&list, usize::MAX, i32::MAX), LevelPurchase::NoEntry);
}

const TUNER_SLOTS: [[u8; 5]; 15] = [
    [1, 1, 1, 0, 0],
    [1, 1, 1, 0, 0],
    [1, 1, 1, 1, 0],
    [1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1],
    [1, 1, 1, 2, 1],
    [1, 1, 1, 2, 2],
    [1, 1, 1, 2, 2],
    [1, 1, 2, 2, 2],
    [1, 1, 2, 3, 3],
    [1, 1, 2, 3, 4],
    [1, 1, 2, 4, 4],
    [1, 1, 3, 4, 4],
    [1, 1, 3, 4, 4],
];

#[test]
fn a_level_for_sale_describes_the_slots_it_adds() {
    let mut loc = Localization::empty(Language::English);
    for (k, t) in [
        ("TXT_EQUIP_BUY_LVL_0", "Buy Access to Level %1d!\n"),
        ("TXT_EQUIP_BUY_LVL_1", "Buy Access to Level %1d!\n%2s"),
        ("TXT_EQUIP_BUY_LVL_2", "Buy Access to Level %1d!\n%2s, %3s"),
        ("TXT_EQUIP_BUY_LVL_3", "Buy Access to Level %1d!\n%2s, %3s, %4s"),
        ("TXT_EQUIP_BUY_LVL_BOOST", "+%1d Mod slots"),
        ("TXT_EQUIP_BUY_LVL_GEAR", "+%1d Gear slots"),
        ("TXT_EQUIP_BUY_LVL_GEOTAG", "+%1d Geotag slots"),
    ] {
        loc.add_port_string(k, t).expect("a free key");
    }
    let rows = TUNER_SLOTS;
    let text =
        |loc: &mut Localization, rows: &[[u8; 5]], level| inventory::level_purchase_description(loc, rows, level);
    assert_eq!(text(&mut loc, &rows, 9), "Buy Access to Level 11!\n+1 Mod slots, +1 Geotag slots");
    assert_eq!(text(&mut loc, &rows, 10), "Buy Access to Level 12!\n+1 Geotag slots");
    assert_eq!(text(&mut loc, &rows, 12), "Buy Access to Level 14!\n+1 Gear slots");
    assert_eq!(text(&mut loc, &rows, 13), "Buy Access to Level 15!\n", "no slot grows");
    assert_eq!(text(&mut loc, &rows, 14), "", "no row after the last: nothing written");
    assert_eq!(text(&mut loc, &rows, -1), "");
    let pair = [[1, 1, 3, 1, 1], [3, 1, 2, 5, 1]];
    assert_eq!(text(&mut loc, &pair, 0), "Buy Access to Level 2!\n, +4 Mod slots");
    let pair = [[0, 0, 0, 0, 0], [1, 1, 1, 1, 0]];
    assert_eq!(text(&mut loc, &pair, 0), "");
    let pair = [[0, 0, 0, 0, 0], [0, 1, 1, 1, 0]];
    assert_eq!(text(&mut loc, &pair, 0), "Buy Access to Level 2!\n, +1 Gear slots, +1 Mod slots");
}

#[test]
fn the_low_survival_popup_comes_below_30_without_the_never_show_flag() {
    let mut loc = Localization::empty(Language::English);
    let body = "Your current gear selection gives you a %1s chance of survival.  Do you still want to proceed?";
    loc.add_port_string("TXT_EQUIP_LOW_SURVIVAL_BODY", body).expect("the text");
    let mut values = UiValues::new();
    assert_eq!(
        low_survival_body(&mut loc, &values, 4).as_deref(),
        Some("Your current gear selection gives you a 4% chance of survival.  Do you still want to proceed?")
    );
    assert!(low_survival_body(&mut loc, &values, 29).is_some_and(|b| b.contains(" 29% ")));
    assert!(low_survival_body(&mut loc, &values, -1).is_some_and(|b| b.contains(" -1% ")));
    for score in [30, 31, 46, 100] {
        assert_eq!(low_survival_body(&mut loc, &values, score), None, "{score}");
    }
    values.set_bool(LOW_SURVIVE_NEVER_SHOW, true);
    for score in [-1, 0, 4, 29] {
        assert_eq!(low_survival_body(&mut loc, &values, score), None, "{score}: never show again");
    }
}
