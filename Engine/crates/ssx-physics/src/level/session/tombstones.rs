use crate::event::{EventDef, EventType, Rival};

pub const TOMBSTONE_ENTITIES: usize = 25;
pub const HIDDEN: f32 = -10_000.0;
pub const MEDAL_KEYS: [&str; 3] =
    ["TXT_EXPLORE_GHOST_MEDAL_GOLD", "TXT_EXPLORE_GHOST_MEDAL_SILVER", "TXT_EXPLORE_GHOST_MEDAL_BRONZE"];
pub const KIND_RIVAL: u32 = 4;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnRecord {
    pub score: u32,
    pub lap: u32,
    pub position: [f32; 3],
    pub name: String,
}

impl OwnRecord {
    pub fn from_row(score: u32, splits: [u32; 4], name: &str) -> Option<Self> {
        (score != u32::MAX).then(|| OwnRecord {
            score,
            lap: splits[0],
            position: [f32::from_bits(splits[1]), f32::from_bits(splits[2]), f32::from_bits(splits[3])],
            name: name.to_string(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TombstoneRecord {
    pub id: u64,
    pub name: String,
    pub name_is_text_id: bool,
    pub kind: u32,
    pub distance_m: u32,
    pub position: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tombstone {
    pub position: [f32; 3],
    pub shown: bool,
    pub id: u64,
    pub name: String,
    pub name_is_text_id: bool,
    pub distance_m: u32,
}

impl Default for Tombstone {
    fn default() -> Self {
        Tombstone {
            position: [HIDDEN; 3],
            shown: false,
            id: 0,
            name: String::new(),
            name_is_text_id: false,
            distance_m: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tombstones {
    pub lap: u32,
    pub pool: Vec<TombstoneRecord>,
    pub entities: Vec<Tombstone>,
    pub race_started: bool,
    start_pool: Vec<TombstoneRecord>,
}

impl Tombstones {
    pub fn for_event(def: &EventDef) -> Option<Self> {
        Self::for_event_with(def, None)
    }

    pub fn for_event_with(def: &EventDef, own: Option<&OwnRecord>) -> Option<Self> {
        let mut t = Tombstones {
            lap: 1,
            pool: Vec::new(),
            entities: vec![Tombstone::default(); TOMBSTONE_ENTITIES],
            race_started: false,
            start_pool: Vec::new(),
        };
        t.start_drop(def, own);
        if t.pool.is_empty() {
            return None;
        }
        t.start_pool = t.pool.clone();
        Some(t)
    }

    fn start_drop(&mut self, def: &EventDef, own: Option<&OwnRecord>) {
        use EventType::*;
        let Some(ty) = def.event_type else { return };
        if matches!(ty, RaceBots | TrickBots | RaceGhosts | TrickGhosts) {
            return;
        }
        if matches!(ty, SurviveToDeath | SurviveToBottom) {
            for (k, row) in leaderboard_order(&def.rivals, ty.ranks_ascending()).into_iter().enumerate() {
                let Some(name) = MEDAL_KEYS.get(k) else { break };
                self.add(row.lap, row.end_position(), row.value() / 1000, name, true);
            }
        }
        if let Some(o) = own {
            self.add(o.lap, o.position, o.score / 1000, &o.name, false);
        }
    }

    pub fn add(&mut self, lap: u32, position: [f32; 3], distance_m: u32, name: &str, name_is_text_id: bool) {
        if lap != self.lap.wrapping_sub(1) {
            return;
        }
        self.pool.push(TombstoneRecord {
            id: u64::from(distance_m),
            name: name.into(),
            name_is_text_id,
            kind: KIND_RIVAL,
            distance_m,
            position,
        });
    }

    pub fn on_race_start(&mut self) {
        if !self.race_started {
            self.place();
            self.race_started = true;
        }
    }

    fn place(&mut self) {
        let n = self.pool.len().min(self.entities.len());
        for (e, r) in self.entities.iter_mut().zip(&self.pool) {
            e.position = r.position;
            e.shown = true;
            e.distance_m = if matches!(r.kind, 2 | 4) { r.distance_m } else { 0 };
            e.name = r.name.clone();
            e.name_is_text_id = r.name_is_text_id;
            e.id = r.id;
        }
        for e in &mut self.entities[n..] {
            (e.position, e.shown) = ([HIDDEN; 3], false);
        }
    }

    pub fn on_loop(&mut self) {
        self.pool.clear();
        self.lap += 1;
    }

    pub fn on_run_ended(&mut self) {
        self.race_started = false;
    }

    pub fn reset(&mut self) {
        self.lap = 1;
        self.pool = self.start_pool.clone();
        self.entities = vec![Tombstone::default(); TOMBSTONE_ENTITIES];
        self.race_started = false;
    }

    pub fn hud_running(&self) -> bool {
        self.race_started
    }

    pub fn hud_entities(&self) -> &[Tombstone] {
        &self.entities
    }

    pub fn shown(&self) -> impl Iterator<Item = &Tombstone> {
        self.entities.iter().filter(|e| e.shown)
    }
}

pub fn leaderboard_order(rows: &[Rival], ascending: bool) -> Vec<&Rival> {
    let mut v: Vec<&Rival> = rows.iter().collect();
    v.sort_by(|a, b| {
        let (a, b) = (a.value(), b.value());
        match (a == u32::MAX, b == u32::MAX) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ if ascending => a.cmp(&b),
            _ => b.cmp(&a),
        }
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(score: u32, lap: u32, at: [f32; 3]) -> Rival {
        Rival {
            starting_idx: 0,
            ghost_id: 1,
            character: 0,
            catchup: 0,
            score,
            tuned_score: u32::MAX,
            lap,
            end_bits: at.map(f32::to_bits),
        }
    }

    fn survive(rows: Vec<Rival>) -> EventDef {
        EventDef {
            name: "exp_test_survive".into(),
            key: 0,
            drop_key: 0,
            event_type: Some(EventType::SurviveToDeath),
            scripts: Vec::new(),
            rivals: rows,
            required_equipment: Vec::new(),
            avalanche_scripts: Vec::new(),
            forced_rider: 0,
            forced_items: [0; 5],
            loadout: None,
            objectives: None,
            message: Default::default(),
        }
    }

    #[test]
    fn the_first_laps_rows_become_medal_tombstones_at_the_race_start() {
        let def = survive(vec![
            row(909_130, 0, [1.0, 2.0, 3.0]),
            row(4_365_093, 1, [4.0, 5.0, 6.0]),
            row(2_044_606, 0, [7.0, 8.0, 9.0]),
        ]);
        let mut t = Tombstones::for_event(&def).expect("two first-lap rows");
        let names: Vec<_> = t.pool.iter().map(|r| (r.name.as_str(), r.distance_m)).collect();
        assert_eq!(names, [("TXT_EXPLORE_GHOST_MEDAL_SILVER", 2044), ("TXT_EXPLORE_GHOST_MEDAL_BRONZE", 909)]);
        assert!(t.shown().next().is_none(), "nothing placed before the race start");
        t.on_race_start();
        let shown: Vec<_> = t.shown().map(|e| (e.position, e.distance_m, e.id)).collect();
        assert_eq!(shown, [([7.0, 8.0, 9.0], 2044, 2044), ([1.0, 2.0, 3.0], 909, 909)]);
        assert_eq!(t.entities.len(), TOMBSTONE_ENTITIES);
        assert_eq!(t.entities[2].position, [HIDDEN; 3]);
        t.on_loop();
        t.on_race_start();
        assert_eq!((t.lap, t.pool.len(), t.shown().count()), (2, 0, 2));
        t.on_run_ended();
        assert!(!t.hud_running());
        t.reset();
        assert_eq!((t.lap, t.pool.len(), t.shown().count()), (1, 2, 0));
    }

    #[test]
    fn races_and_rows_of_later_laps_only_make_no_tombstones() {
        let mut d = survive(vec![row(5_000_000, 2, [0.0; 3])]);
        assert!(Tombstones::for_event(&d).is_none());
        d.event_type = Some(EventType::RaceGhosts);
        d.rivals = vec![row(80_000, 0, [0.0; 3])];
        assert!(Tombstones::for_event(&d).is_none());
    }

    #[test]
    fn the_players_own_record_follows_the_rivals_for_the_types_without_ghost_requests() {
        let at = [10.0f32, 20.0, 30.0];
        let splits = [0, at[0].to_bits(), at[1].to_bits(), at[2].to_bits()];
        let own = OwnRecord::from_row(1_234_567, splits, "Player One").expect("a recorded score");
        assert_eq!((own.lap, own.position), (0, at));
        assert!(OwnRecord::from_row(u32::MAX, splits, "x").is_none(), "SCORE −1: no record (0x824A7B20)");
        let def = survive(vec![row(909_130, 0, [1.0, 2.0, 3.0])]);
        let t = Tombstones::for_event_with(&def, Some(&own)).expect("records");
        let pool: Vec<_> = t.pool.iter().map(|r| (r.name.as_str(), r.name_is_text_id, r.distance_m, r.kind)).collect();
        assert_eq!(pool, [("TXT_EXPLORE_GHOST_MEDAL_GOLD", true, 909, 4), ("Player One", false, 1234, 4)]);
        let mut race = survive(Vec::new());
        race.event_type = Some(EventType::RaceClock);
        assert_eq!(Tombstones::for_event_with(&race, Some(&own)).map(|t| t.pool.len()), Some(1));
        race.event_type = Some(EventType::RaceGhosts);
        assert!(Tombstones::for_event_with(&race, Some(&own)).is_none());
        let later = OwnRecord { lap: 1, ..own };
        race.event_type = Some(EventType::RaceClock);
        assert!(Tombstones::for_event_with(&race, Some(&later)).is_none());
    }
}
