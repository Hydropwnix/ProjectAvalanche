use super::*;

impl ScoreState {
    pub(super) fn capture_banks(&mut self, t: &ScoringTuning, c: &ScoringContext) -> (f32, bool) {
        let (mut captured, mut any) = (0.0, false);
        (self.grind_capture, self.press_capture, self.interaction_capture) = (0.0, 0.0, 0.0);
        if self.prev_is_scoring_rewind && !self.is_scoring_rewind {
            captured += self.rewind_bank;
            if self.rewind_bank < 0.0 {
                any = true;
                self.post_and_push(true, false, false, [0.0; 5]);
            }
            self.rewind_bank = 0.0;
        }
        if !self.wipeout {
            if self.just_landed || self.is_scoring_interactions || self.just_changed_tricks || self.force_capture {
                captured += self.air_bank;
                any = self.air_bank > 0.0;
                if self.air_bank > 0.0 {
                    self.post_and_push(true, false, self.just_landed, [self.air_bank, 0.0, 0.0, 0.0, 0.0]);
                    self.combo_count = (self.combo_count + 1).min(c.tuning.combo_max_count);
                }
                (self.last_capture, self.last_lost, self.air_bank) = (self.air_bank, false, 0.0);
            }
            if self.just_stopped_grinding || self.just_changed_tricks || self.force_capture {
                captured += self.grind_bank;
                self.grind_capture = self.grind_bank;
                if self.grind_bank > 0.0 {
                    any = true;
                    self.post_and_push(true, false, false, [0.0, 0.0, self.grind_bank, 0.0, 0.0]);
                    self.combo_count = (self.combo_count + 1).min(c.tuning.combo_max_count);
                }
                self.grind_bank = 0.0;
            }
            if self.just_stopped_board_pressing || self.force_capture {
                captured += self.press_bank;
                self.press_capture = self.press_bank;
                if self.press_bank > 0.0 {
                    any = true;
                    self.post_and_push(true, false, false, [0.0, 0.0, 0.0, self.press_bank, 0.0]);
                    self.combo_count = (self.combo_count + 1).min(c.tuning.combo_max_count);
                }
                self.press_bank = 0.0;
            }
            if self.just_stopped_interacting || self.force_capture {
                let bank = self.interaction_bank;
                captured += bank;
                self.interaction_capture = bank;
                if bank.abs() > 1.525_878_9e-5 {
                    any = true;
                    self.post_and_push(true, false, false, [0.0, 0.0, 0.0, 0.0, bank]);
                    self.combo_count = (self.combo_count + 1).min(c.tuning.combo_max_count);
                }
                self.interaction_bank = 0.0;
            }
            self.force_capture = false;
            return (captured, any);
        }
        self.pushed_object = false;
        if self.just_wipeout {
            self.post_and_push(false, true, self.just_landed, [0.0; 5]);
            if self.combo_points < 0.0 {
                self.score = (self.score + (self.flow_level + 1) as f32 * self.combo_points).max(0.0);
            }
            self.send_combo_ended(false, self.combo_points, 0.0);
            self.combo_count = 0;
            (self.combo_points, self.combo_timer, self.combo_timer_max, self.combo_pct) = (0.0, 0.0, 0.0, 0.0);
            (self.super_activation, self.super_timer, self.awesome_timer) = (0.0, 0.0, 0.0);
            (self.is_awesome, self.is_super_awesome) = (false, false);
            if self.interaction_bank < 0.0 {
                captured += self.interaction_bank;
                any = true;
            }
            (self.last_capture, self.last_lost, self.air_bank) = (self.air_bank, true, 0.0);
            (self.grind_bank, self.press_bank, self.interaction_bank) = (0.0, 0.0, 0.0);
            self.boost *= t.boost_wipeout_multiplier;
        }
        (captured, any)
    }

    pub(super) fn post_and_push(&mut self, captured: bool, clear: bool, just_landed: bool, banks: [f32; 5]) {
        let m = BanksFlushed {
            is_points_captured: captured,
            is_bad_landing: clear,
            spin_total: self.spin_fs.abs() + self.spin_bs.abs(),
            flip_total: self.flip_fwd.abs() + self.flip_bwd.abs(),
            air_time: self.air_time,
            flight_distance: self.flight_distance,
            grind_time: self.grind_time,
            grind_distance: self.grind_distance,
            air_bank: banks[0],
            flow_bank: banks[1],
            grind_bank: banks[2],
            board_press_bank: banks[3],
            interactions_bank: banks[4],
        };
        if let Some(slot) = self.flushed.iter_mut().find(|f| f.is_none()) {
            *slot = Some(m);
        }
        self.push_record(clear, just_landed);
    }

    pub(super) fn send_combo_ended(&mut self, success: bool, trick_points: f32, bonus_points: f32) {
        self.combo_ended = Some(ComboEnded {
            success,
            is_wingsuit_deployed: self.prev_in_wingsuit || self.just_stopped_wingsuit,
            trick_points,
            bonus_points,
            total_points: trick_points + bonus_points,
            combo_level: self.combo_count,
        });
    }

    pub(super) fn push_record(&mut self, clear: bool, just_landed: bool) {
        if clear {
            (self.last_pushed, self.record) = (None, TrickRecord::default());
            return;
        }
        let rec = self.record;
        if rec.kind == RecordKind::None {
            return;
        }
        if rec.kind == RecordKind::Air && rec.tier >= 3 && self.is_super_awesome && just_landed {
            self.is_big_landing = true;
        }
        self.last_pushed = Some(rec);
        if !matches!(rec.kind, RecordKind::BoardPress | RecordKind::Rewind) && !matches!(rec.interaction, 2 | 3) {
            self.last_record = rec;
        }
        self.record = TrickRecord::default();
    }

    pub(super) fn restart_combo_timer(&mut self, k: &ComboFlowTuning, press: bool) {
        let (tt, dup) = self.last_pushed.map_or((0.0, false), |r| (r.trick_time, r.duplicate));
        let f = if dup {
            k.combo_time_dupe_percentage
        } else if self.is_scoring_board_press || press {
            let n = self.board_press_count;
            k.combo_time_board_press_percentage * if n > 1 { 0.25 / (n - 1) as f32 } else { 1.0 }
        } else if tt < 90.0 {
            k.combo_time_small_percentage
        } else if tt < 180.0 {
            k.combo_time_medium_percentage
        } else {
            1.0
        };
        self.combo_timer_max = k.combo_link_time * f;
        self.combo_timer = self.combo_timer_max;
    }

    pub(super) fn update_flow_level(&mut self, captured: f32, any: bool, in_air: bool, k: &ComboFlowTuning) {
        if any && self.flow_state == FlowState::Positive {
            let [normal, tricky, super_tricky] = FLOW_LEVEL_UP_POINTS;
            let thr = if self.is_super_awesome {
                super_tricky
            } else if self.is_awesome {
                tricky
            } else {
                normal
            };
            if captured >= thr {
                self.flow_level += 1;
            }
        } else if self.flow_level > 0 {
            if self.flow_state != FlowState::Positive && !in_air && !self.is_scoring_grind {
                if self.prev_flow_state == FlowState::Positive {
                    self.flow_level -= 1;
                } else if self.flow_state_timer > k.flow_stagnation_time * 60.0 {
                    self.flow_level -= 1;
                    self.flow_state_timer -= k.flow_stagnation_time * 60.0;
                }
            }
        } else if self.flow_level == 0 && self.flow_state == FlowState::Negative {
            self.flow_level = -1;
        } else if self.flow_level < 0 && self.flow_state != FlowState::Negative {
            self.flow_level = 0;
        }
        self.flow_level = self.flow_level.clamp(-1, 19);
    }

    pub(super) fn add_points(&mut self, pts: f32, combo_end: bool) {
        self.score = (self.score + pts).max(0.0);
        self.base_points = Some(pts);
        if combo_end {
            let (f, cp) = (self.flow_level, self.combo_points);
            let paid = if f > 0 || cp >= 0.0 { (f + 1) as f32 * cp } else { cp };
            self.score += paid;
            let bonus = if f > 0 && cp >= 0.0 { combo_bonus(self.combo_count) } else { 0.0 };
            self.score = (self.score + bonus).max(0.0);
            self.send_combo_ended(true, paid, bonus);
            (self.combo_points, self.combo_count) = (0.0, 0);
        }
    }

    pub(super) fn update_boost_and_awesome(
        &mut self,
        t: &ScoringTuning,
        r: &ResolvedCharacterProperties,
        captured: f32,
        in_air: bool,
        track_finished: bool,
        dt: f32,
    ) {
        let modifier = if self.is_super_awesome {
            r.super_awesome_boost_meter_increase_modifier
        } else if self.is_awesome {
            r.awesome_boost_meter_increase_modifier
        } else {
            r.boost_meter_increase_modifier
        };
        let m = modifier * r.boost_bonus * captured;
        let tricky_len = t.awesome_time * r.tricky_bonus * r.tricky_timer_modifier;
        if !self.is_awesome {
            let old = self.boost;
            self.boost = (self.boost + m / t.points_per_boost).min(t.max_boost);
            self.super_activation = 0.0;
            if old < t.max_boost && old + m / t.points_per_boost > t.max_boost {
                self.awesome_timer = tricky_len;
            }
        } else {
            self.super_activation = (self.super_activation + m / t.super_awesome_points_activation).min(1.0);
            if self.super_activation >= 1.0 && !self.is_super_awesome {
                self.super_timer = t.super_awesome_time * r.tricky_bonus * r.tricky_timer_modifier;
            }
            let combo = self.combo_timer > 0.0;
            let stall = |x: f32| x - dt <= 0.0 && (combo || in_air);
            if !stall(self.super_timer) {
                self.super_timer = (self.super_timer - dt).max(0.0);
            }
            if self.super_activation >= 1.0 && self.super_timer <= 0.0 {
                (self.awesome_timer, self.super_activation) = (0.0, 0.0);
            }
            if self.super_activation < 1.0 && !stall(self.awesome_timer) {
                self.awesome_timer = (self.awesome_timer - dt).max(0.0);
            }
            self.boost = if self.awesome_timer > 0.0 {
                if t.unlimited_boost_while_awesome { t.max_boost } else { self.boost }
            } else {
                t.boost_percent_after_awesome * t.max_boost
            };
        }
        let o = self.overrides;
        if o.force_super_awesome_on {
            self.awesome_timer = tricky_len;
            self.super_timer = t.super_awesome_time * r.tricky_bonus * r.tricky_timer_modifier;
            self.super_activation = 1.0;
        } else if o.force_awesome_on {
            self.awesome_timer = tricky_len;
            if o.force_super_awesome_off {
                (self.super_timer, self.super_activation) = (0.0, 0.0);
            }
        } else if o.force_super_awesome_off || track_finished {
            (self.super_timer, self.super_activation) = (0.0, 0.0);
            (self.awesome_timer, self.boost) = (0.0, 0.0);
        } else if o.force_awesome_off {
            (self.awesome_timer, self.boost) = (0.0, 0.0);
        }
        self.is_awesome = self.awesome_timer > 0.0;
        self.is_super_awesome = self.is_awesome && self.super_activation >= 1.0;
    }

    pub fn restore_from_snapshot(&mut self, snap: &ScoreState) {
        let live = *self;
        *self = *snap;
        (self.air_bank, self.rewind_bank, self.grind_bank) = (live.air_bank, live.rewind_bank, live.grind_bank);
        (self.interaction_bank, self.prev_is_scoring_interactions) =
            (live.interaction_bank, live.prev_is_scoring_interactions);
        (self.just_interacting, self.just_stopped_interacting) = (live.just_interacting, live.just_stopped_interacting);
        (self.interaction_triggers, self.interaction_capture) = (live.interaction_triggers, live.interaction_capture);
        (self.prev_is_grinding, self.just_grinding, self.just_stopped_grinding) =
            (live.prev_is_grinding, live.just_grinding, live.just_stopped_grinding);
        (self.grind_spin_fs, self.grind_spin_bs, self.grind_spin_total) =
            (live.grind_spin_fs, live.grind_spin_bs, live.grind_spin_total);
        (self.grind_distance, self.grind_time) = (live.grind_distance, live.grind_time);
        (self.grind_on_end, self.grind_goofy) = (live.grind_on_end, live.grind_goofy);
        (self.combo_points, self.combo_timer, self.combo_pct) = (live.combo_points, live.combo_timer, live.combo_pct);
        self.just_landed = live.just_landed;
        self.just_airborne = live.just_airborne;
        self.just_wipeout = live.just_wipeout;
        self.just_changed_tricks = live.just_changed_tricks;
        self.just_ended_trick = live.just_ended_trick;
        self.just_locked_trick = live.just_locked_trick;
        self.just_finished_track = live.just_finished_track;
        self.just_stopped_rewinding = live.just_stopped_rewinding;
        (self.prev_in_air, self.prev_is_landed, self.prev_wipeout) =
            (live.prev_in_air, live.prev_is_landed, live.prev_wipeout);
        (self.prev_trick_locked, self.prev_trick_ending) = (live.prev_trick_locked, live.prev_trick_ending);
        (self.prev_track_finished, self.prev_is_scoring_rewind) =
            (live.prev_track_finished, live.prev_is_scoring_rewind);
        self.time_since_rewind = live.time_since_rewind;
        self.time_rewinding_backwards = live.time_rewinding_backwards;
        self.is_rewinding_backwards = live.is_rewinding_backwards;
        self.is_scoring_rewind = live.is_scoring_rewind;
        (self.air_time, self.flight_distance, self.trick_time) = (live.air_time, live.flight_distance, live.trick_time);
        (self.record, self.last_record, self.last_pushed) = (live.record, live.last_record, live.last_pushed);
        (self.on_end, self.flip_dir, self.prev_flow_state) = (live.on_end, live.flip_dir, live.prev_flow_state);
        (self.last_capture, self.last_lost) = (live.last_capture, live.last_lost);
        (self.trick_update, self.flushed, self.combo_ended) = (live.trick_update, live.flushed, live.combo_ended);
        (self.base_points, self.track_ended) = (live.base_points, live.track_ended);
    }
}
