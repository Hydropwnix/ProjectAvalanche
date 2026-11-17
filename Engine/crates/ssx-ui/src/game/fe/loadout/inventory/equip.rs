use super::*;

impl Inventory {
    pub fn slot_counts(&self, db: &CareerDb, profile: &Profile, drop: u32) -> [i64; KINDS] {
        let e = db.event(drop);
        if let Some(e) = e.filter(|e| !e.unk_98e9ebcc.is_empty()) {
            let mut out = [0; KINDS];
            for (o, &v) in out.iter_mut().zip(e.unk_98e9ebcc.iter()) {
                *o = v & 0xFF;
            }
            return out;
        }
        if self.rider == 0 {
            return [1; KINDS];
        }
        let level = Self::rider_level(db, profile, self.rider).0.max(0) as usize;
        let rows = &db.progression.level_slots;
        let row = rows.get(level.min(rows.len().saturating_sub(1))).copied().unwrap_or([1; 5]);
        let mut out = row.map(|v| i64::from(v.min(4)));
        if e.is_some_and(|e| e.drop_type == ssx_assets::career::DropType::Campaign) {
            out[3] = 0;
            out[4] = 0;
        }
        out
    }

    pub(super) fn last_equipped(&self, db: &CareerDb, profile: &Profile, kind: i64, slot: usize) -> Option<(u32, u8)> {
        let stats = &self.rider_stats(db, profile, self.rider);
        let (key, src) = match kind {
            0 => stats.last_suit,
            1 => stats.last_board,
            2 => *stats.last_gear.get(slot)?,
            4 => *stats.last_geotags.get(slot)?,
            _ => return None,
        };
        let owned = |k: u32| self.catalog.entries.get(&k).is_some_and(|e| profile.offered(k, e));
        let ok = match src {
            0 => owned(key),
            1 => profile.generated.contains_key(&key),
            _ => false,
        };
        if ok {
            return Some((key, src));
        }
        if kind == 1 {
            let name = db.riders.iter().find(|r| r.key == self.rider).map(|r| r.name.to_lowercase())?;
            return Some((fnv1(format!("board_{name}_default").as_bytes()), 0));
        }
        None
    }

    pub fn select_rider(&mut self, db: &CareerDb, profile: &Profile, drop: u32, rider: u32) {
        self.rider = rider;
        let info = DropInfo::of(db, profile, drop);
        let ctx = self.store_context(db, profile, info.as_ref());
        let data = self.data.clone();
        self.store.update(&data, &ctx, false, self.now_ms);
        self.equip_rider(db, profile, drop, rider);
        self.store.clear_shown_special();
    }

    pub fn equip_rider(&mut self, db: &CareerDb, profile: &Profile, drop: u32, rider: u32) {
        self.rider = rider;
        let slots = self.slot_counts(db, profile, drop);
        self.category_slots = slots.to_vec();
        self.worn_gear = 0;
        let forced = DropInfo::of(db, profile, drop);
        self.event_type = event_loadouts::event_kind(db, drop);
        for kind in 0..KINDS {
            let mut list = Vec::new();
            for slot in 0..slots[kind] as usize {
                // (a port addition: the loadout last chosen for this kind of event, before the rider's last one)
                let item = self.event_item(db, profile, kind, slot).or_else(|| self.last_equipped(db, profile, kind as i64, slot));
                list.push(item.unwrap_or((fnv1(EMPTY[kind].as_bytes()), 0)));
            }
            if let Some(f) = forced.as_ref().and_then(|d| d.forced(kind as i64)) {
                if list.is_empty() {
                    list.push((f, 0));
                } else {
                    list[0] = (f, 0);
                }
            }
            self.equipped[kind] = list;
        }
        self.update_worn(db, profile);
    }

    /// The item last chosen for this slot in events of this kind (a port addition), while the profile still has it.
    /// (suits, boards, gear, mods and geotags alike; mods too, though the game keeps no last mods: only while still
    /// owned)
    fn event_item(&self, db: &CareerDb, profile: &Profile, kind: usize, slot: usize) -> Option<(u32, u8)> {
        let event = self.event_type.as_deref()?;
        let (key, src) = event_loadouts::get(self.rider, event, kind, slot)?;
        let ok = match src {
            0 => {
                Self::is_empty_item(kind as i64, key, 0)
                    || self.catalog.entries.get(&key).is_some_and(|e| profile.offered(key, e))
                    // (an item every rider (or this one) has from the start: its default board, say)
                    || (kind != 3
                        && db.items.iter().any(|i| {
                            i.key == key && i.kind == kind as i64 && i.default && (i.rider == 0 || i.rider == self.rider)
                        }))
            }
            1 => profile.generated.get(&key).is_some_and(|g| g.kind == kind as i64),
            _ => false,
        };
        ok.then_some((key, src))
    }

    pub fn open_category(&mut self, db: &CareerDb, profile: &Profile, kind: i64, slot: usize) {
        if kind != 2 {
            return;
        }
        let Some(&(key, src)) = self.equipped[2].get(slot) else { return };
        if let Some(t) = self.slot_of(db, profile, key, src).filter(|t| (0..15).contains(t)) {
            self.worn_gear &= !(1u32 << t);
        }
    }

    pub fn close_category(&mut self, db: &CareerDb, profile: &Profile, kind: i64, slot: usize) {
        if kind != 2 {
            return;
        }
        let Some(&(key, src)) = self.equipped[2].get(slot) else { return };
        if let Some(t) = self.slot_of(db, profile, key, src).filter(|&t| t >= 0) {
            self.worn_gear |= 1u32 << (t & 31);
        }
    }

    pub(super) fn update_worn(&mut self, db: &CareerDb, profile: &Profile) {
        let mut mask = 0u32;
        for &(k, s) in &self.equipped[2] {
            if let Some(g) = self.slot_of(db, profile, k, s).filter(|&g| (0..32).contains(&g)) {
                mask |= 1 << g;
            }
        }
        self.worn_gear = mask;
    }

    pub fn equip(&mut self, db: &CareerDb, profile: &Profile, kind: i64, key: u32, src: u8, slot: usize) {
        self.store.clear_shown_special();
        if !(0..KINDS as i64).contains(&kind) {
            return;
        }
        let k = kind as usize;
        let ok = match src {
            0 => db.items.iter().any(|i| i.key == key && i.kind == kind),
            _ => profile.generated.get(&key).is_some_and(|g| g.kind == kind),
        };
        let item = if ok { (key, src) } else { (fnv1(EMPTY[k].as_bytes()), 0) };
        let max = self.category_slots.get(k).copied().unwrap_or(1).max(0) as usize;
        if slot < self.equipped[k].len() {
            self.equipped[k][slot] = item;
        } else if slot < max {
            self.equipped[k].push(item);
        }
        self.update_worn(db, profile);
    }

    pub fn kind_of(&self, db: &CareerDb, profile: &Profile, key: u32, src: u8) -> i64 {
        if src == 1 {
            let key = match self.store.last_bought {
                Some((stock, equip_id)) if stock == key => equip_id,
                _ => key,
            };
            if let Some(g) = profile.generated.get(&key) {
                return g.kind;
            }
            return self.store.stock.iter().flatten().find(|o| o.id == key).map_or(-1, |o| o.kind);
        }
        if let Some(i) = db.items.iter().find(|i| i.key == key) {
            return i.kind;
        }
        if db.riders.iter().any(|r| r.key == key) { RIDER } else { -1 }
    }

    pub fn equip_message(
        &mut self,
        db: &CareerDb,
        profile: &mut Profile,
        drop: u32,
        key: u32,
        src: u8,
        slot: i64,
    ) -> bool {
        let src = if src > 2 { 2 } else { src };
        let kind = self.kind_of(db, profile, key, src);
        let taken = slot >= 0
            && ((kind == RIDER && slot == 0)
                || (0..KINDS as i64).contains(&kind) && (slot as usize) < self.equipped[kind as usize].len());
        if !taken {
            return false;
        }
        if kind == RIDER {
            self.select_rider(db, profile, drop, key);
            self.save_slot(db, profile, RIDER, 0, (key, src));
        } else {
            self.equip(db, profile, kind, key, src, slot as usize);
            if self.equipped[kind as usize].get(slot as usize) == Some(&(key, src)) {
                self.save_slot(db, profile, kind, slot as usize, (key, src));
            }
        }
        self.equip_survival(db, profile, drop);
        true
    }

    pub fn survival_on(&self, db: &CareerDb, profile: &Profile, drop: u32) -> Option<i32> {
        if self.equipped[0].is_empty() {
            return Some(0);
        }
        DropInfo::of(db, profile, drop).map(|d| self.survival_score(db, profile, Some(&d)))
    }

    fn equip_survival(&mut self, db: &CareerDb, profile: &Profile, drop: u32) {
        if let Some(s) = self.survival_on(db, profile, drop) {
            self.survival = s;
        }
    }

    pub fn launch_rider(&self, db: &CareerDb, profile: &Profile, drop: u32) -> Option<u32> {
        use ssx_assets::career::profile::ATTRIB_LAST_RIDER;
        let info = DropInfo::of(db, profile, drop);
        let listed = self.fixed_shown(db, profile, info.as_ref(), RIDER);
        let last = profile.attribs.get(&ATTRIB_LAST_RIDER).and_then(|&v| u32::try_from(v).ok());
        let default = fnv1(ssx_assets::career::world_tour::NEW_GAME_RIDER.as_bytes());
        [last, Some(default)].into_iter().flatten().find(|r| listed.contains(r)).or_else(|| listed.first().copied())
    }

    // PORT(choice): a launch the port adds (a network session's drop or map, the Maps page) equips the profile's last
    // rider (LURI) with their saved loadout on the launched drop before its commit — the drop's forced rider where it
    // forces one, the profile database's default Zoe (0x824A8530) or the list's first where LURI is not listed. The
    // console has no launch without a rider's equip: the equipment's rider select (0x825FC3A0 → 0x82497998), the new
    // game's state 0xB (0x82590BB0), the globe's tutorial branch (0x825EC2C0) and the final drop (0x8257DD30) each
    // equip one first.
    pub fn equip_for_launch(&mut self, db: &CareerDb, profile: &mut Profile, drop: u32) -> Option<u32> {
        let rider = self.launch_rider(db, profile, drop)?;
        self.equip_message(db, profile, drop, rider, 0, 0).then_some(rider)
    }

    // PORT(choice): a launch without the equipment screen (a network session's drop) puts on the drop's required gear
    // from the rider's owned items, as above, without saving it. The console equips none of it by itself: the list
    // 0xD4D58427 is read by the store (0x8248FCA0; 0x82493AB8 offers the main requirement first), the lists' order
    // (0x8248EB80), the survival score (0x8248E540: a requirement not worn sets its base to 29, so the advance
    // 0x825FB798 shows TXT_EQUIP_LOW_SURVIVAL_BODY) and Optimize's gear test (0x8248BDC8 → 0x82497A88), never by a
    // launch (0x82495230, 0x82496F40, 0x82498BA0, 0x8257DD30); every console launch of a drop with requirements passes
    // that screen, where the player equips the item, buys it, or confirms the popup and rides without it.
    pub fn equip_required_for_launch(
        &mut self,
        db: &CareerDb,
        profile: &Profile,
        loc: &mut Localization,
        drop: u32,
    ) -> LaunchGear {
        let Some(d) = DropInfo::of(db, profile, drop) else { return LaunchGear::default() };
        let mut out = LaunchGear::default();
        for _ in 0..d.required.len() {
            let unmet = self.unmet_requirements(&d);
            if self.rider == 0 || unmet.is_empty() {
                break;
            }
            let ctx = self.store_context(db, profile, Some(&d));
            self.build_lists(db, profile, loc, Some(&d), &ctx, 2);
            let fits = |u: &UiItem| unmet.iter().any(|&r| super::order::requirement_met_by_slot(u.slot, r));
            let Some(item) = self.owned_list.iter().find(|u| u.sort > 0 && !u.has(flag::EMPTY) && fits(u)) else {
                break;
            };
            let item = (item.key, item.src);
            let Some(slot) = self.launch_gear_slot(db, profile, &d) else { break };
            self.equip(db, profile, 2, item.0, item.1, slot);
            self.equip_survival(db, profile, drop);
            if self.equipped[2].get(slot) != Some(&item) {
                break;
            }
            out.equipped.push((item, slot));
        }
        out.unmet = self.unmet_requirements(&d);
        out
    }

    pub fn unmet_requirements(&self, d: &DropInfo) -> Vec<i64> {
        d.required
            .iter()
            .copied()
            .filter(|&r| match requirement_gear(r) {
                Some((Some(g), _)) => !((0..32).contains(&g) && self.worn_gear & (1 << g) != 0),
                Some((None, _)) => false,
                None => true,
            })
            .collect()
    }

    // PORT(choice): with every gear slot taken, the launch's required item replaces the last item that meets no
    // requirement and keeps the rest of the player's gear; the console's Optimize (0x82497A88) empties every gear slot
    // and refills them with requirement-meeting items only, and its equipment screen lets the player pick the slot.
    fn launch_gear_slot(&self, db: &CareerDb, profile: &Profile, d: &DropInfo) -> Option<usize> {
        let gear = &self.equipped[2];
        if let Some(i) = gear.iter().position(|&(k, s)| Self::is_empty_item(2, k, s)) {
            return Some(i);
        }
        let first = usize::from(d.forced(2).is_some());
        (first..gear.len()).rev().find(|&i| {
            let (k, s) = gear[i];
            !self.slot_of(db, profile, k, s).is_some_and(|t| super::order::meets_requirement(Some(d), t))
        })
    }

    // TODO(guess): the port writes every slot's row (the optimisation 0x82497A88 writes only the slots it changes); settle: port its per-slot equips (0x82497F2C / 0x8249819C).
    pub fn save_loadout(&self, db: &CareerDb, profile: &mut Profile) {
        for kind in 0..KINDS {
            for (slot, &item) in self.equipped[kind].iter().enumerate() {
                self.save_slot(db, profile, kind as i64, slot, item);
            }
        }
        self.save_slot(db, profile, RIDER, 0, (self.rider, 0));
    }

    pub fn consume_mods(&mut self, profile: &mut Profile) -> Vec<(u32, u8)> {
        let empty = fnv1(EMPTY[3].as_bytes());
        let mut consumed = Vec::new();
        for slot in 0..self.equipped[3].len() {
            let (key, src) = self.equipped[3][slot];
            self.equipped[3][slot] = (empty, 0);
            if key == empty && src == 0 {
                continue;
            }
            if src == 1 {
                self.remove_generated(profile, 3, key);
            } else {
                profile.owned_items.remove(&key);
            }
            consumed.push((key, src));
        }
        consumed
    }

    pub fn remove_generated(&mut self, profile: &mut Profile, kind: usize, key: u32) {
        let id = match self.store.last_bought {
            Some((stock, equip_id)) if stock == key => equip_id,
            _ => key,
        };
        let stock = id > 999_999 && self.store.stock.get(kind).is_some_and(|l| l.iter().any(|o| o.id == id));
        if !stock && !profile.generated.contains_key(&id) {
            return;
        }
        profile.generated.remove(&id);
        if let Some(shown) = self.store.offered.get_mut(kind)
            && let Some(i) = shown.iter().position(|&k| k == key)
        {
            shown.swap_remove(i);
        }
    }

    pub fn save_slot(&self, db: &CareerDb, profile: &mut Profile, kind: i64, slot: usize, item: (u32, u8)) {
        if item.1 == 0 && UNSAVED_ITEMS.contains(&item.0) {
            return;
        }
        if kind == RIDER {
            profile.attribs.insert(ssx_assets::career::profile::ATTRIB_LAST_RIDER, i64::from(item.0));
            event_loadouts::set_last_rider(item.0);
            return;
        }
        let count = self.category_slots.get(kind as usize).copied().unwrap_or(0).max(0) as usize;
        if !(0..KINDS as i64).contains(&kind) || slot >= count {
            return;
        }
        // (a port addition: kept for this kind of event too; mods as well, which the rider's own last loadout doesn't
        // keep)
        if let Some(event) = self.event_type.as_deref() {
            event_loadouts::set(self.rider, event, kind as usize, slot, item);
        }
        if kind == 3 {
            return;
        }
        let row = self.rider_stats(db, profile, self.rider);
        let stats = profile.riders.entry(self.rider).or_insert(row);
        let target = match kind {
            0 => Some(&mut stats.last_suit),
            1 => Some(&mut stats.last_board),
            2 => stats.last_gear.get_mut(slot),
            _ => stats.last_geotags.get_mut(slot),
        };
        if let Some(t) = target {
            *t = item;
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchGear {
    pub equipped: Vec<((u32, u8), usize)>,
    pub unmet: Vec<i64>,
}

pub const UNSAVED_ITEMS: [u32; 8] =
    [0x8650_5C81, 0x371D_D13C, 0xE318_0443, 0xC492_2193, 0x2002_1A25, 0x2039_B98F, 0x2636_71F3, 0xEED8_4B39];

#[cfg(test)]
mod tests {
    use ssx_assets::career::profile::GeneratedItem;

    use super::*;

    #[test]
    fn leaving_the_drop_consumes_the_mods() {
        let db = CareerDb::default();
        let mut inv = Inventory::new(&db);
        let mut profile = Profile::default();
        let empty = fnv1(EMPTY[3].as_bytes());
        inv.category_slots = vec![0, 0, 0, 3, 0];
        inv.equipped[3] = vec![(0x1234, 1), (empty, 0), (0xABCD, 0)];
        profile.generated.insert(0x1234, GeneratedItem { equip_id: 0x1234, kind: 3, ..GeneratedItem::default() });
        profile.owned_items.insert(0xABCD);
        inv.store.offered[3] = vec![0x1234, 0x5678, 0x9ABC];
        assert_eq!(inv.consume_mods(&mut profile), [(0x1234, 1), (0xABCD, 0)]);
        assert_eq!(inv.equipped[3], [(empty, 0), (empty, 0), (empty, 0)]);
        assert!(!profile.generated.contains_key(&0x1234), "the equip_generated row is gone");
        assert!(!profile.owned_items.contains(&0xABCD), "the equipment row is gone");
        assert_eq!(inv.store.offered[3], [0x9ABC, 0x5678], "the last shown entry took its place");
        assert!(inv.consume_mods(&mut profile).is_empty());
        assert_eq!(inv.equipped[3], [(empty, 0), (empty, 0), (empty, 0)]);
        assert_eq!(inv.store.offered[3], [0x9ABC, 0x5678]);
    }

    mod launch {
        use ssx_assets::career::profile::ATTRIB_LAST_RIDER;
        use ssx_assets::career::{DropType, Event, Item, Rider};

        use super::*;
        use crate::game::fe::setup::{GameSetup, commit};
        use crate::game::loc::{Language, Localization};

        const SKY_BOARD: u32 = 0x8650_5C81;
        const PATGEAR1: u32 = 0xE318_0443;
        const ZOE_SUIT: u32 = 0x55BB_536F;
        const H2H: u32 = 0xC145_699A;

        fn key(name: &str) -> u32 {
            fnv1(name.as_bytes())
        }

        fn item(key: u32, kind: i64, slot: i64, rider: u32, locked: bool, attributes: &[(i32, u8)]) -> Item {
            Item {
                key,
                group: 0,
                title: String::new(),
                kind,
                slot,
                rider,
                locked,
                default: !locked,
                level: 0,
                quality: 0,
                cost: 0,
                attributes: attributes.to_vec(),
                unk_b6ef00c6: 0,
                descriptions: Vec::new(),
                content: None,
            }
        }

        fn rider(name: &str) -> Rider {
            Rider {
                key: key(name),
                name: name.into(),
                title: String::new(),
                display_name: String::new(),
                cost: 0,
                locked: false,
                default: true,
                unk_940b9b22: 0,
                stat_bonuses: Vec::new(),
                portrait: String::new(),
                description: String::new(),
                round_icon: String::new(),
                icon: String::new(),
                content: None,
            }
        }

        fn event(name: &str, drop_type: DropType, forced: &[u32]) -> Event {
            Event {
                key: key(name),
                name: name.into(),
                drop: 0,
                event_type: 5,
                drop_type,
                mode: String::new(),
                slot: String::new(),
                peak_short: String::new(),
                hud_title: String::new(),
                hud_body: String::new(),
                goal_body: String::new(),
                results_title: String::new(),
                results_body: String::new(),
                unlock_text: String::new(),
                deadly_descent: false,
                warning: 0,
                time_of_day: 0,
                cost: 0,
                difficulty: 0,
                reward: 0,
                places: None,
                targets: Vec::new(),
                required: Vec::new(),
                unk_b683b553: Vec::new(),
                rivals: Vec::new(),
                forced: forced.to_vec(),
                ghost_drop: false,
                unk_89d02fa4: false,
                unk_12bccfb5: 0,
                unk_0b03fc19: None,
                unk_98e9ebcc: Vec::new(),
                custom_status: 0,
                unk_links: [0; 3],
                peak_ref: 0,
                helicopter: 0,
                map_point: [0.0; 2],
            }
        }

        fn db() -> CareerDb {
            let zoe = key("zoe");
            let mut boost_max = vec![15; 37];
            boost_max[26] = 100;
            let h2h = event("chapter_2_drop_h2h", DropType::Campaign, &[0, SKY_BOARD, PATGEAR1, 0, 0, zoe]);
            assert_eq!(h2h.key, H2H);
            CareerDb {
                riders: vec![rider("zoe"), rider("elise")],
                items: vec![
                    item(ZOE_SUIT, 0, 12, zoe, false, &[]),
                    item(key("board_zoe_default"), 1, 0, zoe, false, &[(0, 30), (1, 38), (2, 27)]),
                    item(key("board_elise_default"), 1, 0, key("elise"), false, &[(0, 44), (1, 34), (2, 17)]),
                    item(SKY_BOARD, 1, 0, 0, true, &[(0, 0), (1, 0), (2, 100), (26, 100)]),
                    item(PATGEAR1, 2, 1, 0, true, &[(5, 100), (6, 100), (7, 0)]),
                ],
                events: vec![
                    h2h,
                    event("exp_rockies_extinguisher_drop_1_race", DropType::Explore, &[]),
                    event("chapter_2_drop_gen_4", DropType::Campaign, &[0, 0, 0, 0, 0, zoe]),
                ],
                boost_max,
                ..Default::default()
            }
        }

        fn committed(inv: &Inventory, db: &CareerDb, profile: &Profile) -> GameSetup {
            commit(inv, db, profile, &mut Localization::empty(Language::English))
        }

        fn anti_gravity(s: &GameSetup) -> Option<u8> {
            s.gear[0].stats.iter().find(|&&(stat, _)| stat == 26).map(|&(_, v)| v)
        }

        #[test]
        fn a_launch_without_the_equipment_screen_drops_the_skyboard_tutorials_items() {
            let db = db();
            let mut profile = Profile::new();
            let mut inv = Inventory::new(&db);
            for key in [key("zoe"), ZOE_SUIT, SKY_BOARD, PATGEAR1] {
                assert!(inv.equip_message(&db, &mut profile, H2H, key, 0, 0));
            }
            let tutorial = committed(&inv, &db, &profile);
            assert_eq!(anti_gravity(&tutorial), Some(100), "the skyboard tutorial rides the sky board");
            assert!(!tutorial.gear[1].is_cleared(), "and PATGEAR1");
            let explore = key("exp_rockies_extinguisher_drop_1_race");
            assert_eq!(inv.equip_for_launch(&db, &mut profile, explore), Some(key("zoe")), "the last rider");
            let s = committed(&inv, &db, &profile);
            assert_eq!(s.character, key("zoe"));
            assert_eq!(inv.equipped[1], [(key("board_zoe_default"), 0)], "the sky board is never saved");
            assert_eq!(anti_gravity(&s), None);
            assert_eq!(s.gear[0].stats[..3], [(0, 30), (1, 38), (2, 27)], "her default board");
            assert!(s.gear[1].is_cleared(), "no PATGEAR1");
        }

        #[test]
        fn the_launchs_rider_is_the_last_one_the_drop_lists() {
            let mut db = db();
            let (zoe, elise) = (key("zoe"), key("elise"));
            let explore = key("exp_rockies_extinguisher_drop_1_race");
            let mut profile = Profile::new();
            profile.attribs.insert(ATTRIB_LAST_RIDER, i64::from(elise));
            let mut inv = Inventory::new(&db);
            assert_eq!(inv.launch_rider(&db, &profile, explore), Some(elise));
            assert_eq!(inv.launch_rider(&db, &profile, 0), Some(elise), "a map");
            assert_eq!(inv.launch_rider(&db, &profile, key("chapter_2_drop_gen_4")), Some(zoe), "Zoe forced");
            assert_eq!(inv.equip_for_launch(&db, &mut profile, explore), Some(elise));
            assert_eq!(inv.equipped[1], [(key("board_elise_default"), 0)]);
            for hostile in [0xDEAD_BEEF, -1, i64::from(u32::MAX) + 1] {
                profile.attribs.insert(ATTRIB_LAST_RIDER, hostile);
                assert_eq!(inv.launch_rider(&db, &profile, explore), Some(zoe), "the database's default");
            }
            db.riders.retain(|r| r.key != zoe);
            assert_eq!(inv.launch_rider(&db, &profile, explore), Some(elise), "the list's first");
            db.riders.clear();
            let before = inv.equipped.clone();
            assert_eq!(inv.equip_for_launch(&db, &mut profile, explore), None);
            assert_eq!(inv.equipped, before, "nothing equipped");
        }

        const EVEREST: &str = "exp_himwest_everest_drop_1_trick";

        fn gear(name: &str) -> u32 {
            key(name)
        }

        fn everest_db(tank_owned: bool) -> CareerDb {
            let zoe = key("zoe");
            let mut db = db();
            db.items.extend([
                item(gear("zoe_oxygen"), 2, 5, zoe, !tank_owned, &[(17, 38), (18, 48)]),
                item(gear("zoe_armour"), 2, 3, zoe, false, &[(11, 34), (12, 52), (3, 20)]),
                item(gear("zoe_wingsuit"), 2, 1, zoe, false, &[(5, 54), (6, 32), (7, 31)]),
            ]);
            let mut e = event(EVEREST, DropType::Explore, &[]);
            (e.required, e.unk_b683b553) = (vec![2, 7], vec![80, 10]);
            db.events.push(e);
            db.progression.level_slots = vec![[1, 1, 2, 0, 0]];
            db
        }

        fn launched(db: &CareerDb, profile: &mut Profile, saved: &[u32]) -> Box<Inventory> {
            let mut inv = Box::new(Inventory::new(db));
            let drop = key(EVEREST);
            assert_eq!(inv.equip_for_launch(db, profile, drop), Some(key("zoe")));
            for (slot, &k) in saved.iter().enumerate() {
                assert!(inv.equip_message(db, profile, drop, k, 0, slot as i64));
            }
            assert_eq!(inv.equip_for_launch(db, profile, drop), Some(key("zoe")));
            inv
        }

        #[test]
        fn optimize_takes_only_the_required_gear() {
            let db = everest_db(true);
            let mut profile = Profile::new();
            let mut inv = launched(&db, &mut profile, &[]);
            let mut loc = Localization::empty(Language::English);
            let d = DropInfo::of(&db, &profile, key(EVEREST)).expect("the drop");
            inv.optimize(&db, &profile, &mut loc, Some(&d));
            let empty = fnv1(EMPTY[2].as_bytes());
            assert_eq!(inv.equipped[2], [(gear("zoe_oxygen"), 0), (empty, 0)]);
            assert!(inv.survival > 30, "every requirement met: no low-survival popup ({})", inv.survival);
        }

        #[test]
        fn a_launch_without_the_equipment_screen_puts_on_the_owned_required_gear() {
            let db = everest_db(true);
            let mut loc = Localization::empty(Language::English);
            let (drop, tank) = (key(EVEREST), (gear("zoe_oxygen"), 0));
            let empty = (fnv1(EMPTY[2].as_bytes()), 0);
            let d = DropInfo::of(&db, &Profile::new(), drop).expect("the drop");
            for (saved, want, put_in) in [
                (vec![gear("zoe_armour")], [(gear("zoe_armour"), 0), tank], Some(1)),
                (vec![gear("zoe_armour"), gear("zoe_wingsuit")], [(gear("zoe_armour"), 0), tank], Some(1)),
                (vec![gear("zoe_wingsuit"), gear("zoe_armour")], [(gear("zoe_wingsuit"), 0), tank], Some(1)),
                (vec![gear("zoe_oxygen"), gear("zoe_wingsuit")], [tank, (gear("zoe_wingsuit"), 0)], None),
                (vec![], [tank, empty], Some(0)),
            ] {
                let mut profile = Profile::new();
                let mut inv = launched(&db, &mut profile, &saved);
                let saved_gear = inv.equipped[2].clone();
                let worn = saved.contains(&tank.0);
                assert_eq!(inv.survival_score(&db, &profile, Some(&d)) < 30, !worn, "saved {saved:x?}");
                let got = inv.equip_required_for_launch(&db, &profile, &mut loc, drop);
                assert_eq!(inv.equipped[2], want, "saved {saved:x?}");
                let equipped = put_in.map(|slot| vec![(tank, slot)]).unwrap_or_default();
                assert_eq!(got, LaunchGear { equipped, unmet: vec![] }, "saved {saved:x?}");
                assert!(inv.survival_score(&db, &profile, Some(&d)) > 30);
                inv.equip_for_launch(&db, &mut profile, drop);
                assert_eq!(inv.equipped[2], saved_gear, "the saved loadout");
            }
        }

        #[test]
        fn a_launchs_equips_end_with_the_survival_score() {
            let mut loc = Localization::empty(Language::English);
            let drop = key(EVEREST);
            for (tank_owned, saved) in
                [(true, vec![]), (true, vec![gear("zoe_armour")]), (false, vec![gear("zoe_armour")])]
            {
                let db = everest_db(tank_owned);
                let d = DropInfo::of(&db, &Profile::new(), drop).expect("the drop");
                let mut profile = Profile::new();
                let mut inv = launched(&db, &mut profile, &saved);
                assert_eq!(inv.survival, inv.survival_score(&db, &profile, Some(&d)), "after the rider's equip");
                assert!(inv.survival < 30, "no tank yet ({})", inv.survival);
                inv.equip_required_for_launch(&db, &profile, &mut loc, drop);
                let worn = inv.survival_score(&db, &profile, Some(&d));
                assert_eq!(inv.survival, worn, "after the launch's gear (tank owned {tank_owned}, saved {saved:x?})");
                assert_eq!(worn > 30, tank_owned, "the tank put on when owned");
            }
        }

        #[test]
        fn a_launch_without_the_required_gear_reports_it() {
            let mut loc = Localization::empty(Language::English);
            let drop = key(EVEREST);
            let db = everest_db(false);
            let mut profile = Profile::new();
            let mut inv = launched(&db, &mut profile, &[gear("zoe_armour")]);
            let before = inv.equipped.clone();
            let got = inv.equip_required_for_launch(&db, &profile, &mut loc, drop);
            assert_eq!(got, LaunchGear { equipped: vec![], unmet: vec![2] });
            assert_eq!(inv.equipped, before);

            let mut db = everest_db(true);
            db.events.last_mut().expect("Everest").required = vec![99, 2, 7];
            let mut inv = launched(&db, &mut profile, &[gear("zoe_armour")]);
            let got = inv.equip_required_for_launch(&db, &profile, &mut loc, drop);
            assert_eq!(got, LaunchGear { equipped: vec![((gear("zoe_oxygen"), 0), 1)], unmet: vec![99] });

            let db = everest_db(true);
            for drop in [key("exp_rockies_extinguisher_drop_1_race"), 0] {
                let mut inv = launched(&db, &mut profile, &[gear("zoe_armour")]);
                let before = inv.equipped.clone();
                assert_eq!(inv.equip_required_for_launch(&db, &profile, &mut loc, drop), LaunchGear::default());
                assert_eq!(inv.equipped, before);
            }
            let mut inv = Inventory::new(&db);
            let got = inv.equip_required_for_launch(&db, &profile, &mut loc, drop);
            assert_eq!((got.equipped, inv.rider), (vec![], 0), "no rider");
        }
    }

    #[test]
    fn the_remove_finds_the_record_first() {
        let db = CareerDb::default();
        let mut inv = Inventory::new(&db);
        let mut profile = Profile::default();
        profile.generated.insert(7, GeneratedItem { equip_id: 7, kind: 3, ..GeneratedItem::default() });
        inv.store.last_bought = Some((1_000_042, 7));
        inv.store.offered[3] = vec![1_000_042, 1_000_043];
        inv.remove_generated(&mut profile, 3, 1_000_042);
        assert!(!profile.generated.contains_key(&7), "the alias reached the row");
        assert_eq!(inv.store.offered[3], [1_000_043]);
        inv.remove_generated(&mut profile, 3, 1_000_043);
        assert_eq!(inv.store.offered[3], [1_000_043], "no stock offer and no row: nothing removed");
    }
}
