use super::*;

struct Elem {
    sub: Box<dyn Controller>,
    scale: f32,
    mirror: bool,
    blend: Option<(Blend, f32, Box<Elem>)>,
}

impl Elem {
    pub(super) fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        if let Some((b, e, old)) = &mut self.blend {
            *e = (*e + dt).min(b.blend_time.max(0.0));
            old.update(env, gs, dt);
            if *e >= b.blend_time || blend_new_weight(b, *e) >= 0.99 {
                self.blend = None;
            }
        }
        self.sub.update(env, gs, dt * self.scale);
    }

    pub(super) fn delta(&self, env: &Env, gs: &GameState) -> Delta {
        let d = self.sub.delta(env, gs);
        let d = if self.mirror { d.mirror() } else { d };
        match &self.blend {
            Some((b, e, old)) => crate::anim_eval::blend_delta(env, b, *e, old.delta(env, gs), d),
            None => d,
        }
    }

    pub(super) fn exit(&mut self, env: &Env, gs: &GameState) {
        self.sub.exit(env, gs);
        if let Some((_, _, old)) = &mut self.blend {
            old.exit(env, gs);
        }
    }

    pub(super) fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        let mut new = AnimPose::empty(env.bind);
        self.sub.pose(env, gs, &mut new);
        if self.mirror {
            new = env.rig.mirror(&new);
        }
        if let Some((b, e, old)) = &self.blend {
            let mut o = AnimPose::empty(env.bind);
            old.pose(env, gs, &mut o);
            crate::anim_eval::apply_blend(env, b, *e, &mut o, &new);
            new = o;
        }
        *out = new;
    }
}

pub struct SeqInst {
    ctrl: CtrlId,
    time: f32,
    length: f32,
    started: usize,
    current: Option<Elem>,
    delta: Option<Delta>,
}

fn seq_anims<'a>(env: &Env<'a>, ctrl: CtrlId) -> &'a [crate::anim_graph::SeqAnim] {
    match &env.g.ctrls[ctrl].1 {
        Ctrl::Sequence { anims, .. } => anims,
        _ => &[],
    }
}

impl SeqInst {
    pub fn new(env: &Env, gs: &GameState, ctrl: CtrlId) -> Self {
        let (length, start) = match &env.g.ctrls[ctrl].1 {
            Ctrl::Sequence { length, start_tick, .. } => (*length, *start_tick),
            _ => (0.0, 0.0),
        };
        let mut s = SeqInst { ctrl, time: 0.0, length, started: 0, current: None, delta: None };
        s.set_time(env, gs, start);
        s
    }

    fn begin(a: &crate::anim_graph::SeqAnim) -> f32 {
        a.start_time + a.start_in_time
    }

    fn start_due(&mut self, env: &Env, gs: &GameState, cut: bool) {
        let anims = seq_anims(env, self.ctrl);
        while let Some(a) = anims.get(self.started) {
            if Self::begin(a) > self.time + 1e-4 {
                break;
            }
            self.started += 1;
            let Some(c) = a.controller else { continue };
            let mut sub = instantiate(env, gs, c);
            let scale = if a.scale != 0.0 { a.scale } else { 1.0 };
            sub.set_time(env, gs, (a.start_in_time + (self.time - Self::begin(a))) * scale);
            let blend = match (self.current.take(), a.blend) {
                (Some(old), Some(b)) if !cut => Some((env.g.blends[b].clone(), 0.0, Box::new(old))),
                _ => None,
            };
            self.current = Some(Elem { sub, scale, mirror: a.flags & 1 != 0, blend });
        }
    }
}

impl Controller for SeqInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        let mut dt = dt.clamp(0.0, (self.length - self.time).max(0.0));
        let anims = seq_anims(env, self.ctrl);
        let mut parts: Vec<Delta> = Vec::new();
        for _ in 0..64 {
            let next = anims.get(self.started).map(Self::begin);
            let step = next.map_or(dt, |b| (b - self.time).clamp(0.0, dt));
            if let Some(e) = &mut self.current {
                e.update(env, gs, step);
                parts.push(e.delta(env, gs));
            }
            self.time += step;
            dt -= step;
            let before = self.started;
            self.start_due(env, gs, false);
            if dt <= 1e-6 || (step <= 0.0 && self.started == before) {
                break;
            }
        }
        self.delta = (parts.len() > 1)
            .then(|| parts.into_iter().reduce(|a, b| Delta::accumulate(a, b, env.physics)).unwrap_or(Delta::None));
    }
    fn delta(&self, env: &Env, gs: &GameState) -> Delta {
        self.delta.unwrap_or_else(|| self.current.as_ref().map_or(Delta::None, |e| e.delta(env, gs)))
    }
    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        if let Some(e) = &self.current {
            e.pose(env, gs, out);
        }
    }
    fn time(&self) -> f32 {
        self.time.min(self.length)
    }
    fn length(&self) -> f32 {
        self.length
    }
    fn set_time(&mut self, env: &Env, gs: &GameState, t: f32) {
        self.time = t.clamp(0.0, self.length.max(0.0));
        self.started = 0;
        self.current = None;
        self.start_due(env, gs, true);
    }
    fn windows(&self, env: &Env) -> Vec<Window> {
        match &env.g.ctrls[self.ctrl].1 {
            Ctrl::Sequence { windows, .. } => windows
                .iter()
                .map(|w| Window {
                    kind: w.kind.clone(),
                    filter: w.filter,
                    time: w.time,
                    length: w.length,
                    blend: w.blend,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        if let Some(e) = &self.current {
            let w = e.blend.as_ref().map_or(1.0, |(b, el, _)| blend_new_weight(b, *el));
            e.sub.label(env, weight * w, out);
            if let Some((_, _, old)) = &e.blend {
                old.sub.label(env, weight * (1.0 - w), out);
            }
        }
    }
    fn path(&self, env: &Env, out: &mut Vec<(String, f32)>) {
        if let Some(e) = &self.current {
            e.sub.path(env, out);
        }
    }
    fn exit(&mut self, env: &Env, gs: &GameState) {
        if let Some(e) = &mut self.current {
            e.exit(env, gs);
        }
    }
}
