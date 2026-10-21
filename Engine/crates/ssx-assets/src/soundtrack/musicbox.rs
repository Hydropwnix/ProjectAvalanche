use std::path::Path;
use std::sync::Arc;

pub mod selection;

pub use selection::{RaceEvent, Session, session};

use anyhow::{Context, Result, bail};
use ssx_formats::eaac::Sps;
use ssx_formats::ealayer3_decode::SectionDecoder;
use ssx_formats::reader::Reader;
use ssx_formats::sbr::{SampleBank, SoundRef};
use ssx_formats::speech::{evt::EventSystem, grf::Graffiti, rep::RepetitionPools};

use crate::music::{Bytes, find_file};

#[derive(Clone, Debug)]
pub struct Song {
    pub id: u32,
    pub unk: [u32; 6],
    pub info: Vec<(String, String)>,
}

impl Song {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.info.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    pub fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("True")
    }
    pub fn label(&self) -> String {
        format!("{} - {}", self.get("Artist").unwrap_or("?"), self.get("Title").unwrap_or("?"))
    }

    pub fn level(&self) -> u32 {
        self.unk[2]
    }

    pub fn weight(&self) -> u32 {
        self.unk[4]
    }

    pub fn default_weight(&self) -> u32 {
        self.unk[3]
    }
}

pub fn select_levelled(songs: &[Song], candidates: &[u32]) -> Vec<u32> {
    let mut level = 1i32;
    let mut out = Vec::new();
    for &id in candidates {
        let Some(s) = songs.iter().find(|s| s.id == id) else { continue };
        if s.weight() == 0 || (s.level() as i32) < level {
            continue;
        }
        if s.level() as i32 > level {
            out.clear();
            level = s.level() as i32;
        }
        out.push(id);
        if s.level() == 3 {
            break;
        }
    }
    out
}

fn cstr(d: &[u8], o: usize) -> Result<String> {
    let s = d.get(o..).context("string offset past the end")?;
    let end = s.iter().position(|&b| b == 0).context("unterminated string")?;
    Ok(String::from_utf8_lossy(&s[..end]).into_owned())
}

pub fn parse_song_info(d: &[u8]) -> Result<Vec<Song>> {
    let mut r = Reader::be(d);
    r.expect(b"MBSI")?;
    let mut h = [0u32; 6];
    for x in &mut h {
        *x = r.u32()?;
    }
    let (size, first) = (h[4] as usize, h[5] as usize);
    if size != 0x24 {
        bail!("MBSI record size {size:#x}");
    }
    let mut songs = Vec::new();
    let mut end = d.len();
    let mut o = first;
    while o + size <= end {
        r.seek(o)?;
        let id = r.u32()?;
        let mut unk = [0u32; 6];
        for x in &mut unk {
            *x = r.u32()?;
        }
        let (count, table) = (r.u32()? as usize, r.u32()? as usize);
        end = end.min(table);
        let mut info = Vec::with_capacity(count);
        for p in 0..count {
            r.seek(table + 8 * p)?;
            let (k, v) = (r.u32()? as usize, r.u32()? as usize);
            info.push((cstr(d, k)?, cstr(d, v)?));
        }
        songs.push(Song { id, unk, info });
        o += size;
    }
    Ok(songs)
}

pub fn parse_graffiti_ids(d: &[u8]) -> Result<Vec<u32>> {
    let mut r = Reader::be(d);
    r.expect(b"GRF0")?;
    r.seek(0x30)?;
    if r.u32()? != 0x18 {
        bail!("GRF0: unexpected layout");
    }
    let n = r.u32()? as usize;
    (0..n).map(|_| Ok(r.u32()?)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RaceType {
    Menu,
    RaceIt,
    TrickIt,
    SurviveIt,
}

pub const TUTORIAL_DROPS: [u32; 2] = [0xC145_699A, 0xB0F3_51A2];

impl RaceType {
    pub fn from_event(ui_drop: u32, kind: i32) -> Option<Self> {
        let class = if TUTORIAL_DROPS.contains(&ui_drop) { 2 } else { kind };
        match class {
            1 | 5 => Some(RaceType::RaceIt),
            2 => Some(RaceType::TrickIt),
            3 | 4 => Some(RaceType::SurviveIt),
            _ => None,
        }
    }

    fn flag(self) -> Option<&'static str> {
        match self {
            RaceType::Menu => None,
            RaceType::RaceIt => Some("RaceIt"),
            RaceType::TrickIt => Some("TrickIt"),
            RaceType::SurviveIt => Some("SurviveIt"),
        }
    }
}

pub struct MusicBox {
    pub songs: Vec<Song>,
    pub selectable: Vec<u32>,
    pub selections: selection::SelectionData,
    pub events: EventSystem,
    pub graffiti: Graffiti,
    pub pools: RepetitionPools,
    bank: SampleBank,
    bank_data: Vec<u8>,
    sbs: Arc<Bytes>,
}

impl MusicBox {
    pub fn load(big_root: &Path) -> Result<Self> {
        let rel = |f: &str| format!("data/xenon/audio/music/{f}");
        let read = |f: &str| -> Result<Vec<u8>> {
            let p = find_file(big_root, &rel(f)).with_context(|| format!("{} not found", rel(f)))?;
            Ok(std::fs::read(p)?)
        };
        let songs = parse_song_info(&read("music_musicboxsonginfo.bin")?)?;
        let grf = read("music_graffitidata.bin")?;
        let selectable = parse_graffiti_ids(&grf)?;
        let graffiti = Graffiti::parse(&grf)?;
        let selections = selection::parse_selections(&read("music_musicboxselections.bin")?)?;
        let events = EventSystem::parse(&read("music_eventsubscr.bin")?)?;
        let pools = RepetitionPools::parse(&read("music_repetitionpools.bin")?)?;
        let bank_data = read("music_samples.sbr")?;
        let bank = SampleBank::parse(&bank_data)?;
        let sbs_path = find_file(big_root, &rel("music_samples.sbs")).context("music_samples.sbs not found")?;
        let sbs = Arc::new(Bytes::open(&sbs_path)?);
        Ok(Self { songs, selectable, selections, events, graffiti, pools, bank, bank_data, sbs })
    }

    pub fn song(&self, id: u32) -> Option<&Song> {
        self.songs.iter().find(|s| s.id == id)
    }

    pub fn set_event_class(&mut self, race: Option<RaceType>) {
        let flag = race.and_then(RaceType::flag);
        for s in self.songs.iter_mut().filter(|s| s.get("Index").is_some()) {
            s.unk[4] = match flag {
                Some(f) if s.flag(f) => 1000,
                Some(_) => 1,
                None => s.default_weight(),
            };
        }
    }

    pub fn apply_song_flags(&mut self, flags: &[(u32, bool)]) {
        for &(id, on) in flags.iter().filter(|(id, _)| *id != 0) {
            if let Some(s) = self.songs.iter_mut().find(|s| s.id == id) {
                s.unk[2] = u32::from(on);
            }
        }
    }

    pub fn duration(&self, id: u32) -> Option<f64> {
        let sps = Sps::read(&self.sbs, self.sps_offset(id).ok()?).ok()?;
        Some(f64::from(sps.header.num_samples) / f64::from(sps.header.sample_rate.max(1)))
    }

    fn sps_offset(&self, id: u32) -> Result<usize> {
        let s = self.bank.sounds.iter().find(|s| s.id == id).with_context(|| format!("no sound {id:#x}"))?;
        match self.bank.resolve(&self.bank_data, s)? {
            SoundRef::Sps { sps_offset } => Ok(sps_offset as usize),
            SoundRef::SnrSns { .. } => bail!("sound {id:#x} is not an SPS sound"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub ids: Vec<u32>,
    pub next: usize,
}

impl Playlist {
    pub fn new(mb: &MusicBox, race: RaceType, seed: u64) -> Self {
        let candidates: Vec<u32> = mb
            .selectable
            .iter()
            .copied()
            .filter(|&id| race.flag().is_none_or(|f| mb.song(id).is_some_and(|s| s.flag(f))))
            .collect();
        let mut ids = select_levelled(&mb.songs, &candidates);
        let mut x = seed | 1;
        for i in (1..ids.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ids.swap(i, (x % (i as u64 + 1)) as usize);
        }
        Self { ids, next: 0 }
    }
}

pub const MUSIC_SUBMIX_LEVEL: f32 = 0.5;

static PLAYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn playing() -> bool {
    PLAYING.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn paused() -> bool {
    PAUSED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_paused(on: bool) {
    PAUSED.store(on, std::sync::atomic::Ordering::Relaxed);
}

struct SongStream {
    id: u32,
    blocks: Vec<(usize, usize, u32)>,
    next: usize,
    channels: usize,
    dec: Option<(usize, SectionDecoder)>,
    buf: Vec<f32>,
    pos: usize,
}

pub struct LicensedMusic {
    pub mb: MusicBox,
    pub playlist: Playlist,
    cur: Option<SongStream>,
    tmp: Vec<f32>,
    pub played: Vec<(u32, String)>,
    once: bool,
    game: Option<RacePosts>,
    fade: Option<(f32, f32, u64)>,
}

struct RacePosts {
    event: RaceEvent,
    session: &'static std::sync::Mutex<Session>,
    clock: u64,
    next: Option<u64>,
}

pub const RACE_EVENT_PERIOD_MS: u64 = 500;

fn ms_frames(ms: u64) -> u64 {
    ms * u64::from(crate::pathfinder::SAMPLE_RATE) / 1000
}

impl LicensedMusic {
    pub fn new(mb: MusicBox, playlist: Playlist) -> Self {
        Self { mb, playlist, cur: None, tmp: Vec::new(), played: Vec::new(), once: false, game: None, fade: None }
    }

    // TODO(guess): a post's song starts at the check's frame (the game checks on its 60 Hz audio-manager update, then the MusicBox opens the stream with disc latency); settle: the MusicBox's stream start (0x83111DE0 → 0x8310BF18) timed on a console.
    pub fn game(mut mb: MusicBox, race: RaceType) -> Self {
        mb.set_event_class(Some(race));
        let posts = RacePosts { event: RaceEvent::new(race), session: session(), clock: 0, next: None };
        Self { game: Some(posts), ..Self::new(mb, Playlist { ids: Vec::new(), next: 0 }) }
    }

    pub fn event_setup(&mut self) {
        if let Some(g) = self.game.as_mut() {
            let first = (f64::from(crate::sfx_game::tuning::MUSIC_RACE_EVENT_DELAY) * 1000.0) as u64;
            g.next = Some(g.clock + ms_frames(first));
        }
    }

    pub fn candidates(mb: &MusicBox, race: RaceType) -> Vec<(u32, u32)> {
        let ev = RaceEvent::new(race);
        mb.candidates(&ev).into_iter().map(|id| (id, mb.song(id).map_or(0, Song::weight))).collect()
    }

    pub fn song(mb: MusicBox, id: u32) -> Self {
        Self { once: true, ..Self::new(mb, Playlist { ids: vec![id], next: 0 }) }
    }

    pub fn playing(&self) -> bool {
        self.cur.is_some()
            || self.game.is_some()
            || (self.playlist.next < self.playlist.ids.len())
            || (!self.once && !self.playlist.ids.is_empty())
    }

    fn open_song(&mut self, id: u32) -> Option<SongStream> {
        let opened = self.mb.sps_offset(id).and_then(|o| Ok(Sps::read(&self.mb.sbs, o)?));
        match opened {
            Ok(sps) => {
                let blocks = sps
                    .blocks
                    .sections
                    .iter()
                    .enumerate()
                    .flat_map(|(s, sec)| sec.iter().map(move |b| (b.offset, s, b.samples)))
                    .collect();
                let label = self.mb.song(id).map_or_else(|| format!("sound {id:#x}"), Song::label);
                self.played.push((id, label));
                let channels = sps.header.channels as usize;
                Some(SongStream { id, blocks, next: 0, channels, dec: None, buf: Vec::new(), pos: 0 })
            }
            Err(e) => {
                eprintln!("musicbox: sound {id:#x}: {e:#}");
                None
            }
        }
    }

    fn open_next(&mut self) -> Option<SongStream> {
        for _ in 0..self.playlist.ids.len() {
            if self.once && self.playlist.next >= self.playlist.ids.len() {
                return None;
            }
            let id = *self.playlist.ids.get(self.playlist.next % self.playlist.ids.len().max(1))?;
            self.playlist.next += 1;
            if let Some(s) = self.open_song(id) {
                return Some(s);
            }
        }
        None
    }

    fn poll(&mut self, now: u64) -> Result<(), u64> {
        let Some(g) = self.game.as_mut() else { return Err(u64::MAX) };
        let Some(next) = g.next.as_mut() else { return Err(u64::MAX) };
        let period = ms_frames(RACE_EVENT_PERIOD_MS);
        if now > *next {
            *next += (now - *next).div_ceil(period) * period;
        }
        if now < *next {
            return Err(*next - now);
        }
        *next = now + period;
        let (event, session) = (g.event, g.session);
        let pick = session.lock().ok().and_then(|mut s| Some((s.select(&self.mb, &event)?, s.seed(), s.picks.len())));
        match pick.and_then(|(id, seed, n)| Some((self.open_song(id)?, seed, n))) {
            Some((s, seed, n)) => {
                let t = now as f64 / f64::from(crate::pathfinder::SAMPLE_RATE);
                let label = self.mb.song(s.id).map_or_else(|| format!("{:#x}", s.id), Song::label);
                eprintln!(
                    "musicbox: music_race_event {event:?} at {t:.2} s → {:#x} {label} (pick {n}, seed {seed:#x})",
                    s.id
                );
                self.cur = Some(s);
                Ok(())
            }
            None => Err(ms_frames(RACE_EVENT_PERIOD_MS)),
        }
    }

    pub fn stop(&mut self, secs: f32) {
        if self.fade.is_some() && secs > 0.0 {
            return;
        }
        if self.cur.is_none() || secs <= 0.0 {
            self.stop_music();
            return;
        }
        let frames = (f64::from(secs) * f64::from(crate::pathfinder::SAMPLE_RATE)).round().max(1.0) as u64;
        self.fade = Some((1.0, 1.0 / frames as f32, frames));
    }

    fn stop_music(&mut self) {
        if let Some(s) = self.cur.take() {
            let label = self.mb.song(s.id).map_or_else(|| format!("{:#x}", s.id), Song::label);
            eprintln!("musicbox: StopMusic ({label})");
        }
        self.fade = None;
    }

    pub fn level_end(&mut self) {
        (self.game, self.playlist.ids) = (None, Vec::new());
        let secs = if paused() { 0.0 } else { crate::sfx_game::tuning::MUSIC_LEVEL_END_FADE };
        self.stop(secs);
    }

    pub fn finished(&self) -> bool {
        self.cur.is_none() && self.game.is_none() && self.fade.is_none() && self.playlist.ids.is_empty()
    }

    pub fn render(&mut self, out: &mut [f32], gain: f32) {
        if paused() {
            out.fill(0.0);
            return;
        }
        self.render_songs(out, gain);
        let Some((mut level, step, mut left)) = self.fade else { return };
        let (frames, _) = out.as_chunks_mut::<2>();
        let fading = frames.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        for frame in &mut frames[..fading] {
            level = (level - step).max(0.0);
            frame[0] *= level;
            frame[1] *= level;
        }
        left -= fading as u64;
        self.fade = Some((level, step, left));
        if left == 0 {
            for frame in &mut frames[fading..] {
                *frame = [0.0; 2];
            }
            self.stop_music();
        }
    }

    fn render_songs(&mut self, out: &mut [f32], gain: f32) {
        PLAYING.store(self.cur.is_some(), std::sync::atomic::Ordering::Relaxed);
        let frames = out.len() / 2;
        let start = self.game.as_ref().map_or(0, |g| g.clock);
        if let Some(g) = self.game.as_mut() {
            g.clock += frames as u64;
        }
        let mut f = 0;
        while f < frames {
            if self.cur.is_none() {
                if self.game.is_some() {
                    if let Err(wait) = self.poll(start + f as u64) {
                        let n = wait.min((frames - f) as u64) as usize;
                        out[f * 2..(f + n) * 2].fill(0.0);
                        f += n;
                    }
                    continue;
                }
                self.cur = self.open_next();
                if self.cur.is_none() {
                    out[f * 2..].fill(0.0);
                    return;
                }
            }
            let s = self.cur.as_mut().unwrap();
            if s.pos * 2 >= s.buf.len() {
                s.buf.clear();
                s.pos = 0;
                if !Self::decode_block(s, &self.mb.sbs, &mut self.tmp) {
                    self.cur = None;
                    continue;
                }
                continue;
            }
            out[2 * f] = s.buf[2 * s.pos] * gain;
            out[2 * f + 1] = s.buf[2 * s.pos + 1] * gain;
            s.pos += 1;
            f += 1;
        }
    }

    fn decode_block(s: &mut SongStream, sbs: &[u8], tmp: &mut Vec<f32>) -> bool {
        let Some(&(offset, section, samples)) = s.blocks.get(s.next) else { return false };
        s.next += 1;
        if s.dec.as_ref().is_none_or(|(x, _)| *x != section) {
            match SectionDecoder::new(s.channels as u8) {
                Ok(d) => s.dec = Some((section, d)),
                Err(_) => return false,
            }
        }
        let Ok(block) = ssx_formats::eaac::Block::read(sbs, offset) else { return false };
        tmp.clear();
        let Ok(n) = s.dec.as_mut().unwrap().1.decode_payload(block.payload, tmp) else {
            eprintln!("musicbox: sound {:#x}: undecodable block at {offset:#x}", s.id);
            return false;
        };
        for k in 0..n.min(samples as usize) {
            if s.channels == 1 {
                s.buf.extend_from_slice(&[tmp[k], tmp[k]]);
            } else {
                s.buf.extend_from_slice(&tmp[k * s.channels..k * s.channels + 2]);
            }
        }
        true
    }

    pub fn now_playing(&self) -> Option<String> {
        self.cur.as_ref().and_then(|s| self.mb.song(s.id)).map(Song::label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: u32, level: u32, weight: u32) -> Song {
        Song { id, unk: [1, 1, level, 100, weight, 4], info: Vec::new() }
    }

    #[test]
    fn a_song_switched_off_is_never_a_candidate() {
        let songs = vec![song(1, 1, 100), song(2, 0, 100), song(3, 1, 100), song(4, 1, 0)];
        assert_eq!(select_levelled(&songs, &[1, 2, 3, 4]), [1, 3], "level 0 and weight 0 are left out");
        let off = vec![song(1, 0, 100), song(2, 0, 100)];
        assert!(select_levelled(&off, &[1, 2]).is_empty(), "the level starts at 1: all off plays nothing");
    }

    #[test]
    fn the_highest_level_wins_and_level_3_ends_the_scan() {
        let songs = vec![song(1, 1, 100), song(2, 2, 100), song(3, 1, 100), song(4, 2, 100), song(5, 3, 100)];
        assert_eq!(select_levelled(&songs, &[1, 2, 3, 4]), [2, 4]);
        assert_eq!(select_levelled(&songs, &[1, 5, 2]), [5]);
    }
}
