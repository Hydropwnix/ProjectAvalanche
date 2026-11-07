use super::vars::{Vars, id};

pub mod tuning {
    pub const INTRO_DELAY: f32 = 6.0;
    pub const LANDING_DELAY: f32 = 0.5;
    pub const AVALANCHE_SIZE_MASS: f32 = 3.0;
    pub const AVALANCHE_PROBE_MASS: f32 = 2.0;
    pub const LANDING_AIR_TIME: f32 = 2.0;
    pub const COLLISION_IMPACT: f32 = 0.01;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Race,
    Trick,
    Survive,
    SurviveToDeath,
    Other,
}

impl EventKind {
    pub fn from_kind(kind: i32) -> Self {
        match kind {
            1 | 5 => Self::Race,
            2 => Self::Trick,
            3 => Self::Survive,
            4 => Self::SurviveToDeath,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeathCauses {
    pub health: bool,
    pub freezing: bool,
    pub asphyxia: bool,
    pub death_volume: bool,
    pub wingsuit_gap: bool,
    pub wingsuit_deployed: bool,
    pub ridge: bool,
    pub jump_short: bool,
    pub heli_grab: bool,
}

#[derive(Clone, Debug)]
pub struct Trigger {
    pub name: &'static str,
    pub params: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrickMsg {
    pub points_captured: bool,
    pub bad_landing: bool,
    pub air_bank: f32,
    pub grind_bank: f32,
    pub interactions_bank: f32,
}

#[derive(Clone, Copy, Debug)]
struct PendingTrick {
    at: f32,
    msg: TrickMsg,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SpeechFrame {
    pub time: f32,
    pub speed: f32,
    pub distance: f32,
    pub track_length: f32,
    pub boost: f32,
    pub boost_enabled: bool,
    pub awesome: bool,
    pub dead: bool,
    pub tunnel: bool,
    pub rewinding: bool,
    pub vignette_temperature: f32,
    pub health: f32,
    pub vignette_oxygen: f32,
    pub internal_temperature: f32,
    pub in_shade: bool,
    pub airborne: bool,
    pub avalanche: Option<(f32, f32, f32, f32)>,
    pub race_ms: Option<i32>,
    pub points: f32,
    pub flight_distance: f32,
    pub bots: i32,
    pub paused: bool,
}

#[derive(Clone, Debug)]
pub struct SpeechGame {
    pub pilot: i32,
    pub rider: i32,
    pub weather: i32,
    pub kind: EventKind,
    pub tutorial: i32,
    pub previous_death: Option<DeathCauses>,
    pub dd: bool,
    pub dd_hold: bool,
    pub goals: [i32; 2],
    intro_at: Option<f32>,
    landing_at: Option<PendingTrick>,
    flight_param: i32,
    wingsuit_param: i32,
    run_started: bool,
    run_ended: bool,
    finished: bool,
    awesome_since: Option<f32>,
    in_avalanche: bool,
    jump_time: Option<f32>,
    air_time: f32,
}

fn death_mask(code: i32) -> i32 {
    match code {
        0xB => 0x104,
        0xA => 0x204,
        7 => 0x44,
        6 => 0x14,
        9 => 0x24,
        8 => 6,
        5 => 5,
        4 => 0xC,
        _ => 4,
    }
}

impl SpeechGame {
    pub fn new(pilot: i32, rider: i32, weather: i32, kind: EventKind) -> Self {
        Self {
            pilot,
            rider,
            weather,
            kind,
            tutorial: 0,
            previous_death: None,
            dd: false,
            dd_hold: false,
            goals: [0; 2],
            intro_at: None,
            landing_at: None,
            flight_param: 0,
            wingsuit_param: 0,
            run_started: false,
            run_ended: false,
            finished: false,
            awesome_since: None,
            in_avalanche: false,
            jump_time: None,
            air_time: 0.0,
        }
    }

    fn cause_code(&self, c: &DeathCauses) -> i32 {
        if c.death_volume {
            if c.wingsuit_gap {
                if c.wingsuit_deployed { 0xB } else { 0xA }
            } else if c.ridge {
                7
            } else if c.jump_short {
                6
            } else {
                3
            }
        } else if c.asphyxia {
            9
        } else if c.freezing {
            8
        } else if self.in_avalanche {
            5
        } else {
            4
        }
    }

    pub fn event_setup(&mut self, vars: &mut Vars, time: f32, avalanche_probability: f32) {
        vars.set(id::RACE_IS_ACTIVE, 0);
        vars.set(id::RIDER_IS_IN_DEATH_STATE, 0);
        let prev = self.previous_death.map_or(0, |c| match self.cause_code(&c) {
            0xB => 9,
            0xA => 10,
            7 => 7,
            6 => 5,
            3 => 3,
            9 => 6,
            8 => 2,
            5 => 1,
            _ => 4,
        });
        vars.set(id::PREVIOUS_DEATH_REASON, prev);
        let win = match self.kind {
            EventKind::Race => 2,
            EventKind::Trick => 1,
            EventKind::Survive => 3,
            EventKind::SurviveToDeath | EventKind::Other => 4,
        };
        vars.set(id::TRACK_WIN_CONDITION, win);
        let [goal, bronze] = self.goals;
        let (race, trick) = match self.kind {
            EventKind::Race => ([goal, bronze], [0, 0]),
            EventKind::Trick => ([0, 0], [goal, bronze]),
            _ => ([0, 0], [0, 0]),
        };
        vars.set(id::GOAL_RACE_TIME_TO_BEAT, race[0]);
        vars.set(id::GOAL_RACE_MINIMUM_TIME_TO_BEAT, race[1]);
        vars.set(id::GOAL_TRICK_POINTS_TOTAL_TO_BEAT, trick[0]);
        vars.set(id::GOAL_TRICK_MINIMUM_POINTS_TO_BEAT, trick[1]);
        vars.set(id::DROP_AVALANCHE_PROBABILITY, (avalanche_probability * 100.0) as i32);
        self.run_ended = false;
        if !(self.dd && self.dd_hold && self.previous_death.is_none()) {
            self.intro_at = Some(time + tuning::INTRO_DELAY);
        }
    }

    pub fn race_start(&mut self, vars: &mut Vars, time: f32, bots: i32, out: &mut Vec<Trigger>) {
        vars.set(id::BOTS_TOTAL_AT_START, bots);
        vars.set(id::BOTS_STILL_ALIVE_DURING_RUN, bots);
        vars.set(id::RACE_IS_ACTIVE, 1);
        vars.set(id::DEATH_REASON, 0);
        self.finished = false;
        if !self.run_started || self.kind == EventKind::SurviveToDeath {
            self.run_started = true;
            let _ = time;
            if matches!(self.tutorial, 1 | 2) {
                out.push(Trigger { name: "trigger_TUTORIAL", params: vec![1] });
            } else {
                out.push(Trigger { name: "trigger_RUN_START", params: vec![self.pilot, self.rider, self.weather] });
            }
        }
    }

    pub fn death(&mut self, vars: &mut Vars, c: &DeathCauses, out: &mut Vec<Trigger>) {
        vars.set(id::RACE_IS_ACTIVE, 0);
        let reason = if c.death_volume {
            if c.wingsuit_gap {
                if c.wingsuit_deployed { 9 } else { 10 }
            } else if c.ridge {
                7
            } else if c.jump_short {
                5
            } else {
                3
            }
        } else if c.asphyxia {
            6
        } else if c.freezing {
            2
        } else if !c.health {
            0
        } else if vars.raw(id::IS_RIDING_IN_AVALANCHE) == 1 {
            1
        } else {
            4
        };
        vars.set(id::DEATH_REASON, reason);
        let code = self.cause_code(c);
        out.push(Trigger { name: "trigger_DEATH", params: vec![self.pilot, self.rider, 0, death_mask(code)] });
    }

    pub fn crash(&mut self, vars: &mut Vars, start: bool, out: &mut Vec<Trigger>) {
        vars.set(id::RIDER_IS_TUMBLING, i32::from(start));
        let name = if start { "trigger_RIDER_TUMBLING_START" } else { "trigger_RIDER_TUMBLING_END" };
        out.push(Trigger { name, params: Vec::new() });
    }

    pub fn run_end(&mut self, vars: &mut Vars, end: &ssx_rules::results::RunEnd, bots: i32, out: &mut Vec<Trigger>) {
        vars.set(id::RACE_IS_ACTIVE, 0);
        self.finished = true;
        let gold = end.gold || (end.code == 1 && bots >= 1);
        let (silver, bronze, won) = (end.silver, end.bronze, end.won);
        let (result_var, flags) = match self.kind {
            _ if end.failed => (0, 0),
            EventKind::Race => {
                let var = if gold {
                    4
                } else if silver {
                    5
                } else if bronze {
                    6
                } else {
                    3
                };
                let medal_flag = if gold {
                    8
                } else if silver {
                    0x10
                } else if bronze {
                    0x20
                } else {
                    0
                };
                (var, if won { medal_flag | 0x40 } else { 4 })
            }
            EventKind::Trick => (if won { 2 } else { 1 }, if won { 1 } else { 2 }),
            _ => (0, if won { 0x40 } else { 0 }),
        };
        vars.set(id::RUN_COMPLETE_RESULT, result_var);
        if !self.run_ended {
            out.push(Trigger { name: "trigger_RUN_END", params: vec![self.pilot, self.rider, self.weather, flags] });
            self.run_ended = true;
        }
    }
}

impl SpeechGame {
    pub fn frame(
        &mut self,
        vars: &mut Vars,
        f: &SpeechFrame,
        voices_ms: (i32, i32),
        random: i32,
        out: &mut Vec<Trigger>,
    ) {
        if !f.paused {
            self.messages(vars, f);
        }
        vars.set(id::PILOT_VOICE_MILLISECONDS_REMAINING, voices_ms.0);
        vars.set(id::RIDER_VOICE_MILLISECONDS_REMAINING, voices_ms.1);
        vars.set(id::RANDOM_NUMBER, random);
        if !self.finished {
            vars.set(id::BOTS_STILL_ALIVE_DURING_RUN, f.bots);
        }
        if let Some(ms) = f.race_ms.filter(|_| !self.finished) {
            vars.set(id::RIDER_CURRENT_TIME, ms);
        }
        if !f.paused {
            vars.set(id::TRICK_POINTS_TOTAL_FOR_RUN, f.points as i32);
            vars.set(id::RIDER_SCORING_FLIGHT_DISTANCE, f.flight_distance as i32);
            match (f.airborne, self.jump_time) {
                (true, None) => self.jump_time = Some(f.time),
                (false, Some(t)) => {
                    self.air_time = f.time - t;
                    self.jump_time = None;
                }
                _ => {}
            }
        }
        if self.intro_at.is_some_and(|t| f.time >= t) {
            self.intro_at = None;
            out.push(self.intro());
        }
        if let Some(p) = self.landing_at.filter(|p| f.time >= p.at) {
            self.landing_at = None;
            out.extend(self.trick_delayed(&p.msg));
        }
    }

    fn messages(&mut self, vars: &mut Vars, f: &SpeechFrame) {
        let pct = |x: f32| (x * 100.0) as i32;
        vars.set(id::RIDER_IS_REWINDING, i32::from(f.rewinding));
        vars.set(id::RIDER_EFFECT_TEMPERATURE, pct(f.vignette_temperature));
        vars.set(id::RIDER_EFFECT_HEALTH, pct(f.health));
        vars.set(id::RIDER_EFFECT_OXYGEN, pct(f.vignette_oxygen));
        vars.set(id::RIDER_INTERNAL_TEMPERATURE, pct(f.internal_temperature));
        vars.set(id::RIDER_EFFECT_TEMPERATURE_IS_IN_SUNLIGHT, i32::from(f.in_shade));
        let length = f.track_length;
        let (left_pct, left) = if self.finished || length <= 0.0 {
            (0, 0)
        } else {
            let q = (length - f.distance) / length;
            ((f64::from(q) * 100.0) as i32, (f64::from(length) - f64::from(f.distance)) as i32)
        };
        vars.set(id::TRACK_PERCENT_REMAINING, left_pct);
        vars.set(id::TRACK_LENGTH_REMAINING, left);
        let kmh = (f.speed * 3.6) as i32;
        vars.set(id::RIDER_CURRENT_SPEED, kmh);
        let still = if kmh == 0 { (vars.raw(id::RIDER_STANDSTILL_TIME_MS) + 33).clamp(0, 0x7FFF_FF9B) } else { 0 };
        vars.set(id::RIDER_STANDSTILL_TIME_MS, still);
        vars.set(id::RIDER_AVAILABLE_BOOST, f.boost as i32);
        vars.set(id::RIDER_BOOST_ENABLED, i32::from(f.boost_enabled));
        vars.set(id::IS_AWESOME, i32::from(f.awesome));
        let awesome_ms = if f.awesome {
            match self.awesome_since {
                Some(t) => ((f.time - t) * 1000.0) as i32,
                None => {
                    self.awesome_since = Some(f.time);
                    0
                }
            }
        } else {
            self.awesome_since = None;
            0
        };
        vars.set(id::TIME_IN_SUPER_AWESOME, awesome_ms);
        vars.set(id::RIDER_IS_IN_DEATH_STATE, i32::from(f.dead));
        vars.set(id::RIDER_IS_IN_A_TUNNEL, i32::from(f.tunnel));
        if let Some((distance, mass, speed, probe)) = f.avalanche {
            vars.set(id::AVALANCHE_DISTANCE_TO_RIDER, distance as i32);
            vars.set(id::AVALANCHE_SIZE, (mass / tuning::AVALANCHE_SIZE_MASS * 100.0).min(100.0) as i32);
            vars.set(id::AVALANCHE_SPEED, (speed * 3.6) as i32);
            vars.set(id::IS_RIDING_IN_AVALANCHE, i32::from(tuning::AVALANCHE_PROBE_MASS < probe));
            self.in_avalanche = tuning::AVALANCHE_PROBE_MASS < probe;
        }
    }

    fn intro(&self) -> Trigger {
        if matches!(self.tutorial, 1 | 2) {
            return Trigger { name: "trigger_TUTORIAL", params: vec![1] };
        }
        match self.previous_death {
            Some(c) => Trigger {
                name: "trigger_RUN_RESTART",
                params: vec![self.pilot, self.rider, death_mask(self.cause_code(&c))],
            },
            None => Trigger { name: "trigger_INTRO", params: vec![self.pilot, self.rider, self.weather] },
        }
    }

    pub fn trick_message(&mut self, vars: &mut Vars, time: f32, m: &TrickMsg) {
        let points = if m.points_captured { m.air_bank as i32 } else { 0 };
        vars.set(id::TRICK_POINTS_TOTAL_FOR_TRICK, points);
        if self.tutorial == 1 {
            return;
        }
        if time - self.jump_time.unwrap_or(0.0) > tuning::LANDING_AIR_TIME {
            self.wingsuit_param = 0;
            self.flight_param = vars.get(id::RIDER_SCORING_FLIGHT_DISTANCE);
        }
        self.landing_at = Some(PendingTrick { at: time + tuning::LANDING_DELAY, msg: *m });
    }

    fn trick_delayed(&mut self, m: &TrickMsg) -> Option<Trigger> {
        if m.grind_bank > 0.0 {
            return Some(Trigger {
                name: "trigger_TRICK_GRINDING",
                params: vec![self.pilot, self.rider, m.grind_bank as i32],
            });
        }
        if m.interactions_bank != 0.0 || self.air_time <= tuning::LANDING_AIR_TIME {
            return None;
        }
        self.air_time = 0.0;
        Some(Trigger {
            name: "trigger_RIDER_LANDING",
            params: vec![
                self.pilot,
                self.rider,
                0,
                m.air_bank as i32,
                1 + i32::from(m.bad_landing),
                self.flight_param,
                self.wingsuit_param,
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paused_frame_holds_the_messages_but_not_the_clock() {
        let (mut g, mut vars, mut out) = (SpeechGame::new(1, 2, 0, EventKind::Race), Vars::default(), Vec::new());
        g.event_setup(&mut vars, 0.0, 0.0);
        let still = |f: SpeechFrame, g: &mut SpeechGame, vars: &mut Vars, out: &mut Vec<Trigger>| {
            g.frame(vars, &f, (0, 0), 0, out);
            vars.raw(id::RIDER_STANDSTILL_TIME_MS)
        };
        let f = SpeechFrame { time: 1.0, ..SpeechFrame::default() };
        let s1 = still(f, &mut g, &mut vars, &mut out);
        let s2 = still(SpeechFrame { time: 1.033, paused: true, ..f }, &mut g, &mut vars, &mut out);
        assert_eq!(s1, s2, "held while paused");
        assert!(out.is_empty());
        still(SpeechFrame { time: 6.0, paused: true, ..f }, &mut g, &mut vars, &mut out);
        assert!(out.iter().any(|t| t.name == "trigger_INTRO"), "the pending intro on the audio clock");
        let s3 = still(SpeechFrame { time: 6.033, ..f }, &mut g, &mut vars, &mut out);
        assert_eq!(s3, s2 + 33, "the messages again after the pause");
    }

    #[test]
    fn the_goals_go_to_the_pair_of_the_events_kind() {
        let read = |kind| {
            let (mut g, mut vars) = (SpeechGame::new(1, 2, 0, kind), Vars::default());
            g.goals = [93_500, 120_000];
            g.event_setup(&mut vars, 0.0, 0.0);
            [
                id::GOAL_RACE_TIME_TO_BEAT,
                id::GOAL_RACE_MINIMUM_TIME_TO_BEAT,
                id::GOAL_TRICK_POINTS_TOTAL_TO_BEAT,
                id::GOAL_TRICK_MINIMUM_POINTS_TO_BEAT,
            ]
            .map(|v| vars.raw(v))
        };
        assert_eq!(read(EventKind::Race), [93_500, 120_000, 0, 0]);
        assert_eq!(read(EventKind::Trick), [0, 0, 93_500, 120_000]);
        assert_eq!(read(EventKind::Survive), [0; 4]);
    }
}
