use super::*;

pub struct ClipInst {
    clip: Option<usize>,
    time: f32,
    prev: f32,
    num_ticks: f32,
    fps_scale: f32,
    name: String,
    delta_trajectory: bool,
}

impl ClipInst {
    pub fn new(env: &Env, ctrl: CtrlId) -> Self {
        match &env.g.ctrls[ctrl].1 {
            Ctrl::Clip { anim, anim_guid, num_ticks, fps_scale, delta_trajectory, .. } => {
                let clip = anim_guid.and_then(|g| env.clip_index.get(&g).copied());
                let drives = |t: usize| clip.is_some_and(|c| env.clips[c].driven.contains(&t));
                ClipInst {
                    clip,
                    time: 0.0,
                    prev: 0.0,
                    num_ticks: *num_ticks,
                    fps_scale: *fps_scale,
                    name: anim.clone(),
                    delta_trajectory: *delta_trajectory != 0 && env.rig.trajectory.is_some_and(drives),
                }
            }
            _ => ClipInst {
                clip: None,
                time: 0.0,
                prev: 0.0,
                num_ticks: 0.0,
                fps_scale: 1.0,
                name: String::new(),
                delta_trajectory: false,
            },
        }
    }

    fn trajectory_at(&self, env: &Env, c: usize, t: usize, tick: f32) -> ([f32; 4], [f32; 3]) {
        let mut nodes = env.bind.nodes.clone();
        env.clips[c].sample_into(tick * self.fps_scale, &mut nodes);
        (ssx_formats::ant_anim::normalize(nodes[t].rotation), nodes[t].translation)
    }
}

impl Controller for ClipInst {
    fn update(&mut self, _env: &Env, _gs: &mut GameState, dt: f32) {
        self.prev = self.time;
        self.time += dt.max(0.0).min((self.num_ticks - self.time).max(0.0));
    }
    fn pose(&self, env: &Env, _gs: &GameState, out: &mut AnimPose) {
        if let Some(c) = self.clip {
            let keep = env
                .rig
                .trajectory
                .filter(|_| self.delta_trajectory)
                .map(|t| (t, out.pose.nodes[t], out.valid_nodes[t]));
            env.clips[c].sample_anim_into(self.time * self.fps_scale, out);
            if let Some((t, srt, valid)) = keep {
                (out.pose.nodes[t], out.valid_nodes[t]) = (srt, valid);
            }
        }
    }
    fn delta(&self, env: &Env, _gs: &GameState) -> Delta {
        let (Some(c), Some(t), true) = (self.clip, env.rig.trajectory, self.delta_trajectory) else {
            return Delta::None;
        };
        let ((q0, t0), (q1, t1)) = (self.trajectory_at(env, c, t, self.prev), self.trajectory_at(env, c, t, self.time));
        let c0 = [-q0[0], -q0[1], -q0[2], q0[3]];
        let d = crate::rigamate::rotate(c0, [t1[0] - t0[0], t1[1] - t0[1], t1[2] - t0[2], 0.0]);
        Delta::Value { q: ssx_formats::ant_anim::normalize(crate::rigamate::quat_mul(c0, q1)), t: [d[0], d[1], d[2]] }
    }
    fn time(&self) -> f32 {
        self.time
    }
    fn length(&self) -> f32 {
        self.num_ticks
    }
    fn set_time(&mut self, _env: &Env, _gs: &GameState, t: f32) {
        self.time = t.clamp(0.0, self.num_ticks.max(0.0));
        self.prev = self.time;
    }
    fn label(&self, _env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        out.push((format!("{}@{:.0}", self.name, self.time * self.fps_scale), weight));
    }
}

type Sub = Option<Box<dyn Controller>>;

fn sub_time(s: &Sub) -> f32 {
    s.as_ref().map_or(0.0, |c| c.time())
}
fn sub_len(s: &Sub) -> f32 {
    s.as_ref().map_or(0.0, |c| c.length())
}

pub struct LoopingInst {
    sub: Sub,
    first: bool,
    start: f32,
    loop_control: bool,
    loop_start: f32,
    loop_end: f32,
    partial: Option<Delta>,
}

impl Controller for LoopingInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        let Some(s) = &mut self.sub else { return };
        if std::mem::replace(&mut self.first, false) && self.start > 0.0 {
            s.update(env, gs, self.start);
        }
        let rem = if self.loop_control { self.loop_end - s.time() } else { s.length() - s.time() }.max(0.0);
        self.partial = None;
        if rem <= dt && s.length() > 0.0 {
            s.update(env, gs, rem);
            self.partial = Some(s.delta(env, gs));
            set_phase(env, gs, s.as_mut(), 0.0);
            if self.loop_control && self.loop_start > 0.0 {
                s.update(env, gs, self.loop_start);
            }
            s.update(env, gs, dt - rem);
        } else {
            s.update(env, gs, dt);
        }
    }
    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        if let Some(s) = &self.sub {
            s.pose(env, gs, out);
        }
    }
    fn delta(&self, env: &Env, gs: &GameState) -> Delta {
        let last = self.sub.as_ref().map_or(Delta::None, |s| s.delta(env, gs));
        self.partial.map_or(last, |p| Delta::accumulate(p, last, env.physics))
    }
    fn time(&self) -> f32 {
        sub_time(&self.sub)
    }
    fn length(&self) -> f32 {
        sub_len(&self.sub)
    }
    fn looping(&self) -> bool {
        true
    }
    fn set_time(&mut self, env: &Env, gs: &GameState, t: f32) {
        let l = sub_len(&self.sub);
        if let Some(s) = &mut self.sub {
            s.set_time(env, gs, if l > 0.0 { t.rem_euclid(l).min(l * (1.0 - f32::EPSILON)) } else { t });
        }
    }
    fn windows(&self, env: &Env) -> Vec<Window> {
        self.sub.as_ref().map_or_else(Vec::new, |s| s.windows(env))
    }
    fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        if let Some(s) = &self.sub {
            s.label(env, weight, out);
        }
    }
    fn path(&self, env: &Env, out: &mut Vec<(String, f32)>) {
        if let Some(s) = &self.sub {
            s.path(env, out);
        }
    }
    fn exit(&mut self, env: &Env, gs: &GameState) {
        if let Some(s) = &mut self.sub {
            s.exit(env, gs);
        }
    }
}

pub struct RateInst {
    sub: Sub,
    signal: Option<SigId>,
    scale: f32,
    range: [f32; 2],
}

pub struct PhaseInst {
    sub: Sub,
    signal: Option<SigId>,
    range: [f32; 2],
}

pub struct RefPoseInst {
    sub: Sub,
    reference: Sub,
    weight: Option<SigId>,
    offset: bool,
}

macro_rules! forward_timeline {
    () => {
        forward_timeline!(no_exit);
        fn exit(&mut self, env: &Env, gs: &GameState) {
            if let Some(s) = &mut self.sub {
                s.exit(env, gs);
            }
        }
        fn delta(&self, env: &Env, gs: &GameState) -> Delta {
            self.sub.as_ref().map_or(Delta::None, |s| s.delta(env, gs))
        }
    };
    (no_exit) => {
        fn time(&self) -> f32 {
            sub_time(&self.sub)
        }
        fn length(&self) -> f32 {
            sub_len(&self.sub)
        }
        fn looping(&self) -> bool {
            self.sub.as_ref().is_some_and(|s| s.looping())
        }
        fn set_time(&mut self, env: &Env, gs: &GameState, t: f32) {
            if let Some(s) = &mut self.sub {
                s.set_time(env, gs, t);
            }
        }
        fn windows(&self, env: &Env) -> Vec<Window> {
            self.sub.as_ref().map_or_else(Vec::new, |s| s.windows(env))
        }
        fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
            if let Some(s) = &self.sub {
                s.label(env, weight, out);
            }
        }
        fn path(&self, env: &Env, out: &mut Vec<(String, f32)>) {
            if let Some(s) = &self.sub {
                s.path(env, out);
            }
        }
    };
}

impl Controller for RateInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        let rate = self.signal.map_or(self.scale, |s| gs.get(s)).max(self.range[0]).min(self.range[1]);
        if let Some(s) = &mut self.sub {
            s.update(env, gs, dt * rate);
        }
    }
    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        if let Some(s) = &self.sub {
            s.pose(env, gs, out);
        }
    }
    forward_timeline!();
}

impl Controller for PhaseInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, _dt: f32) {
        let v = self.signal.map_or(0.0, |s| gs.get(s));
        let span = self.range[1] - self.range[0];
        let p = if span != 0.0 { ((v - self.range[0]) / span).clamp(0.0, 1.0) } else { 0.0 };
        if let Some(s) = &mut self.sub {
            set_phase(env, gs, s.as_mut(), p);
            s.update(env, gs, 0.0);
        }
    }
    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        if let Some(s) = &self.sub {
            s.pose(env, gs, out);
        }
    }
    forward_timeline!();
}

impl Controller for RefPoseInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        if let Some(s) = &mut self.sub {
            s.update(env, gs, dt);
        }
    }
    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        let mut subject = AnimPose::empty(env.bind);
        if let Some(s) = &self.sub {
            s.pose(env, gs, &mut subject);
        }
        let mut reference = AnimPose::empty(env.bind);
        if let Some(r) = &self.reference {
            r.pose(env, gs, &mut reference);
        }
        if self.offset {
            *out = crate::anim_pose::difference(&subject, &reference);
        } else {
            let w = self.weight.map_or(1.0, |s| gs.get(s));
            reference.blend(&subject, w);
            *out = reference;
        }
    }
    forward_timeline!(no_exit);
    fn exit(&mut self, env: &Env, gs: &GameState) {
        for s in [&mut self.sub, &mut self.reference].into_iter().flatten() {
            s.exit(env, gs);
        }
    }
    fn delta(&self, _env: &Env, _gs: &GameState) -> Delta {
        Delta::Untraced
    }
}

pub struct PassInst {
    pub sub: Sub,
    pub class: Option<String>,
    pub exited: bool,
}

impl PassInst {
    pub fn new(sub: Sub, class: Option<String>) -> Self {
        PassInst { sub, class, exited: false }
    }

    fn frozen_carving(&self) -> bool {
        self.exited && self.class.as_deref() == Some("CarvingControllerAsset")
    }
}

impl Controller for PassInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        let dt = if self.frozen_carving() { 0.0 } else { dt };
        if let Some(s) = &mut self.sub {
            s.update(env, gs, dt);
        }
    }
    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        if let Some(s) = &self.sub {
            s.pose(env, gs, out);
        }
    }
    forward_timeline!(no_exit);
    fn exit(&mut self, env: &Env, gs: &GameState) {
        self.exited = true;
        if let Some(s) = &mut self.sub {
            s.exit(env, gs);
        }
    }
    fn delta(&self, env: &Env, gs: &GameState) -> Delta {
        match &self.class {
            None => Delta::None,
            Some(c) if c == "TrajectoryOffsetControllerAsset" => {
                self.sub.as_ref().map_or(Delta::None, |s| s.delta(env, gs))
            }
            Some(_) if self.frozen_carving() => Delta::ZERO,
            Some(c) if env.physics.is_some_and(|p| p.controllers.contains(c)) => Delta::Physics,
            Some(_) => Delta::Untraced,
        }
    }
}

impl LoopingInst {
    pub fn new(sub: Sub, start: f32, loop_control: bool, loop_start: f32, loop_end: f32) -> Self {
        LoopingInst { sub, first: true, start, loop_control, loop_start, loop_end, partial: None }
    }
}

impl PhaseInst {
    pub fn new(sub: Sub, signal: Option<SigId>, range: [f32; 2]) -> Self {
        PhaseInst { sub, signal, range }
    }
}

impl RateInst {
    pub fn new(sub: Sub, signal: Option<SigId>, scale: f32, range: [f32; 2]) -> Self {
        RateInst { sub, signal, scale, range }
    }
}

impl RefPoseInst {
    pub fn new(
        env: &Env,
        gs: &GameState,
        sub: Sub,
        reference: Option<CtrlId>,
        frame: f32,
        weight: Option<SigId>,
        offset: bool,
    ) -> Self {
        let reference = reference.map(|c| {
            let mut r = instantiate(env, gs, c);
            r.set_time(env, gs, frame);
            r
        });
        RefPoseInst { sub, reference, weight, offset }
    }
}
