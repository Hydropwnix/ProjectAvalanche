use super::*;

impl StatsStore {
    pub fn banks_flushed(&mut self, m: &BanksFlushed, record: &TrickRecord) {
        self.reset(TRICK);
        if m.is_bad_landing {
            self.add_int(id::NUM_WIPEOUTS, PERSISTENT, 1);
            self.add_int(id::NUM_WIPEOUTS_TRACK, TRACK, 1);
        }
        let air = m.air_time * 0.016_666_668;
        self.set_float(id::NUM_AIR_TIME_TRICK, TRICK, air);
        self.add_float(id::NUM_AIR_TIME_TRACK, TRACK, air);
        self.add_float(id::NUM_AIR_TIME, PERSISTENT, air);
        for (base, rad) in [(id::NUM_FLIPS_TRICK, m.flip_total), (id::NUM_SPINS_TRICK, m.spin_total)] {
            let deg = f64::from(rad * 57.295_78);
            let count = (deg * 0.002_777_777_845_039_963_7) as i64 as u32;
            self.set_int(base, TRICK, count);
            self.add_int(base + 1, TRACK, count);
            self.add_int(base + 2, PERSISTENT, count);
            let degrees = ((deg * 0.005_555_555_690_079_927_4 + 0.5) as f32 as f64).floor() as f32 * 180.0;
            let d = if base == id::NUM_FLIPS_TRICK { id::NUM_FLIP_DEGREES_TRICK } else { id::NUM_SPIN_DEGREES_TRICK };
            self.set_float(d, TRICK, degrees);
            self.add_float(d + 1, TRACK, degrees);
            self.add_float(d + 2, PERSISTENT, degrees);
        }
        let grind = (f64::from(m.grind_time) * 0.016_666_667_535_901_07) as f32;
        self.set_float(id::NUM_GRIND_TIME_TRICK, TRICK, grind);
        self.add_float(id::NUM_GRIND_TIME_TRICK + 1, TRACK, grind);
        self.add_float(id::NUM_GRIND_TIME_TRICK + 2, PERSISTENT, grind);
        if record.kind == RecordKind::Interaction && record.interaction == 1 {
            self.add_int(id::NUM_TIMES_HELICOPTER_USED_TRACK, TRACK, 1);
            self.add_int(id::NUM_TIMES_HELICOPTER_USED, PERSISTENT, 1);
        }
        let trick = i32::from(record.trick_id.value());
        let mut family = TRICK_FAMILIES[0];
        if record.kind == RecordKind::Air {
            if trick == 1 {
                let key = self.rider.as_ref().map(|r| format!("TrickSignaturePerformed_{r}"));
                let h = key.as_ref().map(|k| fnv1(k.as_bytes()));
                if let (Some(h), Some(k)) = (h, key) {
                    self.names.entry(h).or_insert(k);
                    let done =
                        matches!(self.maps[PERSISTENT].get(&h).map(|r| r.value), Some(StatValue::Int(n)) if n != 0);
                    if !done {
                        self.add_int(id::NUM_UNIQUE_SIGNATURE_TRICKS, PERSISTENT, 1);
                    }
                }
                self.add_int(id::NUM_SIGNATURE_TRICKS_TRACK, TRACK, 1);
                self.add_int(id::NUM_SUPER_UBER_TRICKS_TRACK, TRACK, 1);
                if let Some(h) = h {
                    self.add_int_key(h, PERSISTENT, 1, true);
                }
                return;
            } else if record.super_awesome {
                family = TRICK_FAMILIES[2];
                self.add_int(id::NUM_SUPER_UBER_TRICKS_TRACK, TRACK, 1);
            } else if record.awesome {
                family = TRICK_FAMILIES[1];
                self.add_int(id::NUM_UBER_TRICKS_TRACK, TRACK, 1);
            }
        }
        let Some(name) = trick_name(record.trick_id) else { return };
        let key = format!("{family}{name}");
        let h = fnv1(key.as_bytes());
        self.names.entry(h).or_insert(key);
        self.add_int_key(h, PERSISTENT, 1, true);
    }

    pub fn added_trick_flushed(&mut self, record: &TrickRecord, id: &std::sync::Arc<str>) {
        if !self.writeable || record.kind != RecordKind::Air {
            return;
        }
        let family = usize::from(record.awesome || record.super_awesome) + usize::from(record.super_awesome);
        let n = &mut self.added_tricks.entry(id.clone()).or_default()[family];
        *n = n.saturating_add(1);
    }

    pub fn combo_ended(&mut self, m: &ComboEnded, event_type: Option<i32>) {
        if !m.success {
            if m.is_wingsuit_deployed {
                self.add_int(id::NUM_WIPEOUTS_USING_WINGSUIT_TRACK, TRACK, 1);
            }
            return;
        }
        self.add_int(id::NUM_COMBOS_CHAINED_TRACK, TRACK, m.combo_level);
        self.add_int(id::NUM_COMBOS_CHAINED, PERSISTENT, m.combo_level);
        self.add_float(id::COMBO_SCORE_TRACK, TRACK, m.total_points);
        self.add_float(id::COMBO_SCORE, PERSISTENT, m.total_points);
        self.set_float(id::COMBO_SCORE_TRICK_COMBO, TRACK, m.total_points);
        self.set_int(id::NUM_COMBOS_CHAINED_TRICK_COMBO, TRACK, m.combo_level);
        if trick_event(event_type) {
            self.add_float(id::COMBO_SCORE_TRICK_EVENT_TRACK, TRACK, m.total_points);
            self.add_float(id::COMBO_SCORE_TRICK_EVENT, PERSISTENT, m.total_points);
        }
    }

    pub fn results(&mut self, descent_mm: u32, event_type: Option<i32>, survive_to_death: bool) {
        let d = descent_mm as f32 * 0.001;
        self.add_float(id::NUM_DISTANCE_DESCENDED_TRACK, TRACK, d);
        self.add_float(id::NUM_DISTANCE_DESCENDED, PERSISTENT, d);
        if race_event(event_type) {
            self.add_float(id::NUM_DISTANCE_DESCENDED_IN_RACE_EVENTS, PERSISTENT, d);
        }
        if survive_to_death {
            self.add_int(id::NUM_COMPLETED_PLAY_TILL_DEATH_TRACK, TRACK, 1);
        }
    }

    pub fn survive_lap(&mut self) {
        self.add_int(id::SURVIVE_NUM_LAPS_TRACK, TRACK, 1);
    }

    pub fn holds(&self, name: &str, scope: usize, cmp: Cmp, value: f64) -> bool {
        match self.value(name, scope) {
            Some(StatValue::Int(v)) => cmp.u32(v, value as i64 as u32),
            Some(StatValue::Float(v)) => cmp.f32(v, value as f32),
            None => false,
        }
    }
}
