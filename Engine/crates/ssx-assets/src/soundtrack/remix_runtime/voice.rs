use super::*;

impl Remix {
    pub(super) fn metronome(
        &mut self,
        i: usize,
        mt: &ssx_formats::ptch::Metronome,
        last: &mut [u32; 2],
        next: &mut f32,
        running: &mut bool,
    ) {
        *next -= DT;
        let bpm = self.f32(i, mt.bpm).max(0.0);
        if bpm == 0.0 {
            self.set(i, mt.click_time, Val::D(f64::from(f32::MAX)));
            return;
        }
        let inv = 1.0 / (self.f32(i, mt.clicks_per_beat) * bpm);
        let (interval, half) = (inv * 60.0, inv * 30.0);
        let beat = self.f32(i, mt.beat_time);
        let started = self.fired(i, mt.start, &mut last[0]);
        let stopped = self.fired(i, mt.stop, &mut last[1]);
        let order = if last[0] < last[1] { [true, false] } else { [false, true] };
        for start in order {
            if start && started {
                *next = if beat > 0.0 { beat - interval } else { -interval };
                *running = true;
            } else if !start && stopped {
                self.set(i, mt.click_time, Val::D(f64::from(f32::MAX)));
                *running = false;
            }
        }
        if !*running {
            return;
        }
        let mut t;
        if beat <= 0.0 {
            t = interval + *next;
            if t < 0.0 {
                t = (t / interval).ceil().mul_add(interval, t);
            }
        } else {
            t = (beat / interval).fract() * interval;
            if t - *next < half {
                t += interval;
            }
        }
        if t < DT {
            *next = t;
            self.post(i, mt.click);
        }
        self.set(i, mt.click_time, Val::D(f64::from(t)));
    }

    pub(super) fn voice(&mut self, i: usize, v: &ssx_formats::ptch::Voice, link: &mut VoiceLink, g: &mut Graph) {
        for (k, p) in v.plugins.iter().enumerate() {
            if p.bypass.is_none() {
                continue;
            }
            let on = as_bool(self.get(i, p.bypass));
            if on != link.bypass[k] {
                link.bypass[k] = on;
                g.set_plugin_bypass(link.slot, k, on);
            }
        }
        for (k, p) in v.params.iter().enumerate() {
            let x = self.param_value(i, p);
            if x != link.params[k] {
                link.params[k] = x;
                g.set_plugin_attr(link.slot, usize::from(p.plugin), usize::from(p.index), x);
            }
        }
        for (k, &(slot, plugin)) in v.sends.iter().enumerate() {
            let target = self.loc(i, slot).and_then(|l| self.handle_acc(self.mem.word(l)));
            if target != link.sends[k] || self.ticks == 0 {
                link.sends[k] = target;
                g.set_send_target(link.slot, usize::from(plugin), target);
            }
        }
        // TODO(guess): the read-backs (BeatDetect's BPM, beat time, confidences, meter) are copied into their patch values every update — the voice process (0x8360DA48) builds commands only; settle: the reader of the voice's read-back list (header +0x20).
        for rb in &v.readbacks {
            if let Some(x) = g.plugin_attr(link.slot, usize::from(rb.plugin), usize::from(rb.index)) {
                let val = if rb.kind == 1 { Val::F(x as f32) } else { Val::I(x as i32) };
                self.set(i, rb.value, val);
            }
        }
    }

    pub fn value(&self, inst: &str, port: &str, name: &str) -> Option<f64> {
        let l = self.member(inst, port, name)?;
        Some(as_f64(self.mem.read(l)))
    }
}
