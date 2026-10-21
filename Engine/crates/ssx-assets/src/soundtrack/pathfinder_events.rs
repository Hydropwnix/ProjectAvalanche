use ssx_formats::mpf::{self, Action, Opcode};

use crate::pathfinder::{EVENT_ARENA, Fade, MAX_EVENTS, MAX_PLAYERS, Pathfinder};

const DONE: u32 = 0x1_0000;

#[derive(Clone, Debug)]
pub struct EventInstance {
    pub event: usize,
    pub actions: Vec<[u32; 3]>,
    pub index: usize,
    pub start: u32,
    pub timeout: u32,
    pub resume: u32,
    pub word4: u32,
}

impl EventInstance {
    fn bytes(&self) -> usize {
        mpf::EVENT_HEADER_SIZE + mpf::ACTION_SIZE * self.actions.len()
    }
}

fn with_depths(actions: &[Action]) -> Vec<[u32; 3]> {
    let (mut depth, mut outer) = (0i32, -1i32);
    actions
        .iter()
        .map(|a| {
            let mut w = a.words;
            let d = if a.opcode() == Opcode::Control {
                match a.sub_op() {
                    0 | 1 => {
                        let d = depth;
                        depth += 1;
                        outer += 1;
                        d
                    }
                    2 | 3 => outer,
                    4 => {
                        depth -= 1;
                        outer -= 1;
                        depth
                    }
                    _ => return w,
                }
            } else {
                depth
            };
            w[1] = (w[1] & !0x38) | (((d & 7) as u32) << 3);
            w
        })
        .collect()
}

fn depth(w: &[u32; 3]) -> u32 {
    (w[1] >> 3) & 7
}

fn is_end(w: &[u32; 3]) -> bool {
    (w[1] >> 17) & 0x7F == 1 && (w[1] >> 9) & 7 == 4
}

impl Pathfinder {
    pub(crate) fn instantiate(&mut self, e: usize) -> bool {
        let ev = &self.mpf.events[e];
        let inst = EventInstance {
            event: e,
            actions: with_depths(&ev.actions),
            index: 0,
            start: self.now,
            timeout: 0,
            resume: self.now,
            word4: 0,
        };
        let used: usize = self.events.iter().map(EventInstance::bytes).sum();
        if self.events.len() >= MAX_EVENTS || used + inst.bytes() >= EVENT_ARENA {
            self.logf(format!("event ev{e} dropped (queue full)"));
            return false;
        }
        self.events.push(inst);
        true
    }

    pub(crate) fn run_events(&mut self) -> usize {
        let mut progressed = 0;
        let mut i = 0;
        while i < self.events.len().min(MAX_EVENTS) {
            let e = &self.events[i];
            let finished = e.index >= e.actions.len();
            let timed_out = e.timeout != 0 && e.start.wrapping_add(e.timeout) < self.now;
            if finished || timed_out {
                self.events.remove(i);
                continue;
            }
            progressed += self.run_event(i);
            i += 1;
        }
        progressed
    }

    fn run_event(&mut self, i: usize) -> usize {
        let mut n = 0;
        if self.now < self.events[i].resume {
            return 0;
        }
        while self.events[i].index < self.events[i].actions.len() {
            let k = self.events[i].index;
            if self.events[i].actions[k][1] & DONE == 0 && !self.exec(i, k) {
                return n;
            }
            let e = &mut self.events[i];
            e.index += 1;
            e.resume = self.now;
            n += 1;
        }
        n
    }
}

fn sx(v: u32, bits: u32) -> i32 {
    ((v << (32 - bits)) as i32) >> (32 - bits)
}

impl Pathfinder {
    fn player_for_bit(&self, bit: usize) -> Option<usize> {
        (bit..MAX_PLAYERS).find(|&j| self.players[j].is_some())
    }

    pub(crate) fn exec(&mut self, i: usize, k: usize) -> bool {
        let w0 = self.events[i].actions[k][0];
        for bit in 0..MAX_PLAYERS {
            if w0 & (1 << bit) == 0 {
                continue;
            }
            let Some(j) = self.player_for_bit(bit) else { continue };
            let active = self.players[j].as_ref().unwrap().active;
            let need_active = (w0 >> 28) & 1 != 0;
            if (need_active && !active) || (!need_active && w0 & 0xE000_0000 != 0 && active) {
                continue;
            }
            if let Some(ret) = self.exec_for(i, k, j) {
                return ret;
            }
        }
        self.events[i].actions[k][1] & DONE != 0
    }

    fn exec_for(&mut self, i: usize, k: usize, j: usize) -> Option<bool> {
        let [_, w1, w2] = self.events[i].actions[k];
        let src_a = (w1 >> 14) & 3;
        let src_b = (w1 >> 12) & 3;
        let hi = (w2 as i32) >> 16;
        let lo16 = w2 as i16 as i32;
        let set_done = |s: &mut Self| s.events[i].actions[k][1] |= DONE;
        match (w1 >> 17) & 0x7F {
            1 => return self.exec_control(i, k, j),
            2 => {
                let mut v = self.resolve(hi, src_b, j, i);
                let cap = self.events[i].actions[k][2] as i16 as i32;
                if cap > 0 && v > cap {
                    v = 0;
                }
                if src_b == 1 && (20..=22).contains(&hi) {
                    if cap == 0 || cap > v {
                        let a = &mut self.events[i].actions[k][2];
                        *a = (*a & 0xFFFF_0000) | (v as u32 & 0xFFFF);
                    }
                    if v > 0 {
                        v = self.now as i32 + 3000;
                    }
                }
                if v <= self.now.wrapping_sub(self.events[i].resume) as i32 {
                    let a = &mut self.events[i].actions[k];
                    a[2] &= 0xFFFF_0000;
                    a[1] |= DONE;
                }
            }
            4 => return self.exec_goto(i, k, j),
            5 => {
                let time = self.resolve((w2 & 0xFFFF) as i32, src_b, j, i);
                let target = (w2 >> 24) as i8 as i32;
                let curve = sx(w2 >> 17, 7);
                let now = self.now;
                let p = self.players[j].as_mut().unwrap();
                let old = p.volume_fade.map_or(0x7F, |f| f.curve);
                let from = p.stream_volume;
                p.volume_fade = Some(Fade {
                    start_ms: now,
                    duration_ms: time as u32,
                    from,
                    to: target,
                    curve: if curve >= 0 { curve } else { old },
                });
                p.volume = target as u8;
                let t = p.track;
                set_done(self);
                self.logf(format!("  p{t} volume {from} -> {target} over {time} ms"));
            }
            8 => {
                let v = self.resolve(hi, src_b, j, i);
                set_done(self);
                self.set_value((w2 >> 8) & 0xFF, src_a, v, j, i);
            }
            9 => {
                let id = self.resolve((w2 >> 8) as i32, src_a, j, i) as u32;
                let raw = w2 >> 8;
                let mask = if id == raw { 0xFF_FFFF } else { 0xFFFF };
                let found = self.mpf.events.iter().rposition(|e| e.id & mask == id & mask);
                if let Some(e) = found {
                    let name = self.event_name(self.mpf.events[e].id);
                    self.logf(format!("  posts {name}"));
                    self.instantiate(e);
                }
                set_done(self);
            }
            14 => {
                let a = self.resolve((w2 >> 24) as i32, src_a, j, i);
                let b = self.resolve(lo16, src_b, j, i);
                set_done(self);
                let v = match (w2 >> 16) & 0xFF {
                    1 => a.wrapping_add(b),
                    2 => a.wrapping_sub(b),
                    3 => a.wrapping_mul(b),
                    4 => a.checked_div(b).unwrap_or(a),
                    5 => a.checked_rem(b).unwrap_or(a),
                    _ => a,
                };
                self.set_value(w2 >> 24, src_a, v, j, i);
            }
            15 => {
                let v = self.resolve(hi, src_b, j, i);
                set_done(self);
                let now = self.now;
                let p = self.players[j].as_mut().unwrap();
                if v < 1 {
                    p.paused = w2 & 0x8000 != 0;
                } else if w2 & 0x8000 == 0 {
                    p.resume_at = now + v as u32;
                } else {
                    p.pause_at = now + v as u32;
                }
            }
            17 => {
                let time = self.resolve((w2 & 0x3FFF) as i32, src_b, j, i);
                let target = ((w2 >> 18) as i32).clamp(0, 0x4000);
                let curve = sx(w2 >> 15, 3);
                let now = self.now;
                let p = self.players[j].as_mut().unwrap();
                let old = p.pitch_fade.map_or(0x7F, |f| f.curve);
                let from = i32::from(p.pitch);
                p.pitch_fade = Some(Fade {
                    start_ms: now,
                    duration_ms: time as u32,
                    from,
                    to: target,
                    curve: if curve >= 0 { curve } else { old },
                });
                let t = p.track;
                set_done(self);
                self.logf(format!("  p{t} pitch {from} -> {target} (×1/4096) over {time} ms"));
            }
            op => {
                let name = self.event_name(self.mpf.events[self.events[i].event].id);
                self.logf(format!("  {name}: opcode {op} not ported, skipped"));
                set_done(self);
            }
        }
        None
    }
}

impl Pathfinder {
    fn exec_control(&mut self, i: usize, k: usize, j: usize) -> Option<bool> {
        let w = self.events[i].actions[k];
        let sub = (w[1] >> 9) & 7;
        if sub == 3 {
            self.events[i].actions[k][1] |= DONE;
            return Some(true);
        }
        if sub == 4 {
            self.events[i].actions[k][1] |= DONE;
            let d = depth(&w);
            let head = (0..k).rev().find(|&h| {
                let x = &self.events[i].actions[h];
                depth(x) == d && (x[1] >> 17) & 0x7F == 1 && (x[1] >> 9) & 7 == 0
            });
            let Some(h) = head else { return Some(true) };
            let now = self.now;
            let e = &mut self.events[i];
            e.index = h;
            for a in &mut e.actions[h..=k] {
                a[1] &= !DONE;
            }
            if e.resume == now {
                e.resume += 1;
            }
            return Some(false);
        }
        let a = self.resolve((w[2] as i32) >> 16, (w[1] >> 14) & 3, j, i);
        let b = self.resolve(w[2] as i16 as i32, (w[1] >> 12) & 3, j, i);
        let result = match (w[1] >> 6) & 7 {
            0 => Some(true),
            1 => Some(a == b),
            2 => Some(a != b),
            3 => Some(a > b),
            4 => Some(a < b),
            5 => Some(a >= b),
            6 => Some(a <= b),
            _ => None,
        };
        let mut w1 = self.events[i].actions[k][1];
        if let Some(r) = result {
            w1 = (w1 & !DONE) | (u32::from(r) << 16);
        }
        self.events[i].actions[k][1] = w1;
        if sub == 0 {
            if w1 & DONE == 0 {
                self.skip_block(i, k, false);
            }
            let a = &mut self.events[i].actions[k][1];
            *a ^= DONE;
            return Some(true);
        }
        self.skip_block(i, k, w1 & DONE != 0);
        self.events[i].actions[k][1] |= DONE;
        None
    }

    fn skip_block(&mut self, i: usize, k: usize, taken: bool) {
        let e = &mut self.events[i];
        let d = depth(&e.actions[k]);
        let mut marking = !taken;
        for a in e.actions.iter_mut().skip(k + 1) {
            if depth(a) == d {
                if is_end(a) {
                    a[1] |= DONE;
                    return;
                }
                if !taken {
                    return;
                }
                marking = true;
                a[1] |= DONE;
            } else if marking {
                a[1] |= DONE;
            }
        }
    }

    fn exec_goto(&mut self, i: usize, k: usize, j: usize) -> Option<bool> {
        let [_, w1, w2] = self.events[i].actions[k];
        let count = self.mpf.nodes.len() as i32;
        let mut target = self.resolve((w2 as i32) >> 16, (w1 >> 14) & 3, j, i);
        let part = self.resolve(sx(w2 >> 8, 8), (w1 >> 12) & 3, j, i);
        if target < -1 || target > count {
            target = -1;
        }
        let track = i32::from(self.players[j].as_ref().unwrap().track);
        if target >= 0 {
            let found = if part < 0 {
                Some(target).filter(|&t| t < count)
            } else {
                let mut starts = self
                    .mpf
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.sample == mpf::SECTION_START && i32::from(n.part) == part);
                if target < 1 { None } else { starts.nth(target as usize - 1).map(|(x, _)| x as i32) }
            };
            match found {
                Some(t) if i32::from(self.mpf.nodes[t as usize].track) == track => target = t,
                _ => return None,
            }
        }
        let p = self.players[j].as_ref().unwrap();
        if p.flag5 {
            return None;
        }
        let old = p.current;
        if w2 & 0x80 != 0 {
            self.flush(j);
        }
        let level = i32::from(self.players[j].as_ref().unwrap().level);
        let next = self.walk(old, target, level, true);
        let name = self.event_name(self.mpf.events[self.events[i].event].id);
        self.logf(format!(
            "  {name}: p{track} goto n{target} -> n{next}{}",
            if w2 & 0x80 != 0 { " (flush)" } else { "" }
        ));
        let p = self.players[j].as_mut().unwrap();
        p.beat = -1;
        p.current = next;
        self.start_node(j);
        self.events[i].actions[k][1] |= DONE;
        None
    }
}

impl Pathfinder {
    pub(crate) fn resolve(&mut self, v: i32, source: u32, j: usize, i: usize) -> i32 {
        match source {
            2 => self.variables.get(v as usize).copied().unwrap_or(0),
            1 => self.property(v, j, i),
            _ => v,
        }
    }

    fn property(&mut self, id: i32, j: usize, i: usize) -> i32 {
        let st = self.status(j);
        let p = self.players[j].as_ref().unwrap();
        let node = (p.current >= 0).then(|| &self.mpf.nodes[p.current as usize]);
        match id {
            1 => i32::from(p.level),
            2 => p.current as i16 as i32,
            3 => node.map_or(-1, |n| i32::from(n.section)),
            4 => node.map_or(-1, |n| i32::from(n.part)),
            5 => self.events[i].timeout as i32,
            6 => sx(self.events[i].word4 >> 16, 4),
            7 => i32::from(p.flags_1_3),
            8 => i32::from(p.prop8),
            9 => i32::from(p.prop9),
            10 => i32::from(p.active),
            11 => {
                let (cur, level) = (p.current, i32::from(p.level));
                if cur < 0 {
                    return -1;
                }
                let n = self.choose_next(cur as usize, level, false);
                self.walk(cur, n, level, false)
            }
            12 => i32::from(p.flag5),
            13 => st.duration as i32,
            14 => i32::from(p.paused),
            15 => i32::from(p.pitch),
            16 => st.node.map_or(-1, |n| n as i16 as i32),
            17 => {
                // TODO(guess): states 2 and 4 depend on the stream's entry queries (vtable +0x20 / +0x24, the bank load), not traced and not read on the disc (mpf_verify); settle: 0x835C8EF8 with the stream class 0x8222DAE0.
                if p.paused {
                    3
                } else if p.current < 0 {
                    1
                } else if p.volume_fade.is_some() {
                    6
                } else {
                    5
                }
            }
            18 => ((self.random() / 0x17) & 0xFFFF) as i32,
            19 => self.now as i32,
            20 => st.to_beat as i32,
            21 => st.to_bar as i32,
            22 => st.remaining as i32,
            23 => {
                if p.volume_fade.is_some() {
                    p.stream_volume
                } else {
                    i32::from(p.volume as i8)
                }
            }
            25 => st.bar as i32,
            26 => st.beat as i32,
            _ => id,
        }
    }

    fn set_value(&mut self, id: u32, scope: u32, v: i32, j: usize, i: usize) {
        if scope == 2 {
            if let Some(x) = self.variables.get_mut(id as usize) {
                *x = v;
            }
            let name = self.mpf.variables.get(id as usize).map_or(String::new(), |x| x.name.clone());
            self.logf(format!("  set {name} = {v}"));
            return;
        }
        if scope != 1 {
            return;
        }
        let clamp127 = |v: i32| if v < 0x80 { v.max(0) } else { 0x7F };
        match id {
            1 => self.players[j].as_mut().unwrap().level = v as u8,
            5 => self.events[i].timeout = v as u32,
            6 => {
                let v = v.clamp(-7, 7);
                let e = &mut self.events[i];
                e.word4 = (e.word4 & !0xF_0000) | ((v as u32 & 0xF) << 16);
            }
            7 => self.players[j].as_mut().unwrap().flags_1_3 = (v & 7) as u8,
            8 => self.players[j].as_mut().unwrap().prop8 = clamp127(v) as u16,
            9 => self.players[j].as_mut().unwrap().prop9 = clamp127(v) as u16,
            10 => self.players[j].as_mut().unwrap().active = true,
            12 => self.players[j].as_mut().unwrap().flag5 = v != 0,
            14 => self.players[j].as_mut().unwrap().paused = v != 0,
            15 => {
                if (0..=0x4000).contains(&v) {
                    let p = self.players[j].as_mut().unwrap();
                    p.pitch = v as u16;
                    let t = p.track;
                    self.logf(format!("  p{t} pitch = {v}"));
                }
            }
            23 => {
                let p = self.players[j].as_mut().unwrap();
                p.set_volume(clamp127(v) as u8);
                let t = p.track;
                self.logf(format!("  p{t} volume = {}", clamp127(v)));
            }
            _ => {
                self.logf(format!("  property {id} = {v} not ported"));
            }
        }
    }
}
