mod direct;
mod sources;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ssx_formats::abk::SampleEntry;
use ssx_formats::eaac::{Block, Codec, SoundHeader, Storage, read_v0_blocks};

use crate::sfx_aems::{Bank, MIXER_RATE, VoiceHost};
use crate::sfx_bus::{self, Graph, SourceHost};
use crate::sfx_dsp::{self, BLOCK, Resampler};

pub use direct::*;
pub use sources::*;

pub fn voice_patch_table(big_root: &std::path::Path) -> Result<HashMap<i32, (String, String)>> {
    use crate::vault_db::fnv1;
    use ssx_formats::attrib::{FieldData, Schema, Value, Vault, collection};
    let dir = big_root.join("bootcache/data/xenon/attribdb");
    let read = |stem: &str| -> Result<(Vec<u8>, Vec<u8>)> {
        Ok((std::fs::read(dir.join(format!("{stem}.vlt")))?, std::fs::read(dir.join(format!("{stem}.bin")))?))
    };
    let (sv, sb) = read("attribdb_schema")?;
    let schema = Schema::from_vault(&Vault::parse(&sv, &sb)?).context("reading the schema")?;
    let (v, b) = read("attribdb")?;
    let vault = Vault::parse(&v, &b)?;
    let mut table = HashMap::new();
    for c in collection::collections(&vault)?.iter().filter(|c| c.class == fnv1("audio_mixer_voices")) {
        for f in c.decode(&vault, &schema)?.fields {
            let FieldData::Single(Value::Raw { bytes, pointers }) = &f.data else { continue };
            let text =
                |off: usize| pointers.iter().find(|p| p.offset as usize == off).and_then(|p| vault.cstr(p.target).ok());
            let (Some(patch), Some(voice)) = (text(4), text(8)) else { continue };
            let id = bytes.get(..4).map_or(0, |w| i32::from_be_bytes([w[0], w[1], w[2], w[3]]));
            let id = if c.key == fnv1("default") { -1 } else { id };
            table.insert(id, (patch.to_string(), voice.to_string()));
        }
    }
    Ok(table)
}

struct BankAudio {
    name: String,
    data: Arc<Vec<u8>>,
    ast: Option<Arc<crate::music::Bytes>>,
    sounds: Vec<Option<(u32, SoundHeader)>>,
}

struct AemsVoice {
    slot: usize,
    source: u32,
    volume: f32,
    dry: f32,
    aux: f32,
    pan_fresh: bool,
    ava_faders: bool,
}

const AVA_FADERS: [&str; 3] = ["Ava_Gain_Port3", "Ava_Gain_Port4", "Ava_Gain_Port5"];

fn azimuth_slots(channels: usize) -> [usize; 8] {
    let mut s = [0; 8];
    match channels {
        1 => {}
        4 => s[2..5].copy_from_slice(&[1, 2, 3]),
        _ => {
            s[1..5].copy_from_slice(&[1, 2, 3, 4]);
            if channels == 8 {
                s[5..7].copy_from_slice(&[5, 6]);
            }
        }
    }
    s
}

pub struct Mixer {
    banks: Vec<BankAudio>,
    ram: HashMap<(usize, usize), Arc<RamSound>>,
    pub graph: Graph,
    pub sources: Sources,
    table: HashMap<i32, (String, String)>,
    voices: Vec<Option<AemsVoice>>,
    music_slot: Option<usize>,
    movie_slot: Option<usize>,
    pcm: Vec<Option<(usize, u32)>>,
    pub events: Vec<String>,
    pub verbose: bool,
}

impl Mixer {
    pub fn new(set: Arc<sfx_bus::PatchSet>, table: HashMap<i32, (String, String)>) -> Self {
        Self {
            banks: Vec::new(),
            ram: HashMap::new(),
            graph: Graph::new(set),
            sources: Sources::default(),
            table,
            voices: Vec::new(),
            music_slot: None,
            movie_slot: None,
            pcm: Vec::new(),
            events: Vec::new(),
            verbose: false,
        }
    }

    pub fn add_bank(&mut self, bank: &Bank) {
        let sounds = bank.abk.s10a.sounds.iter().map(|s| s.as_ref().map(|s| (s.offset, s.header.clone()))).collect();
        self.banks.push(BankAudio { name: bank.name.clone(), data: bank.data.clone(), ast: bank.ast.clone(), sounds });
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    pub fn set_music(&mut self, on: bool) {
        match (on, self.music_slot) {
            (true, None) => {
                self.music_slot = self.graph.add_aems_voice("music_voice_patch", "MusicSource", 2, MUSIC_SOURCE)
            }
            (false, Some(s)) => {
                self.graph.remove_voice(s);
                self.music_slot = None;
            }
            _ => {}
        }
    }

    pub fn start_movie(&mut self, feed: Arc<crate::sfx::movie::MovieAudio>) -> bool {
        self.stop_movie();
        let plugins = feed.plugins();
        let (slot, _) = self.graph.add_core_voice(crate::sfx::movie::VOICE, 0, &plugins);
        self.graph.set_voice_source(slot, Some(crate::sfx::movie::MOVIE_SOURCE));
        let target = self.graph.submix_input(crate::sfx::movie::BUS);
        self.graph.set_send_target(slot, plugins.len() - 1, target);
        self.events.push(format!(
            "movie voice start ({} ch, {} Hz) → {}",
            feed.channels,
            feed.rate,
            crate::sfx::movie::BUS
        ));
        feed.attach();
        self.sources.movie = Some(feed);
        self.movie_slot = Some(slot);
        target.is_some()
    }

    pub fn stop_movie(&mut self) {
        if let Some(slot) = self.movie_slot.take() {
            self.graph.remove_voice(slot);
            self.events.push("movie voice end".into());
        }
        self.sources.movie = None;
    }

    pub fn movie_playing(&self) -> bool {
        self.movie_slot.is_some()
    }

    fn ram_sound(&mut self, b: usize, idx: usize) -> Result<Arc<RamSound>> {
        if let Some(r) = self.ram.get(&(b, idx)) {
            return Ok(r.clone());
        }
        let t0 = self.graph.profiling().then(std::time::Instant::now);
        let bank = &self.banks[b];
        let (offset, header) = bank.sounds.get(idx).cloned().flatten().context("empty S10A slot")?;
        let r = Arc::new(RamSound::decode(&bank.data, offset, &header)?);
        if let Some(t0) = t0 {
            let name = format!("(decode {} {idx}: {} frames)", self.banks[b].name, header.num_samples);
            self.graph.profile_add(&name, t0.elapsed().as_nanos() as u64);
        }
        self.ram.insert((b, idx), r.clone());
        Ok(r)
    }

    pub fn add_bank_decoded(&mut self, bank: &Bank, ram: DecodedRam) {
        let b = self.banks.len();
        self.add_bank(bank);
        for (idx, sound) in ram.0 {
            self.ram.insert((b, idx), sound);
        }
    }

    pub fn decode_ram_all(&mut self, threads: usize) -> Result<()> {
        struct Job {
            key: (usize, usize),
            data: Arc<Vec<u8>>,
            offset: u32,
            header: SoundHeader,
        }
        type Decoded = Vec<((usize, usize), RamSound)>;
        let mut todo: Vec<Job> = Vec::new();
        for (b, bank) in self.banks.iter().enumerate() {
            for (idx, slot) in bank.sounds.iter().enumerate() {
                if let Some((offset, header)) = slot
                    && header.storage == Storage::Ram
                    && !self.ram.contains_key(&(b, idx))
                {
                    todo.push(Job { key: (b, idx), data: bank.data.clone(), offset: *offset, header: header.clone() });
                }
            }
        }
        todo.sort_by_key(|t| std::cmp::Reverse(u64::from(t.header.num_samples) * u64::from(t.header.channels)));
        let next = std::sync::atomic::AtomicUsize::new(0);
        let done: Vec<Result<Decoded>> = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..threads.max(1))
                .map(|_| {
                    scope.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some(job) = todo.get(i) else { return Ok(out) };
                            out.push((job.key, RamSound::decode(&job.data, job.offset, &job.header)?));
                        }
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| bail!("RAM sound decoder panicked"))).collect()
        });
        for part in done {
            for (key, sound) in part? {
                self.ram.insert(key, Arc::new(sound));
            }
        }
        Ok(())
    }

    pub fn decoded_ram_sizes(&self) -> Vec<(String, usize, usize, usize)> {
        let mut v: Vec<(String, usize, usize, usize)> = self
            .ram
            .iter()
            .map(|(&(b, i), r)| {
                (self.banks[b].name.clone(), i, r.sections.iter().map(Vec::len).sum::<usize>() / r.channels, r.channels)
            })
            .collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.2 * e.3));
        v
    }

    pub fn decoded_ram(&self) -> (usize, usize) {
        let floats = self.ram.values().map(|r| r.sections.iter().map(Vec::len).sum::<usize>()).sum();
        (self.ram.len(), floats)
    }

    fn open(&mut self, b: usize, entry: &SampleEntry) -> Result<Source> {
        let idx = usize::from(entry.sound_index);
        let (_, header) = self.banks[b].sounds.get(idx).cloned().flatten().context("empty S10A slot")?;
        let label = format!("{} sound {idx}", self.banks[b].name);
        let frames = match header.storage {
            Storage::Ram => Frames::Ram(self.ram_sound(b, idx)?),
            Storage::Stream => {
                let ast = self.banks[b].ast.clone().context("streamed sound without .ast")?;
                let cursor = Cursor::new(Data::Ast(ast), header.clone(), entry.stream_offset as usize)?;
                Frames::Stream { cursor: Box::new(cursor), buf: Vec::new(), base: 0, ended: false }
            }
            Storage::Gigasample => bail!("gigasample sound"),
        };
        let channels = usize::from(header.channels);
        if channels > 8 {
            bail!("{channels} channels");
        }
        Ok(Source {
            frames,
            channels,
            rate: header.sample_rate,
            num_samples: header.num_samples,
            looped: header.looped,
            resampler: Resampler::default(),
            paused: false,
            finished: false,
            label,
            fade: None,
        })
    }

    fn voice(&mut self, id: u32) -> Option<&mut AemsVoice> {
        self.voices.get_mut(id as usize - 1).and_then(Option::as_mut)
    }
}

impl Mixer {
    fn apply_param(&mut self, id: u32, param: u8, value: i32) {
        let Some(v) = self.voices.get_mut(id as usize - 1).and_then(Option::as_mut) else { return };
        let g = &mut self.graph;
        let level = |x: i32| x.clamp(0, 0x7FFF) as f32 * 3.051_851e-5;
        match param {
            0 => g.set_attr(v.slot, b"Rsp0", 0, f64::from(value.clamp(0, 0xFFFF) as f32 * 0.000_244_140_62)),
            2 => {
                v.volume = level(value);
                g.set_attr(v.slot, b"Gai0", 0, f64::from(v.dry * v.volume));
            }
            3 => {
                if g.has_plugin(v.slot, b"Pn21") {
                    if v.pan_fresh {
                        for (a, d) in [(7, 30.0), (8, 110.0), (9, 150.0)] {
                            g.set_attr(v.slot, b"Pn21", a, d);
                        }
                        v.pan_fresh = false;
                    }
                    g.set_attr(v.slot, b"Pn21", 0, f64::from(value as f32 * 0.005_493_164));
                }
            }
            5 => v.aux = level(value),
            6 => g.set_attr(v.slot, b"LI20", 0, f64::from(value.clamp(0, 0xFFFF))),
            7 => g.set_attr(v.slot, b"HI20", 0, f64::from(value.clamp(0, 0xFFFF))),
            8 => {
                v.dry = level(value);
                g.set_attr(v.slot, b"Gai0", 0, f64::from(v.dry * v.volume));
            }
            11..=13 if v.ava_faders => g.fade_by_name(AVA_FADERS[usize::from(param - 11)], level(value), 0.1),
            _ => {}
        }
    }

    pub fn process_block(&mut self, out: &mut [f32]) {
        let mut ended = Vec::new();
        let dac = self.graph.process(&mut self.sources, &mut ended);
        sfx_bus::downmix(&dac, out);
        self.block_ended(ended);
    }

    pub fn process_block_dac(&mut self, out: &mut [f32]) {
        let mut ended = Vec::new();
        let dac = self.graph.process(&mut self.sources, &mut ended);
        sfx_bus::interleave_dac(&dac, out);
        self.block_ended(ended);
    }

    fn block_ended(&mut self, ended: Vec<usize>) {
        if self.movie_slot.is_some_and(|m| ended.contains(&m)) {
            self.stop_movie();
        }
        for slot in ended {
            if let Some(v) = self.voices.iter().flatten().find(|v| v.slot == slot)
                && let Some(s) = self.sources.voices.get_mut(v.source as usize).and_then(Option::as_mut)
            {
                s.finished = true;
            }
        }
    }
}

impl VoiceHost for Mixer {
    fn start(&mut self, bank: usize, entry: &SampleEntry, params: &[(u8, i32)]) -> Option<u32> {
        let src = match self.open(bank, entry) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("sfx: {} sound {}: {e:#}", self.banks[bank].name, entry.sound_index);
                return None;
            }
        };
        let id9 = params.iter().find(|p| p.0 == 9).map(|p| p.1);
        let (patch, voice) = id9.and_then(|id| self.table.get(&id)).or_else(|| self.table.get(&-1)).cloned()?;
        let channels = src.channels;
        let label = src.label.clone();
        let sid = self.sources.voices.iter().position(Option::is_none).unwrap_or(self.sources.voices.len());
        if sid == self.sources.voices.len() {
            self.sources.voices.push(None);
        }
        self.sources.voices[sid] = Some(src);
        let Some(slot) = self.graph.add_aems_voice(&patch, &voice, channels, sid as u32) else {
            eprintln!("sfx: voice patch {patch} / {voice} not found");
            self.sources.voices[sid] = None;
            return None;
        };
        let ava_faders = voice == "Ava_Level_4_Voice";
        let pan_fresh = channels > 1;
        let v = AemsVoice { slot, source: sid as u32, volume: 1.0, dry: 1.0, aux: 0.0, pan_fresh, ava_faders };
        let id = self.voices.iter().position(Option::is_none).unwrap_or(self.voices.len());
        if id == self.voices.len() {
            self.voices.push(None);
        }
        self.voices[id] = Some(v);
        let vid = id as u32 + 1;
        if self.graph.has_plugin(slot, b"Pn21") {
            let slots = azimuth_slots(channels);
            let deg =
                |k: usize| f64::from(f32::from(entry.azimuths.get(k).copied().unwrap_or(0)) * 256.0 * 0.005_493_164);
            self.graph.set_attr(slot, b"Pn21", 0, deg(0));
            if channels > 1 {
                self.graph.set_attr(slot, b"Pn21", 1, 0.0);
                self.graph.set_attr(slot, b"Pn21", 7, deg(slots[2]));
                self.graph.set_attr(slot, b"Pn21", 8, deg(slots[4]));
                if channels > 6 {
                    self.graph.set_attr(slot, b"Pn21", 9, deg(slots[6]));
                }
                if channels >= 6 {
                    self.graph.set_attr(slot, b"Pn21", 6, 1.0);
                }
            } else {
                self.graph.set_attr(slot, b"Pn21", 0, deg(slots[1]));
            }
        }
        for &(p, value) in params {
            self.apply_param(vid, p, value);
        }
        let pan = if self.graph.has_plugin(slot, b"Pn21") {
            format!(" Pan2D1 az {:?}", entry.azimuths)
        } else {
            String::new()
        };
        self.events.push(format!("voice {vid} start {label} ({channels} ch{pan}) → {patch}/{voice} params {params:?}"));
        Some(vid)
    }

    fn set_param(&mut self, voice: u32, id: u8, value: i32) {
        if self.voice(voice).is_some() {
            self.apply_param(voice, id, value);
            if self.verbose && id <= 8 {
                self.events.push(format!("voice {voice} param {id} = {value}"));
            }
        }
    }

    fn destroy(&mut self, voice: u32) {
        if let Some(v) = self.voices.get_mut(voice as usize - 1).and_then(Option::take) {
            self.graph.remove_voice(v.slot);
            self.sources.voices[v.source as usize] = None;
            if self.verbose {
                self.events.push(format!("voice {voice} destroyed"));
            }
        }
    }

    fn pause(&mut self, voice: u32) {
        if let Some(v) = self.voice(voice) {
            let s = v.source as usize;
            if let Some(src) = self.sources.voices[s].as_mut() {
                src.paused = true;
            }
        }
    }

    fn resume(&mut self, voice: u32) {
        if let Some(v) = self.voice(voice) {
            let s = v.source as usize;
            if let Some(src) = self.sources.voices[s].as_mut() {
                src.paused = false;
            }
        }
    }

    fn status(&mut self, voice: u32) -> Option<(i32, i32)> {
        let s = self.voice(voice)?.source as usize;
        let v = self.sources.voices.get(s)?.as_ref()?;
        if v.finished {
            return None;
        }
        let pos = v.resampler.index as f64;
        let ms = |frames: f64| (frames * 1000.0 / f64::from(v.rate)) as i32;
        let total = if v.looped { f64::from(i32::MAX) } else { f64::from(v.num_samples) };
        Some((ms(total - pos).max(0), ms(pos)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_sounds_loop_their_last_section() {
        let r = RamSound { channels: 1, looped: true, sections: vec![vec![1.0, 2.0], vec![3.0, 4.0, 5.0]] };
        let got: Vec<f32> = (0..8).map(|i| r.frame(i).unwrap()[0]).collect();
        assert_eq!(got, [1.0, 2.0, 3.0, 4.0, 5.0, 3.0, 4.0, 5.0]);
        let one = RamSound { channels: 1, looped: false, sections: vec![vec![1.0, 2.0]] };
        assert!(one.frame(2).is_none());
    }

    #[test]
    fn stream_resampler_keeps_the_signal() {
        let mut r = StreamResampler::new(44_100, 48_000);
        let mut t = 0usize;
        let mut src = |b: &mut [f32]| {
            for f in b.chunks_mut(2) {
                f.fill(t as f32);
                t += 1;
            }
        };
        let mut out = vec![0f32; 960];
        r.render(&mut out, &mut src);
        r.render(&mut out, &mut src);
        assert!((out[0] - 440.97).abs() < 0.05, "{}", out[0]);
    }
}
