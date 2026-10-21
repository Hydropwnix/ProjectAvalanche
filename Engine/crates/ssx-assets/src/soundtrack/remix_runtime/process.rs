use super::*;

impl Remix {
    pub(super) fn process(&mut self, i: usize, m: usize, g: &mut Graph) {
        let kind = self.insts[i].patch.modules[m].kind.clone();
        let mut state = std::mem::replace(&mut self.insts[i].state[m], ModState::Stateless);
        match (&kind, &mut state) {
            (ModuleKind::Copy(c), _) => {
                for &[s, d] in &c.values {
                    self.copy(i, s, d);
                }
                for &[s, d] in c.events.iter().chain(&c.signals) {
                    if let (Some(a), Some(b)) = (self.loc(i, s), self.loc(i, d)) {
                        let w = self.mem.word(a);
                        self.mem.set_word(b, w);
                    }
                }
            }
            (ModuleKind::BoolTrigger(b), ModState::Trigger { timer, count, has_prev, prev }) => {
                *timer += DT;
                let input = as_bool(self.get(i, b.input));
                if *timer < self.f32(i, b.min_retrigger_period) {
                    self.insts[i].state[m] = state;
                    return;
                }
                let maxc = as_i32(self.get(i, b.max_trigger_count));
                if maxc >= 0 && *count == maxc {
                    self.insts[i].state[m] = state;
                    return;
                }
                let flag = |r: &Self, x: Ref| as_bool(r.get(i, x));
                let mut fire = false;
                if *has_prev {
                    fire |= flag(self, b.on_true) && !prev[0] && input;
                    fire |= flag(self, b.on_false) && prev[0] && !input;
                }
                fire |= flag(self, b.while_true) && input;
                fire |= flag(self, b.while_false) && !input;
                if fire {
                    self.post(i, b.output);
                    *count += 1;
                    *timer = 0.0;
                }
                *has_prev = true;
                prev[0] = input;
            }
            (ModuleKind::RangeTrigger(r), ModState::Trigger { timer, count, has_prev, prev }) => {
                *timer += DT;
                let x = self.f64(i, r.input);
                let maxc = as_i32(self.get(i, r.max_trigger_count));
                let (hi, lo) = (self.f64(i, r.max), self.f64(i, r.min));
                if self.f32(i, r.min_retrigger_period) <= *timer && (maxc < 0 || *count != maxc) && lo <= hi {
                    let flag = |s: &Self, x: Ref| as_bool(s.get(i, x));
                    let below = (flag(self, r.includes_max) && x == hi) || x < hi;
                    let above = (flag(self, r.includes_min) && x == lo) || lo < x;
                    let inside = below && above;
                    let mut fire =
                        (inside && flag(self, r.while_in_range)) || (!inside && flag(self, r.while_not_in_range));
                    if *has_prev {
                        fire |= flag(self, r.on_exit_to_above) && prev[0] && !below;
                        fire |= flag(self, r.on_exit_to_below) && prev[0] && !above;
                        fire |= flag(self, r.on_enter_from_below) && !prev[2] && inside;
                        fire |= flag(self, r.on_enter_from_above) && !prev[1] && inside;
                        if r.unk_mode == 1 {
                            fire |= !prev[2] && !below;
                            fire |= !prev[1] && !above;
                        }
                    }
                    if fire {
                        self.post(i, r.output);
                        *count += 1;
                        *timer = 0.0;
                    }
                    *prev = [inside, below, above];
                    *has_prev = true;
                }
            }
            (ModuleKind::Envelope(e), ModState::Envelope { time, last, voices }) => {
                self.envelope(i, e, time, last, voices);
            }
            (ModuleKind::Expression(e), _) => self.expression(i, &e.program),
            (ModuleKind::Merge(mg), ModState::Last(last)) => {
                let mut best: Option<usize> = None;
                for (k, (_, gate)) in mg.inputs.iter().enumerate() {
                    if !self.fired(i, gate.event, &mut last[k]) {
                        continue;
                    }
                    best = Some(match best {
                        None => k,
                        Some(b) => {
                            let (pb, pk) = (mg.inputs[b].0, mg.inputs[k].0);
                            let later = if mg.prefer_latest { last[b] < last[k] } else { last[k] < last[b] };
                            if mg.use_priority && pb != pk {
                                if pb < pk { k } else { b }
                            } else if later {
                                k
                            } else {
                                b
                            }
                        }
                    });
                }
                if let Some(b) = best {
                    for (k, &d) in mg.output.values.iter().enumerate() {
                        self.copy(i, mg.inputs[b].1.values[k], d);
                    }
                    self.post(i, mg.output.event);
                }
            }
            (ModuleKind::Route(rt), ModState::Last(last)) => {
                if self.fired(i, rt.input.event, &mut last[0]) {
                    let sel = as_u32(self.get(i, rt.selector)).wrapping_sub(1) as usize;
                    if let Some(out) = rt.outputs.get(sel) {
                        for (k, &d) in out.values.iter().enumerate() {
                            self.copy(i, rt.input.values[k], d);
                        }
                        self.post(i, out.event);
                    }
                }
            }
            (ModuleKind::Switch(sw), ModState::Last(last)) => {
                let sel = (as_u32(self.get(i, sw.selector)).wrapping_add(0xFFFF) & 0xFFFF) as usize;
                for (k, gate) in sw.inputs.iter().enumerate() {
                    if self.fired(i, gate.event, &mut last[k]) && k == sel {
                        for (n, &d) in sw.output.values.iter().enumerate() {
                            self.copy(i, gate.values[n], d);
                        }
                        self.post(i, sw.output.event);
                    }
                }
            }
            (ModuleKind::Curve(c), ModState::Curve(cache)) => {
                for (k, &(inp, out)) in c.outputs.iter().enumerate() {
                    let x = self.f64(i, inp);
                    if x != cache[k] {
                        cache[k] = x;
                        self.set(i, out, Val::D(c.eval(x)));
                    }
                }
            }
            (ModuleKind::Metronome(mt), ModState::Metronome { last, next, running }) => {
                self.metronome(i, mt, last, next, running);
            }
            (ModuleKind::Voice(v), ModState::Voice(link)) => self.voice(i, v, link, g),
            _ => {}
        }
        self.insts[i].state[m] = state;
    }
}

impl Remix {
    fn envelope(
        &mut self,
        i: usize,
        e: &ssx_formats::ptch::Envelope,
        time: &mut f64,
        last: &mut [u32; 2],
        voices: &mut [Option<EnvVoice>],
    ) {
        *time += f64::from(DT);
        let started = self.fired(i, e.start, &mut last[0]);
        let stopped = self.fired(i, e.stop, &mut last[1]);
        let order = if last[0] < last[1] { [true, false] } else { [false, true] };
        for start in order {
            if start
                && started
                && let Some(v) = voices.iter_mut().find(|v| v.is_none())
            {
                {
                    let rel = self.f32(i, e.release_level);
                    let a_end = self.f32(i, e.attack).max(0.0);
                    let d_end = self.f32(i, e.decay).max(0.0) + a_end;
                    let s_end = self.f32(i, e.sustain).max(0.0) + d_end;
                    let r_end = self.f32(i, e.release).max(0.0) + s_end;
                    *v = Some(EnvVoice {
                        t0: *time + f64::from(self.f32(i, e.start_time)),
                        a_end,
                        d_end,
                        s_end,
                        r_end,
                        a_lvl: self.f32(i, e.attack_level) - rel,
                        s_lvl: self.f32(i, e.sustain_level) - rel,
                        infinite: as_bool(self.get(i, e.infinite_sustain)),
                    });
                }
            }
            if !start && stopped {
                let t = *time + f64::from(self.f32(i, e.fade_out_time));
                for slot in voices.iter_mut() {
                    let Some(v) = slot.as_mut() else { continue };
                    let (phase, level) = env_phase(v, t);
                    let rel = (t - v.t0) as f32;
                    match phase {
                        0 => *slot = None,
                        1..=3 => {
                            if phase == 1 {
                                v.a_end = rel;
                                v.a_lvl = level;
                            }
                            if phase <= 2 {
                                v.s_lvl = level;
                                v.d_end = rel;
                            }
                            let old_s = v.s_end;
                            v.s_end = rel;
                            v.infinite = false;
                            v.r_end = (v.r_end - old_s) + rel;
                        }
                        _ => {}
                    }
                }
            }
        }
        let (mut hi, mut lo) = (0.0f64, 0.0f64);
        for slot in voices.iter_mut() {
            let Some(v) = slot.as_ref() else { continue };
            match env_phase(v, *time) {
                (0, _) => {}
                (5, _) => *slot = None,
                (_, level) => {
                    hi = hi.max(f64::from(level));
                    lo = lo.min(f64::from(level));
                }
            }
        }
        let base = self.f32(i, e.release_level);
        let out = ((f64::from(base) + lo) as f32 as f64 + hi) as f32;
        self.set(i, e.output, Val::F(out));
    }
}
