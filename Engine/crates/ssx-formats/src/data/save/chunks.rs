use crate::reader::Reader;
use crate::t3db::Database;
use crate::{Error, Result};

fn put(out: &mut Vec<u8>, w: u32) {
    out.extend(w.to_be_bytes());
}

fn pairs(r: &mut Reader) -> Result<Vec<(u32, u32)>> {
    let n = r.u32()? as usize;
    (0..n).map(|_| Ok((r.u32()?, r.u32()?))).collect()
}

fn put_pairs(out: &mut Vec<u8>, v: &[(u32, u32)]) {
    put(out, v.len() as u32);
    for &(k, x) in v {
        put(out, k);
        put(out, x);
    }
}

fn done(r: &Reader, what: &str) -> Result<()> {
    if r.remaining() != 0 {
        return Err(Error::invalid(format!("save: {what} chunk has {} bytes left", r.remaining())));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseChunk {
    pub stream: Vec<u8>,
}

impl DatabaseChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let n = r.u32()? as usize;
        let stream = r.bytes(n)?.to_vec();
        done(&r, "database")?;
        Ok(DatabaseChunk { stream })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.stream.len() + 4);
        put(&mut out, self.stream.len() as u32);
        out.extend(&self.stream);
        out
    }

    pub fn database(&self) -> Result<Database> {
        Database::parse(&self.stream)
    }

    pub fn from_database(db: &Database) -> Self {
        DatabaseChunk { stream: db.write() }
    }
}

pub const ONLINE_BYTES: usize = 0x410;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlowChunk {
    pub peaks: Vec<(u32, [u32; 5])>,
    pub ranges: Vec<(u32, [u32; 5])>,
    pub drops: Vec<(u32, u32)>,
    pub chapter_drops: Vec<u32>,
    pub counts: Vec<(u32, u32, u32, u8)>,
    pub bought: Vec<(u32, u8)>,
    pub time: u32,
    pub new_game_state: u32,
    pub focus: [Vec<(u32, u32)>; 3],
    pub current_range: u32,
    pub last_drop: u32,
    pub audio: Vec<u8>,
}

impl FlowChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let modes = |r: &mut Reader| -> Result<Vec<(u32, [u32; 5])>> {
            let n = r.u32()? as usize;
            (0..n).map(|_| Ok((r.u32()?, [r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?]))).collect()
        };
        let peaks = modes(&mut r)?;
        let ranges = modes(&mut r)?;
        let drops = pairs(&mut r)?;
        let n = r.u32()? as usize;
        let chapter_drops = (0..n).map(|_| r.u32()).collect::<Result<_>>()?;
        let n = r.u32()? as usize;
        let counts = (0..n).map(|_| Ok((r.u32()?, r.u32()?, r.u32()?, r.u8()?))).collect::<Result<_>>()?;
        let n = r.u32()? as usize;
        let bought = (0..n).map(|_| Ok((r.u32()?, r.u8()?))).collect::<Result<_>>()?;
        let (time, new_game_state) = (r.u32()?, r.u32()?);
        let focus = [pairs(&mut r)?, pairs(&mut r)?, pairs(&mut r)?];
        let (current_range, last_drop) = (r.u32()?, r.u32()?);
        let audio = r.bytes(r.remaining())?.to_vec();
        Ok(FlowChunk {
            peaks,
            ranges,
            drops,
            chapter_drops,
            counts,
            bought,
            time,
            new_game_state,
            focus,
            current_range,
            last_drop,
            audio,
        })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for map in [&self.peaks, &self.ranges] {
            put(&mut out, map.len() as u32);
            for (k, words) in map {
                put(&mut out, *k);
                words.iter().for_each(|&w| put(&mut out, w));
            }
        }
        put_pairs(&mut out, &self.drops);
        put(&mut out, self.chapter_drops.len() as u32);
        self.chapter_drops.iter().for_each(|&d| put(&mut out, d));
        put(&mut out, self.counts.len() as u32);
        for &(d, c, f, u) in &self.counts {
            put(&mut out, d);
            put(&mut out, c);
            put(&mut out, f);
            out.push(u);
        }
        put(&mut out, self.bought.len() as u32);
        for &(d, b) in &self.bought {
            put(&mut out, d);
            out.push(b);
        }
        put(&mut out, self.time);
        put(&mut out, self.new_game_state);
        self.focus.iter().for_each(|m| put_pairs(&mut out, m));
        put(&mut out, self.current_range);
        put(&mut out, self.last_drop);
        out.extend(&self.audio);
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatRecord {
    pub key: u32,
    pub value: u32,
    pub key_again: u32,
    pub scope: u32,
    pub kind: u32,
    pub initial: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatsChunk {
    pub awards: Vec<(u32, u32)>,
    pub persistent: Vec<StatRecord>,
}

impl StatsChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let awards = pairs(&mut r)?;
        let n = r.u32()? as usize;
        let persistent = (0..n)
            .map(|_| {
                Ok(StatRecord {
                    key: r.u32()?,
                    value: r.u32()?,
                    key_again: r.u32()?,
                    scope: r.u32()?,
                    kind: r.u32()?,
                    initial: r.u32()?,
                })
            })
            .collect::<Result<_>>()?;
        done(&r, "stats")?;
        Ok(StatsChunk { awards, persistent })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_pairs(&mut out, &self.awards);
        put(&mut out, self.persistent.len() as u32);
        for s in &self.persistent {
            [s.key, s.value, s.key_again, s.scope, s.kind, s.initial].iter().for_each(|&w| put(&mut out, w));
        }
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BadgeChunk {
    pub badges: Vec<u32>,
}

impl BadgeChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let n = r.u32()? as usize;
        let badges = (0..n).map(|_| r.u32()).collect::<Result<_>>()?;
        done(&r, "badges")?;
        Ok(BadgeChunk { badges })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put(&mut out, self.badges.len() as u32);
        self.badges.iter().for_each(|&b| put(&mut out, b));
        out
    }
}

pub const GEOTAG_SLOTS: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeotagChunk {
    pub slots: Vec<u64>,
}

impl Default for GeotagChunk {
    fn default() -> Self {
        GeotagChunk { slots: vec![0; GEOTAG_SLOTS] }
    }
}

impl GeotagChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let n = r.u32()? as usize;
        if n != GEOTAG_SLOTS * 8 {
            return Err(Error::invalid(format!("save: geotag chunk of {n} bytes, the game writes 16000")));
        }
        let slots = (0..GEOTAG_SLOTS).map(|_| r.u64()).collect::<Result<_>>()?;
        done(&r, "geotag")?;
        Ok(GeotagChunk { slots })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.slots.len() * 8);
        put(&mut out, (self.slots.len() * 8) as u32);
        self.slots.iter().for_each(|s| out.extend(s.to_be_bytes()));
        out
    }

    pub fn sets(&self) -> (Vec<u64>, Vec<u64>) {
        let half = |r: std::ops::Range<usize>| self.slots[r].iter().copied().filter(|&k| k != 0).collect();
        (half(0..GEOTAG_SLOTS / 2), half(GEOTAG_SLOTS / 2..GEOTAG_SLOTS))
    }

    pub fn from_sets(collected: &[u64], online: &[u64]) -> Self {
        let mut c = GeotagChunk::default();
        for (i, &k) in collected.iter().take(GEOTAG_SLOTS / 2).enumerate() {
            c.slots[i] = k;
        }
        for (i, &k) in online.iter().take(GEOTAG_SLOTS / 2).enumerate() {
            c.slots[GEOTAG_SLOTS / 2 + i] = k;
        }
        c
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FriendsChunk {
    pub time: u32,
    pub ring: Vec<u32>,
    pub map: Vec<(u32, u32)>,
    pub unk_skipped: u32,
}

impl FriendsChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let time = r.u32()?;
        let n = r.u32()? as usize;
        let ring = (0..n).map(|_| r.u32()).collect::<Result<_>>()?;
        let map = pairs(&mut r)?;
        let unk_skipped = r.u32()?;
        done(&r, "friends")?;
        Ok(FriendsChunk { time, ring, map, unk_skipped })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put(&mut out, self.time);
        put(&mut out, self.ring.len() as u32);
        self.ring.iter().for_each(|&w| put(&mut out, w));
        put_pairs(&mut out, &self.map);
        put(&mut out, self.unk_skipped);
        out
    }
}

pub const TUNING_BYTES: usize = 0x25800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuningChunk {
    pub unk_word: u32,
    pub buffer: Vec<u8>,
}

impl Default for TuningChunk {
    fn default() -> Self {
        TuningChunk { unk_word: 0, buffer: vec![0; TUNING_BYTES] }
    }
}

impl TuningChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let unk_word = r.u32()?;
        let buffer = r.bytes(TUNING_BYTES)?.to_vec();
        done(&r, "tuning")?;
        Ok(TuningChunk { unk_word, buffer })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.buffer.len());
        put(&mut out, self.unk_word);
        out.extend(&self.buffer);
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivityChunk {
    pub ids: Vec<u64>,
    pub unk_words: [u32; 3],
}

impl ActivityChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let n = r.u32()? as usize;
        let ids = (0..n).map(|_| r.u64()).collect::<Result<_>>()?;
        let unk_words = [r.u32()?, r.u32()?, r.u32()?];
        done(&r, "activity")?;
        Ok(ActivityChunk { ids, unk_words })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put(&mut out, self.ids.len() as u32);
        self.ids.iter().for_each(|i| out.extend(i.to_be_bytes()));
        self.unk_words.iter().for_each(|&w| put(&mut out, w));
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrontEndChunk {
    pub controller_scheme: u32,
    pub low_survive_never_show: u8,
    pub restart_never_show: u8,
}

impl FrontEndChunk {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let c =
            FrontEndChunk { controller_scheme: r.u32()?, low_survive_never_show: r.u8()?, restart_never_show: r.u8()? };
        done(&r, "front end")?;
        Ok(c)
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = self.controller_scheme.to_be_bytes().to_vec();
        out.push(self.low_survive_never_show);
        out.push(self.restart_never_show);
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeySetChunk {
    pub keys: Vec<u32>,
}

impl KeySetChunk {
    pub fn parse(data: &[u8], after: &[u8]) -> Result<Self> {
        let mut padded = data.to_vec();
        padded.extend((0..3).map(|i| after.get(i).copied().unwrap_or(0)));
        let mut r = Reader::be(&padded);
        let n = r.u32()? as usize;
        if data.len() != 4 * n + 1 {
            return Err(Error::invalid(format!("save: key set of {n} keys in {} bytes", data.len())));
        }
        let keys = (0..n).map(|_| r.u32()).collect::<Result<_>>()?;
        Ok(KeySetChunk { keys })
    }

    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 4 * self.keys.len());
        put(&mut out, self.keys.len() as u32);
        self.keys.iter().for_each(|&k| put(&mut out, k));
        out
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = self.bytes();
        out.truncate(4 * self.keys.len() + 1);
        out
    }

    pub fn spill(&self) -> [u8; 3] {
        let b = self.bytes();
        [b[b.len() - 3], b[b.len() - 2], b[b.len() - 1]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_round_trip() {
        let flow = FlowChunk {
            peaks: vec![(7, [0, 1, 0, 1, 0])],
            ranges: vec![(9, [0, 2, 0, 0, 0])],
            drops: vec![(1, 2), (5, 1)],
            chapter_drops: vec![1, 5],
            counts: vec![(1, 3, 1, 0)],
            bought: vec![(77, 1)],
            time: 1234,
            new_game_state: 22,
            focus: [vec![(9, 7)], vec![(7, 1)], vec![]],
            current_range: 9,
            last_drop: 1,
            audio: vec![1, 2, 3],
        };
        assert_eq!(FlowChunk::parse(&flow.write()).unwrap(), flow);
        assert_eq!(flow.write().len(), 4 + 24 + 4 + 24 + 4 + 16 + 4 + 8 + 4 + 13 + 4 + 5 + 8 + 12 + 12 + 4 + 8 + 3);
        let stats = StatsChunk {
            awards: vec![(1, 1 << 24)],
            persistent: vec![StatRecord { key: 5, value: 3, key_again: 5, scope: 2, kind: 0, initial: 0 }],
        };
        assert_eq!(StatsChunk::parse(&stats.write()).unwrap(), stats);
        let geo = GeotagChunk::from_sets(&[3, 9], &[4]);
        assert_eq!(GeotagChunk::parse(&geo.write()).unwrap().sets(), (vec![3, 9], vec![4]));
        assert_eq!(geo.write().len(), 16004);
        let fr = FriendsChunk { time: 5, ring: vec![1, 2], map: vec![(3, 4)], unk_skipped: 0 };
        assert_eq!(FriendsChunk::parse(&fr.write()).unwrap(), fr);
        let act = ActivityChunk { ids: vec![1 << 40], unk_words: [1, 2, 3] };
        assert_eq!(ActivityChunk::parse(&act.write()).unwrap(), act);
        let fe = FrontEndChunk { controller_scheme: 1, low_survive_never_show: 1, restart_never_show: 0 };
        assert_eq!(FrontEndChunk::parse(&fe.write()).unwrap(), fe);
        assert_eq!(BadgeChunk::parse(&BadgeChunk { badges: vec![8] }.write()).unwrap().badges, vec![8]);
        assert_eq!(TuningChunk::parse(&TuningChunk::default().write()).unwrap(), TuningChunk::default());
    }

    #[test]
    fn the_key_sets_last_key_spills_three_bytes_past_the_chunk() {
        assert_eq!(KeySetChunk::default().write(), vec![0]);
        assert_eq!(KeySetChunk::default().spill(), [0; 3]);
        assert_eq!(KeySetChunk::parse(&[0], &[]).unwrap().keys, Vec::<u32>::new());
        let k = KeySetChunk { keys: vec![0x1122_3344, 0x5566_7788] };
        let bytes = k.write();
        assert_eq!(bytes.len(), 9);
        assert_eq!(k.spill(), [0x66, 0x77, 0x88]);
        assert_eq!(KeySetChunk::parse(&bytes, &k.spill()).unwrap(), k);
        assert_eq!(KeySetChunk::parse(&bytes, &[]).unwrap().keys, vec![0x1122_3344, 0x5500_0000]);
    }
}
