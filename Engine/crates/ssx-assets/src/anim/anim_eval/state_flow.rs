use super::*;

pub struct NodeInst {
    pub node: usize,
    pub subject: Option<Box<dyn Controller>>,
    pub blend: Option<BlendInst>,
    pub frozen: bool,
    pub active: Vec<SigId>,
}

pub struct SfInst {
    pub ctrl: CtrlId,
    pub current: Box<NodeInst>,
    pub taken: Vec<String>,
    first: bool,
    exited: bool,
    delta: Delta,
}

const NEVER: f32 = 1e35;

fn flow<'a>(env: &Env<'a>, ctrl: CtrlId) -> &'a StateFlow {
    match &env.g.ctrls[ctrl].1 {
        Ctrl::StateFlow(sf) => sf,
        _ => unreachable!("not a state flow"),
    }
}

fn node_entry_ok(env: &Env, sf: &StateFlow, node: &SfNode, gs: &GameState, depth: u32) -> bool {
    if depth > 10 {
        return false;
    }
    let bools = node.entry_true.iter().all(|&s| gs.bool(s)) && node.entry_false.iter().all(|&s| !gs.bool(s));
    let enums = node.entry_enum_true.iter().all(|c| gs.get(c.signal) as i64 == c.value)
        && node.entry_enum_false.iter().all(|c| gs.get(c.signal) as i64 != c.value);
    if !(bools && enums) {
        return false;
    }
    if node.conduit {
        return node.transitions.iter().any(|&t| conditions_ok(env, sf, t, gs, depth + 1));
    }
    if let Some(sub) = node.subject
        && let Ctrl::StateFlow(inner) = &env.g.ctrls[sub].1
    {
        return inner.nodes.get(inner.start).is_some_and(|n| node_entry_ok(env, inner, n, gs, depth + 1));
    }
    true
}

fn conditions_ok(env: &Env, sf: &StateFlow, t: usize, gs: &GameState, depth: u32) -> bool {
    let tr = &sf.transitions[t];
    let bools = tr.cond_true.iter().all(|&s| gs.bool(s)) && tr.cond_false.iter().all(|&s| !gs.bool(s));
    let enums = tr.enum_true.iter().all(|c| gs.get(c.signal) as i64 == c.value)
        && tr.enum_false.iter().all(|c| gs.get(c.signal) as i64 != c.value);
    bools && enums && sf.nodes.get(tr.to).is_some_and(|n| node_entry_ok(env, sf, n, gs, depth))
}

fn conduit_chain(env: &Env, sf: &StateFlow, t: usize, gs: &GameState) -> Vec<usize> {
    let mut chain = vec![t];
    let mut node = sf.transitions[t].to;
    for depth in 1..=10 {
        let Some(n) = sf.nodes.get(node).filter(|n| n.conduit) else { break };
        let Some(&next) = n.transitions.iter().find(|&&c| conditions_ok(env, sf, c, gs, depth)) else { break };
        chain.push(next);
        node = sf.transitions[next].to;
    }
    chain
}

impl NodeInst {
    pub(super) fn new(env: &Env, gs: &GameState, sf: &StateFlow, node: usize) -> Self {
        let n = &sf.nodes[node];
        let mut subject = n.subject.map(|c| instantiate(env, gs, c));
        if let Some(s) = &mut subject
            && n.start_tick != 0.0
        {
            s.set_time(env, gs, n.start_tick);
        }
        NodeInst { node, subject, blend: None, frozen: false, active: n.active_signals.clone() }
    }

    pub(super) fn time(&self) -> f32 {
        self.subject.as_ref().map_or(0.0, |s| s.time())
    }

    pub(super) fn windows(&self, env: &Env, sf: &StateFlow, kind: &str) -> Vec<Window> {
        let Some(s) = &self.subject else { return Vec::new() };
        let nested = sf.nodes[self.node].subject.is_some_and(|c| matches!(env.g.ctrls[c].1, Ctrl::StateFlow(_)));
        s.windows(env)
            .into_iter()
            .filter(|w| w.kind == kind && (w.filter == sf.identifier || (w.filter == 0 && !nested)))
            .collect()
    }

    fn time_until(&self, env: &Env, sf: &StateFlow, t: usize, gs: &GameState) -> f32 {
        if !conditions_ok(env, sf, t, gs, 0) {
            return NEVER;
        }
        let chain = conduit_chain(env, sf, t, gs);
        let kind = chain.iter().rev().find_map(|&c| sf.transitions[c].window.as_ref());
        let Some(kind) = kind else { return 0.0 };
        let now = self.time();
        let mut best = NEVER;
        for w in self.windows(env, sf, kind) {
            let (ds, de) = (w.time - now, w.time + w.length - now);
            let t = if ds <= 0.0 && de >= 0.0 {
                0.0
            } else if ds > 0.0 {
                ds
            } else {
                NEVER
            };
            best = best.min(t);
        }
        best
    }

    fn next_transition(&self, env: &Env, sf: &StateFlow, gs: &GameState) -> Option<(f32, usize)> {
        if self.frozen {
            return None;
        }
        let mut best: Option<(f32, usize)> = None;
        for &t in &sf.nodes[self.node].transitions {
            let time = self.time_until(env, sf, t, gs);
            if time < best.map_or(NEVER, |b| b.0) {
                best = Some((time, t));
                if time <= 0.0 {
                    break;
                }
            }
        }
        best
    }

    pub(super) fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        if self.frozen {
            return;
        }
        if self.blend.as_ref().is_some_and(|b| b.done(dt)) {
            self.blend = None;
        }
        if let Some(b) = &mut self.blend {
            b.elapsed = (b.elapsed + dt).min(b.blend.blend_time.max(0.0));
            b.prev.update(env, gs, dt);
        }
        if let Some(s) = &mut self.subject {
            s.update(env, gs, dt);
        }
    }

    pub(super) fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        let mut new = AnimPose::empty(env.bind);
        if let Some(s) = &self.subject {
            s.pose(env, gs, &mut new);
        }
        match &self.blend {
            Some(b) => {
                let mut old = AnimPose::empty(env.bind);
                b.prev.pose(env, gs, &mut old);
                apply_blend(env, &b.blend, b.elapsed, &mut old, &new);
                if b.blend.class == "AccelBlendAsset"
                    && b.blend.dof_curves.is_none()
                    && let Some(t) = env.rig.trajectory
                {
                    old.pose.nodes[t] = new.pose.nodes[t];
                    old.valid_nodes[t] = new.valid_nodes[t];
                }
                *out = old;
            }
            None => *out = new,
        }
    }

    pub(super) fn delta(&self, env: &Env, gs: &GameState) -> Delta {
        let new = self.subject.as_ref().map_or(Delta::None, |s| s.delta(env, gs));
        if self.frozen {
            return new.zeroed();
        }
        match &self.blend {
            Some(b) => blend_delta(env, &b.blend, b.elapsed, b.prev.delta(env, gs), new),
            None => new,
        }
    }

    pub(super) fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        let w = self.blend.as_ref().map_or(1.0, |b| b.new_weight());
        if let Some(s) = &self.subject {
            s.label(env, weight * w, out);
        }
        if let Some(b) = &self.blend {
            b.prev.label(env, weight * (1.0 - w), out);
        }
    }

    fn limit(&mut self, depth: u32, max: u32) {
        if depth + 1 >= max {
            if let Some(b) = &mut self.blend {
                b.prev.frozen = true;
                b.prev.blend = None;
            }
            return;
        }
        if let Some(b) = &mut self.blend {
            b.prev.limit(depth + 1, max);
        }
    }

    pub(super) fn exit(&mut self, env: &Env, gs: &GameState) {
        for &s in &self.active {
            gs.set(s, 0.0);
        }
        if let Some(s) = &mut self.subject {
            s.exit(env, gs);
        }
    }
}

impl SfInst {
    pub fn new(env: &Env, gs: &GameState, ctrl: CtrlId) -> Self {
        let sf = flow(env, ctrl);
        let current = Box::new(NodeInst::new(env, gs, sf, sf.start));
        for &s in &current.active {
            gs.set(s, 1.0);
        }
        SfInst { ctrl, current, taken: Vec::new(), first: true, exited: false, delta: Delta::None }
    }

    fn transitions_enabled(&self, env: &Env) -> bool {
        flow(env, self.ctrl).transition_as_blend_slave || !self.exited
    }

    fn take(&mut self, env: &Env, gs: &mut GameState, t: usize, window_blend: Option<BlendId>) {
        let sf = flow(env, self.ctrl);
        let tr = &sf.transitions[t];
        if !self.exited {
            self.current.exit(env, gs);
        }
        let blend = tr
            .override_blend
            .or(window_blend)
            .map(|b| env.g.blends[b].clone())
            .unwrap_or_else(Blend::default_transition);
        let mut next = NodeInst::new(env, gs, sf, tr.to);
        let old_time = self.current.time();
        if let Some(s) = &mut next.subject {
            match tr.branch_in_phase {
                1 => s.set_time(env, gs, tr.destination_phase + old_time),
                2 => s.set_time(env, gs, tr.destination_phase),
                _ if tr.destination_phase > 1.5e-5 => s.set_time(env, gs, tr.destination_phase),
                _ => {
                    let old_loops = self.current.subject.as_ref().is_some_and(|o| o.looping());
                    if old_loops && s.looping() {
                        s.set_time(env, gs, old_time);
                    }
                }
            }
        }
        let consumed: Vec<SigId> = sf.nodes[tr.from]
            .active_signals
            .iter()
            .chain(&tr.cond_true)
            .chain(&tr.cond_false)
            .chain(&sf.nodes[tr.to].entry_true)
            .chain(&sf.nodes[tr.to].entry_false)
            .copied()
            .filter(|&s| env.g.signals[s].trigger)
            .collect();
        for s in consumed {
            gs.set(s, 0.0);
        }
        for &s in &sf.nodes[tr.to].active_signals {
            gs.set(s, 1.0);
        }
        let prev = std::mem::replace(&mut self.current, Box::new(next));
        self.current.blend = Some(BlendInst { blend, elapsed: 0.0, prev });
        self.current.limit(0, sf.max_active.max(1));
        if self.exited {
            self.current.exit(env, gs);
        }
        self.taken.push(tr.name.clone());
    }
}

impl Controller for SfInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, mut dt: f32) {
        self.taken.clear();
        let first = std::mem::replace(&mut self.first, false);
        if !self.transitions_enabled(env) {
            self.current.update(env, gs, dt);
            self.delta = self.current.delta(env, gs);
            return;
        }
        let mut advanced = false;
        let mut parts: Option<Delta> = None;
        for _ in 0..25 {
            let sf = flow(env, self.ctrl);
            let Some((t_next, tr)) = self.current.next_transition(env, sf, gs) else { break };
            let conduit = sf.nodes[self.current.node].conduit;
            if t_next >= NEVER || (dt <= t_next && !(t_next <= 0.0 && (conduit || first))) {
                break;
            }
            if t_next > 0.0 {
                self.current.update(env, gs, t_next);
                dt -= t_next;
                advanced = true;
                let d = self.current.delta(env, gs);
                parts = Some(parts.map_or(d, |p| Delta::accumulate(p, d, env.physics)));
            }
            let chain = conduit_chain(env, sf, tr, gs);
            let blends: Vec<Option<BlendId>> = chain
                .iter()
                .map(|&c| {
                    let kind = sf.transitions[c].window.as_ref()?;
                    self.current.windows(env, sf, kind).into_iter().next()?.blend
                })
                .collect();
            for (&link, blend) in chain.iter().zip(blends) {
                self.take(env, gs, link, blend);
            }
        }
        if !advanced || dt > 0.001 {
            self.current.update(env, gs, dt);
            let d = self.current.delta(env, gs);
            self.delta = parts.map_or(d, |p| Delta::accumulate(p, d, env.physics));
        } else {
            self.delta = parts.unwrap_or(Delta::None);
        }
    }

    fn delta(&self, _env: &Env, _gs: &GameState) -> Delta {
        self.delta
    }

    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        self.current.pose(env, gs, out);
    }

    fn time(&self) -> f32 {
        self.current.time()
    }

    fn windows(&self, env: &Env) -> Vec<Window> {
        self.current.subject.as_ref().map_or_else(Vec::new, |s| s.windows(env))
    }

    fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        self.current.label(env, weight, out);
    }
    fn path(&self, env: &Env, out: &mut Vec<(String, f32)>) {
        out.push((flow(env, self.ctrl).nodes[self.current.node].name.clone(), self.current.time()));
        if let Some(s) = &self.current.subject {
            s.path(env, out);
        }
    }
    fn as_flow(&self) -> Option<&SfInst> {
        Some(self)
    }
    fn exit(&mut self, env: &Env, gs: &GameState) {
        if std::mem::replace(&mut self.exited, true) {
            return;
        }
        self.current.exit(env, gs);
    }
}

impl SfInst {
    pub fn running_nodes(&self, env: &Env, dt: f32) -> Vec<String> {
        let sf = flow(env, self.ctrl);
        let mut node = &*self.current;
        let mut out = vec![sf.nodes[node.node].name.clone()];
        while !node.frozen
            && let Some(b) = &node.blend
            && !b.done(dt)
            && !b.prev.frozen
        {
            node = &b.prev;
            out.push(sf.nodes[node.node].name.clone());
        }
        out
    }

    pub fn node_path(&self, env: &Env) -> String {
        flow(env, self.ctrl).nodes[self.current.node].name.clone()
    }
}
