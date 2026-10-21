use ssx_assets::career::{CareerDb, Profile};

use crate::game::fe::inventory::{Inventory, RIDER};
use crate::game::loc::Localization;

pub const CHARACTER_KEY: u16 = 0xA7;
pub const GEAR_VALUES: u16 = 0x233;
pub const GEAR_TYPES: usize = 15;
pub const BOOST_TYPE: i64 = 8;
pub const JACKET_TYPE: usize = 12;
pub const PANTS_TYPE: usize = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GearItem {
    pub stats: [(i8, u8); 6],
    pub slot: i32,
    pub description: u32,
}

impl Default for GearItem {
    fn default() -> Self {
        GearItem::CLEARED
    }
}

impl GearItem {
    pub const CLEARED: GearItem = GearItem { stats: [(-1, 0); 6], slot: -1, description: 0 };

    pub fn set_slot(&mut self, slot: i32) {
        if self.slot != slot {
            *self = GearItem::CLEARED;
        }
        self.slot = slot;
    }

    pub fn set_stat(&mut self, stat: i32, value: u8) {
        let stat = stat as i8;
        if let Some(e) = self.stats.iter_mut().find(|e| e.0 == stat || e.0 == -1) {
            *e = (stat, value);
        }
    }

    pub fn is_cleared(&self) -> bool {
        self.slot == -1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub kind: i64,
    pub index: usize,
    pub key: u32,
    pub src: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameSetup {
    pub character: u32,
    pub gear: [GearItem; GEAR_TYPES],
    pub sources: [Option<Source>; GEAR_TYPES],
}

pub fn commit(inv: &Inventory, db: &CareerDb, profile: &Profile, loc: &mut Localization) -> GameSetup {
    let mut setup = GameSetup::default();
    let rider = if inv.rider != 0 { inv.record(db, profile, loc, inv.rider, 0) } else { None };
    setup.character = rider.as_ref().filter(|r| r.kind == RIDER).map_or(0, |r| r.descriptions[0]);
    let bonus = |stat: i64| -> Option<u32> {
        let r = db.riders.iter().find(|r| r.key == setup.character)?;
        r.stat_bonuses.iter().find(|&&(s, _)| i64::from(s) == stat).map(|&(_, b)| u32::from(b))
    };
    let mut gear = setup.gear;
    let mut sources = setup.sources;
    for kind in 0..4usize {
        let count = inv.category_slots.get(kind).copied().unwrap_or(0).max(0) as usize;
        let mut boosts = 0i64;
        for index in 0..count {
            let Some(&(key, src)) = inv.equipped[kind].get(index) else { continue };
            if key == u32::MAX || Inventory::is_empty_item(kind as i64, key, src) {
                continue;
            }
            let Some(rec) = inv.record(db, profile, loc, key, src) else { continue };
            let mut slot = rec.slot;
            if slot == BOOST_TYPE {
                slot = BOOST_TYPE + boosts;
                boosts = (boosts + 1).min(3);
            }
            let source = Source { kind: kind as i64, index, key, src };
            if (0..GEAR_TYPES as i64).contains(&slot) {
                let mut item = GearItem::CLEARED;
                item.set_slot(slot as i32);
                item.description = if rec.rider != 0 && rec.rider != setup.character {
                    rider_description(db, setup.character, rec.descriptions[0], slot)
                } else {
                    rec.descriptions[0]
                };
                for (k, &(stat, value)) in rec.stats.iter().enumerate() {
                    if stat == -1 {
                        continue;
                    }
                    let mut raw = u32::from(value);
                    if kind != 3
                        && let Some(b) = bonus(stat)
                    {
                        raw += b;
                    }
                    let scaled = kind == 3 || k > 2;
                    let v = ssx_physics::gear::committed_value(stat as i32, raw, scaled, &db.boost_max);
                    item.set_stat(stat as i32, v);
                }
                gear[slot as usize] = item;
                sources[slot as usize] = Some(source);
            }
            if slot == JACKET_TYPE as i64 {
                let mut pants = GearItem::CLEARED;
                pants.set_slot(PANTS_TYPE as i32);
                pants.description = rec.descriptions[1];
                gear[PANTS_TYPE] = pants;
                sources[PANTS_TYPE] = Some(source);
            }
        }
    }
    setup.gear = gear;
    setup.sources = sources;
    setup
}

// TODO(guess): the class iterator's order (class 0x837E7E00, `+0x2C` / `+0x30`) is taken as the vaults' collection order; matters for 15 of 912 looks; settle: the attribute class's iteration (0x8391B814 vtable +0x2C / +0x30).
fn rider_description(db: &CareerDb, rider: u32, description: u32, slot: i64) -> u32 {
    let Some(id) = db.descriptions.iter().find(|d| d.key == description).map(|d| d.id) else { return 0 };
    db.descriptions.iter().find(|d| d.rider == rider && d.id == id && d.slot == slot).map_or(0, |d| d.key)
}

impl GameSetup {
    pub fn committed_items(&self) -> Vec<ssx_physics::gear::CommittedItem> {
        self.gear
            .iter()
            .filter(|g| !g.is_cleared())
            .map(|g| ssx_physics::gear::CommittedItem {
                slot: g.slot,
                stats: g.stats.iter().filter(|&&(s, _)| (0..37).contains(&s)).map(|&(s, v)| (s as usize, v)).collect(),
            })
            .collect()
    }

    pub fn describe(&self, db: &CareerDb) -> String {
        let rider = db.riders.iter().find(|r| r.key == self.character).map_or("none", |r| r.name.as_str());
        let mut out = format!("rider {rider} ({:#010x})", self.character);
        for (t, g) in self.gear.iter().enumerate().filter(|(_, g)| !g.is_cleared()) {
            let stats: Vec<String> = g.stats.iter().filter(|s| s.0 != -1).map(|(s, v)| format!("{s}={v}")).collect();
            let from = self.sources[t]
                .map_or(String::new(), |s| format!(" from {}/{} {:#010x}/{}", s.kind, s.index, s.key, s.src));
            out.push_str(&format!("; {} {:#010x} {{{}}}{from}", GEAR_NAMES[t], g.description, stats.join(",")));
        }
        out
    }
}

pub const GEAR_NAMES: [&str; GEAR_TYPES] = [
    "Board",
    "Wingsuit",
    "IceAxe",
    "Armour",
    "Thermal",
    "Oxygen",
    "HeadLamp",
    "PulseGoggles",
    "Boost1",
    "Boost2",
    "Boost3",
    "Boost4",
    "Jacket",
    "Pants",
    "SmokeCanister",
];

#[cfg(test)]
mod tests {
    use ssx_assets::career::{CareerDb, Description, Item, Profile, Rider};

    use super::*;
    use crate::game::loc::{Language, Localization};

    const ZOE: u32 = 0x10;
    const ELISE: u32 = 0x11;

    fn item(key: u32, kind: i64, slot: i64, rider: u32, attributes: &[(i32, u8)], descriptions: &[u32]) -> Item {
        Item {
            key,
            group: 0,
            title: String::new(),
            kind,
            slot,
            rider,
            locked: false,
            default: false,
            level: 0,
            quality: 0,
            cost: 0,
            attributes: attributes.to_vec(),
            unk_b6ef00c6: 0,
            descriptions: descriptions.to_vec(),
        }
    }

    fn rider(key: u32, name: &str, stat_bonuses: &[(i32, u8)]) -> Rider {
        Rider {
            key,
            name: name.into(),
            title: String::new(),
            display_name: String::new(),
            cost: 0,
            locked: false,
            unk_349e965b: false,
            unk_940b9b22: 0,
            stat_bonuses: stat_bonuses.to_vec(),
            portrait: String::new(),
            description: String::new(),
            round_icon: String::new(),
            icon: String::new(),
        }
    }

    fn description(key: u32, rider: u32, id: i64, slot: i64) -> Description {
        Description { key, rider, id, slot, excluded: false, icon: String::new() }
    }

    fn db() -> CareerDb {
        let mut boost_max = vec![15; 37];
        boost_max[31] = 3;
        CareerDb {
            riders: vec![rider(ZOE, "zoe", &[(0, 5), (31, 7), (0, 50)]), rider(ELISE, "elise", &[])],
            items: vec![
                item(0x100, 1, 0, ZOE, &[(0, 30), (1, 38), (2, 27), (26, 100), (-1, 0)], &[0x900]),
                item(0x101, 0, 12, ZOE, &[], &[0x901, 0x902]),
                item(0x102, 3, 8, 0, &[(31, 40), (1, 10)], &[]),
                item(0x103, 3, 8, 0, &[(0, 100)], &[]),
                item(0x104, 2, 3, ELISE, &[(11, 34), (12, 52), (37, 9), (12, 99)], &[0x905]),
            ],
            descriptions: vec![description(0x905, ELISE, 4, 3), description(0x906, ZOE, 4, 3)],
            boost_max,
            ..Default::default()
        }
    }

    fn inventory(db: &CareerDb) -> Inventory {
        let mut inv = Inventory::new(db);
        inv.rider = ZOE;
        inv.category_slots = vec![1, 1, 1, 2, 0];
        inv.equipped = [vec![(0x101, 0)], vec![(0x100, 0)], vec![(0x104, 0)], vec![(0x102, 0), (0x103, 0)], vec![]];
        inv
    }

    #[test]
    fn set_stat_takes_the_stats_entry_or_the_first_free_one() {
        let mut g = GearItem::CLEARED;
        g.set_slot(3);
        g.set_stat(11, 10);
        g.set_stat(12, 20);
        g.set_stat(11, 30);
        assert_eq!(&g.stats[..3], &[(11, 30), (12, 20), (-1, 0)]);
        g.set_slot(3);
        assert_eq!(g.stats[0], (11, 30), "the same slot keeps the entries");
        g.set_slot(4);
        assert_eq!(g, GearItem { slot: 4, ..GearItem::CLEARED });
    }

    #[test]
    fn the_commit_writes_each_type_once_with_the_riders_bonus() {
        let db = db();
        let mut loc = Localization::empty(Language::English);
        let s = commit(&inventory(&db), &db, &Profile::default(), &mut loc);
        assert_eq!(s.character, ZOE);
        assert_eq!(s.gear[0].stats[..4], [(0, 35), (1, 38), (2, 27), (26, 15)]);
        assert_eq!((s.gear[0].slot, s.gear[0].description), (0, 0x900));
        assert_eq!((s.gear[12].slot, s.gear[12].description), (12, 0x901));
        assert_eq!((s.gear[13].slot, s.gear[13].description, s.gear[13].stats), (13, 0x902, [(-1, 0); 6]));
        assert_eq!(s.gear[8].stats[..2], [(31, 2), (1, 2)]);
        assert_eq!(s.gear[9].stats[0], (0, 15));
        assert_eq!((s.gear[3].slot, s.gear[3].description), (3, 0x906));
        assert_eq!(s.gear[3].stats[..3], [(11, 34), (12, 15), (37, 0)]);
        assert!(s.gear[1].is_cleared() && s.gear[14].is_cleared());
        let items = s.committed_items();
        assert_eq!(items.iter().map(|i| i.slot).collect::<Vec<_>>(), [0, 3, 8, 9, 12, 13]);
        assert_eq!(items[1].stats, [(11, 34), (12, 15)], "stat 37 is not summed");
    }

    #[test]
    fn empty_items_slots_past_the_count_and_no_rider_commit_nothing() {
        let db = db();
        let mut inv = inventory(&db);
        inv.rider = 0;
        inv.category_slots = vec![1, 0, 1, 1, 0];
        inv.equipped[2] = vec![(ssx_formats::attrib::fnv1(b"gear_empty"), 0)];
        let mut loc = Localization::empty(Language::English);
        let s = commit(&inv, &db, &Profile::default(), &mut loc);
        assert_eq!(s.character, 0);
        assert!(s.gear[0].is_cleared(), "the board's slot is past the count");
        assert!(s.gear[3].is_cleared(), "the kind's empty item");
        assert_eq!(s.gear[8].stats[0], (31, 2));
        assert!(s.gear[9].is_cleared(), "one boost slot");
    }

    #[test]
    fn a_launch_with_nothing_equipped_commits_the_inventorys_default_equipment() {
        let empty = |n: &str| ssx_formats::attrib::fnv1(n.as_bytes());
        let mut db = db();
        db.default_equipment = vec![0x101, 0x100, 0x104, empty("boost_empty"), empty("geotag_empty"), ELISE];
        let inv = Inventory::new(&db);
        assert_eq!(inv.rider, ELISE, "element 5, the rider");
        assert_eq!(inv.equipped[..3], [vec![(0x101, 0)], vec![(0x100, 0)], vec![(0x104, 0)]]);
        assert_eq!(inv.equipped[3], [(empty("boost_empty"), 0)], "the kind's empty item stays");
        let mut loc = Localization::empty(Language::English);
        let s = commit(&inv, &db, &Profile::default(), &mut loc);
        assert_eq!(s.character, ELISE);
        assert!(!s.gear[0].is_cleared() && !s.gear[12].is_cleared(), "the board and the suit");
        assert_eq!(Inventory::new(&CareerDb::default()).rider, 0, "without the collection nothing is equipped");
    }
}
