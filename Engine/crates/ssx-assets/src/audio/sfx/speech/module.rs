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
    sbs: Arc<crate::sfx_stream::StreamFile>,
    sps_heads: HashMap<u32, ssx_formats::eaac::SoundHeader>,
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
        let sbs = crate::sfx_stream::StreamFile::open(&sbs)?;
        let mut sps_heads = HashMap::new();
        for s in &sbr.sounds {
            if let Ok(ssx_formats::sbr::SoundRef::Sps { sps_offset }) = sbr.resolve(&sbr_data, s)
                && !sps_heads.contains_key(&sps_offset)
                && let Ok(h) = sbs.read_sps_header(u64::from(sps_offset))
            {
                sps_heads.insert(sps_offset, h);
            }
        }
        Ok(Self {
            name: name.to_string(),
            graffiti: Graffiti::parse(&std::fs::read(grf)?)?,
            sbr,
            sbr_data,
            sbs,
            sps_heads,
            ids,
        })
    }

    pub fn has(&self, sample: u32) -> bool {
        self.ids.contains_key(&sample)
    }

    // TODO(guess): an SPS sample (the NPC banks) counts its frames / rate like a sample whose header is in the bank:
    // 0x835ECD20 counts a queued entry only when its +8 (the header SndPlayer1's 0x82A20840 reads, an SNR header or an
    // SPS `H` block) is set, and where an SPS entry's +8 comes from is not traced; settle: the writer of the speech
    // voice's queue entries (vector +0x68, 12 bytes each) for an SPS sample.
    pub fn seconds(&self, sample: u32) -> f32 {
        let Some((header, _, _)) = self.sound(sample) else { return 0.0 };
        if header.num_samples > 100 && header.sample_rate > 0 {
            header.num_samples as f32 / header.sample_rate as f32
        } else {
            0.0
        }
    }

    fn sound(&self, sample: u32) -> Option<(ssx_formats::eaac::SoundHeader, u64, crate::sfx_stream::BlockKind)> {
        use crate::sfx_stream::BlockKind;
        use ssx_formats::sbr::SoundRef;
        let s = self.sbr.sounds.get(*self.ids.get(&sample)?)?;
        match self.sbr.resolve(&self.sbr_data, s).ok()? {
            SoundRef::SnrSns { header, sns_offset, .. } => Some((header, u64::from(sns_offset), BlockKind::V0)),
            SoundRef::Sps { sps_offset } => {
                Some((self.sps_heads.get(&sps_offset)?.clone(), u64::from(sps_offset), BlockKind::Sps))
            }
        }
    }

    pub fn stream(&self, sample: u32) -> Result<crate::sfx_mixer::StreamedSound> {
        let (header, offset, kind) = self.sound(sample).context("no such sample")?;
        let ring = crate::sfx_stream::Ring::SPEECH;
        crate::sfx_mixer::StreamedSound::open(self.sbs.clone(), header, offset, kind, ring)
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
