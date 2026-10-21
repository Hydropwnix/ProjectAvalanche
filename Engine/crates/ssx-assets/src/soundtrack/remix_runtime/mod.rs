mod expression;
mod inputs;
mod process;
mod update;
mod voice;

use std::path::Path;

use anyhow::Result;
use ssx_formats::ptch::{BaseType, Instr, ModuleKind, Patch, Ref};

use crate::sfx_bus::{CorePlugin, Graph, VarRef};

pub use inputs::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    I(i32),
    U(u32),
    F(f32),
    D(f64),
    B(bool),
}

type Loc = (usize, usize, BaseType);

#[derive(Default)]
struct Mem {
    blocks: Vec<Vec<u8>>,
}

impl Mem {
    fn add(&mut self, init: &[u8]) -> usize {
        self.blocks.push(init.to_vec());
        self.blocks.len() - 1
    }

    fn word(&self, (b, o, _): Loc) -> u32 {
        self.blocks[b].get(o..o + 4).map_or(0, |w| u32::from_be_bytes(w.try_into().unwrap_or([0; 4])))
    }

    fn set_word(&mut self, (b, o, _): Loc, w: u32) {
        if let Some(d) = self.blocks[b].get_mut(o..o + 4) {
            d.copy_from_slice(&w.to_be_bytes());
        }
    }

    fn read(&self, l: Loc) -> Option<Val> {
        let w = self.word(l);
        Some(match l.2 {
            BaseType::Word | BaseType::Int => Val::I(w as i32),
            BaseType::UInt => Val::U(w),
            BaseType::Float => Val::F(f32::from_bits(w)),
            BaseType::Double => {
                let lo = self.word((l.0, l.1 + 4, l.2));
                Val::D(f64::from_bits(u64::from(w) << 32 | u64::from(lo)))
            }
            BaseType::Bool => Val::B(w != 0),
            _ => return None,
        })
    }

    fn write(&mut self, l: Loc, v: Val) {
        let num = match v {
            Val::I(x) => f64::from(x),
            Val::U(x) => f64::from(x),
            Val::F(x) => f64::from(x),
            Val::D(x) => x,
            Val::B(b) => {
                if l.2 == BaseType::Bool {
                    self.set_word(l, u32::from(b));
                }
                return;
            }
        };
        match l.2 {
            BaseType::Word | BaseType::Int => self.set_word(l, num as i32 as u32),
            BaseType::UInt => self.set_word(l, num as i64 as u32),
            BaseType::Float => self.set_word(l, (num as f32).to_bits()),
            BaseType::Double => {
                let bits = num.to_bits();
                self.set_word(l, (bits >> 32) as u32);
                self.set_word((l.0, l.1 + 4, l.2), bits as u32);
            }
            _ => {}
        }
    }
}

fn as_f64(v: Option<Val>) -> f64 {
    match v {
        Some(Val::I(x)) => f64::from(x),
        Some(Val::U(x)) => f64::from(x),
        Some(Val::F(x)) => f64::from(x),
        Some(Val::D(x)) => x,
        _ => 0.0,
    }
}

fn as_i32(v: Option<Val>) -> i32 {
    match v {
        Some(Val::I(x)) => x,
        Some(Val::B(_)) | None => 0,
        other => as_f64(other) as i32,
    }
}

fn as_u32(v: Option<Val>) -> u32 {
    match v {
        Some(Val::U(x)) => x,
        Some(Val::I(x)) => x as u32,
        Some(Val::B(_)) | None => 0,
        other => as_f64(other) as i64 as u32,
    }
}

fn as_bool(v: Option<Val>) -> bool {
    matches!(v, Some(Val::B(true)))
}

#[derive(Clone, Copy, Debug)]
struct EnvVoice {
    t0: f64,
    a_end: f32,
    d_end: f32,
    s_end: f32,
    r_end: f32,
    a_lvl: f32,
    s_lvl: f32,
    infinite: bool,
}

#[derive(Debug, Default)]
struct VoiceLink {
    slot: usize,
    params: Vec<f64>,
    bypass: Vec<bool>,
    sends: Vec<Option<usize>>,
}

#[derive(Debug)]
enum ModState {
    Stateless,
    Trigger { timer: f32, count: i32, has_prev: bool, prev: [bool; 3] },
    Envelope { time: f64, last: [u32; 2], voices: Vec<Option<EnvVoice>> },
    Last(Vec<u32>),
    Curve(Vec<f64>),
    Metronome { last: [u32; 2], next: f32, running: bool },
    Voice(VoiceLink),
}

struct Inst {
    name: &'static str,
    patch: Patch,
    map: Vec<usize>,
    state: Vec<ModState>,
}

pub struct Remix {
    mem: Mem,
    insts: Vec<Inst>,
    stamp: u32,
    pub inputs: GameInputs,
    pub remix_setting: bool,
    remix_on: bool,
    remix_dirty: bool,
    handles: Vec<usize>,
    pitch_var: Option<VarRef>,
    pub trace: Vec<String>,
    ticks: u64,
}

const DT: f32 = crate::remix::TICK;

impl Remix {
    pub fn load(big_root: &Path, g: &mut Graph) -> Result<Self> {
        let set = crate::remix::RemixPatches::load(big_root)?;
        let mut mem = Mem::default();
        let mut outputs = Vec::new();
        let mut insts = Vec::new();
        for (name, p) in set.patches {
            let mut map = vec![usize::MAX; p.ports.len() + 1];
            for (k, port) in p.ports.iter().enumerate().filter(|(_, q)| !q.input) {
                map[k] = mem.add(&port.init);
                outputs.push((port.name.clone(), p.interfaces[port.interface as usize].hash, map[k]));
            }
            map[p.ports.len()] = mem.add(&p.locals_init);
            insts.push(Inst { name, patch: p, map, state: Vec::new() });
        }
        for inst in &mut insts {
            for (k, port) in inst.patch.ports.iter().enumerate().filter(|(_, q)| q.input) {
                let hash = inst.patch.interfaces[port.interface as usize].hash;
                inst.map[k] = match outputs.iter().find(|o| o.0 == port.name && o.1 == hash) {
                    Some(o) => o.2,
                    None => mem.add(&port.init),
                };
            }
        }
        let mut r = Self {
            mem,
            insts,
            stamp: 0,
            inputs: GameInputs::default(),
            remix_setting: true,
            remix_on: false,
            remix_dirty: false,
            handles: Vec::new(),
            pitch_var: None,
            trace: Vec::new(),
            ticks: 0,
        };
        for i in 0..r.insts.len() {
            let states = (0..r.insts[i].patch.modules.len()).map(|m| r.init_module(i, m, g)).collect();
            r.insts[i].state = states;
        }
        r.pitch_var = Some(g.vars.resolve(0, "gPitchIn"));
        // TODO(guess): eager startup pre-runs replace the retail pending-port readiness lifecycle (0x83612830 leaves input pointers null; 0x836110A8 gates activation in 0x83618C68 on all bound ports): each wrapper's frame callback 0x830F9188 moves one phase per call (resource request 0x835FAB08, load poll 0x830F90A0, create with key 0, connect ports, run),
        // new instances wait on the pending list System +0x18C, and 0x83618C68 moves only the ready ones (instance +0x40: every port pointer bound) to the active list +0x19C; settle: the resource / tick phases (when a resource completes, the wrapper frames against the audio jobs: a trace or a capture) before porting the schedule.
        r.run(g);
        r.remix_on = true;
        if let Some(l) = r.member("HarmonyMusicBuses", "Harmony Music Interface", "Remix Enabled") {
            r.mem.write(l, Val::B(true));
        }
        r.run(g);
        if let Some(l) = r.member("HarmonyMusicBuses", "Harmony Music Interface", "Music Input") {
            let acc = r.handle_acc(r.mem.word(l));
            g.redirect_send("MusicSubmix", acc);
            g.redirect_send("UserMusicSubmix", acc);
        }
        if let (Some(acc), Some(l)) =
            (g.submix_input("Music_Master_BUS"), r.member("MainDspPatch", "Music Master Out", "Music Output"))
        {
            let h = r.handle(acc);
            r.mem.set_word(l, h);
        }
        Ok(r)
    }

    fn handle(&mut self, acc: usize) -> u32 {
        if let Some(k) = self.handles.iter().position(|&a| a == acc) {
            return k as u32 + 1;
        }
        self.handles.push(acc);
        self.handles.len() as u32
    }

    fn handle_acc(&self, h: u32) -> Option<usize> {
        (h as usize).checked_sub(1).and_then(|k| self.handles.get(k)).copied()
    }

    fn loc(&self, i: usize, r: Ref) -> Option<Loc> {
        if r.is_none() {
            return None;
        }
        let b = *self.insts[i].map.get(r.block())?;
        let base = r.vtype().base();
        (r.offset() + base.size() <= self.mem.blocks.get(b)?.len()).then_some((b, r.offset(), base))
    }

    fn member(&self, inst: &str, port: &str, name: &str) -> Option<Loc> {
        let k = self.insts.iter().position(|x| x.name == inst)?;
        let p = &self.insts[k].patch;
        let (pi, q) = p.ports.iter().enumerate().find(|(_, q)| q.name == port)?;
        let l = &p.interfaces[q.interface as usize].layout;
        let m = [&l.events, &l.signals, &l.fields].into_iter().flatten().flatten().find(|m| m.name == name)?;
        Some((self.insts[k].map[pi], m.offset as usize, m.vtype.base()))
    }

    fn get(&self, i: usize, r: Ref) -> Option<Val> {
        self.loc(i, r).and_then(|l| self.mem.read(l))
    }

    fn f64(&self, i: usize, r: Ref) -> f64 {
        as_f64(self.get(i, r))
    }

    fn f32(&self, i: usize, r: Ref) -> f32 {
        as_f64(self.get(i, r)) as f32
    }

    fn set(&mut self, i: usize, r: Ref, v: Val) {
        if let Some(l) = self.loc(i, r) {
            self.mem.write(l, v);
        }
    }

    fn copy(&mut self, i: usize, src: Ref, dst: Ref) {
        if let Some(v) = self.get(i, src) {
            self.set(i, dst, v);
        }
    }

    fn post_event(&mut self, l: Loc) {
        self.stamp += 1;
        self.mem.set_word(l, self.stamp);
    }

    fn post(&mut self, i: usize, r: Ref) {
        if let Some(l) = self.loc(i, r) {
            self.post_event(l);
        }
    }

    fn fired(&self, i: usize, r: Ref, last: &mut u32) -> bool {
        let Some(l) = self.loc(i, r) else { return false };
        let v = self.mem.word(l);
        let f = *last < v;
        if f {
            *last = v;
        }
        f
    }
}

fn env_phase(v: &EnvVoice, t: f64) -> (u8, f32) {
    let t0 = v.t0;
    let phase = if t < t0 {
        0
    } else if t < f64::from(v.a_end) + t0 {
        1
    } else if t < f64::from(v.d_end) + t0 {
        2
    } else if !v.infinite && f64::from(v.s_end) + t0 <= t {
        if t < f64::from(v.r_end) + t0 { 4 } else { 5 }
    } else {
        3
    };
    let rel = (t - t0) as f32;
    let lerp = |x0: f32, x1: f32, y0: f32, y1: f32| ((rel - x0) / (x1 - x0)) * (y1 - y0) + y0;
    let level = match phase {
        1 => lerp(0.0, v.a_end, 0.0, v.a_lvl),
        2 => lerp(v.a_end, v.d_end, v.a_lvl, v.s_lvl),
        3 => v.s_lvl,
        4 => lerp(v.s_end, v.r_end, v.s_lvl, 0.0),
        _ => 0.0,
    };
    (phase, level)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(a: f32, d: f32, s: f32, r: f32, a_lvl: f32, s_lvl: f32, infinite: bool) -> EnvVoice {
        EnvVoice { t0: 1.0, a_end: a, d_end: a + d, s_end: a + d + s, r_end: a + d + s + r, a_lvl, s_lvl, infinite }
    }

    #[test]
    fn envelope_stages_interpolate_relative_to_the_release_level() {
        let v = voice(0.0, 2.0, 0.0, 0.3, 0.0, -1.0, true);
        assert_eq!(env_phase(&v, 0.5).0, 0);
        assert_eq!(env_phase(&v, 1.0), (2, 0.0));
        assert!((env_phase(&v, 2.0).1 + 0.5).abs() < 1e-6);
        assert_eq!(env_phase(&v, 10.0), (3, -1.0));
        let v = voice(1.0, 0.0, 1.0, 1.0, 1.0, 0.5, false);
        assert!((env_phase(&v, 1.5).1 - 0.5).abs() < 1e-6);
        assert_eq!(env_phase(&v, 2.5), (3, 0.5));
        assert!((env_phase(&v, 3.5).1 - 0.25).abs() < 1e-6);
        assert_eq!(env_phase(&v, 4.5).0, 5);
    }

    #[test]
    fn posts_follow_the_controller_gate_and_rewind_latch() {
        let mut g = GameInputs::default();
        g.post("BoostStart", 0.0);
        g.post("RaceEventType", 1.0);
        assert!(g.events.is_empty() && g.race_event_type == 1, "disabled: only the event type");
        g.enabled = true;
        g.armed = true;
        g.post("BoostStart", 0.0);
        g.post("WipeoutStart", 1.0);
        g.post("UpdateCarve", -0.4);
        assert_eq!(g.events, ["Boost Start"]);
        assert_eq!(g.carve, -0.4);
        let mut f = FrameInputs { rewinding: true, dead_by_death_plane: true, ..FrameInputs::default() };
        g.frame(&f);
        g.events_frame(0.0);
        g.frame(&f);
        g.events_frame(16.0);
        assert_eq!(g.events, ["Boost Start", "Rewind Start", "Player Death Start"]);
        g.events.clear();
        f.rewinding = false;
        g.frame(&f);
        g.events_frame(33.0);
        assert_eq!(g.events, ["Rewind Stop"], "the death post waits for the flag to clear");
    }

    #[test]
    fn a_death_timer_end_posts_the_death_and_disables_the_posts_two_seconds_later() {
        let mut g = GameInputs { enabled: true, armed: true, ..GameInputs::default() };
        let f = FrameInputs { ended_by_death: true, ..FrameInputs::default() };
        g.frame(&f);
        g.events_frame(0.0);
        assert_eq!(g.events, ["Player Death Start"]);
        let updates = (1..200).take_while(|&k| {
            if k % 2 == 0 {
                g.frame(&f);
            }
            g.events_frame(f64::from(k) * 1000.0 / 60.0);
            g.enabled
        });
        assert_eq!(updates.count(), 120, "disabled at the first update after the 2 s timer has passed");
        assert_eq!(g.events, ["Player Death Start"], "later posts are dropped");
    }
}
