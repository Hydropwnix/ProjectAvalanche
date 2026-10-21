use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use ssx_formats::speech::{evt::EventSystem, grf::Graffiti, kwd::Keywords, rep::RepetitionPools, spc};

use super::context::Lcg;

pub struct Bank {
    pub name: String,
    pub graffiti: Graffiti,
    sbr: ssx_formats::sbr::SampleBank,
    sbr_data: Vec<u8>,
    sbs: Arc<crate::music::Bytes>,
    ids: HashMap<u32, usize>,
}

impl Bank {
    pub fn load(big_root: &Path, lang: &str, dir: &str, name: &str) -> Result<Self> {
        let base = format!("data/xenon/audio/speech/{lang}/{dir}/{name}");
        let find = |kind: &str, ext: &str| {
            let rel = format!("{base}_{kind}_{lang}.{ext}");
            crate::music::find_file(big_root, &rel).with_context(|| rel.clone())
        };
        let sbr_data = std::fs::read(find("samples", "sbr")?)?;
        let sbs = find("samples", "sbs")?;
        let grf = find("graffitidata", "bin")?;
        let sbr = ssx_formats::sbr::SampleBank::parse(&sbr_data)?;
        let ids = sbr.sounds.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        Ok(Self {
            name: name.to_string(),
            graffiti: Graffiti::parse(&std::fs::read(grf)?)?,
            sbr,
            sbr_data,
            sbs: Arc::new(crate::music::Bytes::open(&sbs)?),
            ids,
        })
    }

    pub fn has(&self, sample: u32) -> bool {
        self.ids.contains_key(&sample)
    }

    pub fn seconds(&self, sample: u32) -> f32 {
        use ssx_formats::eaac::Sps;
        use ssx_formats::sbr::SoundRef;
        let Some(s) = self.ids.get(&sample).map(|&i| &self.sbr.sounds[i]) else { return 0.0 };
        let header = match self.sbr.resolve(&self.sbr_data, s) {
            Ok(SoundRef::SnrSns { header, .. }) => header,
            Ok(SoundRef::Sps { sps_offset }) => match Sps::read(&self.sbs, sps_offset as usize) {
                Ok(sps) => sps.header.clone(),
                Err(_) => return 0.0,
            },
            Err(_) => return 0.0,
        };
        if header.num_samples > 100 && header.sample_rate > 0 {
            header.num_samples as f32 / header.sample_rate as f32
        } else {
            0.0
        }
    }

    pub fn stream(&self, sample: u32) -> Result<crate::sfx_mixer::StreamedSound> {
        use ssx_formats::eaac::{Sps, read_v0_blocks};
        use ssx_formats::sbr::SoundRef;
        let s = &self.sbr.sounds[*self.ids.get(&sample).context("no such sample")?];
        match self.sbr.resolve(&self.sbr_data, s)? {
            SoundRef::SnrSns { header, sns_offset, .. } => {
                let b = read_v0_blocks(&header, &self.sbs, sns_offset as usize)?;
                crate::sfx_mixer::StreamedSound::new(self.sbs.clone(), header, &b)
            }
            SoundRef::Sps { sps_offset } => {
                let sps = Sps::read(&self.sbs, sps_offset as usize)?;
                crate::sfx_mixer::StreamedSound::new(self.sbs.clone(), sps.header.clone(), &sps.blocks)
            }
        }
    }

    pub fn decode(&self, sample: u32) -> Result<(Vec<f32>, u32, usize)> {
        use ssx_formats::eaac::{Sps, read_v0_blocks};
        use ssx_formats::sbr::SoundRef;
        let s = &self.sbr.sounds[*self.ids.get(&sample).context("no such sample")?];
        let (header, blocks) = match self.sbr.resolve(&self.sbr_data, s)? {
            SoundRef::SnrSns { header, sns_offset, .. } => {
                let b = read_v0_blocks(&header, &self.sbs, sns_offset as usize)?;
                (header, b)
            }
            SoundRef::Sps { sps_offset } => {
                let sps = Sps::read(&self.sbs, sps_offset as usize)?;
                (sps.header.clone(), sps.blocks)
            }
        };
        let d = ssx_formats::xma_decode::decode_sound(&header, &blocks)?;
        Ok((d.samples, d.sample_rate, usize::from(d.channels)))
    }
}

type Picks = Vec<(usize, u32, u8)>;

#[derive(Clone, Debug)]
pub struct Line {
    pub event: String,
    pub voice: i32,
    pub samples: Vec<(usize, u32, u8)>,
}

pub struct SpeechModule {
    pub events: EventSystem,
    pub sentences: spc::SentenceData,
    pools: RepetitionPools,
    history: HashMap<u8, VecDeque<u32>>,
    keywords: Option<Keywords>,
    reserved: Vec<u32>,
    pub banks: Vec<Bank>,
}

fn shuffled(n: usize, rng: &mut Lcg) -> Vec<usize> {
    let mut bag: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n);
    let mut left = n;
    while left > 0 {
        let k = rng.below(left as u32) as usize;
        out.push(bag[k]);
        bag[k] = bag[left - 1];
        left -= 1;
    }
    out
}

impl SpeechModule {
    pub fn new(
        events: EventSystem,
        sentences: spc::SentenceData,
        pools: RepetitionPools,
        keywords: Option<Keywords>,
    ) -> Self {
        Self { events, sentences, pools, history: HashMap::new(), keywords, reserved: Vec::new(), banks: Vec::new() }
    }

    fn score(&self, pool: u8, sample: u32) -> f32 {
        self.history
            .get(&pool)
            .and_then(|h| h.iter().rev().position(|&s| s == sample))
            .map_or(0.0, |k| 1.0 / (k + 1) as f32)
    }

    fn blocked(&self, sample: u32) -> bool {
        self.keywords.as_ref().is_some_and(|k| k.of(sample).any(|w| self.reserved.contains(&w)))
    }

    fn query(&self, tags: &[(u32, u32, bool)]) -> Vec<(usize, u32)> {
        let n = tags.len() as u32;
        let mut out = Vec::new();
        for (b, bank) in self.banks.iter().enumerate() {
            let lookup = |&(tag, value, bits): &(u32, u32, bool)| -> Vec<u32> {
                if !bits {
                    return bank.graffiti.find(tag, value, n).map(|e| e.samples.clone()).unwrap_or_default();
                }
                let mut v: Vec<u32> = Vec::new();
                for bit in (0..32).map(|k| 1u32 << k).filter(|b| value & b != 0) {
                    for s in bank.graffiti.find(tag, bit, n).into_iter().flat_map(|e| e.samples.iter()) {
                        if !v.contains(s) {
                            v.push(*s);
                        }
                    }
                }
                v
            };
            let Some(first) = tags.first() else { continue };
            let mut set = lookup(first);
            for t in &tags[1..] {
                let other = lookup(t);
                set.retain(|s| other.contains(s));
            }
            out.extend(set.into_iter().map(|s| (b, s)));
        }
        out
    }

    fn choose_sample(
        &self,
        phrase: &spc::Phrase,
        params: &[i32],
        ev: Option<&ssx_formats::speech::evt::Event>,
        rng: &mut Lcg,
    ) -> Option<(usize, u32, f32)> {
        let tags: Vec<(u32, u32, bool)> = phrase
            .tags
            .iter()
            .map(|t| match t.kind {
                1 => {
                    let k = t.cur.max(0) as usize;
                    let ty = ev.and_then(|e| e.params.get(k)).and_then(|&id| self.events.param_type(id));
                    (t.id, params.get(k).copied().unwrap_or(-1) as u32, ty.is_some_and(|ty| ty.kind == 1))
                }
                _ => (t.id, t.value as u32, false),
            })
            .collect();
        let found = self.query(&tags);
        let mut best: Option<(usize, u32, f32)> = None;
        for k in shuffled(found.len(), rng) {
            let (b, s) = found[k];
            if !self.banks[b].has(s) {
                continue;
            }
            let score = self.score(phrase.pool, s);
            if score < best.map_or(1.1, |x| x.2) && !self.blocked(s) {
                best = Some((b, s, score));
                if score == 0.0 {
                    break;
                }
            }
        }
        best
    }

    pub fn choose(&self, event: &str, params: &[i32], rng: &mut Lcg) -> Option<Line> {
        let group = self.sentences.group(event)?;
        let ev = self.events.event(event);
        let mut order = shuffled(group.sentences.len(), rng);
        order.sort_by_key(|&i| group.sentences[i].rank as i32);
        let mut best = 1.1f32;
        let mut chosen: Option<(u32, Picks)> = None;
        for i in order {
            let s = &group.sentences[i];
            if chosen.as_ref().is_some_and(|c| c.0 < s.rank) {
                break;
            }
            if rng.below(100) >= s.probability {
                continue;
            }
            let mut worst = -1.0f32;
            let mut picks = Vec::new();
            let ok = s.phrases.iter().all(|p| match self.choose_sample(p, params, ev, rng) {
                Some((b, smp, sc)) => {
                    worst = worst.max(sc);
                    picks.push((b, smp, p.pool));
                    true
                }
                None => false,
            });
            if ok && worst < best {
                best = worst;
                chosen = Some((s.rank, picks));
                if best == 0.0 {
                    break;
                }
            }
        }
        let (_, samples) = chosen?;
        Some(Line { event: event.to_string(), voice: params.first().copied().unwrap_or(0), samples })
    }

    pub fn played(&mut self, line: &Line) {
        for &(_, s, pool) in &line.samples {
            let size = self.pools.pool(u32::from(pool)).map_or(0, |p| p.size as usize);
            let h = self.history.entry(pool).or_default();
            h.push_back(s);
            while h.len() > size {
                h.pop_front();
            }
            if let Some(k) = &self.keywords {
                self.reserved.extend(k.of(s));
            }
        }
    }
}
