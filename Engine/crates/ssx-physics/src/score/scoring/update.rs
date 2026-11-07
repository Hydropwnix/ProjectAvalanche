use super::*;

impl ScoreState {
    pub(super) fn reset_from_defaults(&mut self, rewound_back: bool, reset: bool) {
        self.is_big_landing = false;
        let force_reset = std::mem::take(&mut self.overrides.force_reset);
        if reset {
            let keep = *self;
            *self = Self {
                descent: keep.descent,
                prev_is_landed: keep.prev_is_landed,
                prev_trick_ending: keep.prev_trick_ending,
                just_ended_trick: keep.just_ended_trick,
                just_locked_trick: keep.just_locked_trick,
                just_finished_track: keep.just_finished_track,
                is_rewinding_backwards: keep.is_rewinding_backwards,
                time_rewinding_backwards: keep.time_rewinding_backwards,
                force_capture: keep.force_capture,
                overrides: self.overrides,
                prev_flow_state: keep.flow_state,
                ..Self::default()
            };
        } else if rewound_back || force_reset {
            self.flow_level = 0;
            self.flow_state_timer = 0.0;
            self.is_scoring_air = false;
            (self.is_scoring_board_press, self.is_scoring_interactions) = (false, false);
            (self.is_scoring_grind, self.scoring_grind_group) = (false, 0);
            (self.spin_fs, self.spin_bs, self.flip_fwd, self.flip_bwd) = (0.0, 0.0, 0.0, 0.0);
            (self.duplicate, self.trick_id, self.tweak_time, self.wipeout) = (false, TrickType::None, 0.0, false);
            (self.combo_count, self.combo_timer_max) = (0, 0.0);
        }
    }

    pub(super) fn edges_and_accumulators(&mut self, i: &ScoringInputs, c: &ScoringContext, dt: f32) {
        let active = c.rewind_active;
        let rw = active && c.rewind_speed <= SPEED_EPS;
        if !active && self.time_rewinding_backwards <= 0.0 {
            self.time_since_rewind += dt;
        } else {
            self.time_since_rewind = 0.0;
        }
        let in_air = i.in_air;
        self.wipeout = i.wipeout;
        self.buffered_super_grind = if i.super_grind { 5 } else { (self.buffered_super_grind - 1).max(0) };
        self.just_grinding = !self.prev_is_grinding && i.is_grinding;
        self.just_stopped_grinding = self.prev_is_grinding && !i.is_grinding && !i.is_grind_trick;
        let is_landed = !in_air && (!c.on_vertical_surface_for_scoring || i.wipeout);
        self.is_board_press = i.is_board_press
            && !(in_air || i.tricking || i.is_grind_trick || self.is_scoring_air || self.is_scoring_grind);
        self.just_board_pressing = !self.prev_is_board_pressing && self.is_board_press;
        self.just_stopped_board_pressing = self.prev_is_board_pressing && !self.is_board_press;
        self.just_interacting = !self.prev_is_scoring_interactions && self.is_scoring_interactions;
        self.just_stopped_interacting = self.prev_is_scoring_interactions && !self.is_scoring_interactions;
        self.just_landed = is_landed && !self.prev_is_landed;
        self.just_airborne = in_air && !self.prev_in_air;
        self.just_wipeout = i.wipeout && !self.prev_wipeout;
        self.just_locked_trick = i.trick_locked && !self.prev_trick_locked;
        self.just_ended_trick = self.prev_trick_ending && !i.trick_ending;
        self.just_finished_track = c.track_finished && !self.prev_track_finished;
        self.just_stopped_wingsuit = self.prev_in_wingsuit && !c.in_wingsuit;
        self.prev_in_wingsuit = c.in_wingsuit;
        self.just_stopped_rewinding = c.rewind_terminating;
        self.has_touched_ground |= !in_air;
        (self.prev_in_air, self.prev_is_landed, self.prev_wipeout) = (in_air, is_landed, i.wipeout);
        (self.prev_trick_locked, self.prev_trick_ending) = (i.trick_locked, i.trick_ending);
        (self.prev_track_finished, self.prev_is_scoring_rewind) = (c.track_finished, self.is_scoring_rewind);
        self.prev_is_grinding = i.is_grinding || i.is_grind_trick;
        self.prev_is_board_pressing = self.is_board_press;
        self.prev_is_scoring_interactions = self.is_scoring_interactions;
        if self.just_airborne || self.just_changed_tricks {
            (self.spin_fs, self.spin_bs, self.flip_fwd, self.flip_bwd) = (0.0, 0.0, 0.0, 0.0);
            (self.air_time, self.flight_distance, self.trick_time) = (0.0, 0.0, 0.0);
            (self.trick_id, self.flip_dir) = (TrickType::None, 0);
        }
        if self.just_grinding || self.just_changed_tricks || rw {
            (self.grind_time, self.grind_distance) = (0.0, 0.0);
            (self.grind_spin_fs, self.grind_spin_bs, self.grind_spin_total) = (0.0, 0.0, 0.0);
            (self.trick_time, self.trick_id, self.scoring_grind_group) = (0.0, TrickType::None, -1);
            self.grind_goofy = i.goofy;
        }
        if self.just_board_pressing || self.is_scoring_grind {
            (self.press_time, self.press_distance) = (0.0, 0.0);
            if self.just_board_pressing {
                self.board_press_count += 1;
            }
        }
        let mut restart_combo = false;
        let rec = self.record;
        if !self.just_ended_trick && (i.tricking || i.trick_ending) {
            let t = i.trick_type;
            let rec_has_trick = rec.trick_id != TrickType::None;
            let grind = rec.kind == RecordKind::Grind;
            self.just_changed_tricks = t != TrickType::None
                && t != TrickType::Signature
                && !self.just_locked_trick
                && t != rec.trick_id
                && if grind { !rec_has_trick } else { rec_has_trick };
            if !self.just_changed_tricks && !i.trick_ending && (!grind || i.grind_group == rec.grind_group) {
                self.trick_id = t;
                self.scoring_grind_group = i.grind_group;
            }
            if !rw {
                self.trick_time += dt;
            }
            restart_combo = true;
            self.board_press_count = 0;
        } else {
            self.trick_id = TrickType::None;
            self.scoring_grind_group = -1;
            self.just_changed_tricks = self.just_ended_trick && rec.kind == RecordKind::Grind;
        }
        self.tweak_time = if i.tweaking && !rw { self.tweak_time + dt } else { 0.0 };
        if in_air && !c.in_wingsuit && !i.wipeout {
            if !active {
                self.air_time += dt;
                self.flight_distance += (c.position - c.prev_position).length();
            }
            if !rw {
                self.accumulate_rotation(i, dt);
            }
            if i.spin_speed.abs() > COMBO_ROTATION_SPEED || i.flip_speed.abs() > COMBO_ROTATION_SPEED {
                restart_combo = true;
                self.board_press_count = 0;
            }
        }
        if i.is_grinding && !rw {
            let v = i.grind_spin_vel;
            if v != 0.0 {
                if (v > 0.0) != self.grind_goofy {
                    self.grind_spin_fs += v.abs() * dt;
                } else {
                    self.grind_spin_bs += v.abs() * dt;
                }
            }
            self.grind_spin_total = self.grind_spin_fs.abs() + self.grind_spin_bs.abs();
            self.grind_time += dt;
            self.grind_distance += (c.position - c.prev_position).length();
            restart_combo = true;
            self.board_press_count = 0;
        }
        if self.is_scoring_interactions {
            self.pushed_object = false;
            self.board_press_count = 0;
            restart_combo = true;
        }
        self.is_rewinding_backwards = rw;
        if rw {
            self.time_rewinding_backwards += dt;
        } else {
            self.time_rewinding_backwards -= dt;
            if self.time_rewinding_backwards < 0.0 {
                self.time_since_rewind = self.time_since_rewind.max(dt);
                self.time_rewinding_backwards = 0.0;
            }
        }
        if c.is_uphill && !in_air {
            self.time_uphill += dt;
            restart_combo = true;
        } else if !in_air {
            self.time_uphill = 0.0;
        }
        if self.is_board_press && !in_air && !i.tricking && !rw {
            self.press_time += dt;
            self.press_distance += (c.position - c.prev_position).length();
            self.restart_combo_timer(&c.tuning, true);
        }
        if restart_combo {
            self.restart_combo_timer(&c.tuning, false);
        }
    }

    pub(super) fn accumulate_rotation(&mut self, i: &ScoringInputs, dt: f32) {
        let s = i.spin_speed;
        if (s > 0.0) != i.goofy && s != 0.0 {
            self.spin_bs += s.abs() * dt;
        } else {
            self.spin_fs += s.abs() * dt;
        }
        let f = i.flip_speed.abs() * dt;
        if i.flip_accel.abs() <= SPEED_EPS {
            match self.flip_dir {
                1 => self.flip_fwd += f,
                2 => self.flip_bwd += f,
                _ => {}
            }
        } else if i.flip_accel > 0.0 {
            (self.flip_fwd, self.flip_dir) = (self.flip_fwd + f, 1);
        } else {
            (self.flip_bwd, self.flip_dir) = (self.flip_bwd + f, 2);
        }
    }

    pub(super) fn update_flow_state(&mut self, i: &ScoringInputs, c: &ScoringContext, dt: f32) {
        let k = &c.tuning;
        let rw2 = c.rewind_active || c.rewind_terminating;
        let fast = c.player_speed >= k.flow_min_speed;
        let descending = c.velocity_y <= 0.0 && c.position.y <= c.level_start_y;
        let in_air = i.in_air;
        if (fast && !i.is_grinding) || in_air {
            self.trailing_descent += k.descent_rate * dt;
        }
        self.trailing_descent = self.trailing_descent.max(self.descent - k.descent_trail_distance);
        let desc_fast = self.descent > self.trailing_descent || in_air;
        if !rw2 {
            if !descending {
                self.flow_uphill_timer += dt;
            }
            if !fast {
                self.flow_too_slow_timer += dt;
            }
            if fast && descending {
                self.flow_downhill_timer += dt;
            } else if !fast {
                self.flow_downhill_timer = 0.0;
            }
        } else {
            self.flow_downhill_timer = 0.0;
        }
        let old = self.flow_state;
        let mut new = old;
        match old {
            FlowState::None => {
                if descending && desc_fast && fast && !self.wipeout {
                    new = FlowState::Positive;
                    (self.flow_uphill_timer, self.flow_too_slow_timer) = (0.0, 0.0);
                } else if !in_air && !descending && self.flow_uphill_timer >= k.time_uphill_for_neg_flow {
                    new = FlowState::Negative;
                    self.flow_downhill_timer = 0.0;
                }
            }
            FlowState::Positive => {
                if self.wipeout
                    || self.flow_too_slow_timer >= k.time_too_slow_for_no_flow
                    || self.flow_uphill_timer >= k.time_uphill_for_no_flow
                    || !desc_fast
                {
                    new = FlowState::None;
                } else if fast && descending {
                    (self.flow_uphill_timer, self.flow_too_slow_timer) = (0.0, 0.0);
                }
            }
            FlowState::Negative => {
                if self.flow_downhill_timer >= k.time_downhill_for_flow && !in_air && !self.just_landed && !rw2 {
                    new = FlowState::None;
                    (self.flow_uphill_timer, self.flow_downhill_timer) = (0.0, 0.0);
                }
            }
        }
        if new != old {
            self.flow_state_timer = 0.0;
        }
        (self.prev_flow_state, self.flow_state) = (old, new);
    }

    pub(super) fn tricky(&self, rewind_active: bool) -> f32 {
        if self.time_since_rewind > TRICKY_AFTER_REWIND_TICKS && !rewind_active {
            if self.is_super_awesome {
                return SUPER_TRICKY_MULTIPLIER;
            } else if self.is_awesome {
                return TRICKY_MULTIPLIER;
            }
        }
        1.0
    }

    pub(super) fn duplicate_or_rewind(&self, rewind_active: bool) -> f32 {
        if rewind_active || self.time_rewinding_backwards > 0.0 {
            REWIND_MULTIPLIER
        } else if self.duplicate {
            DUPLICATE_MULTIPLIER
        } else if self.time_since_rewind > 0.0 {
            rewind_ramp(self.time_since_rewind)
        } else {
            1.0
        }
    }
}
