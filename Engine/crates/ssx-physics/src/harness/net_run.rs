use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};

use crate::graph::{LiveBlocks, RiderGraph, RiderRigOps};
use crate::rider::RiderState;
use crate::session::{Execution, Session};

pub const MAGIC: &[u8; 4] = b"NRUN";
pub const VERSION: u32 = 1;
pub const RECORD_BYTES: usize = 4 + 11 * 4 + POSE_BLOCK_SIZE;
pub const MAX_EXECUTIONS: usize = 30 * 3600;
const MAX_STRING: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct NetRunExec {
    pub block: Option<PoseBlock>,
    pub teleport: bool,
    pub respawn: bool,
    pub rewinding: bool,
    pub impact: bool,
    pub epoch: u8,
    pub waiting_before: bool,
    pub waiting_after: bool,
    pub handover: bool,
    pub race_start: bool,
    pub split: Option<(u8, u32)>,
    pub combo_end: bool,
    pub result: Option<(bool, u32, u32, u32)>,
    pub clock_ms: u32,
    pub points: u32,
    pub descent_mm: u32,
    pub rewinds: u16,
    pub loops: u16,
    pub dt: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetRun {
    pub event: String,
    pub track: String,
    pub character: String,
    pub execs: Vec<NetRunExec>,
}

impl NetRun {
    pub fn go(&self) -> Option<usize> {
        self.execs.iter().position(|e| e.waiting_before && !e.waiting_after)
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        let file = std::fs::File::create(path).with_context(|| format!("writing {}", path.display()))?;
        let mut w = std::io::BufWriter::new(file);
        w.write_all(MAGIC)?;
        w.write_all(&VERSION.to_le_bytes())?;
        for s in [&self.event, &self.track, &self.character] {
            if s.len() > MAX_STRING {
                bail!("a run's name of {} bytes", s.len());
            }
            w.write_all(&(s.len() as u16).to_le_bytes())?;
            w.write_all(s.as_bytes())?;
        }
        if self.execs.len() > MAX_EXECUTIONS {
            bail!("{} executions", self.execs.len());
        }
        w.write_all(&(self.execs.len() as u32).to_le_bytes())?;
        for e in &self.execs {
            let f1 = [
                e.block.is_some(),
                e.teleport,
                e.respawn,
                e.rewinding,
                e.impact,
                e.waiting_before,
                e.waiting_after,
                e.handover,
            ];
            let f2 = [e.race_start, e.split.is_some(), e.combo_end, e.result.is_some(), e.result.is_some_and(|r| r.0)];
            let bits = |b: &[bool]| b.iter().enumerate().fold(0u8, |m, (i, &on)| m | (u8::from(on) << i));
            w.write_all(&[bits(&f1), bits(&f2), e.epoch, e.split.map_or(0, |s| s.0)])?;
            let (_, rms, rpts, rdesc) = e.result.unwrap_or_default();
            let words = [
                e.split.map_or(0, |s| s.1),
                e.clock_ms,
                e.points,
                e.descent_mm,
                u32::from(e.rewinds),
                u32::from(e.loops),
                e.dt.to_bits(),
                rms,
                rpts,
                rdesc,
                0,
            ];
            for x in words {
                w.write_all(&x.to_le_bytes())?;
            }
            match &e.block {
                Some(b) => w.write_all(&b.to_bytes())?,
                None => w.write_all(&[0u8; POSE_BLOCK_SIZE])?,
            }
        }
        w.flush()?;
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::parse(&data).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut c = Cursor { data, at: 0 };
        if c.take(4)? != MAGIC {
            bail!("not a run file");
        }
        if c.word()? != VERSION {
            bail!("a run file of another version");
        }
        let mut strings = Vec::new();
        for _ in 0..3 {
            let n = usize::from(u16::from_le_bytes(c.take(2)?.try_into().context("a length")?));
            if n > MAX_STRING {
                bail!("a name of {n} bytes");
            }
            strings.push(String::from_utf8(c.take(n)?.to_vec()).context("a name")?);
        }
        let count = c.word()? as usize;
        let rest = data.len() - c.at;
        if count > MAX_EXECUTIONS || count.checked_mul(RECORD_BYTES) != Some(rest) {
            bail!("{count} executions in {rest} bytes");
        }
        let mut execs = Vec::with_capacity(count);
        for _ in 0..count {
            let r = c.take(RECORD_BYTES)?;
            let (f1, f2, epoch, split) = (r[0], r[1], r[2], r[3]);
            let w = |k: usize| -> Result<u32> { word(&r[4 + 4 * k..8 + 4 * k]) };
            let bit = |f: u8, i: u8| f & (1 << i) != 0;
            let block = match bit(f1, 0) {
                true => Some(PoseBlock::parse(&r[48..])?),
                false => None,
            };
            execs.push(NetRunExec {
                block,
                teleport: bit(f1, 1),
                respawn: bit(f1, 2),
                rewinding: bit(f1, 3),
                impact: bit(f1, 4),
                waiting_before: bit(f1, 5),
                waiting_after: bit(f1, 6),
                handover: bit(f1, 7),
                race_start: bit(f2, 0),
                split: if bit(f2, 1) { Some((split, w(0)?)) } else { None },
                combo_end: bit(f2, 2),
                result: if bit(f2, 3) { Some((bit(f2, 4), w(7)?, w(8)?, w(9)?)) } else { None },
                clock_ms: w(1)?,
                points: w(2)?,
                descent_mm: w(3)?,
                rewinds: u16::try_from(w(4)?).context("rewinds")?,
                loops: u16::try_from(w(5)?).context("loops")?,
                dt: f32::from_bits(w(6)?),
                epoch,
            });
        }
        let [event, track, character]: [String; 3] = strings.try_into().map_err(|_| anyhow::anyhow!("names"))?;
        Ok(Self { event, track, character, execs })
    }
}

fn word(b: &[u8]) -> Result<u32> {
    Ok(u32::from_le_bytes(b.try_into().context("a word")?))
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).context("overflow")?;
        let s = self.data.get(self.at..end).context("the file ends early")?;
        self.at = end;
        Ok(s)
    }

    fn word(&mut self) -> Result<u32> {
        word(self.take(4)?)
    }
}

pub fn waiting(s: &RiderState) -> bool {
    s.start.as_ref().is_some_and(|l| l.intro.as_ref().map_or(l.idle(), |i| i.waits_for_go()))
}

pub struct NetRunRecorder {
    pub run: NetRun,
    blocks: LiveBlocks,
    rig_ops: RiderRigOps,
    waiting_before: bool,
    at_start_before: bool,
}

impl NetRunRecorder {
    pub fn new(big_root: &Path, event: &str, track: &str, character: &str, graph: &RiderGraph) -> Result<Self> {
        let (rig_ops, _) = RiderRigOps::load(big_root, track, character, graph)?;
        Ok(Self::with_rig_ops(event, track, character, rig_ops))
    }

    pub fn with_rig_ops(event: &str, track: &str, character: &str, rig_ops: RiderRigOps) -> Self {
        let run = NetRun { event: event.into(), track: track.into(), character: character.into(), execs: Vec::new() };
        Self { run, blocks: LiveBlocks::default(), rig_ops, waiting_before: false, at_start_before: false }
    }

    pub fn before(&mut self, s: &RiderState) {
        (self.waiting_before, self.at_start_before) = (waiting(s), s.start.is_some());
    }

    pub fn after(&mut self, i: usize, s: &RiderState, g: &RiderGraph, e: &Execution, session: &Session) -> Result<()> {
        if self.run.execs.len() >= MAX_EXECUTIONS {
            return Ok(());
        }
        let posed = self.rig_ops.apply(s, g);
        let _ = self.blocks.record(i as u64 + 1, s, g, &posed, e, e.rewind.active);
        let sample = self.blocks.take();
        let x = &session.event;
        let result =
            e.event.results.then(|| x.result.as_ref().map(|r| (r.failed, r.time_ms, r.points, r.descent_mm))).flatten();
        self.run.execs.push(NetRunExec {
            block: sample.as_ref().map(|b| b.block.clone()),
            teleport: sample.as_ref().is_some_and(|b| b.teleport),
            respawn: sample.as_ref().is_some_and(|b| b.respawn),
            rewinding: sample.as_ref().is_some_and(|b| b.rewinding),
            impact: sample.as_ref().is_some_and(|b| b.impact),
            epoch: sample.as_ref().map_or(self.blocks.epoch, |b| b.epoch),
            waiting_before: self.waiting_before,
            waiting_after: waiting(s),
            handover: self.at_start_before && s.start.is_none(),
            race_start: e.race_start,
            split: e.event.split.and_then(|(k, ms)| u8::try_from(k).ok().map(|k| (k, ms))),
            combo_end: s.score.combo_ended.is_some(),
            result,
            clock_ms: x.clock.ms(),
            points: s.score.score.max(0.0) as u32,
            descent_mm: x.descent_mm(s.score.descent),
            rewinds: u16::try_from(x.rewinds).unwrap_or(u16::MAX),
            loops: u16::try_from(x.loops).unwrap_or(u16::MAX),
            dt: e.dt,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_round_trips_and_damage_is_refused() {
        let exec = |i: u32| NetRunExec {
            block: None,
            teleport: i.is_multiple_of(2),
            respawn: false,
            rewinding: i == 3,
            impact: false,
            epoch: 2,
            waiting_before: i == 0,
            waiting_after: false,
            handover: i == 1,
            race_start: i == 2,
            split: (i == 3).then_some((4, 1234)),
            combo_end: false,
            result: (i == 4).then_some((false, 9000, 77, 55)),
            clock_ms: i * 33,
            points: i,
            descent_mm: 10 * i,
            rewinds: 1,
            loops: 0,
            dt: 2.0,
        };
        let run =
            NetRun { event: "e".into(), track: "t".into(), character: "c".into(), execs: (0..5).map(exec).collect() };
        let dir = std::env::temp_dir().join(format!("ssx-net-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("r.nrun");
        run.write(&p).unwrap();
        let back = NetRun::read(&p).unwrap();
        assert_eq!(back, run);
        assert_eq!(back.go(), Some(0));
        let data = std::fs::read(&p).unwrap();
        assert!(NetRun::parse(&data[..data.len() - 1]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
