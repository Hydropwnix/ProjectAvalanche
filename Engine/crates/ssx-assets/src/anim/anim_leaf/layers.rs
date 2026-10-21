use super::*;

pub struct LayersInst {
    ctrl: CtrlId,
    layers: Vec<Option<Box<dyn Controller>>>,
    masks: Vec<Option<crate::anim_pose::PoseMask>>,
    mask_arrays: Vec<Option<(f32, MaskItems)>>,
    weights: Vec<f32>,
}

type MaskItems = Vec<(crate::anim_pose::PoseMask, f32)>;

fn mask_at(items: &[(crate::anim_pose::PoseMask, f32)], v: f32) -> Option<crate::anim_pose::PoseMask> {
    let (first, last) = (items.first()?, items.last()?);
    if items.len() == 1 || v <= first.1 + 0.001 {
        return Some(first.0.clone());
    }
    if v >= last.1 - 0.001 {
        return Some(last.0.clone());
    }
    let k = (1..items.len()).find(|&k| v <= items[k].1 + 0.001)?;
    if v >= items[k].1 - 0.001 {
        return Some(items[k].0.clone());
    }
    let w = (items[k].1 - v) / (items[k].1 - items[k - 1].1);
    crate::anim_pose::PoseMask::weighted(&[(&items[k - 1].0, w), (&items[k].0, 1.0 - w)])
}

const EPS: f32 = 1.5e-5;

fn layer_list<'a>(env: &Env<'a>, ctrl: CtrlId) -> &'a [crate::anim_graph::Layer] {
    match &env.g.ctrls[ctrl].1 {
        Ctrl::Layers(l) => l,
        _ => &[],
    }
}

impl LayersInst {
    pub fn new(env: &Env, gs: &GameState, ctrl: CtrlId) -> Self {
        let list = layer_list(env, ctrl);
        let mask_arrays: Vec<_> = list
            .iter()
            .map(|l| {
                let a = l.mask_array.as_ref()?;
                let v = a.value.map_or(0.0, |s| gs.get(s));
                Some((v, a.items.iter().map(|(m, x)| (env.rig.mask(m), *x)).collect::<Vec<_>>()))
            })
            .collect();
        let masks = list
            .iter()
            .zip(&mask_arrays)
            .map(|(l, a)| match a {
                Some((v, items)) => mask_at(items, *v),
                None => l.mask.as_ref().map(|m| env.rig.mask(m)),
            })
            .collect();
        let weights = list.iter().enumerate().map(|(i, l)| if i == 0 { 1.0 } else { Self::weight(l, gs) }).collect();
        let mut s = LayersInst { ctrl, layers: (0..list.len()).map(|_| None).collect(), masks, mask_arrays, weights };
        s.create_due(env, gs);
        s
    }

    pub(super) fn weight(l: &crate::anim_graph::Layer, gs: &GameState) -> f32 {
        l.signal.map_or(1.0, |s| gs.get(s))
    }

    fn create_due(&mut self, env: &Env, gs: &GameState) {
        for (i, l) in layer_list(env, self.ctrl).iter().enumerate() {
            if self.layers[i].is_none() && (i == 0 || l.update_mode == 1 || Self::weight(l, gs) > EPS) {
                self.layers[i] = l.controller.map(|c| instantiate(env, gs, c));
            }
        }
    }
}

impl Controller for LayersInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        self.create_due(env, gs);
        let list = layer_list(env, self.ctrl);
        let mut prev_phase = 0.0;
        let mut base_phase = 0.0;
        for (i, l) in list.iter().enumerate() {
            if let (Some(a), Some((cur, items))) = (&l.mask_array, &mut self.mask_arrays[i]) {
                let v = a.value.map_or(0.0, |s| gs.get(s));
                *cur = (*cur + (v - *cur).clamp(-a.max_dec * dt, a.max_inc * dt)).clamp(a.range[0], a.range[1]);
                self.masks[i] = mask_at(items, *cur);
            }
            let w = Self::weight(l, gs);
            self.weights[i] = if i == 0 { 1.0 } else { w };
            let Some(c) = &mut self.layers[i] else { continue };
            let run = match l.update_mode {
                0 => i == 0,
                2 => w > EPS || i == 0,
                _ => true,
            };
            if run {
                let step = match l.update_rate {
                    1 if i > 0 => (c.length() * base_phase - c.time()).max(0.0),
                    2 if i > 0 => (c.length() * prev_phase - c.time()).max(0.0),
                    _ => dt,
                };
                c.update(env, gs, step);
            }
            let p = phase(c.as_ref());
            if i == 0 {
                base_phase = p;
            }
            prev_phase = p;
        }
    }

    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        let list = layer_list(env, self.ctrl);
        if let Some(c) = &self.layers.first().and_then(|x| x.as_ref()) {
            c.pose(env, gs, out);
        }
        for (i, l) in list.iter().enumerate().skip(1) {
            let w = Self::weight(l, gs);
            let Some(c) = &self.layers[i] else { continue };
            if w.abs() <= EPS {
                continue;
            }
            let mask = self.masks[i].as_ref();
            let mut layer = AnimPose::empty(env.bind);
            c.pose(env, gs, &mut layer);
            if l.blend_mode == 0 {
                crate::anim_pose::blend_masked(out, &layer, w, mask);
            } else {
                crate::anim_pose::add_layer(out, &layer, w, mask, l.blend_mode == 2);
            }
        }
    }

    fn time(&self) -> f32 {
        self.layers.first().and_then(|x| x.as_ref()).map_or(0.0, |c| c.time())
    }
    fn length(&self) -> f32 {
        self.layers.first().and_then(|x| x.as_ref()).map_or(0.0, |c| c.length())
    }
    fn looping(&self) -> bool {
        self.layers.first().and_then(|x| x.as_ref()).is_some_and(|c| c.looping())
    }
    fn set_time(&mut self, env: &Env, gs: &GameState, t: f32) {
        if let Some(Some(c)) = self.layers.first_mut() {
            c.set_time(env, gs, t);
        }
    }
    fn windows(&self, env: &Env) -> Vec<Window> {
        self.layers.first().and_then(|x| x.as_ref()).map_or_else(Vec::new, |c| c.windows(env))
    }
    fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        for (c, w) in self.layers.iter().zip(&self.weights) {
            if let Some(c) = c {
                c.label(env, weight * w.abs(), out);
            }
        }
    }
    fn path(&self, env: &Env, out: &mut Vec<(String, f32)>) {
        for c in self.layers.iter().flatten() {
            c.path(env, out);
        }
    }
    // TODO(guess): a layer masked to 0 on `DeltaTrajectory` leaves the base's delta set as it leaves every other masked DOF; settle: the layer kernels' delta handling (override 0x82DA44B8, additive 0x82DA5848).
    fn delta(&self, env: &Env, gs: &GameState) -> Delta {
        let list = layer_list(env, self.ctrl);
        let masked_out = |l: &crate::anim_graph::Layer| {
            l.mask_array.is_none()
                && l.mask.as_ref().is_some_and(|m| {
                    m.defaults.iter().all(|(n, v)| n != "DeltaTrajectory" || *v == 0.0)
                        && m.slots.iter().all(|(n, vs)| n != "DeltaTrajectory" || vs.iter().all(|v| *v == 0.0))
                })
        };
        let others_out = list
            .iter()
            .zip(&self.layers)
            .skip(1)
            .all(|(l, c)| c.is_none() || Self::weight(l, gs).abs() <= EPS || masked_out(l));
        match self.layers.first().and_then(|x| x.as_ref()) {
            Some(base) if others_out => base.delta(env, gs),
            _ => Delta::Untraced,
        }
    }
    fn as_flow(&self) -> Option<&crate::anim_eval::SfInst> {
        self.layers.first().and_then(|x| x.as_ref()).and_then(|c| c.as_flow())
    }
    fn exit(&mut self, env: &Env, gs: &GameState) {
        for c in self.layers.iter_mut().flatten() {
            c.exit(env, gs);
        }
    }
}
