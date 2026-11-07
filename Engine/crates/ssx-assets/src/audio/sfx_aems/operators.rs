use super::*;

pub(super) fn envelope(s: &mut [u8], st: usize, tick: f32) -> i32 {
    let gate = rd(s, st + rh(s, st) as usize);
    let prev = rb(s, st + 2);
    let seg_target = |s: &[u8], i: usize| (rdf(s, st + 0x18 + 8 * i), rdf(s, st + 0x1C + 8 * i));
    let mut hold = false;
    if gate == 1 && prev == 0 {
        wb(s, st + 3, 0);
        let start = rdf(s, st + 0x14);
        wrf(s, st + 0xC, start);
        let (ms, target) = seg_target(s, 0);
        wrf(s, st + 4, ms);
        wrf(s, st + 8, (target - start) / ms * tick);
        hold = true;
    } else if gate == 3 && prev != 3 && i32::from(rb(s, st + 3)) < i32::from(rh(s, st + 0x12) as i16) {
        let i = rh(s, st + 0x12) as u8;
        wb(s, st + 3, i);
        let value = rdf(s, st + 0xC);
        let (ms, target) = seg_target(s, i as usize);
        wrf(s, st + 4, ms);
        wrf(s, st + 8, (target - value) / ms * tick);
        hold = true;
    }
    if !hold {
        let seg = rb(s, st + 3);
        if (gate == 1 || gate == 3) && seg < rb(s, st + 0x10) {
            let left = rdf(s, st + 4) - tick;
            wrf(s, st + 4, left);
            if left > 0.0 {
                let v = rdf(s, st + 0xC) + rdf(s, st + 8);
                wrf(s, st + 0xC, v);
            } else {
                let (_, reached) = seg_target(s, seg as usize);
                let seg = seg + 1;
                wb(s, st + 3, seg);
                wrf(s, st + 0xC, reached);
                if seg >= rb(s, st + 0x10) {
                    wrf(s, st + 0xC, 0.0);
                } else {
                    let (ms, target) = seg_target(s, seg as usize);
                    wrf(s, st + 4, ms);
                    wrf(s, st + 8, (target - reached) / ms * tick);
                }
            }
        } else if gate != 2 {
            wrf(s, st + 0xC, 0.0);
        }
    }
    wb(s, st + 2, gate as u8);
    round(rdf(s, st + 0xC))
}

pub(super) fn curve(data: &[u8], p: usize, x: i32) -> i32 {
    let width = rb(data, p);
    let n = i32::from(rh(data, p + 2));
    let (min, max) = (rd(data, p + 4), rd(data, p + 8));
    let scale = rdf(data, p + 0xC);
    let x = clamp(x, min, max).wrapping_sub(min);
    let point = |i: i32| -> i32 {
        let at = |w: i64| (p as i64 + 0x10 + i64::from(i) * w) as usize;
        match width {
            2 => i32::from(rh(data, at(2)) as i16),
            1 => i32::from(rb(data, at(1)) as i8),
            _ => rd(data, at(4)),
        }
    };
    if scale == 1.0 {
        return point(x);
    }
    let f = x as f32 * scale;
    let i = round(f - 0.5);
    let frac = f - i as f32;
    let j = if i + 1 >= n { n - 1 } else { i + 1 };
    let (a, b) = (point(i) as f32, point(j) as f32);
    round((b - a).mul_add(frac, a))
}

pub(super) fn delay(s: &mut [u8], st: usize, tick: f32) -> i32 {
    let input = st + rh(s, st) as usize;
    let len = rh(s, st + 2);
    if rd(s, input + 4) != rd(s, st + 8) {
        let d = rd(s, input + 4);
        wr(s, st + 8, d);
        if d < 0 {
            wr(s, input + 4, 0);
        }
        let mut ticks = (rd(s, input + 4) as f32 / tick + 0.5) as i32;
        if ticks >= i32::from(len) {
            ticks = i32::from(len) - 1;
        }
        let read = rh(s, st + 6);
        wh(s, st + 4, (i32::from(read) + ticks) as u16);
    }
    if rh(s, st + 4) >= len {
        let w = rh(s, st + 4) - len;
        wh(s, st + 4, w);
    }
    if rh(s, st + 6) >= len {
        wh(s, st + 6, 0);
    }
    let (w, r) = (rh(s, st + 4) as usize, rh(s, st + 6) as usize);
    let v = rd(s, input);
    wr(s, st + 0xC + 4 * w, v);
    let out = rd(s, st + 0xC + 4 * r);
    wh(s, st + 4, w as u16 + 1);
    wh(s, st + 6, r as u16 + 1);
    out
}

pub(super) fn lfo(s: &mut [u8], st: usize, tick: f32) -> i32 {
    let period = rd(s, st + 8);
    if period < 1 {
        return 0;
    }
    let depth = f64::from(rd(s, st + 0xC));
    let inc = f64::from(tick / period as f32);
    while rdf(s, st + 4) >= 1.0 {
        let p = rdf(s, st + 4) - 1.0;
        wrf(s, st + 4, p);
    }
    let p = f64::from(rdf(s, st + 4));
    let v: f64 = match rb(s, st) {
        0 => {
            let idx = round((p * 1024.0) as f32);
            f64::from((f64::from(sine(idx)) * depth) as f32 * 1.525_878_9e-5)
        }
        1 => {
            if p < 0.5 {
                0.0
            } else {
                depth
            }
        }
        2 => f64::from((p * depth) as f32),
        _ => {
            if p >= 0.5 {
                f64::from((f64::from((1.0 - p) as f32) * depth) as f32 * 2.0)
            } else {
                f64::from((p * depth) as f32 * 2.0)
            }
        }
    };
    wrf(s, st + 4, (p + inc) as f32);
    if v < 0.0 { (v - 0.5) as i32 } else { (v + 0.5) as i32 }
}

pub(super) fn glide(s: &mut [u8], st: usize, tick: f32) -> i32 {
    let target = rd(s, st + 0x18);
    let ft = target as f32;
    let cur = rdf(s, st);
    if ft == cur {
        return target;
    }
    if target != rd(s, st + 8) || rd(s, st + 0x10) != rd(s, st + 0xC) {
        let t = rd(s, st + 0x10);
        wr(s, st + 8, target);
        wr(s, st + 0xC, t);
        if t <= 0 {
            wrf(s, st, ft);
            return target;
        }
        wrf(s, st + 4, (ft - cur) / t as f32 * tick * (1.0 / 4096.0));
    }
    let step = rdf(s, st + 4);
    let new = (rd(s, st + 0x14) as f32).mul_add(step, cur);
    wrf(s, st, new);
    if (step < 0.0 && new < ft) || (step >= 0.0 && new > ft) {
        wrf(s, st, ft);
    }
    round(rdf(s, st))
}

impl Aems {
    pub(super) fn player(&mut self, inst: &mut Instance, st: usize, host: &mut dyn VoiceHost) -> i32 {
        let s = &mut inst.state;
        let cmd = clamp(rd(s, st + 0x18), 0, 2);
        let cur = i32::from(rb(s, st + 0xC) as i8);
        let np = rb(s, st + 0xE) as usize;
        let outputs = (rb(s, st + 0xF) != 0).then_some(st + 0x1C + 12 * np);
        let clear = |s: &mut [u8]| {
            if let Some(o) = outputs {
                wr(s, o, 0);
                wr(s, o + 4, 0);
            }
        };
        let mut ret = cmd;
        if cmd != cur {
            let voice = rd(s, st + 8) as u32;
            match cmd {
                0 => {
                    if voice != 0 {
                        host.destroy(voice);
                        wr(s, st + 8, 0);
                        clear(s);
                    }
                }
                1 if voice != 0 => host.resume(voice),
                1 => {
                    let prev = i32::from(rb(s, st + 0xD) as i8);
                    if cur != 2 || prev != 1 {
                        let data = &self.banks[inst.bank].data;
                        let table = rd(s, st + 4) as usize;
                        let n = rd(data, table);
                        let idx = rd(s, st + 0x14);
                        let idx = if idx < n { idx.max(0) } else { n - 1 };
                        let e = table + 4 + 12 * idx.max(0) as usize;
                        let entry = SampleEntry {
                            sound_index: rh(data, e),
                            unk_2: rb(data, e + 2),
                            azimuths: [
                                rb(data, e + 3),
                                rb(data, e + 4),
                                rb(data, e + 5),
                                rb(data, e + 6),
                                rb(data, e + 7),
                            ],
                            stream_offset: rd(data, e + 8) as u32,
                        };
                        let params: Vec<(u8, i32)> =
                            (0..np).map(|k| (rb(s, st + 0x1C + 12 * k), rd(s, st + 0x24 + 12 * k))).collect();
                        let choice = if entry.is_unused() { None } else { self.choose(inst, st as u32, &params) };
                        let s = &mut inst.state;
                        let voice = if entry.is_unused() {
                            None
                        } else {
                            host.start(inst.bank, &entry, &params, choice.as_ref())
                        };
                        wr(s, st + 8, voice.unwrap_or(0) as i32);
                        if voice.is_some() {
                            for k in 0..np {
                                let v = rd(s, st + 0x24 + 12 * k);
                                wr(s, st + 0x20 + 12 * k, v);
                            }
                        } else {
                            clear(s);
                        }
                        if self.verbose {
                            let b = &self.banks[inst.bank];
                            let text = format!(
                                "  {} module {} play entry {idx} (sound {}) {params:?} → voice {voice:?}",
                                b.name, inst.module, entry.sound_index
                            );
                            self.logf(text);
                        }
                    }
                }
                _ => {
                    if voice != 0 {
                        host.pause(voice);
                    }
                }
            }
            let s = &mut inst.state;
            wb(s, st + 0xD, cur as u8);
            wb(s, st + 0xC, cmd as u8);
        }
        let s = &mut inst.state;
        let voice = rd(s, st + 8) as u32;
        if cmd == 1 && voice != 0 {
            for k in 0..np {
                let (last, value) = (rd(s, st + 0x20 + 12 * k), rd(s, st + 0x24 + 12 * k));
                if value != last {
                    host.set_param(voice, rb(s, st + 0x1C + 12 * k), value);
                    wr(s, st + 0x20 + 12 * k, value);
                }
            }
            ret = match host.status(voice) {
                None => {
                    host.destroy(voice);
                    wr(s, st + 8, 0);
                    clear(s);
                    0
                }
                Some((remaining, position)) => {
                    if let Some(o) = outputs {
                        wr(s, o, remaining);
                        wr(s, o + 4, position);
                    }
                    1
                }
            };
        }
        if rd(&inst.state, st + 8) == 0 { 0 } else { ret }
    }
}
