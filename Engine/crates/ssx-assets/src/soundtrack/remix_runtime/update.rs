use super::*;

impl Remix {
    pub(super) fn init_module(&mut self, i: usize, m: usize, g: &mut Graph) -> ModState {
        let kind = self.insts[i].patch.modules[m].kind.clone();
        match &kind {
            ModuleKind::BoolTrigger(b) => ModState::Trigger {
                timer: self.f32(i, b.min_retrigger_period),
                count: 0,
                has_prev: false,
                prev: [false; 3],
            },
            ModuleKind::RangeTrigger(r) => ModState::Trigger {
                timer: self.f32(i, r.min_retrigger_period),
                count: 0,
                has_prev: false,
                prev: [false; 3],
            },
            ModuleKind::Envelope(e) => {
                ModState::Envelope { time: 0.0, last: [0; 2], voices: vec![None; usize::from(e.voices)] }
            }
            ModuleKind::Merge(mg) => ModState::Last(vec![0; mg.inputs.len()]),
            ModuleKind::Route(_) => ModState::Last(vec![0]),
            ModuleKind::Switch(s) => ModState::Last(vec![0; s.inputs.len()]),
            ModuleKind::Curve(c) => ModState::Curve(vec![f64::INFINITY; c.outputs.len()]),
            ModuleKind::Metronome(mt) => {
                self.set(i, mt.click_time, Val::D(f64::from(f32::MAX)));
                ModState::Metronome { last: [0; 2], next: 0.0, running: false }
            }
            ModuleKind::Voice(v) => {
                let mut plugins: Vec<CorePlugin> = v
                    .plugins
                    .iter()
                    .map(|p| CorePlugin { fourcc: p.fourcc, channels: usize::from(p.channels), ctor: Vec::new() })
                    .collect();
                for c in &v.ctor_params {
                    let value = match c.kind {
                        1 => f64::from(self.f32(i, c.value)),
                        _ => f64::from(as_i32(self.get(i, c.value))),
                    };
                    if let Some(p) = plugins.get_mut(usize::from(c.plugin)) {
                        let k = usize::from(c.index);
                        if p.ctor.len() <= k {
                            let fourcc = p.fourcc;
                            let n = crate::sfx_bus::descriptor(&fourcc).0.max(k + 1);
                            p.ctor.resize(n, f64::NAN);
                        }
                        p.ctor[k] = value;
                    }
                }
                for p in &mut plugins {
                    let fourcc = p.fourcc;
                    for (k, c) in p.ctor.iter_mut().enumerate() {
                        if c.is_nan() {
                            *c = crate::sfx_bus::ctor_default(&fourcc, k);
                        }
                    }
                }
                let (slot, input) = g.add_core_voice(&v.name, v.unk_level, &plugins);
                let mut link = VoiceLink { slot, ..VoiceLink::default() };
                if let Some(acc) = input {
                    let h = self.handle(acc);
                    for &r in &v.inputs {
                        if let Some(l) = self.loc(i, r) {
                            self.mem.set_word(l, h);
                        }
                    }
                }
                for p in &v.params {
                    let x = self.param_value(i, p);
                    g.set_plugin_attr(slot, usize::from(p.plugin), usize::from(p.index), x);
                    link.params.push(x);
                }
                link.bypass = vec![false; v.plugins.len()];
                link.sends = vec![None; v.sends.len()];
                ModState::Voice(link)
            }
            _ => ModState::Stateless,
        }
    }

    pub(super) fn param_value(&self, i: usize, p: &ssx_formats::ptch::PluginParam) -> f64 {
        match p.kind {
            1 => f64::from(self.f32(i, p.value)),
            2 | 4 => self.f64(i, p.value),
            5 => f64::from(u8::from(as_bool(self.get(i, p.value)))),
            _ => f64::from(as_i32(self.get(i, p.value))),
        }
    }

    pub fn tick(&mut self, g: &mut Graph) {
        if self.remix_setting != self.remix_on {
            self.remix_on = self.remix_setting;
            self.remix_dirty = true;
        }
        if self.remix_dirty {
            if let Some(l) = self.member("HarmonyMusicBuses", "Harmony Music Interface", "Remix Enabled") {
                self.mem.write(l, Val::B(self.remix_on));
            }
            self.remix_dirty = false;
        }
        self.write_game_inputs();
        self.run(g);
        let pitch = match self.member("PitchPatch", "Pitch Remix Out", "Pitch Into Mixer") {
            Some(l) if self.remix_on => as_f64(self.mem.read(l)),
            _ => 1.0,
        };
        if let Some(v) = self.pitch_var {
            g.vars.set_number(v, pitch);
        }
        self.ticks += 1;
    }

    pub(super) fn run(&mut self, g: &mut Graph) {
        // TODO(guess): the fixed `PATCHES` iteration stands for the ready-gated active-list order (initial keys 0; 0x83618C68 skips unbound instances via 0x836110A8, appends ready ones via 0x836135C0; 0x830F9188 creates / connects on separate wrapper frames): the active list System +0x19C is ordered by the unsigned key +0x2C, equal keys at the tail,
        // so a late-ready instance does not regain its constructor index; settle: the resource / tick phase and later key changes (a trace or a capture) — constructor order alone is no proof (0x830B9AB0 ticks the wrappers in `PATCHES` order).
        for i in 0..self.insts.len() {
            for m in 0..self.insts[i].patch.modules.len() {
                self.process(i, m, g);
            }
        }
    }

    fn write_game_inputs(&mut self) {
        let gi = std::mem::take(&mut self.inputs.events);
        let port = "Game Inputs";
        let set = |r: &mut Self, name: &str, v: Val| {
            if let Some(l) = r.member("GameInputPatch", port, name) {
                r.mem.write(l, v);
            }
        };
        for e in EVENT_ORDER {
            if !gi.contains(&e) {
                continue;
            }
            match e {
                "Land" => set(self, "Land Intensity", Val::F(self.inputs.land_intensity)),
                "Tunnel Start" => set(self, "Tunnel Size", Val::I(self.inputs.tunnel_size)),
                _ => {}
            }
            if let Some(l) = self.member("GameInputPatch", port, e) {
                self.post_event(l);
            }
            let started = match e {
                "Boost Start" | "Boost Stop" => Some(("BoostStarted", "BoostStopped", e == "Boost Start")),
                "Tumble Start" | "Tumble Stop" => Some(("TumbleStarted", "TumbleStopped", e == "Tumble Start")),
                _ => None,
            };
            if let Some((a, b, on)) = started {
                set(self, a, Val::I(i32::from(on)));
                set(self, b, Val::I(i32::from(!on)));
            }
        }
        let i = &self.inputs.clone();
        set(self, "Carve Amount", Val::F(i.carve));
        set(self, "Pitch Input", Val::F(i.pitch));
        set(self, "Ms In Air", Val::I(i.ms_in_air));
        set(self, "Ms To Land", Val::I(i.ms_to_land));
        set(self, "Oxygen Level", Val::F(i.oxygen));
        set(self, "Internal Temp", Val::F(i.temperature));
        set(self, "Physical Health", Val::F(i.health));
        set(self, "Race Event Type", Val::I(i.race_event_type));
    }
}
