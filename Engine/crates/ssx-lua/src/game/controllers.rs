pub mod tracks;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub use tracks::Track;

use super::graph::{Arg, Graph, ObjId};
use super::objectives::{Cmp, GsType, GsValue, Name, game_state_type};

#[derive(Clone, Debug, PartialEq)]
pub struct TrackDef {
    pub kind: &'static str,
    pub args: Vec<Arg>,
    pub methods: Vec<(&'static str, Vec<Arg>)>,
    pub condition: Option<usize>,
    pub start: f32,
    pub end: f32,
}

impl TrackDef {
    pub fn arg(&self, i: usize) -> &Arg {
        self.args.get(i - 1).unwrap_or(&Arg::Nil)
    }

    pub fn has(&self, method: &str) -> bool {
        self.methods.iter().any(|(m, _)| *m == method)
    }

    pub fn method(&self, method: &str) -> Option<&[Arg]> {
        self.methods.iter().rev().find(|(m, _)| *m == method).map(|(_, a)| a.as_slice())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CondDef {
    And(usize, usize),
    Or(usize, usize),
    Not(usize),
    GameState { id: u16, cmp: Cmp, value: GsValue },
    GearItem(i32),
    GearStat { stat: i32, cmp: Cmp, value: i32 },
    NamedFlag(Name),
    PlayerProximity(f32),
    PostedEvent(Name),
    GlobalPostedEvent(i32),
    StateTime(f32),
    TriggerVolume { volume: String, entity: String, client: String },
    NotPorted(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub target: Name,
    pub conditions: Vec<usize>,
    pub subs: Vec<usize>,
    pub tracks: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: Name,
    pub enter: Vec<usize>,
    pub update: Vec<usize>,
    pub exit: Vec<usize>,
    pub transitions: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateSet {
    pub name: Name,
    pub states: Vec<State>,
}

impl StateSet {
    pub fn state(&self, name: &Name) -> Option<usize> {
        self.states.iter().position(|s| s.name == *name)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateSets {
    pub sets: Vec<StateSet>,
    pub tracks: Vec<TrackDef>,
    pub kinds: Vec<Track>,
    pub conditions: Vec<CondDef>,
    pub transitions: Vec<Transition>,
    pub scripts: Vec<(String, Option<String>)>,
    pub problems: Vec<String>,
}

impl StateSets {
    pub fn set(&self, name: &str) -> Option<usize> {
        let n = Name::new(name);
        self.sets.iter().position(|s| s.name == n)
    }

    pub fn from_graph(g: &Graph) -> Self {
        #[derive(Clone, Copy)]
        enum O {
            Set(usize),
            State(usize, usize),
            Transition(usize),
            Track(usize),
            Cond(usize),
        }
        let mut out = StateSets { scripts: g.scripts.clone(), ..Default::default() };
        let mut objs: BTreeMap<ObjId, O> = BTreeMap::new();
        let text = |a: &Arg| a.str().map(Name::new).unwrap_or_else(|| Name::new(""));
        for c in &g.calls {
            let b = c.binding;
            let recv = if b.recv.is_empty() { None } else { c.arg(1).obj().and_then(|o| objs.get(&o).copied()) };
            let cond = |i: usize, objs: &BTreeMap<ObjId, O>| match c.arg(i).obj().and_then(|o| objs.get(&o)) {
                Some(O::Cond(k)) => Some(*k),
                _ => None,
            };
            let track = |i: usize, objs: &BTreeMap<ObjId, O>| match c.arg(i).obj().and_then(|o| objs.get(&o)) {
                Some(O::Track(k)) => Some(*k),
                _ => None,
            };
            let made: Option<O> = match (b.name, recv) {
                ("CreateStateSet", _) => {
                    out.sets.push(StateSet { name: text(c.arg(1)), states: Vec::new() });
                    Some(O::Set(out.sets.len() - 1))
                }
                ("AddState", Some(O::Set(s))) => {
                    let st = State {
                        name: text(c.arg(2)),
                        enter: Vec::new(),
                        update: Vec::new(),
                        exit: Vec::new(),
                        transitions: Vec::new(),
                    };
                    out.sets[s].states.push(st);
                    Some(O::State(s, out.sets[s].states.len() - 1))
                }
                ("AddTransition" | "AddSubTransition", r) => {
                    let target = text(c.arg(2));
                    out.transitions.push(Transition {
                        target,
                        conditions: Vec::new(),
                        subs: Vec::new(),
                        tracks: Vec::new(),
                    });
                    let t = out.transitions.len() - 1;
                    match r {
                        Some(O::State(s, k)) if b.name == "AddTransition" => out.sets[s].states[k].transitions.push(t),
                        Some(O::Transition(p)) if b.name == "AddSubTransition" => out.transitions[p].subs.push(t),
                        _ => out.problems.push(format!("{} on no receiver", b.name)),
                    }
                    Some(O::Transition(t))
                }
                ("AddCondition", Some(O::Transition(t))) => {
                    match cond(2, &objs) {
                        Some(k) => out.transitions[t].conditions.push(k),
                        None => out.problems.push("AddCondition without a condition".into()),
                    }
                    None
                }
                ("AddTransitionTrack", Some(O::Transition(t))) => {
                    out.transitions[t].tracks.extend(track(2, &objs));
                    None
                }
                ("AddEnterTrack" | "AddExitTrack" | "AddUpdateTrack", Some(O::State(s, k))) => {
                    if let Some(tr) = track(2, &objs) {
                        let st = &mut out.sets[s].states[k];
                        match b.name {
                            "AddEnterTrack" => st.enter.push(tr),
                            "AddExitTrack" => st.exit.push(tr),
                            _ => {
                                out.tracks[tr].start = c.arg(3).num().unwrap_or(0.0) as f32;
                                out.tracks[tr].end = c.arg(4).num().unwrap_or(-1.0) as f32;
                                st.update.push(tr);
                            }
                        }
                    }
                    None
                }
                ("SetCondition", Some(O::Track(t))) => {
                    out.tracks[t].condition = cond(2, &objs);
                    None
                }
                (m, Some(O::Track(t))) => {
                    out.tracks[t].methods.push((m, c.args[1..].to_vec()));
                    None
                }
                (name, None) if name.ends_with("Track") => {
                    let def = TrackDef {
                        kind: name,
                        args: c.args.clone(),
                        methods: Vec::new(),
                        condition: None,
                        start: 0.0,
                        end: 0.0,
                    };
                    out.tracks.push(def);
                    Some(O::Track(out.tracks.len() - 1))
                }
                (name, None) if name.ends_with("Condition") || matches!(name, "AND" | "OR" | "NOT") => {
                    let d = cond_def(c, &|i| cond(i, &objs));
                    out.conditions.push(d);
                    Some(O::Cond(out.conditions.len() - 1))
                }
                (name, _) if name.starts_with("Float") => None,
                (name, _) => {
                    out.problems.push(format!("{name}: no receiver it applies to"));
                    None
                }
            };
            if let (Some(o), Some(m)) = (c.ret, made) {
                objs.insert(o, m);
            }
        }
        out.kinds = out.tracks.iter().map(Track::of).collect();
        out
    }
}

fn cond_def(c: &super::graph::Call, cond: &dyn Fn(usize) -> Option<usize>) -> CondDef {
    let num = |i: usize| c.arg(i).num().unwrap_or(0.0);
    let int = |i: usize| crate::number::to_int(num(i));
    let cmp = |i: usize| Cmp::from_i32(int(i)).unwrap_or(Cmp::EqualTo);
    let s = |i: usize| c.arg(i).str().unwrap_or_default().to_string();
    match c.name() {
        "AND" => CondDef::And(cond(1).unwrap_or(usize::MAX), cond(2).unwrap_or(usize::MAX)),
        "OR" => CondDef::Or(cond(1).unwrap_or(usize::MAX), cond(2).unwrap_or(usize::MAX)),
        "NOT" => CondDef::Not(cond(1).unwrap_or(usize::MAX)),
        "GameStateCondition" => {
            let id = int(1);
            let value = match game_state_type(id) {
                Some(GsType::Bool) => GsValue::Bool(c.arg(3).truthy()),
                Some(GsType::Float) => GsValue::Float(num(3) as f32),
                Some(GsType::Int) => GsValue::Int(int(3)),
                _ => return CondDef::NotPorted("GameStateCondition of a vector / quaternion"),
            };
            CondDef::GameState { id: id as u16, cmp: cmp(2), value }
        }
        "GearItemCondition" => CondDef::GearItem(int(1)),
        "GearStatCondition" => CondDef::GearStat { stat: int(1), cmp: cmp(2), value: int(3) },
        "NamedFlagCondition" => CondDef::NamedFlag(Name::new(&s(1))),
        "PlayerProximityCondition" => CondDef::PlayerProximity(num(1) as f32),
        "PostedEventCondition" => CondDef::PostedEvent(Name::new(&s(1))),
        "GlobalPostedEventCondition" => CondDef::GlobalPostedEvent(int(1)),
        "StateTimeCondition" => CondDef::StateTime(num(1) as f32),
        "TriggerVolumeCondition" => CondDef::TriggerVolume { volume: s(1), entity: s(2), client: s(3) },
        other => CondDef::NotPorted(other),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Begin,
    Update(f32),
    End,
}

#[derive(Clone, Copy, Debug)]
pub struct TrackCall<'a> {
    pub track: usize,
    pub def: &'a TrackDef,
    pub kind: &'a Track,
    pub inst: u64,
}

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

fn new_instance() -> u64 {
    NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed)
}

pub trait ControllerWorld {
    fn game_state(&mut self, _id: u16) -> Option<GsValue> {
        None
    }
    fn gear_item(&mut self, _item: i32) -> Option<bool> {
        None
    }
    fn gear_stat(&mut self, _stat: i32) -> Option<i32> {
        None
    }
    fn named_flag(&mut self, _name: &Name) -> bool {
        false
    }
    fn posted_event(&mut self, _event: &Name) -> bool {
        false
    }
    fn global_posted_event(&mut self, _i: i32) -> bool {
        false
    }
    fn trigger_volume(&mut self, _volume: &str, _entity: &str, _client: &str) -> Option<bool> {
        None
    }
    fn player_distance(&mut self) -> Option<f32> {
        None
    }
    fn track(&mut self, _call: TrackCall<'_>, _phase: Phase) {}
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInst {
    pub track: usize,
    pub id: u64,
    pub started: bool,
    pub ended: bool,
    pub sub: Option<Box<Controller>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransInst {
    pub transition: usize,
    pub conditions: Vec<usize>,
    pub subs: Vec<TransInst>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    pub sets: Arc<StateSets>,
    pub set: usize,
    pub state: Option<usize>,
    pub time: f32,
    pub tracks: Vec<TrackInst>,
    pub transitions: Vec<TransInst>,
}

impl Controller {
    pub fn new(sets: Arc<StateSets>, set: usize) -> Self {
        Controller { sets, set, state: None, time: 0.0, tracks: Vec::new(), transitions: Vec::new() }
    }

    pub fn state_name(&self) -> Option<&str> {
        Some(&self.sets.sets[self.set].states[self.state?].name.text)
    }

    pub fn set_state(&mut self, name: Option<&Name>, w: &mut dyn ControllerWorld) {
        let set = &self.sets.sets[self.set];
        let next = match name {
            Some(n) => set.state(n),
            None => (!set.states.is_empty()).then_some(0),
        };
        self.exit(w);
        (self.state, self.time) = (next, 0.0);
        if let Some(k) = next {
            self.enter(k, w);
        }
    }

    fn call(&self, t: usize, inst: u64) -> TrackCall<'_> {
        TrackCall { track: t, def: &self.sets.tracks[t], kind: &self.sets.kinds[t], inst }
    }

    fn one_shot(&self, t: usize, w: &mut dyn ControllerWorld) {
        if let Some(i) = self.make_track(t, w) {
            w.track(self.call(t, i.id), Phase::Begin);
            w.track(self.call(t, i.id), Phase::Update(0.0));
            w.track(self.call(t, i.id), Phase::End);
        }
    }

    fn make_track(&self, t: usize, w: &mut dyn ControllerWorld) -> Option<TrackInst> {
        if let Some(c) = self.sets.tracks[t].condition
            && self.make_cond(c, w)
            && !self.test(c, w)
        {
            return None;
        }
        Some(TrackInst { track: t, id: new_instance(), started: false, ended: false, sub: None })
    }

    fn make_cond(&self, c: usize, w: &mut dyn ControllerWorld) -> bool {
        match &self.sets.conditions.get(c) {
            Some(CondDef::GearItem(i)) => w.gear_item(*i).is_some(),
            Some(CondDef::GearStat { stat, .. }) => w.gear_stat(*stat).is_some(),
            Some(_) => true,
            None => false,
        }
    }

    fn make_transition(&self, t: usize, w: &mut dyn ControllerWorld) -> TransInst {
        let def = &self.sets.transitions[t];
        let conditions = def.conditions.iter().copied().filter(|&c| self.make_cond(c, w)).collect();
        let subs = def.subs.iter().map(|&s| self.make_transition(s, w)).collect();
        TransInst { transition: t, conditions, subs }
    }

    fn enter(&mut self, k: usize, w: &mut dyn ControllerWorld) {
        let sets = self.sets.clone();
        let st = &sets.sets[self.set].states[k];
        for &t in &st.enter {
            self.one_shot(t, w);
        }
        for &t in &st.update {
            if let Some(i) = self.make_track(t, w) {
                self.tracks.push(i);
            }
        }
        for &t in &st.transitions {
            let i = self.make_transition(t, w);
            self.transitions.push(i);
        }
    }

    pub fn exit(&mut self, w: &mut dyn ControllerWorld) {
        for mut i in std::mem::take(&mut self.tracks) {
            if i.started && !i.ended {
                self.end_track(&mut i, w);
            }
        }
        self.transitions.clear();
        if let Some(k) = self.state.take() {
            let sets = self.sets.clone();
            for &t in &sets.sets[self.set].states[k].exit {
                self.one_shot(t, w);
            }
        }
    }

    fn test(&self, c: usize, w: &mut dyn ControllerWorld) -> bool {
        match &self.sets.conditions[c] {
            CondDef::And(a, b) => self.test(*a, w) && self.test(*b, w),
            CondDef::Or(a, b) => self.test(*a, w) || self.test(*b, w),
            CondDef::Not(a) => !self.test(*a, w),
            CondDef::GameState { id, cmp, value } => w.game_state(*id).is_some_and(|v| cmp.value(v, *value)),
            CondDef::GearItem(i) => w.gear_item(*i).unwrap_or(false),
            CondDef::GearStat { stat, cmp, value } => w.gear_stat(*stat).is_some_and(|v| cmp.i32(v, *value)),
            CondDef::NamedFlag(n) => w.named_flag(n),
            CondDef::PlayerProximity(d) => w.player_distance().is_some_and(|x| x < *d),
            CondDef::PostedEvent(e) => w.posted_event(e),
            CondDef::GlobalPostedEvent(i) => w.global_posted_event(*i),
            CondDef::StateTime(t) => self.time.partial_cmp(t).is_some_and(|o| o != std::cmp::Ordering::Less),
            CondDef::TriggerVolume { volume, entity, client } => {
                w.trigger_volume(volume, entity, client).unwrap_or(false)
            }
            CondDef::NotPorted(_) => false,
        }
    }

    fn fires(&self, t: &TransInst, w: &mut dyn ControllerWorld) -> Option<usize> {
        for &c in &t.conditions {
            if !self.test(c, w) {
                return None;
            }
        }
        if t.subs.is_empty() {
            return Some(t.transition);
        }
        t.subs.iter().find_map(|s| self.fires(s, w))
    }

    pub fn update(&mut self, dt: f32, w: &mut dyn ControllerWorld) {
        let fired = self.transitions.iter().find_map(|t| self.fires(t, w));
        if let Some(t) = fired {
            let sets = self.sets.clone();
            let tr = &sets.transitions[t];
            for &k in &tr.tracks {
                self.one_shot(k, w);
            }
            self.set_state(Some(&tr.target), w);
        }
        self.update_tracks(dt, w);
    }

    fn update_tracks(&mut self, dt: f32, w: &mut dyn ControllerWorld) {
        self.time += dt;
        let mut tracks = std::mem::take(&mut self.tracks);
        for i in &mut tracks {
            let (start, end) = (self.sets.tracks[i.track].start, self.sets.tracks[i.track].end);
            let mut d = dt;
            if !i.started {
                if start >= self.time {
                    continue;
                }
                self.begin_track(i, w);
                d = self.time - start;
            }
            if 0.0 < d && !i.ended {
                let ending = 0.0 <= end && end <= self.time;
                if ending {
                    d -= self.time - end;
                }
                self.run_track(i, d, w);
                if ending {
                    self.end_track(i, w);
                }
            }
        }
        self.tracks = tracks;
    }

    fn begin_track(&self, i: &mut TrackInst, w: &mut dyn ControllerWorld) {
        i.started = true;
        let def = &self.sets.tracks[i.track];
        if def.kind == "SubStateTrack" {
            let mut sub = Controller::new(self.sets.clone(), self.set);
            let name = def.arg(1).str().map(Name::new);
            sub.set_state(name.as_ref(), w);
            i.sub = Some(Box::new(sub));
        }
        w.track(self.call(i.track, i.id), Phase::Begin);
    }

    fn run_track(&self, i: &mut TrackInst, d: f32, w: &mut dyn ControllerWorld) {
        if let Some(sub) = i.sub.as_mut() {
            sub.update(d, w);
        }
        w.track(self.call(i.track, i.id), Phase::Update(d));
    }

    fn end_track(&self, i: &mut TrackInst, w: &mut dyn ControllerWorld) {
        if let Some(mut sub) = i.sub.take() {
            sub.exit(w);
        }
        w.track(self.call(i.track, i.id), Phase::End);
        i.ended = true;
    }

    pub fn states(&self) -> Vec<String> {
        let mut v: Vec<String> = self.state_name().map(str::to_string).into_iter().collect();
        for i in &self.tracks {
            if let Some(s) = &i.sub {
                v.extend(s.states());
            }
        }
        v
    }
}

#[cfg(test)]
mod tests;
