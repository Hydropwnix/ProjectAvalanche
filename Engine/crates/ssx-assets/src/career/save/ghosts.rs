use anyhow::{Result, anyhow, bail};
use ssx_formats::save::ghosts::{CommonChunk, SLOTS};
use ssx_formats::t3db::{Database, FLAG_WRITTEN};

use super::{Coding, SaveSchema};

pub const COMMON_GROUP: &str = "SSXV_COMMON";
const TABLE: &str = "common_explore_scores";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GhostRow {
    pub score: u32,
    pub slot: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnGhosts {
    slots: Vec<u32>,
    db: Database,
    names: Names,
}

#[derive(Clone, Debug, PartialEq)]
struct Names {
    table: String,
    track: (String, Coding),
    score: (String, Coding),
    ghost: (String, Coding),
    ghost_bits: u32,
}

impl Names {
    fn of(schema: &SaveSchema) -> Result<Self> {
        let ghost = schema.field(TABLE, "GHOSTID")?;
        Ok(Names {
            table: schema.table_short(TABLE)?,
            track: schema.coding(TABLE, "TRACKID")?,
            score: schema.coding(TABLE, "SCORE")?,
            ghost: (ghost.short.clone(), Coding::of(ghost)),
            ghost_bits: ghost.depth,
        })
    }
}

impl OwnGhosts {
    pub fn fresh(schema: &SaveSchema) -> Result<Self> {
        let names = Names::of(schema)?;
        let mut db = schema.database.clone();
        for t in &mut db.tables {
            let short = t.name_str();
            match schema.meta.tables.iter().find(|m| m.short == short) {
                Some(m) if m.save_groups.as_deref() == Some(COMMON_GROUP) => {
                    t.reserve(m.max_insert.unwrap_or(0).clamp(0, 0xFFFF) as u16);
                }
                _ => t.flags &= !FLAG_WRITTEN,
            }
        }
        Ok(OwnGhosts { slots: vec![0; SLOTS], db, names })
    }

    pub fn from_chunk(chunk: &CommonChunk, schema: &SaveSchema) -> Result<Self> {
        let names = Names::of(schema)?;
        let db = Database::parse(&chunk.database)?;
        let own = OwnGhosts { slots: chunk.slots.clone(), db, names };
        let t = own.table()?;
        for f in [&own.names.track.0, &own.names.score.0, &own.names.ghost.0] {
            if t.field(f).is_none() {
                bail!("ssxvghosts: {TABLE} has no field {f}");
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in t.live_rows() {
            let (drop, row) = own.read_row(r)?;
            if !seen.insert(drop) {
                bail!("ssxvghosts: two rows of drop {drop:#010x}");
            }
            if let Some(s) = row.slot
                && own.slots[s] != drop
            {
                // PORT(choice): a row whose slot the table gives another drop is refused with the file; the console
                // writes a slot's key when it hands the slot out (0x824BFD68) and the row with it (0x825190E8), so
                // its files never differ there.
                bail!("ssxvghosts: drop {drop:#010x}'s ghost slot {s} holds drop {:#010x}", own.slots[s]);
            }
        }
        Ok(own)
    }

    pub fn to_chunk(&self) -> CommonChunk {
        CommonChunk { slots: self.slots.clone(), database: self.db.write() }
    }

    pub fn slots(&self) -> &[u32] {
        &self.slots
    }

    fn table(&self) -> Result<&ssx_formats::t3db::Table> {
        self.db.table(&self.names.table).ok_or_else(|| anyhow!("ssxvghosts: the database has no {TABLE}"))
    }

    fn read_row(&self, r: usize) -> Result<(u32, GhostRow)> {
        let t = self.table()?;
        let get = |(short, c): &(String, Coding)| -> Result<u32> {
            t.get(r, short).map(|v| c.decode(v)).ok_or_else(|| anyhow!("ssxvghosts: {TABLE} row {r} has no {short}"))
        };
        let drop = get(&self.names.track)?;
        let score = get(&self.names.score)?;
        let ghost = get(&self.names.ghost)?;
        let all_ones = self.names.ghost.1.decode(((1u64 << self.names.ghost_bits) - 1) as u32);
        let slot = if (ghost as usize) < SLOTS {
            Some(ghost as usize)
        } else if ghost == all_ones {
            None
        } else {
            // PORT(choice): a GHOSTID that is neither a slot (0…255) nor the −1 the keep stores (all nine bits) is
            // refused with the file; the console writes only those (0x825190E8 with 0x824BFD68's 0…255 or −1).
            bail!("ssxvghosts: drop {drop:#010x}'s GHOSTID {ghost} is no slot");
        };
        Ok((drop, GhostRow { score, slot }))
    }

    pub fn row(&self, drop: u32) -> Option<GhostRow> {
        let t = self.table().ok()?;
        t.live_rows().filter_map(|r| self.read_row(r).ok()).find(|(d, _)| *d == drop).map(|(_, row)| row)
    }

    pub fn own_slot(&self, drop: u32, profile_best: u32) -> Option<usize> {
        self.row(drop).filter(|r| r.score == profile_best).and_then(|r| r.slot)
    }

    pub fn keep(&mut self, drop: u32, score: u32) -> Result<Option<usize>> {
        let slot = match self.row(drop).and_then(|r| r.slot) {
            Some(s) => Some(s),
            None => self.allocate(drop),
        };
        self.set_row(drop, score, slot)?;
        Ok(slot)
    }

    fn allocate(&mut self, drop: u32) -> Option<usize> {
        let s = self.slots.iter().position(|&k| k == 0)?;
        self.slots[s] = drop;
        Some(s)
    }

    fn set_row(&mut self, drop: u32, score: u32, slot: Option<usize>) -> Result<()> {
        let names = self.names.clone();
        let doomed: Vec<usize> = {
            let t = self.table()?;
            t.live_rows().filter(|&r| t.get(r, &names.track.0).map(|v| names.track.1.decode(v)) == Some(drop)).collect()
        };
        let t = self.db.table_mut(&names.table).ok_or_else(|| anyhow!("ssxvghosts: the database has no {TABLE}"))?;
        for r in doomed {
            t.delete_row(r)?;
        }
        let r = t.insert_row()?;
        let ghost = slot.map_or(u32::MAX, |s| s as u32);
        t.put(r, &names.track.0, names.track.1.encode(drop))?;
        t.put(r, &names.score.0, names.score.1.encode(score))?;
        t.put(r, &names.ghost.0, names.ghost.1.encode(ghost))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::schema;
    use super::*;
    use crate::career::profile::{EventOutcome, EventRule, Profile};
    use crate::career::{DropType, Rules};

    const DROP: u32 = 0xAE9C_36C8;

    #[test]
    fn a_fresh_chunk_writes_only_the_common_table_with_its_rows_reserved() {
        let own = OwnGhosts::fresh(&schema()).unwrap();
        let c = own.to_chunk();
        let db = Database::parse(&c.database).unwrap();
        let written: Vec<(String, u16, u16)> = db
            .tables
            .iter()
            .filter(|t| t.flags & FLAG_WRITTEN != 0)
            .map(|t| (t.name_str(), t.row_count, t.rows_used))
            .collect();
        assert_eq!(written, [("tggg".to_string(), 300, 0)]);
        assert_eq!(c.slots, vec![0; SLOTS]);
        assert_eq!(OwnGhosts::from_chunk(&c, &schema()).unwrap().to_chunk(), c);
    }

    #[test]
    fn keep_takes_a_slot_once_and_replaces_the_row() {
        let s = schema();
        let mut own = OwnGhosts::fresh(&s).unwrap();
        assert_eq!(own.row(DROP), None);
        assert_eq!(own.keep(7, 50_000).unwrap(), Some(0));
        assert_eq!(own.keep(DROP, 73_320).unwrap(), Some(1));
        assert_eq!(own.row(DROP), Some(GhostRow { score: 73_320, slot: Some(1) }));
        assert_eq!(own.keep(DROP, 70_001).unwrap(), Some(1));
        assert_eq!(own.row(DROP), Some(GhostRow { score: 70_001, slot: Some(1) }));
        assert_eq!(&own.slots()[..3], [7, DROP, 0]);
        let c = own.to_chunk();
        let back = OwnGhosts::from_chunk(&c, &s).unwrap();
        assert_eq!(back.row(DROP), Some(GhostRow { score: 70_001, slot: Some(1) }));
        let t = back.table().unwrap();
        assert_eq!((t.rows_used, t.free_count, t.live_rows().count()), (2, 0, 2));
        assert_eq!(back.own_slot(DROP, 70_001), Some(1));
        assert_eq!(back.own_slot(DROP, 73_320), None);
        assert_eq!(back.own_slot(0x1234, 70_001), None);
    }

    #[test]
    fn a_full_slot_table_keeps_the_row_without_a_ghost() {
        let s = schema();
        let mut own = OwnGhosts::fresh(&s).unwrap();
        for k in 1..=SLOTS as u32 {
            assert_eq!(own.keep(k, 1).unwrap(), Some(k as usize - 1));
        }
        assert_eq!(own.keep(DROP, 9).unwrap(), None);
        let back = OwnGhosts::from_chunk(&own.to_chunk(), &s).unwrap();
        assert_eq!(back.row(DROP), Some(GhostRow { score: 9, slot: None }));
    }

    #[test]
    fn a_tampered_chunk_is_refused() {
        let s = schema();
        let mut own = OwnGhosts::fresh(&s).unwrap();
        own.keep(DROP, 5).unwrap();
        let mut c = own.to_chunk();
        c.slots[0] = 0x1111;
        assert!(OwnGhosts::from_chunk(&c, &s).is_err(), "a slot of another drop");
        let mut bad = own.clone();
        bad.set_row(0x2222, 5, Some(300)).unwrap();
        assert!(OwnGhosts::from_chunk(&bad.to_chunk(), &s).is_err(), "a GHOSTID beyond the table");
        let mut c = own.to_chunk();
        let at = c.database.len() / 2;
        c.database[at] ^= 0x10;
        assert!(OwnGhosts::from_chunk(&c, &s).is_err(), "the database's CRC");
    }

    fn rules(ghost_drop: bool) -> Rules {
        let mut r = Rules { explore_rewards: vec![[5000, 10000, 20000, 2500]; 2], ..Default::default() };
        let rule = EventRule {
            key: DROP,
            name: "exp_alps_grandgolliat_drop_7_race".into(),
            drop_type: DropType::Explore,
            event_type: 5,
            reward: 1,
            places: None,
            goal: None,
            cost: 0,
            peak: 0,
            range: 0,
            ghost_drop,
        };
        r.events.insert(DROP, rule);
        r
    }

    #[test]
    fn the_ghost_is_kept_on_a_new_best_only() {
        let race = |value: u32, failed: bool| EventOutcome {
            event: DROP,
            value,
            failed,
            leaderboard: vec![u32::MAX; 3],
            ..Default::default()
        };
        let r = rules(true);
        let mut p = Profile::default();
        assert!(p.apply_result(&r, &race(80_000, false)).keep_ghost, "the first finish");
        assert!(!p.apply_result(&r, &race(80_000, false)).keep_ghost, "an equal time");
        assert!(!p.apply_result(&r, &race(90_000, false)).keep_ghost, "a slower one");
        assert!(!p.apply_result(&r, &race(70_000, true)).keep_ghost, "a failed one");
        assert!(p.apply_result(&r, &race(70_000, false)).keep_ghost, "a faster one");
        let mut q = Profile::default();
        assert!(!q.apply_result(&rules(false), &race(80_000, false)).keep_ghost, "a drop without ghosts");
    }
}
