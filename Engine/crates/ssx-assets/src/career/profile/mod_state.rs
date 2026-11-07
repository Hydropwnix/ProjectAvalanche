use ssx_content::ContentId;
use ssx_content::profile_state::{ModState, StateValue};

use super::{ATTRIB_LAST_RIDER, Profile, RiderStats, SRC_NONE};
use crate::career::CareerDb;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModKey {
    Rider { id: ContentId, stand_in: u32 },
    Item { id: ContentId, item: u32 },
}

const SLOTS: [&str; 10] =
    ["suit", "board", "gear0", "gear1", "gear2", "gear3", "geotag0", "geotag1", "geotag2", "geotag3"];

fn slots(s: &RiderStats) -> [(u32, u8); 10] {
    let g = s.last_gear;
    let t = s.last_geotags;
    [s.last_suit, s.last_board, g[0], g[1], g[2], g[3], t[0], t[1], t[2], t[3]]
}

fn slot_mut(s: &mut RiderStats, i: usize) -> &mut (u32, u8) {
    match i {
        0 => &mut s.last_suit,
        1 => &mut s.last_board,
        2..=5 => &mut s.last_gear[i - 2],
        _ => &mut s.last_geotags[(i - 6).min(3)],
    }
}

fn key_of(state: &ModState, id: &ContentId, field: &str) -> Option<u32> {
    state.int(id, field).and_then(|v| u32::try_from(v).ok())
}

fn src_of(state: &ModState, id: &ContentId, field: &str) -> Option<u8> {
    state.int(id, field).and_then(|v| u8::try_from(v).ok()).filter(|s| *s <= SRC_NONE)
}

impl Profile {
    pub fn adopt_mod_state(&mut self, db: &CareerDb) {
        self.mod_keys.clear();
        self.mod_disc_riders = db
            .riders
            .iter()
            .filter(|r| r.content.as_ref().is_none_or(|c| !c.added))
            .map(|r| (r.key, r.name.clone()))
            .collect();
        for r in &db.riders {
            if let Some(c) = r.content.as_ref().filter(|c| c.added) {
                self.mod_keys.insert(r.key, ModKey::Rider { id: c.id.clone(), stand_in: c.stand_in });
            }
        }
        for i in &db.items {
            if let Some(c) = i.content.as_ref().filter(|c| c.added) {
                for &k in &i.descriptions {
                    self.mod_keys.insert(k, ModKey::Item { id: c.id.clone(), item: i.key });
                }
                self.mod_keys.insert(i.key, ModKey::Item { id: c.id.clone(), item: i.key });
            }
        }
        let state = self.mod_state.clone();
        for (&key, m) in &self.mod_keys.clone() {
            match m {
                ModKey::Rider { id, .. } => {
                    if state.bool(id, "owned") == Some(true) {
                        self.owned_items.insert(key);
                    }
                    if state.bool(id, "unlocked") == Some(true) {
                        self.unlocked_riders.insert(key);
                    }
                    if state.entries.contains_key(id) {
                        let mut s = RiderStats::default();
                        let n = |f: &str| state.int(id, f).unwrap_or(0);
                        (s.money_earned, s.next_level_offset) = (n("money_earned"), n("next_level_offset"));
                        (s.max_achieved_level, s.max_purchased_level) =
                            (n("max_achieved_level"), n("max_purchased_level"));
                        for (i, slot) in SLOTS.iter().enumerate() {
                            if let (Some(k), Some(src)) =
                                (key_of(&state, id, &format!("{slot}_id")), src_of(&state, id, &format!("{slot}_src")))
                            {
                                *slot_mut(&mut s, i) = (k, src);
                            }
                        }
                        if state.int(id, "money_earned").is_some() {
                            self.riders.insert(key, s);
                        }
                    }
                    if state.bool(id, "last_rider") == Some(true) {
                        self.attribs.insert(ATTRIB_LAST_RIDER, i64::from(key));
                    }
                }
                ModKey::Item { id, item } => {
                    if *item == key && state.bool(id, "owned") == Some(true) {
                        self.owned_items.insert(key);
                    }
                }
            }
        }
        for (id, _) in state.entries.iter().filter(|(id, _)| id.namespace() == ssx_content::BASE_NAMESPACE) {
            let Some((&rider, _)) = self.mod_disc_riders.iter().find(|(_, n)| id.path() == n.as_str()) else {
                continue;
            };
            for (i, slot) in SLOTS.iter().enumerate() {
                if let (Some(k), Some(src)) =
                    (key_of(&state, id, &format!("{slot}_id")), src_of(&state, id, &format!("{slot}_src")))
                    && self.mod_keys.contains_key(&k)
                {
                    *slot_mut(self.riders.entry(rider).or_default(), i) = (k, src);
                }
            }
        }
    }

    pub fn disc_view(&self) -> Profile {
        let mut p = self.clone();
        for (key, m) in &self.mod_keys {
            p.owned_items.remove(key);
            p.unlocked_riders.remove(key);
            p.unlocked_items.remove(key);
            p.riders.remove(key);
            if let ModKey::Rider { stand_in, .. } = m
                && p.attribs.get(&ATTRIB_LAST_RIDER).is_some_and(|&v| u32::try_from(v).ok() == Some(*key))
            {
                p.attribs.insert(ATTRIB_LAST_RIDER, i64::from(*stand_in));
            }
        }
        for s in p.riders.values_mut() {
            for i in 0..SLOTS.len() {
                if self.mod_keys.contains_key(&slot_mut(s, i).0) {
                    *slot_mut(s, i) = (0, SRC_NONE);
                }
            }
        }
        p.mod_state = ModState::default();
        p.mod_keys.clear();
        p.mod_disc_riders.clear();
        p
    }

    pub fn state_to_write(&self) -> ModState {
        let mut st = self.mod_state.clone();
        for (&key, m) in &self.mod_keys {
            match m {
                ModKey::Rider { id, .. } => {
                    st.entries.remove(id);
                    let mut set = |f: &str, v: StateValue| {
                        st.set(id, f, v);
                    };
                    if self.owned_items.contains(&key) {
                        set("owned", StateValue::Bool(true));
                    }
                    if self.unlocked_riders.contains(&key) {
                        set("unlocked", StateValue::Bool(true));
                    }
                    if let Some(s) = self.riders.get(&key) {
                        set("money_earned", StateValue::Int(s.money_earned));
                        set("next_level_offset", StateValue::Int(s.next_level_offset));
                        set("max_achieved_level", StateValue::Int(s.max_achieved_level));
                        set("max_purchased_level", StateValue::Int(s.max_purchased_level));
                        for (slot, (k, src)) in SLOTS.iter().zip(slots(s)) {
                            set(&format!("{slot}_id"), StateValue::Int(i64::from(k)));
                            set(&format!("{slot}_src"), StateValue::Int(i64::from(src)));
                        }
                    }
                    if self.attribs.get(&ATTRIB_LAST_RIDER).is_some_and(|&v| u32::try_from(v).ok() == Some(key)) {
                        set("last_rider", StateValue::Bool(true));
                    }
                }
                ModKey::Item { id, item } => {
                    if *item == key {
                        st.entries.remove(id);
                        if self.owned_items.contains(&key) {
                            st.set(id, "owned", StateValue::Bool(true));
                        }
                    }
                }
            }
        }
        for (key, name) in &self.mod_disc_riders {
            let Ok(id) = ContentId::new(ssx_content::BASE_NAMESPACE, ssx_content::Kind::Character, name) else {
                continue;
            };
            let row = self.riders.get(key);
            for (i, slot) in SLOTS.iter().enumerate() {
                let (fid, fsrc) = (format!("{slot}_id"), format!("{slot}_src"));
                let held = row.map(|s| slots(s)[i]).filter(|(k, _)| self.mod_keys.contains_key(k));
                let read = key_of(&st, &id, &fid);
                match held {
                    Some((k, src)) => {
                        st.set(&id, &fid, StateValue::Int(i64::from(k)));
                        st.set(&id, &fsrc, StateValue::Int(i64::from(src)));
                    }
                    None if read.is_some_and(|k| self.mod_keys.contains_key(&k)) => {
                        if let Some(e) = st.entries.get_mut(&id) {
                            e.remove(&fid);
                            e.remove(&fsrc);
                        }
                    }
                    None => {}
                }
            }
            if st.entries.get(&id).is_some_and(|e| e.is_empty()) {
                st.entries.remove(&id);
            }
        }
        st
    }
}

impl Profile {
    pub fn music_switches(&self, keys: &[ContentId]) -> (Vec<(ContentId, bool)>, Vec<String>) {
        let mut reasons = Vec::new();
        let switches = keys
            .iter()
            .map(|k| {
                let on = ssx_content::audio::switches::read(&self.mod_state, k).unwrap_or_else(|e| {
                    reasons.push(e);
                    true
                });
                (k.clone(), on)
            })
            .collect();
        (switches, reasons)
    }

    pub fn set_music_switches(&mut self, switches: &[(ContentId, bool)]) -> Vec<ContentId> {
        let mut refused = Vec::new();
        for (key, on) in switches {
            if !ssx_content::audio::switches::write(&mut self.mod_state, key, *on) {
                refused.push(key.clone());
            }
        }
        refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::career::{Item, ItemContent, Rider, RiderContent};
    use ssx_formats::attrib::fnv1;

    fn id(s: &str) -> ContentId {
        s.parse().unwrap()
    }

    fn rider(key: u32, name: &str, content: Option<RiderContent>) -> Rider {
        Rider {
            key,
            name: name.into(),
            title: String::new(),
            display_name: name.into(),
            cost: 0,
            locked: false,
            default: false,
            unk_940b9b22: 0,
            stat_bonuses: Vec::new(),
            portrait: String::new(),
            description: String::new(),
            round_icon: String::new(),
            icon: String::new(),
            content,
        }
    }

    fn setup() -> (CareerDb, Profile, u32, u32, u32) {
        let (elise, sam, suit) = (fnv1(b"elise"), 0x1111_0001, 0x2222_0002);
        let mut db = CareerDb::default();
        db.riders.push(rider(elise, "elise", None));
        let added = RiderContent {
            id: id("sam:character/sam"),
            stand_in: elise,
            portrait: None,
            round_icon: None,
            icon: None,
            added: true,
            default_board: None,
            disc_stat_bonuses: None,
        };
        db.riders.push(rider(sam, "sam:character/sam", Some(added)));
        db.items.push(Item {
            key: suit,
            group: 0,
            title: String::new(),
            kind: 1,
            slot: 12,
            rider: 0,
            locked: false,
            default: false,
            level: 0,
            quality: 0,
            cost: 0,
            attributes: Vec::new(),
            unk_b6ef00c6: 0,
            descriptions: vec![suit],
            content: Some(ItemContent { id: id("sam:gear/suit"), added: true, disc_attributes: None }),
        });
        let text = r#"format = 1
["sam:character/sam"]
owned = true
unlocked = true
money_earned = 1200
next_level_offset = 0
max_achieved_level = 2
max_purchased_level = 1
suit_id = 572653570
suit_src = 0
last_rider = true
["sam:gear/suit"]
owned = true
["ssx:character/elise"]
board_id = 572653570
board_src = 0
["gone:character/old"]
owned = true
"#;
        let mut p = Profile { mod_state: ModState::parse(text.as_bytes()).unwrap(), ..Profile::default() };
        p.riders.insert(elise, RiderStats::default());
        (db, p, elise, sam, suit)
    }

    #[test]
    fn mounted_content_enters_the_tables_and_leaves_the_disc_layout() {
        let (db, mut p, elise, sam, suit) = setup();
        p.adopt_mod_state(&db);
        assert!(p.owned_items.contains(&sam) && p.unlocked_riders.contains(&sam) && p.owned_items.contains(&suit));
        assert_eq!((p.riders[&sam].money_earned, p.riders[&sam].max_achieved_level), (1200, 2));
        assert_eq!(p.riders[&sam].last_suit, (suit, 0));
        assert_eq!(p.riders[&elise].last_board, (suit, 0));
        assert_eq!(p.attribs.get(&ATTRIB_LAST_RIDER), Some(&i64::from(sam)));
        let disc = p.disc_view();
        assert!(!disc.owned_items.contains(&sam) && !disc.owned_items.contains(&suit));
        assert!(!disc.unlocked_riders.contains(&sam) && !disc.riders.contains_key(&sam));
        assert_eq!(disc.attribs.get(&ATTRIB_LAST_RIDER), Some(&i64::from(elise)));
        assert_eq!(disc.riders[&elise].last_board, (0, SRC_NONE));
        assert!(disc.mod_state.entries.is_empty());
        let st = p.state_to_write();
        for (id, fields) in &p.mod_state.entries {
            for (k, v) in fields {
                assert_eq!(st.get(id, k), Some(v), "{id} {k}");
            }
        }
        assert_eq!(st.int(&id("sam:character/sam"), "board_src"), Some(i64::from(SRC_NONE)));
        let again = ModState::parse(st.write().as_bytes()).unwrap();
        assert_eq!(again, st);
        let (_, mut q, ..) = setup();
        q.mod_state = again;
        q.adopt_mod_state(&db);
        assert_eq!(q.state_to_write(), st);
    }

    #[test]
    fn an_added_rider_starts_with_its_stand_ins_defaults() {
        let (mut db, _, elise, sam, _) = setup();
        let (suit, board, neon) =
            (fnv1(b"suit_default_elise"), fnv1(b"board_elise_default"), fnv1(b"proof:board/neon"));
        let exists = |k: u32| [suit, board, neon].contains(&k);
        let s = RiderStats::created_for(&db, sam, exists);
        assert_eq!((s.last_suit, s.last_board), ((suit, 0), (board, 0)));
        db.riders[1].content.as_mut().unwrap().default_board = Some(neon);
        let s = RiderStats::created_for(&db, sam, exists);
        assert_eq!((s.last_suit, s.last_board), ((suit, 0), (neon, 0)));
        assert_eq!(RiderStats::created_for(&db, elise, exists), RiderStats::created("elise", exists));
    }

    #[test]
    fn music_switches_live_in_the_state_by_switch_id() {
        let text = "format = 1\n[\"gone:music/old/1\"]\non = false\n[\"tunes:music/list/2\"]\non = 3\n\
                    [\"tunes:music/list/1\"]\non = false\n";
        let mut p = Profile { mod_state: ModState::parse(text.as_bytes()).unwrap(), ..Profile::default() };
        let keys = [id("tunes:music/list/1"), id("tunes:music/list/2"), id("tunes:music/list/3")];
        let (switches, reasons) = p.music_switches(&keys);
        assert_eq!(switches.iter().map(|(_, on)| *on).collect::<Vec<_>>(), [false, true, true]);
        assert_eq!(reasons.len(), 1);
        assert!(reasons[0].starts_with("tunes:music/list/2.on: an integer"), "{}", reasons[0]);
        let refused =
            p.set_music_switches(&[(keys[0].clone(), true), (keys[1].clone(), true), (keys[2].clone(), false)]);
        assert!(refused.is_empty());
        let st = p.state_to_write();
        assert_eq!(st.entries.len(), 2, "{}", st.write());
        assert_eq!(st.bool(&id("tunes:music/list/3"), "on"), Some(false));
        assert_eq!(st.bool(&id("gone:music/old/1"), "on"), Some(false), "a removed package's switch is kept");
        let back = Profile { mod_state: ModState::parse(st.write().as_bytes()).unwrap(), ..Profile::default() };
        assert_eq!(back.music_switches(&[id("gone:music/old/1")]).0, [(id("gone:music/old/1"), false)]);
    }

    #[test]
    fn a_removed_mod_loses_nothing_and_gains_nothing() {
        let (_, mut p, elise, sam, suit) = setup();
        p.adopt_mod_state(&CareerDb { riders: vec![rider(elise, "elise", None)], ..CareerDb::default() });
        assert!(!p.owned_items.contains(&sam) && !p.owned_items.contains(&suit) && !p.riders.contains_key(&sam));
        assert_eq!(p.riders[&elise].last_board, RiderStats::default().last_board);
        assert_eq!(p.state_to_write(), p.mod_state);
        let (db, mut p, _, sam, _) = setup();
        p.adopt_mod_state(&db);
        p.riders.get_mut(&sam).unwrap().money_earned = 5000;
        let st = p.state_to_write();
        assert_eq!(st.int(&id("sam:character/sam"), "money_earned"), Some(5000));
        assert_eq!(st.bool(&id("gone:character/old"), "owned"), Some(true));
    }
}
