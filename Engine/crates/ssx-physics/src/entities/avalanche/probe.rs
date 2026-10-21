use super::*;

pub(super) fn mean_frame(r0: Vec3, r1: Vec3, r2: Vec3) -> Option<Quat> {
    let l = r0.length();
    if l <= EPS {
        return None;
    }
    let x = r0 / l;
    let z = x.cross(r1);
    let z = if z.length() > EPS { z.normalize() } else { norm_or_zero(r2) };
    let y = z.cross(x);
    Some(Quat::from_mat3(&glam::Mat3::from_cols(x, y, z)).normalize())
}

impl AvalancheSim {
    pub(super) fn measure(
        &self,
        centre: Vec3,
        falloff: &Curve,
        lifetime: &Curve,
        gain: f32,
        old: Measure,
        dt: f32,
    ) -> Measure {
        let weight = |p: &Particle| p.scale * falloff.eval((p.pos - centre).length()) * lifetime.eval(p.age);
        let (mut m, mut r0, mut r1, mut r2) = (0.0f32, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        for p in self.particles() {
            let w = weight(p);
            m += w;
            r0 += p.frame * Vec3::X * w;
            r1 += p.frame * Vec3::Y * w;
            r2 += p.frame * Vec3::Z * w;
        }
        let e = decay(gain, dt);
        let mut out = old;
        if m <= EPS {
            out = Measure { span: old.span, ..Measure::default() };
        } else {
            let inv = 1.0 / m;
            let (r0, r1, r2) = (r0 * inv, r1 * inv, r2 * inv);
            let speed = r0.length();
            let target = mean_frame(r0, r1, r2).unwrap_or(old.orientation);
            if old.mass.abs() <= EPS {
                (out.orientation, out.mass, out.speed) = (target, m, speed);
            } else {
                let o = if target.dot(old.orientation) > 0.0 { old.orientation } else { -old.orientation };
                out.orientation = (target + (o - target) * e).normalize();
                out.speed = e * (old.speed - speed) + speed;
                out.mass = e * (old.mass - m) + m;
            }
        }
        let (mut ms, mut span) = (0.0f32, Vec3::ZERO);
        let inv_q = out.orientation.conjugate();
        for p in self.particles() {
            let w = weight(p);
            ms += w;
            span += (inv_q * (p.pos - centre)).abs() * w;
        }
        out.span = if ms <= EPS {
            Vec3::ZERO
        } else if out.mass.abs() <= EPS {
            span / ms
        } else {
            e * (old.span - span / ms) + span / ms
        };
        out
    }

    pub fn gather(
        &mut self,
        t: &AvalancheTuning,
        probe_pos: Vec3,
        board_pos: Vec3,
        board_z: Vec3,
        sa: Option<&SurfaceAnalysis>,
    ) {
        let i = self.inputs;
        let dt = i.dt;
        if dt.abs() <= EPS {
            return;
        }
        let pt = t.particles(i.switches.tuning_set).clone();
        for (k, p) in self.sloughs.iter_mut().chain(self.slabs.iter_mut()).enumerate() {
            if p.alive() && p.flags & FLAG_SIMULATED != 0 && p.flags & FLAG_FROZEN == 0 {
                p.contact = self.hits[k];
            }
        }
        let culling_sq = t.culling_distance * t.culling_distance;
        for kind in [Kind::Slough, Kind::Slab] {
            let tr = if kind == Kind::Slough { pt.slough } else { pt.slab };
            for k in 0..if kind == Kind::Slough { SLOUGH_RECORDS } else { SLAB_RECORDS } {
                let mut p = *self.get_mut(kind, k);
                if p.alive() {
                    update_record(&mut p, &tr, t.surface_epsilon, &mut self.rng, i.pos, culling_sq, dt);
                    *self.get_mut(kind, k) = p;
                }
            }
        }
        let v_dir = norm_or_zero(i.vel);
        let tip = board_pos + board_z * (v_dir.dot(board_z) * t.board_half_length);
        self.tip_prev = if i.warped { tip } else { self.tip };
        self.tip = tip;
        self.emit_spray(&pt, dt);
        self.carve_slabs(&pt, dt);
        if self.landing_impact.abs() > EPS {
            self.impact_slabs(&pt, sa);
        }
        self.decay_slabs(&pt, dt);
        if i.big_landing {
            self.splosion_sloughs(&pt, sa);
            self.splosion(&pt);
        }
        let (sum, w) = self.particles().fold((Vec3::ZERO, 0.0f32), |(s, w), p| (s + p.pos * p.scale, w + p.scale));
        self.aggregate_pos = if w.abs() <= EPS { Vec3::ZERO } else { sum / w };
        let (c, f, l, g) = (self.aggregate_pos, &t.aggregate_falloff, &t.aggregate_lifetime, t.aggregate_gain);
        self.aggregate = self.measure(c, f, l, g, self.aggregate, dt);
        let (f, l, g) = (&t.probe_falloff, &t.probe_lifetime, t.probe_gain);
        self.probe = self.measure(probe_pos, f, l, g, self.probe, dt);
    }

    pub fn published(&self, t: &AvalancheTuning) -> Published {
        let mm = t.particles(self.inputs.switches.tuning_set).mass_multiplier;
        Published {
            aggregate_pos: self.aggregate_pos,
            aggregate: Measure { mass: self.aggregate.mass * mm, ..self.aggregate },
            probe: Measure { mass: self.probe.mass * mm, ..self.probe },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Published {
    pub aggregate_pos: Vec3,
    pub aggregate: Measure,
    pub probe: Measure,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AvalancheFrame {
    pub enabled: bool,
    pub vel: Vec3,
    pub mass: f32,
    pub ramp: f32,
    pub value: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RiderAvalanche {
    pub published: Published,
    pub switches: AvalancheSwitches,
    pub ride_in_ticks: f32,
    pub ride_not_in_ticks: f32,
    pub prev_tumble_value: f32,
    pub delay_reentry_timer: f32,
    pub bailed_because_of_avalanche: bool,
    pub damage_running_total: f32,
    pub frame: AvalancheFrame,
}

impl RiderAvalanche {
    pub fn read_inputs(&mut self, t: &CarvingAvalancheTuning, enabled: bool, ride_ticks: f32) -> AvalancheFrame {
        let p = self.published.probe;
        let vel = (p.orientation * Vec3::X) * p.speed;
        let value = if enabled { vel.length() * p.mass } else { 0.0 };
        self.frame = AvalancheFrame { enabled, vel, mass: p.mass, ramp: t.effect_ramp_up.eval(ride_ticks), value };
        self.frame
    }

    pub fn drag_factor(t: &CarvingAvalancheTuning, f: &AvalancheFrame, alive: bool) -> f32 {
        if f.enabled && alive { t.drag_modifier.eval(f.mass) } else { 1.0 }
    }

    pub fn wind(&self, t: &CarvingAvalancheTuning, f: &AvalancheFrame, alive: bool, air_time: f32, n: Vec3) -> Vec3 {
        let mut k =
            if f.enabled && alive { t.wind_decay.eval(self.ride_in_ticks) * t.wind_modifier.eval(f.mass) } else { 0.0 };
        if air_time > t.min_air_time_for_ramp_up {
            k *= f.ramp;
        }
        (f.vel - n * f.vel.dot(n)) * k
    }

    pub fn friction_factor(&self, t: &CarvingAvalancheTuning, f: &AvalancheFrame, alive: bool, air_time: f32) -> f32 {
        let mut fm = if f.enabled && alive { t.friction_modifier.eval(f.value) } else { 1.0 };
        if air_time > t.min_air_time_for_ramp_up {
            fm = 1.0 + (fm - 1.0) * f.ramp;
        }
        1.0 + (fm - 1.0) * t.friction_ramp_up.eval(self.ride_in_ticks)
    }

    pub fn balance(
        &mut self,
        t: &CarvingAvalancheTuning,
        bail_on_landing: &mut bool,
        level_complete: bool,
        recovery: bool,
    ) -> f32 {
        let a = self.frame.value;
        let mut d = t.damage.eval(a);
        if *bail_on_landing {
            d *= t.tumble_damage_multiplier;
        }
        self.damage_running_total += d;
        let carving_damage = t.damage_multiplier * d;
        if level_complete {
            *bail_on_landing = false;
            return carving_damage;
        }
        if !*bail_on_landing
            && a > t.tumble_enter
            && self.delay_reentry_timer < 0.0
            && a > t.reentry_multiplier.eval(self.ride_not_in_ticks) * self.prev_tumble_value
        {
            *bail_on_landing = true;
            self.bailed_because_of_avalanche = true;
            return carving_damage;
        }
        let exit = *bail_on_landing
            && self.bailed_because_of_avalanche
            && (t.tumble_exit_decay.eval(self.ride_in_ticks) > a || (recovery && t.tumble_enter > a));
        if exit {
            *bail_on_landing = false;
            self.prev_tumble_value = a;
            self.delay_reentry_timer = t.delay_reentry;
        }
        carving_damage
    }

    pub fn end_of_update(&mut self, bail_on_landing: bool, dt: f32) {
        self.delay_reentry_timer -= dt;
        if bail_on_landing && self.bailed_because_of_avalanche {
            (self.ride_in_ticks, self.ride_not_in_ticks) = (self.ride_in_ticks + dt, 0.0);
        } else {
            (self.ride_not_in_ticks, self.ride_in_ticks) = (self.ride_not_in_ticks + dt, 0.0);
        }
    }
}

impl RiderAvalanche {
    pub fn fresh_controller(&mut self) {
        (self.ride_in_ticks, self.ride_not_in_ticks, self.prev_tumble_value, self.delay_reentry_timer) =
            (0.0, 0.0, 0.0, 0.0);
        self.bailed_because_of_avalanche = false;
    }
}
