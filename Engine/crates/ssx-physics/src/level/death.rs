#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GameFlowFlags {
    pub track_finished: bool,
    pub nis_active: bool,
    pub race_proper_started: bool,
    pub force_descent_in_air: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vitals {
    pub health: f32,
    pub temperature: f32,
    pub oxygen: f32,
    pub death_permanent: bool,
    pub in_air: bool,
}

impl Default for Vitals {
    fn default() -> Self {
        Self { health: 1.0, temperature: 1.0, oxygen: 1.0, death_permanent: false, in_air: false }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RiderDead {
    pub by_health: bool,
    pub by_freezing: bool,
    pub by_asphyxia: bool,
    pub by_death_volume: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathState {
    pub trigger: bool,
    pub dead_by_death_plane: bool,
    pub alive: bool,
    pub time_since_death_by_death_plane: f32,
    pub time_since_death: f32,
    pub healthy: bool,
}

impl Default for DeathState {
    fn default() -> Self {
        DeathState {
            trigger: false,
            dead_by_death_plane: false,
            alive: true,
            time_since_death_by_death_plane: 0.0,
            time_since_death: 0.0,
            healthy: true,
        }
    }
}

impl DeathState {
    pub fn update_health(&mut self, track_started: bool, flags: &GameFlowFlags, dt: f32) -> Option<RiderDead> {
        let mut msg = None;
        if self.trigger && track_started && !flags.track_finished && !flags.nis_active {
            self.dead_by_death_plane = true;
            self.trigger = false;
            self.alive = false;
            self.time_since_death_by_death_plane = 0.0;
            msg = Some(RiderDead { by_death_volume: true, ..RiderDead::default() });
        } else {
            self.trigger = false;
        }
        if self.dead_by_death_plane {
            self.time_since_death_by_death_plane += dt;
        }
        msg
    }

    pub fn update_vitals(&mut self, v: &Vitals, dt: f32) -> Option<RiderDead> {
        let mut first = None;
        let mut died = false;
        let causes = [
            (v.health <= 0.0 && v.death_permanent, RiderDead { by_health: true, ..RiderDead::default() }),
            (v.temperature <= 0.0 && !v.in_air, RiderDead { by_freezing: true, ..RiderDead::default() }),
            (v.oxygen <= 0.0 && !v.in_air, RiderDead { by_asphyxia: true, ..RiderDead::default() }),
        ];
        for (holds, msg) in causes {
            if holds {
                self.time_since_death += dt;
                (died, self.healthy, self.alive) = (true, false, false);
                first = first.or(Some(msg));
            }
        }
        if !died {
            self.time_since_death = 0.0;
        }
        first
    }

    pub fn reset(&mut self) {
        *self = DeathState::default();
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }

    pub fn raises_descent(&self) -> bool {
        !self.dead_by_death_plane
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeathTimer {
    pub remaining: f32,
    pub length: f32,
    pub rewind_time: f32,
    pub rewinding: bool,
    pub hint: bool,
}

pub const DEATH_VOLUME_REWIND_WINDOW_S: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathTimerEvent {
    None,
    RewoundOutOfDeath,
    EventEnds,
}

impl Default for DeathTimer {
    fn default() -> Self {
        DeathTimer { remaining: -1.0, length: 0.0, rewind_time: 0.0, rewinding: false, hint: false }
    }
}

impl DeathTimer {
    pub fn on_rider_dead(&mut self, msg: &RiderDead, rewind_enabled: bool, rewinds_left: i32, hint_allowed: bool) {
        if self.remaining >= 0.0 {
            return;
        }
        let t = if msg.by_death_volume { DEATH_VOLUME_REWIND_WINDOW_S } else { 0.0 };
        (self.remaining, self.length) = (t, t);
        if rewind_enabled && rewinds_left > 0 && self.length > 0.0 && hint_allowed {
            self.hint = true;
        }
    }
}

impl DeathTimer {
    pub fn update(&mut self, dt_s: f32, rewinding: bool) -> DeathTimerEvent {
        let mut ev = DeathTimerEvent::None;
        if self.rewinding && !rewinding {
            if self.remaining >= 0.0 {
                self.remaining += self.rewind_time;
                if self.remaining > self.length {
                    self.remaining = -1.0;
                    self.hint = false;
                    ev = DeathTimerEvent::RewoundOutOfDeath;
                }
            }
            self.rewind_time = 0.0;
        }
        if rewinding {
            self.rewind_time += dt_s;
            if self.remaining > 0.0 {
                self.hint = self.rewind_time + self.remaining < self.length;
            }
        }
        self.rewinding = rewinding;
        if !rewinding && self.remaining >= 0.0 {
            self.remaining -= dt_s;
            if self.remaining < 0.0 {
                ev = DeathTimerEvent::EventEnds;
            }
        }
        ev
    }

    pub fn countdown(&mut self, dt_s: f32) -> DeathTimerEvent {
        if self.remaining >= 0.0 {
            self.remaining -= dt_s;
            if self.remaining < 0.0 {
                return DeathTimerEvent::EventEnds;
            }
        }
        DeathTimerEvent::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_death_volume_kills_only_in_gameplay_and_counts_the_time() {
        let mut d = DeathState { trigger: true, ..DeathState::default() };
        let nis = GameFlowFlags { nis_active: true, ..GameFlowFlags::default() };
        assert_eq!(d.update_health(true, &nis, 2.0), None);
        assert!(d.alive && !d.trigger, "outside gameplay the trigger is dropped");
        d.trigger = true;
        assert_eq!(d.update_health(false, &GameFlowFlags::default(), 2.0), None, "the track has not started");
        assert!(d.alive && !d.trigger, "outside gameplay the trigger is dropped");
        d.trigger = true;
        let msg = d.update_health(true, &GameFlowFlags::default(), 2.0).expect("MsgRiderDead");
        assert!(msg.by_death_volume && !d.alive && d.dead_by_death_plane && !d.raises_descent());
        assert_eq!(d.time_since_death_by_death_plane, 2.0, "zeroed, then this update's deltaTicks");
        assert_eq!(d.update_health(true, &GameFlowFlags::default(), 2.0), None);
        assert_eq!(d.time_since_death_by_death_plane, 4.0);
        d.trigger = true;
        assert!(d.update_health(true, &GameFlowFlags::default(), 2.0).is_some());
        assert_eq!(d.time_since_death_by_death_plane, 2.0);
        d.reset();
        assert_eq!(d, DeathState::default());
    }

    #[test]
    fn the_death_timer_ends_the_event_unless_rewound_out_of() {
        let msg = RiderDead { by_death_volume: true, ..RiderDead::default() };
        let mut t = DeathTimer::default();
        t.on_rider_dead(&msg, true, 3, true);
        assert!(t.hint && t.remaining == 4.0);
        let dt = 1.0 / 30.0;
        (0..30).for_each(|_| assert_eq!(t.update(dt, false), DeathTimerEvent::None));
        (0..15).for_each(|_| assert_eq!(t.update(dt, true), DeathTimerEvent::None));
        assert!(t.hint);
        assert_eq!(t.update(dt, false), DeathTimerEvent::None);
        assert!((t.remaining - (3.5 - dt)).abs() < 1e-4, "{}", t.remaining);
        (0..60).for_each(|_| {
            t.update(dt, true);
        });
        assert!(!t.hint, "the hint goes off once the rewind covers the death");
        assert_eq!(t.update(dt, false), DeathTimerEvent::RewoundOutOfDeath);
        assert_eq!(t.remaining, -1.0);
        t.on_rider_dead(&msg, true, 0, true);
        assert!(!t.hint, "no rewinds left: no hint");
        let ends = (0..200).position(|_| t.update(dt, false) == DeathTimerEvent::EventEnds);
        assert!(matches!(ends, Some(119..=120)), "{ends:?}");
        let mut u = DeathTimer::default();
        u.on_rider_dead(&RiderDead { by_health: true, ..RiderDead::default() }, true, 3, true);
        assert!(!u.hint);
        assert_eq!(u.update(dt, false), DeathTimerEvent::EventEnds);
    }
}
