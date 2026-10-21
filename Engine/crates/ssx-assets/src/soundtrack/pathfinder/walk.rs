use super::*;

impl Pathfinder {
    pub fn tick(&mut self) {
        if self.ticks.is_multiple_of(u64::from(STREAM_TICKS)) {
            self.update(false);
        }
        self.update(true);
        self.ticks += 1;
        self.now += TICK_MS;
    }

    fn update(&mut self, bank: bool) {
        self.choose_master();
        let period = if bank { BANK_PREFETCH_MS } else { 50 };
        if !bank {
            self.run_events();
        }
        for i in 0..MAX_PLAYERS {
            if self.players[i].is_some() {
                self.service(i, bank, period);
            }
        }
        if !bank {
            while self.run_events() > 0 {}
        }
    }

    fn choose_master(&mut self) {
        let mut best = (0u32, None);
        for (i, p) in self.players.iter().enumerate() {
            if let Some(p) = p
                && !p.paused
                && p.is_playing()
                && p.prefetch_ms > best.0
            {
                best = (p.prefetch_ms, Some(i));
            }
        }
        self.master = best.1;
    }

    fn service(&mut self, i: usize, bank: bool, period: u32) {
        let now = self.now;
        let Some(p) = self.players[i].as_mut() else { return };
        if p.is_playing() {
            if let Some(f) = p.volume_fade {
                let (v, done) = f.value(now);
                if done {
                    p.volume_fade = None;
                }
                if let Some(v) = v {
                    p.stream_volume = (i32::from(p.master_volume) as f32 * v as f32 * 0.01) as i32;
                }
            }
            if let Some(f) = p.pitch_fade {
                let (v, done) = f.value(now);
                if done {
                    p.pitch_fade = None;
                }
                if let Some(v) = v
                    && (0..=0x4000).contains(&v)
                {
                    p.pitch = v as u16;
                }
            }
        }
        if p.pause_at != 0 && p.pause_at <= period / 2 + now {
            p.paused = true;
            p.pause_at = 0;
        }
        if p.resume_at != 0 && p.resume_at <= period / 2 + now {
            p.paused = false;
            p.resume_at = 0;
        }
        if p.bank != bank || !p.is_playing() || p.paused {
            return;
        }
        let queued = p.stream.remaining_ms(p.pitch);
        let due = if p.start_at == 0 { queued < p.prefetch_ms } else { p.start_at <= now };
        if due {
            self.advance(i);
        }
    }

    fn advance(&mut self, i: usize) {
        let Some(p) = &self.players[i] else { return };
        let (cur, beat, level, bank) = (p.current, p.beat, p.level, p.bank);
        if cur < 0 {
            return;
        }
        let node = &self.mpf.nodes[cur as usize];
        let pieces = i32::from(node.beats_per_bar) * i32::from(node.bars);
        let stay = beat >= 1 && node.word3 & mpf::FLAG_BEAT_SLICED != 0 && i32::from(beat) <= pieces;
        let (next, new_beat) = if stay {
            (self.walk(cur, cur, i32::from(level), true), beat)
        } else {
            let n = self.choose_next(cur as usize, i32::from(level), true);
            let nb = if bank { 1 } else { -1 };
            (if n < 0 { -1 } else { self.walk(cur, n, i32::from(level), true) }, nb)
        };
        let p = self.players[i].as_mut().unwrap();
        p.current = next;
        p.beat = if next < 0 { -1 } else { new_beat };
        if next < 0 {
            let t = p.track;
            self.logf(format!("p{t} end of graph after n{cur}"));
        }
        self.start_node(i);
    }
}

impl Pathfinder {
    pub(crate) fn choose_next(&mut self, node: usize, level: i32, commit: bool) -> i32 {
        let n = &self.mpf.nodes[node];
        let mut level = level;
        let Some(p) = self.players.get_mut(n.track as usize).and_then(Option::as_mut) else { return -1 };
        if p.loop_node == node as i32 {
            level = ((u32::from(p.loop_left) | (p.prefetch_ms << 5)) & 0x7F) as i32;
            if (n.loop_count as i32) << 27 > 0 && commit {
                let old = p.loop_left;
                p.loop_left = old.wrapping_sub(1) & 0x1F;
                if old == 0 {
                    p.loop_node = -1;
                }
            }
        }
        match n.choose_branch(level) {
            Some(b) => i32::from(b.target),
            None => -1,
        }
    }

    pub(crate) fn walk(&mut self, _cur: i32, target: i32, level: i32, commit: bool) -> i32 {
        let count = self.mpf.nodes.len() as i32;
        if target < 0 || target >= count {
            return -1;
        }
        let owner = self.mpf.nodes[target as usize].track as usize;
        let mut n = target;
        loop {
            let node = self.mpf.nodes[n as usize].clone();
            if node.sample > 0 {
                return n;
            }
            let mut lv = level;
            if commit {
                match node.sample {
                    mpf::SECTION_START => {
                        if let Some(p) = self.players[owner].as_mut() {
                            p.section = Some(n as usize);
                        }
                        if node.level_mode == 1 {
                            lv = (self.random() & 0x7F) as i32;
                        }
                    }
                    mpf::LOOP_MARK => {
                        if let Some(p) = self.players[owner].as_mut()
                            && node.loop_count != 0
                            && p.loop_node != n
                        {
                            p.loop_node = n;
                            p.loop_left = node.loop_count;
                        }
                    }
                    mpf::RANDOM_LEVEL => lv = (self.random() & 0x7F) as i32,
                    mpf::POST_EVENT => {
                        if let Some(e) = self.mpf.event_by_id(node.word3 >> 8) {
                            let name = self.event_name(node.word3 >> 8);
                            self.logf(format!("node n{n} posts {name}"));
                            self.instantiate(e);
                        }
                    }
                    _ => {}
                }
            }
            let next = self.choose_next(n as usize, lv, commit);
            if !(0..count).contains(&next) {
                return -1;
            }
            n = next;
        }
    }

    pub(crate) fn start_node(&mut self, i: usize) {
        let master = self.master.filter(|&m| m != i);
        let master_status = master.map(|m| self.status(m));
        let master_frames = master.and_then(|m| self.beat_frames(m));
        let (frame, now) = (self.frame, self.now);
        let Some(p) = self.players[i].as_mut() else { return };
        if p.current < 0 {
            p.section = None;
            p.current = -1;
            p.beat = -1;
            return;
        }
        let ni = p.current as usize;
        let node = &self.mpf.nodes[ni];
        let Some(rec) = self.mpf.node_sample(node).copied() else { return };
        let record = node.sample as usize - 1;
        let sec = &self.mpf.nodes[node.section as usize];
        p.section = Some(node.section as usize);
        let pieces = u32::from(node.beats_per_bar) * u32::from(node.bars);
        let mut piece = 0u8;
        let mut duration = rec.duration_ms;
        let mut start_frame = None;
        if p.bank {
            let sliced = sec.word3 & mpf::FLAG_BEAT_SLICED != 0;
            if p.beat < 0
                && sec.sync_mode != 0
                && let Some(ms) = master_status
            {
                let (start, beat) = sync_start(node, rec.duration_ms, sec.sync_mode, &ms);
                p.beat = (beat & 0x1F) as i8;
                p.start_at = if start == 0 { 0u32.wrapping_sub(BANK_EARLY_MS) } else { now + start - BANK_EARLY_MS };
                // TODO(guess): the first piece is placed exactly on the master's beat frame (the exe queues it 50 ms early and its scheduler 0x835C8878 / 0x835CD4E8 places it); settle: trace that start-time computation.
                p.sync_frame = master_frames.map(|(to_beat, _)| frame + to_beat);
                let t = p.track;
                self.logf(format!("p{t} n{ni} synced: beat {beat} in {start} ms"));
                return;
            }
            if p.beat < 1 {
                p.beat = 1;
            }
            p.start_at = 0;
            let b = p.beat;
            if !sliced || {
                p.beat += 1;
                pieces <= b as u32
            } {
                p.beat = 0;
            }
            if sliced {
                piece = b as u8;
                duration = rec.duration_ms / pieces.max(1);
                start_frame = p.sync_frame.take();
            }
        } else {
            p.beat = -1;
        }
        let frames = self.segment_frames(node.track as usize, record, piece);
        let id = self.next_id();
        let p = self.players[i].as_mut().unwrap();
        p.stream.items.push_back(Queued {
            id,
            node: ni as u16,
            record,
            piece,
            frames,
            duration_ms: duration,
            start_frame,
        });
        let t = p.track;
        let what = if piece > 0 { format!(" beat {piece}") } else { String::new() };
        self.logf(format!("p{t} queue n{ni}{what} (record {record}, {duration} ms)"));
    }

    fn segment_frames(&self, track: usize, record: usize, piece: u8) -> u32 {
        let rec = self.mpf.samples[record];
        let ms_frames = (u64::from(rec.duration_ms) * u64::from(SAMPLE_RATE) / 1000) as u32;
        if self.mpf.tracks[track].num_subbanks == 0 {
            return self.lengths.records.get(record).copied().unwrap_or(ms_frames);
        }
        let entry = (rec.location & 0xFFFF) as usize + usize::from(piece.max(1)) - 1;
        let entries = self.lengths.entries.get(track).map(Vec::as_slice).unwrap_or(&[]);
        if piece == 0 {
            let n = self.mpf.nodes.iter().find(|n| n.sample as i32 - 1 == record as i32);
            let k = n.map_or(1, |n| usize::from(n.beats_per_bar) * usize::from(n.bars)).max(1);
            return entries.get(entry..entry + k).map_or(ms_frames, |e| e.iter().sum());
        }
        entries.get(entry).copied().unwrap_or(ms_frames / 4)
    }
}
