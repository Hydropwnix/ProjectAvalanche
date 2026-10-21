use super::*;

impl ScoreState {
    pub(super) fn evaluate_and_award(
        &mut self,
        t: &ScoringTuning,
        r: &ResolvedCharacterProperties,
        i: &ScoringInputs,
        c: &ScoringContext,
        dt: f32,
    ) {
        let expiring = self.combo_timer > 0.0 && self.combo_timer - dt <= 0.0;
        let hold = (expiring && i.in_air) || self.is_scoring_rewind;
        let combo_end = (expiring && !hold) || self.just_finished_track;
        if !hold {
            self.combo_timer -= dt;
        }
        self.combo_timer = self.combo_timer.max(0.0);
        let q = self.combo_timer / self.combo_timer_max;
        let q = if -q >= 0.0 { 0.0 } else { q };
        self.combo_pct = if 1.0 - q >= 0.0 { q } else { 1.0 };
        self.force_capture |= combo_end;
        if !c.track_finished || self.force_capture {
            self.evaluate_rewind_list(c, dt);
            self.evaluate_interaction_list(i, c, dt);
            let capture = combo_end || self.is_scoring_interactions || self.just_changed_tricks;
            self.evaluate_air_list(i, c, capture, dt);
            self.evaluate_grind_list(i, c, combo_end || self.just_changed_tricks, dt);
            self.evaluate_press_list(i, c, combo_end);
            self.update_trick_record(i, c);
        }
        let (captured, any) = self.capture_banks(t, c);
        self.update_flow_level(captured, any, i.in_air, &c.tuning);
        if any && !combo_end {
            self.restart_combo_timer(&c.tuning, false);
        }
        let p = r.trick_point_bonus * captured;
        if self.flow_level >= 0 || p < 0.0 {
            self.combo_points += p;
        }
        let pts = if self.flow_level > 0 || p >= 0.0 { (self.flow_level + 1) as f32 * p } else { p };
        self.add_points(pts, combo_end);
        let wingsuit = c.in_wingsuit || self.just_stopped_wingsuit;
        if c.track_finished && (self.is_awesome || self.is_super_awesome || wingsuit || self.prev_is_boosting) {
            self.track_ended = Some(TrackEnded {
                tricky: self.is_awesome,
                uber_tricky: self.is_super_awesome,
                wingsuit,
                boosting: self.prev_is_boosting,
            });
        }
        self.award = if any { captured } else { 0.0 };
        self.update_boost_and_awesome(t, r, self.award, i.in_air, c.track_finished, dt);
        self.prev_is_boosting = c.is_boosting;
    }

    pub(super) fn evaluate_rewind_list(&mut self, c: &ScoringContext, dt: f32) {
        let on = self.is_rewinding_backwards && self.time_rewinding_backwards > 0.0;
        if on {
            let level = rewind_time_level(self.time_rewinding_backwards);
            self.rewind_bank += -1000.0 * self.tricky(c.rewind_active) * level / 60.0 * dt;
        }
        self.is_scoring_rewind = on;
    }

    pub(super) fn evaluate_interaction_list(&mut self, i: &ScoringInputs, c: &ScoringContext, dt: f32) {
        let heli = i.helicopter_interaction;
        let wipeout = i.lua_wipeout;
        let uphill = c.is_uphill
            && self.time_uphill > UPHILL_TIME_LEVEL[0].0
            && !i.is_grinding
            && !i.is_grind_trick
            && !c.rewind_active;
        let conditions = [heli, false, self.pushed_object && !wipeout, uphill];
        let heli_mult = self.duplicate_or_rewind(c.rewind_active);
        let mut any = false;
        for (k, on) in conditions.into_iter().enumerate() {
            let n = self.interaction_triggers[k];
            if !on {
                self.interaction_triggers[k] = 0;
                continue;
            }
            let p = match k {
                0..=2 if n > 0 => 0.0,
                0 => 20_000.0 * if heli_mult < 1.0 { heli_mult * 0.10 } else { heli_mult },
                1 => 5_000.0,
                2 => 1_000.0,
                _ => -5_000.0 * uphill_time_level(self.time_uphill) / 60.0 * dt,
            };
            self.interaction_triggers[k] = if k == 3 { n + 1 } else { 1 };
            self.interaction_bank += p;
            any = true;
        }
        self.is_scoring_interactions = any;
    }

    pub(super) fn evaluate_air_list(&mut self, i: &ScoringInputs, c: &ScoringContext, capture: bool, dt: f32) {
        let tricky = self.tricky(c.rewind_active);
        let dup = self.duplicate_or_rewind(c.rewind_active);
        let avail = i.in_air && !c.in_wingsuit && self.time_uphill < UPHILL_TIME_NO_AIR_TRICKS;
        let points = self.trick_id.points() * tricky * dup;
        let mut any = false;
        if avail && self.trick_id != TrickType::None && self.trick_time > 15.0 && i.trick_locked && !i.wipeout {
            self.air_bank += points * trick_time_level(self.trick_time) / 60.0 * dt;
            any = true;
        }
        if avail && self.tweak_time > 0.0 {
            self.air_bank += points * 0.10;
            any = true;
        }
        let degs = [self.spin_fs, self.spin_bs, self.flip_fwd, self.flip_bwd].map(f32::to_degrees);
        for (k, deg) in degs.into_iter().enumerate() {
            let on = avail && !i.wipeout && deg >= 180.0;
            let a = &mut self.on_end[k];
            if a.met && (!on || capture) && !a.paid {
                self.air_bank += if k < 2 { spin_points(deg) } else { flip_points(deg) } * dup;
                a.paid = true;
                any = true;
            } else if !on {
                *a = OnEndAsset::default();
            } else {
                a.met = true;
                any = true;
            }
        }
        self.is_scoring_air = any;
    }

    pub(super) fn evaluate_grind_list(&mut self, i: &ScoringInputs, c: &ScoringContext, capture: bool, dt: f32) {
        let tricky = self.tricky(c.rewind_active);
        let dup = self.duplicate_or_rewind(c.rewind_active);
        let terrain = if i.super_grind { 1.0 } else { 1.2 };
        let mut any = false;
        if i.is_grinding && self.trick_id != TrickType::None && self.trick_time > 15.0 {
            let level = grind_trick_time_level(self.trick_time);
            self.grind_bank += self.trick_id.points() * terrain * tricky * dup * level / 60.0 * dt;
            any = true;
        }
        let short = i.is_grinding && self.grind_time < 1200.0;
        let conds = [
            short && self.grind_distance > 20.0,
            short && self.grind_spin_fs.to_degrees() >= 180.0,
            short && self.grind_spin_bs.to_degrees() >= 180.0,
        ];
        for (k, on) in conds.into_iter().enumerate() {
            let a = &mut self.grind_on_end[k];
            if a.met && (!on || capture) && !a.paid {
                self.grind_bank += match k {
                    0 => self.grind_distance * 25.0 * terrain * tricky * dup,
                    1 => grind_spin_points(self.grind_spin_fs.to_degrees()) * terrain * dup,
                    _ => grind_spin_points(self.grind_spin_bs.to_degrees()) * terrain * dup,
                };
                a.paid = true;
                any = true;
            } else if !on {
                *a = OnEndAsset::default();
            } else {
                a.met = true;
                any = true;
            }
        }
        self.is_scoring_grind = any;
    }

    pub(super) fn evaluate_press_list(&mut self, i: &ScoringInputs, c: &ScoringContext, capture: bool) {
        let on = self.is_board_press
            && self.press_distance > BOARDPRESS_MIN_DIST
            && c.player_speed * crate::TICKS_PER_SECOND > BOARDPRESS_MIN_SPEED
            && !i.lua_wipeout
            && !self.is_scoring_air
            && !i.tricking
            && !i.trick_ending
            && !c.is_uphill;
        let a = &mut self.press_on_end;
        let mut any = false;
        if a.met && (!on || capture) && !a.paid {
            self.press_bank += self.press_distance * BOARDPRESS_DIST_POINTS;
            a.paid = true;
            any = true;
        } else if !on {
            *a = OnEndAsset::default();
        } else {
            a.met = true;
            any = true;
        }
        self.is_scoring_board_press = any;
    }

    pub(super) fn update_trick_record(&mut self, i: &ScoringInputs, c: &ScoringContext) {
        let rec = &mut self.record;
        if self.is_scoring_rewind || rec.kind == RecordKind::Rewind {
            rec.kind = RecordKind::Rewind;
            rec.trick_time = self.rewind_bank;
        } else if self.is_scoring_interactions || rec.kind == RecordKind::Interaction {
            rec.kind = RecordKind::Interaction;
            if i.helicopter_interaction {
                rec.interaction = 1;
            } else if self.pushed_object {
                rec.interaction = 2;
            } else if c.is_uphill {
                rec.interaction = 3;
                rec.trick_time = self.interaction_bank;
            }
        } else {
            if self.is_scoring_air || self.is_scoring_grind {
                rec.trick_time = self.trick_time;
                if self.trick_id != TrickType::None {
                    rec.trick_id = self.trick_id;
                    rec.goofy = i.goofy;
                    rec.awesome |= self.is_awesome;
                    rec.super_awesome |= self.is_super_awesome;
                }
            }
            let pressing_record = rec.kind == RecordKind::BoardPress;
            if self.is_scoring_board_press || (self.just_stopped_board_pressing && pressing_record) {
                rec.kind = RecordKind::BoardPress;
                rec.distance = self.press_distance;
                if self.is_board_press {
                    rec.nose_press = if i.goofy { i.press_value > 0.0 } else { i.press_value < 0.0 };
                }
            } else if self.is_scoring_air {
                rec.kind = RecordKind::Air;
                rec.distance = self.flight_distance;
                rec.spin_fs_deg = spin_degrees_rounded(self.spin_fs);
                rec.spin_bs_deg = spin_degrees_rounded(self.spin_bs);
                rec.flips_fwd = flip_count(self.flip_fwd);
                rec.flips_bwd = flip_count(self.flip_bwd);
                rec.tier = trick_time_tier(rec.trick_time, c.tuning.air_trick_tiers);
                rec.tweaked |= i.tweaking;
            } else if self.is_scoring_grind {
                rec.kind = RecordKind::Grind;
                rec.distance = self.grind_distance;
                rec.spin_fs_deg = spin_degrees_rounded(self.grind_spin_fs);
                rec.spin_bs_deg = spin_degrees_rounded(self.grind_spin_bs);
                rec.tier = trick_time_tier(rec.trick_time, c.tuning.grind_trick_tiers);
                rec.tweaked = self.buffered_super_grind > 0;
                rec.grind_group = self.scoring_grind_group;
            }
        }
        if i.trick_locked || self.trick_id == TrickType::None {
            self.duplicate_trick_check();
        }
        self.trick_update = Some(self.record);
    }

    pub(super) fn duplicate_trick_check(&mut self) {
        (self.duplicate, self.record.duplicate) = (false, false);
        let (rec, last) = (&self.record, &self.last_record);
        if !matches!(rec.kind, RecordKind::Air | RecordKind::Grind)
            || last.kind != rec.kind
            || last.trick_id != rec.trick_id
            || rec.trick_id == TrickType::None
            || last.awesome != rec.awesome
            || last.super_awesome != rec.super_awesome
        {
            return;
        }
        if last.goofy != rec.goofy && last.kind == RecordKind::Air && last.trick_id != TrickType::Signature {
            return;
        }
        (self.duplicate, self.record.duplicate) = (true, true);
    }
}
