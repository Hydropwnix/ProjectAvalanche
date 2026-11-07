use std::collections::BTreeMap;
use std::sync::Arc;

use ssx_content::audio::music::{Action, HELD, Interactive, When};

const RATE: f64 = crate::pathfinder::SAMPLE_RATE as f64;

#[derive(Clone, Debug, Default)]
pub struct ScoreInbox {
    pub posts: Vec<(u32, String)>,
    pub control: Option<u32>,
    pub volume: Option<i32>,
    pub stop: bool,
    pub paused: Option<bool>,
    pub state: u8,
}

pub type Frames = Arc<Vec<f32>>;

struct SectionAudio {
    name: String,
    pcm: Frames,
    region: (usize, usize),
    repeat: bool,
    next: Option<String>,
    layers: BTreeMap<String, Frames>,
}

#[derive(Clone, Copy, Debug)]
struct Ramp {
    value: f32,
    target: f32,
    step: f32,
}

impl Ramp {
    fn new(v: f32) -> Self {
        Self { value: v, target: v, step: 0.0 }
    }

    fn to(&mut self, target: f32, seconds: f32) {
        self.target = target;
        let frames = (seconds * RATE as f32).max(1.0);
        self.step = (target - self.value).abs() / frames;
        if seconds <= 0.0 {
            self.value = target;
        }
    }

    fn next(&mut self) -> f32 {
        if self.value < self.target {
            self.value = (self.value + self.step).min(self.target);
        } else if self.value > self.target {
            self.value = (self.value - self.step).max(self.target);
        }
        self.value
    }
}

pub struct ScorePlayer {
    def: Interactive,
    sections: Vec<SectionAudio>,
    stingers: BTreeMap<String, Frames>,
    current: Option<usize>,
    pos: f64,
    pending: Option<(usize, When)>,
    held: Option<usize>,
    layers: BTreeMap<String, Ramp>,
    volume: Ramp,
    rate: Ramp,
    stopping: Option<u64>,
    stopped: bool,
    paused: bool,
    playing_stingers: Vec<(Frames, f64)>,
    pub log: Vec<String>,
}

pub type Decoded = BTreeMap<String, Frames>;

impl ScorePlayer {
    pub fn new(def: Interactive, decoded: &Decoded) -> Result<Self, String> {
        let get = |rel: &str| decoded.get(rel).cloned().ok_or_else(|| format!("{rel}: not decoded"));
        let mut sections = Vec::new();
        for s in &def.sections {
            let pcm = get(&s.file)?;
            let n = pcm.len() / 2;
            let region = match s.looping {
                Some(r) => {
                    let (a, b) = ((r.start * RATE) as usize, ((r.end * RATE) as usize).min(n));
                    if a >= b {
                        return Err(format!("section {}: its loop region lies outside its {n} frames", s.name));
                    }
                    (a, b)
                }
                None => (0, n),
            };
            let mut layers = BTreeMap::new();
            for (layer, rel) in &s.layers {
                let stem = get(rel)?;
                if stem.len().abs_diff(pcm.len()) > 2 * 64 {
                    return Err(format!(
                        "section {}: stem {rel} has {} frames, the section's file {n} (stems play in step)",
                        s.name,
                        stem.len() / 2
                    ));
                }
                layers.insert(layer.clone(), stem);
            }
            sections.push(SectionAudio {
                name: s.name.clone(),
                pcm,
                region,
                repeat: s.repeat,
                next: s.next.clone(),
                layers,
            });
        }
        let mut stingers = BTreeMap::new();
        for st in &def.stingers {
            stingers.insert(st.name.clone(), get(&st.file)?);
        }
        let layers = def.layers.iter().map(|(n, l)| (n.clone(), Ramp::new(*l))).collect();
        let current = def.start.as_ref().and_then(|s| sections.iter().position(|x| &x.name == s));
        let pos = current.map_or(0.0, |_| 0.0);
        Ok(Self {
            def,
            sections,
            stingers,
            current,
            pos,
            pending: None,
            held: None,
            layers,
            volume: Ramp::new(1.0),
            rate: Ramp::new(1.0),
            stopping: None,
            stopped: false,
            paused: false,
            playing_stingers: Vec::new(),
            log: Vec::new(),
        })
    }

    fn section(&self, name: &str) -> Option<usize> {
        if name == HELD {
            return self.held;
        }
        self.sections.iter().position(|s| s.name == name)
    }

    pub fn take(&mut self, inbox: &mut ScoreInbox) {
        for (value, name) in std::mem::take(&mut inbox.posts) {
            let key = format!("0x{value:08x}");
            let action = self.def.events.get(&name).or_else(|| self.def.events.get(&key)).cloned();
            match action {
                Some(a) => {
                    self.log.push(format!("score: {name} → {}", describe(&a)));
                    self.apply(&a);
                }
                None => self.log.push(format!("score: {name}: no action")),
            }
        }
        if let Some(level) = inbox.control.take()
            && let Some((_, a)) = self.def.control.range(..=level.min(127) as u8).next_back()
        {
            let a = a.clone();
            self.log.push(format!("score: control level {level} → {}", describe(&a)));
            self.apply(&a);
        }
        if let Some(v) = inbox.volume.take() {
            self.volume.to(v.clamp(0, 100) as f32 / 100.0, 0.0);
        }
        if std::mem::take(&mut inbox.stop) {
            self.stopped = true;
            self.current = None;
            self.playing_stingers.clear();
            self.log.push("score: stop".into());
        }
        if let Some(p) = inbox.paused.take() {
            self.paused = p;
        }
        inbox.state = if self.paused {
            3
        } else if self.current.is_some() && !self.stopped {
            5
        } else {
            1
        };
    }

    fn apply(&mut self, a: &Action) {
        if a.hold {
            self.held = self.current;
        }
        if let Some((name, when)) = &a.section
            && let Some(k) = self.section(name)
        {
            if std::mem::take(&mut self.stopped) {
                (self.volume, self.stopping) = (Ramp::new(1.0), None);
            }
            if self.current.is_none() || *when == When::Now {
                self.switch(k);
            } else {
                self.pending = Some((k, *when));
            }
        }
        if let Some(st) = &a.stinger
            && !self.stopped
            && let Some(pcm) = self.stingers.get(st)
        {
            self.playing_stingers.push((pcm.clone(), 0.0));
        }
        for (layer, level) in &a.layers {
            if let Some(r) = self.layers.get_mut(layer) {
                r.to(*level, a.fade);
            }
        }
        if let Some(v) = a.volume {
            self.volume.to(v, a.fade);
        }
        if let Some(p) = a.pitch {
            self.rate.to(p, a.fade);
        }
        if let Some(s) = a.stop {
            self.stopping = Some((f64::from(s) * RATE) as u64);
            self.volume.to(0.0, s);
        }
    }

    fn switch(&mut self, k: usize) {
        self.current = Some(k);
        self.pos = 0.0;
        self.pending = None;
        self.log.push(format!("score: section {} plays", self.sections[k].name));
    }

    fn bar_frames(&self) -> Option<f64> {
        let bpm = self.def.bpm?;
        Some(60.0 / bpm * f64::from(self.def.beats_per_bar) * RATE)
    }

    pub fn render(&mut self, out: &mut [f32]) {
        if self.paused {
            return;
        }
        let gain = self.def.gain;
        for f in out.as_chunks_mut::<2>().0 {
            let rate = f64::from(self.rate.next().max(0.0));
            let vol = self.volume.next();
            let (mut l, mut r) = (0.0f32, 0.0f32);
            if let Some(k) = self.current {
                let s = &self.sections[k];
                let i = self.pos as usize;
                let frac = (self.pos - i as f64) as f32;
                let at = |pcm: &[f32], i: usize| -> (f32, f32) {
                    let j = i.min(pcm.len() / 2 - 1);
                    (pcm[2 * j], pcm[2 * j + 1])
                };
                let mix = |pcm: &[f32], g: f32, l: &mut f32, r: &mut f32| {
                    if pcm.len() < 2 || i >= pcm.len() / 2 {
                        return;
                    }
                    let (a, b) = (at(pcm, i), at(pcm, i + 1));
                    *l += (a.0 + (b.0 - a.0) * frac) * g;
                    *r += (a.1 + (b.1 - a.1) * frac) * g;
                };
                mix(&s.pcm, 1.0, &mut l, &mut r);
                for (name, stem) in &s.layers {
                    let g = self.layers.get_mut(name).map_or(0.0, Ramp::next);
                    mix(stem, g, &mut l, &mut r);
                }
                let before = self.pos;
                self.pos += rate;
                let end = s.region.1 as f64;
                if let (Some((next, When::Bar)), Some(bar)) = (self.pending, self.bar_frames())
                    && (before / bar).floor() != (self.pos / bar).floor()
                {
                    self.switch(next);
                } else if self.pos >= end {
                    match (self.pending, s.repeat) {
                        (Some((next, _)), _) => self.switch(next),
                        (None, true) => {
                            let len = (s.region.1 - s.region.0).max(1) as f64;
                            self.pos = s.region.0 as f64 + (self.pos - end) % len;
                        }
                        (None, false) => match s.next.as_deref().and_then(|n| self.section(n)) {
                            Some(next) => self.switch(next),
                            None => self.current = None,
                        },
                    }
                }
            }
            self.playing_stingers.retain_mut(|(pcm, pos)| {
                let i = *pos as usize;
                if i >= pcm.len() / 2 {
                    return false;
                }
                l += pcm[2 * i];
                r += pcm[2 * i + 1];
                *pos += rate;
                true
            });
            f[0] += l * gain * vol;
            f[1] += r * gain * vol;
            if let Some(left) = self.stopping.as_mut() {
                *left = left.saturating_sub(1);
                if *left == 0 {
                    self.stopping = None;
                    self.stopped = true;
                    self.current = None;
                    self.playing_stingers.clear();
                }
            }
        }
    }
}

fn describe(a: &Action) -> String {
    let mut v = Vec::new();
    if a.hold {
        v.push("hold".to_string());
    }
    if let Some((s, w)) = &a.section {
        v.push(format!("section {s} ({w:?})"));
    }
    if let Some(s) = &a.stinger {
        v.push(format!("stinger {s}"));
    }
    for (l, x) in &a.layers {
        v.push(format!("layer {l} {x}"));
    }
    if let Some(x) = a.volume {
        v.push(format!("volume {x}"));
    }
    if let Some(x) = a.pitch {
        v.push(format!("pitch {x}"));
    }
    if let Some(x) = a.stop {
        v.push(format!("stop over {x} s"));
    }
    if v.is_empty() { "nothing".into() } else { v.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCORE: &[u8] = br#"type = "interactive"
bpm = 120
beats_per_bar = 4
start = "intro"
[[layer]]
name = "drums"
level = 0.0
[[section]]
name = "intro"
file = "intro.wav"
[[section]]
name = "main"
file = "main.wav"
layers.drums = "drums.wav"
[[section]]
name = "danger"
file = "danger.wav"
[[section]]
name = "win"
file = "win.wav"
repeat = false
[[stinger]]
name = "hit"
file = "hit.wav"
[events]
Music_Start = { section = "main", when = "bar" }
Danger = { section = "danger", hold = true }
Back = { section = "@held" }
Hit = { stinger = "hit" }
Win = { section = "win" }
Die = { stop = 0.5 }
[control]
50 = { layers = { drums = 1.0 } }
"#;

    fn constant(v: f32, seconds: f64) -> Frames {
        Arc::new(vec![v; 2 * (seconds * RATE) as usize])
    }

    fn player() -> ScorePlayer {
        let id = "ssx:music/pathfinder/ssxdd/0".parse().unwrap();
        let ssx_content::audio::music::MusicDef::Interactive(def) =
            ssx_content::audio::music::parse_toml(SCORE, Some(&id)).unwrap()
        else {
            panic!("not a score")
        };
        let mut d = Decoded::new();
        for (f, v, s) in [
            ("intro.wav", 0.1, 4.0),
            ("main.wav", 0.2, 4.0),
            ("drums.wav", 0.05, 4.0),
            ("danger.wav", 0.3, 4.0),
            ("win.wav", 0.4, 1.0),
            ("hit.wav", 0.5, 0.1),
        ] {
            d.insert(f.to_string(), constant(v, s));
        }
        ScorePlayer::new(def, &d).unwrap()
    }

    fn run(p: &mut ScorePlayer, seconds: f64) -> f32 {
        let mut out = vec![0.0; 2 * (seconds * RATE) as usize];
        p.render(&mut out);
        out[out.len() - 2]
    }

    fn post(p: &mut ScorePlayer, name: &str) -> ScoreInbox {
        let mut inbox = ScoreInbox { posts: vec![(0, name.to_string())], ..Default::default() };
        p.take(&mut inbox);
        inbox
    }

    #[test]
    fn a_bar_switch_waits_for_the_bar_and_a_held_section_returns() {
        let mut p = player();
        assert!((run(&mut p, 0.5) - 0.1).abs() < 1e-6, "the start section");
        assert_eq!(post(&mut p, "Music_Start").state, 5);
        assert!((run(&mut p, 1.4) - 0.1).abs() < 1e-6);
        assert!((run(&mut p, 0.2) - 0.2).abs() < 1e-6);
        post(&mut p, "Danger");
        assert!((run(&mut p, 0.1) - 0.3).abs() < 1e-6, "at once, the section left held");
        post(&mut p, "Back");
        assert!((run(&mut p, 0.1) - 0.2).abs() < 1e-6, "back to the held section");
        let mut inbox = ScoreInbox { control: Some(70), ..Default::default() };
        p.take(&mut inbox);
        assert!((run(&mut p, 0.1) - 0.25).abs() < 1e-6, "level 70 takes the level-50 action: the drums in");
        post(&mut p, "Hit");
        assert!((run(&mut p, 0.05) - 0.75).abs() < 1e-6, "the stinger over the section");
        assert!((run(&mut p, 0.1) - 0.25).abs() < 1e-6, "the stinger ended");
    }

    #[test]
    fn a_section_that_does_not_repeat_ends_and_a_stop_fades_out() {
        let mut p = player();
        post(&mut p, "Win");
        assert!((run(&mut p, 0.5) - 0.4).abs() < 1e-6);
        assert_eq!(run(&mut p, 0.6), 0.0, "the 1 s section without `next` ended");
        assert_eq!(post(&mut p, "Music_Start").state, 5, "nothing played: a bar switch takes place at once");
        assert!((run(&mut p, 0.1) - 0.2).abs() < 1e-6);
        let mut p = player();
        post(&mut p, "Die");
        let mid = run(&mut p, 0.25);
        assert!(mid > 0.0 && mid < 0.1, "fading: {mid}");
        assert_eq!(run(&mut p, 0.3), 0.0);
        assert_eq!(post(&mut p, "Hit").state, 1, "stopped");
        assert_eq!(run(&mut p, 0.05), 0.0, "no stinger on a stopped score");
        let mut inbox = ScoreInbox { paused: Some(true), ..Default::default() };
        p.take(&mut inbox);
        assert_eq!(inbox.state, 3);
    }

    #[test]
    fn a_stem_must_play_in_step_with_its_section() {
        let id = "ssx:music/pathfinder/ssxdd/0".parse().unwrap();
        let ssx_content::audio::music::MusicDef::Interactive(def) =
            ssx_content::audio::music::parse_toml(SCORE, Some(&id)).unwrap()
        else {
            panic!("not a score")
        };
        let mut d = Decoded::new();
        for f in ["intro.wav", "main.wav", "danger.wav", "win.wav", "hit.wav"] {
            d.insert(f.to_string(), constant(0.1, 4.0));
        }
        d.insert("drums.wav".to_string(), constant(0.1, 3.0));
        let e = ScorePlayer::new(def, &d).err().unwrap();
        assert!(e.contains("stems play in step"), "{e}");
    }
}
