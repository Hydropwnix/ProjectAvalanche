use super::*;

impl Graph {
    pub fn add_aems_voice(&mut self, patch: &str, voice: &str, sample_channels: usize, source: u32) -> Option<usize> {
        let set = self.set.clone();
        let key = patch.to_ascii_lowercase();
        let p = set.voices.get(&key)?;
        let vars = self.vars.add(p, set.names[&key].clone());
        let inp = self.vars.resolve(vars, "mInputSampleChannels");
        self.vars.set_number(VarRef { kind: 2, ..inp }, sample_channels as f64);
        let out = self.vars.resolve(vars, "mOutputChannels");
        self.vars.set_number(VarRef { kind: 2, ..out }, 6.0);
        let def = p.modules.iter().find_map(|m| match &m.kind {
            ModuleV1Kind::Voice(v) if v.name.eq_ignore_ascii_case(voice) => Some(v.clone()),
            _ => None,
        });
        let Some(def) = def else {
            self.vars.remove(vars);
            return None;
        };
        Some(self.add_voice(def, vars, Some(source)))
    }

    pub fn remove_voice(&mut self, slot: usize) {
        if let Some(v) = self.voices.get_mut(slot).and_then(Option::take) {
            self.order.retain(|&i| i != slot);
            if v.vars > 1 {
                self.vars.remove(v.vars);
            }
        }
    }

    pub fn set_attr(&mut self, slot: usize, fourcc: &[u8; 4], attr: usize, value: f64) {
        let v = self.voices.get_mut(slot).and_then(Option::as_mut);
        if let Some(a) =
            v.and_then(|v| v.plugins.iter_mut().find(|p| &p.fourcc == fourcc)).and_then(|p| p.attrs.get_mut(attr))
        {
            *a = value;
        }
    }

    pub fn attr(&self, slot: usize, fourcc: &[u8; 4], attr: usize) -> Option<f64> {
        let v = self.voices.get(slot)?.as_ref()?;
        v.plugins.iter().find(|p| &p.fourcc == fourcc)?.attrs.get(attr).copied()
    }

    pub fn fade_by_name(&mut self, voice: &str, to: f32, seconds: f32) {
        let Some(v) = self.voices.iter_mut().flatten().find(|v| v.name == voice) else { return };
        let Some(p) = v.plugins.iter_mut().find(|p| &p.fourcc == b"GaF0") else { return };
        let from = match &p.stage {
            Stage::Fader { fade: Some(f) } => f.from + f.step * f.done as f32,
            _ => p.attrs[0] as f32,
        };
        let n = ((seconds * crate::sfx_aems::MIXER_RATE as f32) as u32).max(1);
        p.stage = Stage::Fader { fade: Some(FaderRamp { from, to, step: (to - from) / n as f32, n, done: 0 }) };
    }

    pub fn fading(&self, voice: &str) -> bool {
        let Some(v) = self.voices.iter().flatten().find(|v| v.name == voice) else { return false };
        v.plugins.iter().any(|p| matches!(&p.stage, Stage::Fader { fade: Some(f) } if f.done < f.n))
    }

    pub fn set_attr_by_name(&mut self, voice: &str, fourcc: &[u8; 4], attr: usize, value: f64) {
        if let Some(slot) = self.voices.iter().position(|v| v.as_ref().is_some_and(|v| v.name == voice)) {
            self.set_attr(slot, fourcc, attr, value);
        }
    }

    pub(super) fn reverb_ir(&mut self, voice: &str) -> Option<&mut dsp::reverb_ir::ReverbIr> {
        let v = self.voices.iter_mut().flatten().find(|v| v.name == voice)?;
        v.plugins.iter_mut().find_map(|p| match &mut p.stage {
            Stage::ReverbIr(r) => Some(r.as_mut()),
            _ => None,
        })
    }

    pub fn reverb_apply(&mut self, voice: &str, slot: usize, ir: Arc<dsp::reverb_ir::ImpulseResponse>) -> bool {
        self.reverb_ir(voice).map(|r| r.apply(slot, ir)).is_some()
    }

    pub fn reverb_release(&mut self, voice: &str) {
        if let Some(r) = self.reverb_ir(voice) {
            r.release_all();
        }
    }

    pub fn pan3d_send(&mut self, voice: &str, packet: Option<&dsp::pan3d::Packet>) -> bool {
        let Some(v) = self.voices.iter_mut().flatten().find(|v| v.name == voice) else { return false };
        let Some(pan) = v.plugins.iter_mut().find_map(|p| match &mut p.stage {
            Stage::Pan3d(pan) => Some(pan),
            _ => None,
        }) else {
            return false;
        };
        pan.send(packet);
        true
    }

    pub fn take_profile(&mut self) -> Vec<(String, u64, u64)> {
        let mut v: Vec<(String, u64, u64)> =
            self.profile.as_mut().map(|p| p.drain().map(|(k, (ns, n))| (k, ns, n)).collect()).unwrap_or_default();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    pub fn profiling(&self) -> bool {
        self.profile.is_some()
    }

    pub fn profile_add(&mut self, name: &str, ns: u64) {
        if let Some(p) = self.profile.as_mut() {
            let e = p.entry(name.to_string()).or_default();
            e.0 += ns;
            e.1 += 1;
        }
    }

    pub fn voice_count(&self) -> usize {
        self.voices.iter().flatten().count()
    }

    pub fn has_plugin(&self, slot: usize, fourcc: &[u8; 4]) -> bool {
        self.voices.get(slot).and_then(Option::as_ref).is_some_and(|v| v.plugins.iter().any(|p| &p.fourcc == fourcc))
    }

    pub(super) fn update_voice(&mut self, slot: usize) {
        let Some(v) = self.voices.get_mut(slot).and_then(Option::as_mut) else { return };
        if v.core {
            return;
        }
        let muted = v.mute.is_some_and(|m| self.vars.number(VarRef { kind: 2, ..m }) == 1.0);
        let out = if muted { 0.0 } else { self.vars.number(v.gain) };
        self.vars.set_number(v.mute_output, out);
        for &(r, pl, attr) in &v.params {
            let x = self.vars.number(r);
            if let Some(p) = v.plugins.get_mut(pl)
                && let (Some(a), Some(&(_, lo, hi))) = (p.attrs.get_mut(attr), p.ranges.get(attr))
            {
                *a = x.max(lo).min(hi);
            }
        }
    }

    pub fn controller_update(&mut self, dt: f32) {
        for i in 0..self.snaps.len() {
            self.update_snapshot(i, dt);
        }
        for i in 0..self.voices.len() {
            self.update_voice(i);
        }
        for k in 0..self.selectors.len() {
            let want = self.selectors[k].wanted();
            if self.selectors[k].shown != Some(want) {
                let r = self.vars.resolve(0, self.selectors[k].variable);
                let name = self.selectors[k].table[want];
                self.vars.set_string(r, name);
                self.log.push(format!("snapshot {} = {name}", self.selectors[k].variable));
                self.selectors[k].shown = Some(want);
            }
        }
    }

    fn update_snapshot(&mut self, i: usize, dt: f32) {
        let sel = self.vars.string(self.snaps[i].selector);
        if sel != self.snaps[i].current {
            let had = !self.snaps[i].current.is_empty();
            self.snaps[i].current = sel.clone();
            let def = &self.snaps[i].def;
            if let Some(e) = def.snapshots.iter().find(|e| e.name == sel) {
                let time = if had && !e.fade_time.is_empty() {
                    self.vars.number(self.vars.resolve(0, &e.fade_time)) as f32
                } else {
                    0.0
                };
                let mut values = Vec::new();
                for v in &def.values[usize::from(e.first)..usize::from(e.first) + usize::from(e.count)] {
                    let kind = match v.kind {
                        0 => 0,
                        _ => 2,
                    };
                    let src = VarRef { kind, ..self.vars.resolve(0, &v.source) };
                    let dst = VarRef { kind, ..self.vars.resolve(0, &v.target) };
                    values.push((src, dst, v.kind, v.curve, self.vars.number(dst), self.vars.number(src)));
                }
                self.snaps[i].fade = Some(Fade { time, elapsed: 0.0, values });
            }
        }
        let Some(f) = self.snaps[i].fade.as_mut() else { return };
        f.elapsed += dt;
        if f.elapsed < f.time {
            let t = if f.time == 0.0 { 1.0 } else { f.elapsed / f.time };
            for &(_, dst, kind, curve, start, end) in &f.values {
                if kind > 2 {
                    continue;
                }
                let rising = start.abs() <= end.abs();
                let half_pi = std::f32::consts::FRAC_PI_2;
                let shape = match (curve, rising) {
                    (1, true) | (2, false) => (t * half_pi).sin(),
                    (1, false) | (2, true) => 1.0 - ((t + 1.0) * half_pi).sin(),
                    _ => t,
                };
                self.vars.set_number(dst, (end - start).mul_add(f64::from(shape), start));
            }
        } else {
            let values = std::mem::take(&mut f.values);
            self.snaps[i].fade = None;
            for (src, dst, kind, ..) in values {
                if kind == 3 {
                    let s = self.vars.string(src);
                    self.vars.set_string(dst, &s);
                } else {
                    let x = self.vars.number(src);
                    self.vars.set_number(dst, x);
                }
            }
        }
    }
}
