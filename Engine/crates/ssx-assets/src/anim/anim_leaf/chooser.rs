use super::*;

pub struct ChooserInst {
    ctrl: CtrlId,
    choice: i64,
    child: Option<Box<dyn Controller>>,
    blend: Option<SwitchBlend>,
    exited: bool,
    partial: Option<Delta>,
}

fn choose(gs: &GameState, policy: &str, signal: Option<SigId>, entries: &[ChooserEntry], prev: i64) -> usize {
    let n = entries.len();
    let pick: i64 = match policy {
        "EnumChooserPolicyAsset" => {
            let v = signal.map_or(-1, |s| gs.get(s) as i64);
            if v == -1 {
                -1
            } else {
                entries
                    .iter()
                    .position(|e| matches!(e.selector, Selector::Enum(x) if x == v))
                    .or_else(|| entries.iter().position(|e| matches!(e.selector, Selector::Enum(-1))))
                    .map_or(-1, |i| i as i64)
            }
        }
        "LinearSignalChooserPolicyAsset" => {
            let s = match entries.first().map(|e| &e.selector) {
                Some(Selector::Linear { signal: Some(sig), .. }) => gs.get(*sig),
                _ => 0.0,
            };
            let mut best = (1e35f32, -1i64);
            for (i, e) in entries.iter().enumerate() {
                let Selector::Linear { threshold, .. } = e.selector else { break };
                let d = (threshold - s).abs();
                if d < best.0 {
                    best = (d, i as i64);
                } else {
                    break;
                }
            }
            best.1
        }
        "SignalChooserPolicyAsset" => entries
            .iter()
            .position(|e| match &e.selector {
                Selector::Signals { all_true, all_false } => {
                    all_true.iter().all(|&x| gs.bool(x)) && all_false.iter().all(|&x| !gs.bool(x))
                }
                _ => true,
            })
            .map_or(-1, |i| i as i64),
        "RandomChooserPolicyAsset" => {
            let weight = |e: &ChooserEntry| match e.selector {
                Selector::Random { weight } => weight,
                _ => 0.0,
            };
            let total: f32 = entries.iter().map(weight).sum();
            if total > 0.0 {
                let mut r = total * f32::from(gs.random_u16()) * 1.525_902_2e-5;
                entries
                    .iter()
                    .position(|e| {
                        r -= weight(e);
                        r <= 0.0
                    })
                    .map_or(n as i64 - 1, |i| i as i64)
            } else {
                -1
            }
        }
        _ => prev.max(0),
    };
    if pick < 0 || pick as usize >= n { n.saturating_sub(1) } else { pick as usize }
}

struct ChooserData<'a> {
    policy: &'a str,
    signal: Option<SigId>,
    entries: &'a [ChooserEntry],
    every_update: bool,
    as_blend_slave: bool,
    looping: bool,
    on_loop: bool,
    phase_match: bool,
    blend: Option<BlendId>,
}

fn chooser_data<'a>(env: &Env<'a>, ctrl: CtrlId) -> Option<ChooserData<'a>> {
    match &env.g.ctrls[ctrl].1 {
        Ctrl::Chooser {
            policy,
            policy_signal,
            entries,
            choose_every_update,
            choose_as_blend_slave,
            looping,
            choose_on_loop,
            phase_match_previous,
            blend,
            ..
        } => Some(ChooserData {
            policy,
            signal: *policy_signal,
            entries,
            every_update: *choose_every_update,
            as_blend_slave: *choose_as_blend_slave,
            looping: *looping,
            on_loop: *choose_on_loop,
            phase_match: *phase_match_previous,
            blend: *blend,
        }),
        _ => None,
    }
}

pub fn chooser_is_dynamic(env: &Env, ctrl: CtrlId) -> bool {
    matches!(&env.g.ctrls[ctrl].1, Ctrl::Chooser { choose_on_reset, choose_every_update, looping, .. }
        if *choose_on_reset || *choose_every_update || *looping)
}

pub fn chooser_static(env: &Env, gs: &GameState, ctrl: CtrlId) -> Option<CtrlId> {
    let d = chooser_data(env, ctrl)?;
    let i = choose(gs, d.policy, d.signal, d.entries, -1);
    d.entries.get(i)?.controller
}

impl ChooserInst {
    pub fn new(env: &Env, gs: &GameState, ctrl: CtrlId) -> Self {
        let mut c = ChooserInst { ctrl, choice: -1, child: None, blend: None, exited: false, partial: None };
        c.rechoose(env, gs, false);
        c
    }

    fn rechoose(&mut self, env: &Env, gs: &GameState, looped: bool) {
        let Some(d) = chooser_data(env, self.ctrl) else { return };
        let i = choose(gs, d.policy, d.signal, d.entries, self.choice) as i64;
        if i != self.choice || self.child.is_none() {
            let new = d.entries.get(i as usize).and_then(|e| e.controller).map(|c| instantiate(env, gs, c));
            let old_phase = self.child.as_ref().map_or(0.0, |c| phase(c.as_ref()));
            match (self.child.take(), d.blend, new) {
                (Some(old), Some(b), Some(new)) if self.choice != -1 => {
                    self.blend = Some(SwitchBlend { blend: env.g.blends[b].clone(), elapsed: 0.0, old });
                    self.child = Some(new);
                }
                (_, _, new) => {
                    self.blend = None;
                    self.child = new;
                }
            }
            if let Some(c) = &mut self.child
                && !looped
                && d.phase_match
                && old_phase > 0.0
            {
                set_phase(env, gs, c.as_mut(), old_phase);
            }
            if self.exited
                && let Some(c) = &mut self.child
            {
                c.exit(env, gs);
            }
            self.choice = i;
        }
        if looped && let Some(c) = &mut self.child {
            set_phase(env, gs, c.as_mut(), 0.0);
        }
    }
}

impl ChooserInst {
    fn composed_delta(&self, env: &Env, gs: &GameState) -> Delta {
        let new = self.child.as_ref().map_or(Delta::None, |c| c.delta(env, gs));
        match &self.blend {
            Some(b) => crate::anim_eval::blend_delta(env, &b.blend, b.elapsed, b.old.delta(env, gs), new),
            None => new,
        }
    }
}

impl Controller for ChooserInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, mut dt: f32) {
        let Some(d) = chooser_data(env, self.ctrl) else { return };
        let mut looped = false;
        self.partial = None;
        let remaining = self.child.as_ref().map(|c| c.length() - c.time());
        if d.looping
            && let Some(remaining) = remaining
            && remaining <= dt
        {
            looped = true;
            let part = remaining.max(0.0);
            if let Some(b) = &mut self.blend {
                b.update(env, gs, part);
            }
            if let Some(c) = &mut self.child {
                c.update(env, gs, part);
            }
            self.partial = Some(self.composed_delta(env, gs));
            if self.blend.as_ref().is_some_and(|b| b.done()) {
                self.blend = None;
            }
            dt = (dt - remaining).max(0.0);
        }
        let reset = (d.on_loop && looped) || d.every_update;
        if self.child.is_none() || (reset && (d.as_blend_slave || !self.exited)) {
            self.rechoose(env, gs, looped);
        }
        if let Some(b) = &mut self.blend {
            b.update(env, gs, dt);
            if b.done() {
                self.blend = None;
            }
        }
        if let Some(c) = &mut self.child {
            c.update(env, gs, dt);
        }
    }

    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        let mut new = AnimPose::empty(env.bind);
        if let Some(c) = &self.child {
            c.pose(env, gs, &mut new);
        }
        match &self.blend {
            Some(b) => {
                let mut old = AnimPose::empty(env.bind);
                b.old.pose(env, gs, &mut old);
                crate::anim_eval::apply_blend(env, &b.blend, b.elapsed, &mut old, &new);
                *out = old;
            }
            None => *out = new,
        }
    }

    fn delta(&self, env: &Env, gs: &GameState) -> Delta {
        let last = self.composed_delta(env, gs);
        self.partial.map_or(last, |p| Delta::accumulate(p, last, env.physics))
    }

    fn time(&self) -> f32 {
        self.child.as_ref().map_or(0.0, |c| c.time())
    }
    fn length(&self) -> f32 {
        self.child.as_ref().map_or(0.0, |c| c.length())
    }
    fn looping(&self) -> bool {
        self.child.as_ref().is_some_and(|c| c.looping())
    }
    fn set_time(&mut self, env: &Env, gs: &GameState, t: f32) {
        if let Some(c) = &mut self.child {
            c.set_time(env, gs, t);
        }
    }
    fn windows(&self, env: &Env) -> Vec<Window> {
        self.child.as_ref().map_or_else(Vec::new, |c| c.windows(env))
    }
    fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        let wo = self.blend.as_ref().map_or(0.0, |b| b.old_weight());
        if let Some(c) = &self.child {
            c.label(env, weight * (1.0 - wo), out);
        }
        if let Some(b) = &self.blend {
            b.old.label(env, weight * wo, out);
        }
    }
    fn path(&self, env: &Env, out: &mut Vec<(String, f32)>) {
        if let Some(c) = &self.child {
            c.path(env, out);
        }
    }
    fn exit(&mut self, env: &Env, gs: &GameState) {
        self.exited = true;
        if let Some(c) = &mut self.child {
            c.exit(env, gs);
        }
        if let Some(b) = &mut self.blend {
            b.old.exit(env, gs);
        }
    }
}
