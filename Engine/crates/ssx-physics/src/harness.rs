use glam::Vec3;

use crate::rewind::{RewindPad, RewindState, RewindStep};
use crate::rider::{Input, RiderState, TrickInput, World};
use crate::session::Session;

const FLAG_IN_AIR: u8 = 0x08;
const FLAG_WINGSUIT: u8 = 0x40;

#[derive(Clone, Debug)]
pub struct CourseLine {
    pub id: u32,
    pub points: Vec<Vec3>,
    pub along: Vec<f32>,
    pub speed: Vec<f32>,
    pub vy: Vec<f32>,
    pub in_air: Vec<bool>,
    pub wingsuit: Vec<bool>,
    pub jumps: Vec<f32>,
}

impl CourseLine {
    pub fn from_replay(r: &ssx_assets::ghosts::GhostReplay) -> Self {
        let all: Vec<Vec3> = r.poses.iter().map(|p| Vec3::from_array(p.trajectory_position)).collect();
        let keep = forward_samples(&all);
        let points: Vec<Vec3> = keep.iter().map(|&i| all[i]).collect();
        let mut along = Vec::with_capacity(points.len());
        let mut d = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                d += p.distance(points[i - 1]);
            }
            along.push(d);
        }
        let poses: Vec<&ssx_assets::ghosts::GhostPose> = keep.iter().map(|&i| &r.poses[i]).collect();
        let mut line = CourseLine {
            jumps: Vec::new(),
            id: r.id,
            along,
            speed: poses.iter().map(|p| Vec3::from_array(p.velocity).length()).collect(),
            vy: poses.iter().map(|p| p.velocity[1]).collect(),
            in_air: poses.iter().map(|p| p.flags & FLAG_IN_AIR != 0).collect(),
            wingsuit: poses.iter().map(|p| p.flags & FLAG_WINGSUIT != 0).collect(),
            points,
        };
        line.jumps = line.find_jumps();
        line
    }

    pub fn nearest_disc(big_root: &std::path::Path, start: Vec3, end: Vec3) -> anyhow::Result<(Self, f32, f32)> {
        let csv = std::fs::read_to_string(big_root.join("bootcache/data/common/ghosts/ghostdb.csv"))?;
        let mut ids: Vec<u32> = crate::event::parse_ghost_db(&csv).iter().map(|(_, r)| r.ghost_id as u32).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut best: Option<((u32, f32), Self, f32, f32)> = None;
        for id in ids {
            let Ok(r) = ssx_assets::ghosts::GhostReplay::load(big_root, id) else { continue };
            let line = Self::from_replay(&r);
            let Some((is, ds)) = line.nearest_in(start, 0..line.points.len()) else { continue };
            let Some((_, de)) = line.nearest_in(end, is..line.points.len()) else { continue };
            if ds > 80.0 || de > 150.0 {
                continue;
            }
            let key = (r.header.build, -(ds + de));
            if best.as_ref().is_some_and(|b| (b.0.0, b.0.1) >= key) {
                continue;
            }
            best = Some((key, line, ds, de));
        }
        best.map(|(_, l, ds, de)| (l, ds, de)).ok_or_else(|| anyhow::anyhow!("no disc replay passes the level start"))
    }

    fn nearest_in(&self, p: Vec3, range: std::ops::Range<usize>) -> Option<(usize, f32)> {
        range
            .map(|i| (i, self.points[i].distance_squared(p)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, d)| (i, d.sqrt()))
    }

    fn at(&self, d: f32) -> (Vec3, usize) {
        let n = self.points.len();
        let k = self.along.partition_point(|&a| a < d).min(n - 1);
        if k == 0 {
            return (self.points[0], 0);
        }
        let (a0, a1) = (self.along[k - 1], self.along[k]);
        let t = if a1 > a0 { ((d - a0) / (a1 - a0)).clamp(0.0, 1.0) } else { 1.0 };
        let (p0, p1, p2, p3) =
            (self.points[k.saturating_sub(2)], self.points[k - 1], self.points[k], self.points[(k + 1).min(n - 1)]);
        let (t2, t3) = (t * t, t * t * t);
        let q = 0.5
            * (2.0 * p1
                + (p2 - p0) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3);
        (q, k)
    }

    fn project(&self, p: Vec3, range: std::ops::Range<usize>) -> Option<(usize, f32, f32)> {
        let n = self.points.len();
        range
            .filter(|&i| i + 1 < n)
            .map(|i| {
                let (a, b) = (self.points[i], self.points[i + 1]);
                let ab = b - a;
                let l2 = ab.length_squared();
                let t = if l2 > 1e-6 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
                (i, t, (a + ab * t).distance(p))
            })
            .min_by(|x, y| x.2.total_cmp(&y.2))
    }

    fn along_at(&self, i: usize, t: f32) -> f32 {
        let next = self.along.get(i + 1).copied().unwrap_or(self.along[i]);
        self.along[i] + (next - self.along[i]) * t
    }

    fn find_jumps(&self) -> Vec<f32> {
        (0..self.points.len().saturating_sub(1))
            .filter(|&i| !self.in_air[i] && self.in_air[i + 1] && self.vy[i + 1] - self.vy[i] > JUMP_VY)
            .map(|i| 0.5 * (self.along[i] + self.along[i + 1]))
            .collect()
    }
}

fn forward_samples(p: &[Vec3]) -> Vec<usize> {
    let mut keep: Vec<usize> = Vec::with_capacity(p.len());
    let mut along: Vec<f32> = Vec::with_capacity(p.len());
    for (i, &q) in p.iter().enumerate() {
        if let Some(&last) = keep.last() {
            let step = q.distance(p[last]);
            if step > WARP_M {
                break;
            }
            let end = along.last().copied().unwrap_or(0.0);
            let back =
                (0..keep.len().saturating_sub(1)).rev().filter(|&j| along[j + 1] < end - RETRACE_BACK_M).find(|&j| {
                    let (a, b) = (p[keep[j]], p[keep[j + 1]]);
                    let ab = b - a;
                    let t = if ab.length_squared() > 1e-6 {
                        ((q - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    (a + ab * t).distance(q) < RETRACE_M
                });
            if let Some(j) = back {
                keep.truncate(j + 1);
                along.truncate(j + 1);
            }
            let end = along.last().copied().unwrap_or(0.0);
            along.push(end + q.distance(p[*keep.last().unwrap_or(&i)]));
        } else {
            along.push(0.0);
        }
        keep.push(i);
    }
    keep
}

const RETRACE_M: f32 = 3.0;
const RETRACE_BACK_M: f32 = 10.0;
const WARP_M: f32 = 200.0;

const JUMP_VY: f32 = 0.08;
const JUMP_HOLD_S: f32 = 0.3;

#[derive(Clone, Copy, Debug, Default)]
pub struct CourseControls {
    pub input: Input,
    pub tricks: TrickInput,
    pub r1: bool,
    pub rewind: RewindPad,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub offset: f32,
    pub speed: Speed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Follow,
    Tuck,
    Coast,
    Brake,
}

impl Default for Style {
    fn default() -> Self {
        Style { offset: 0.0, speed: Speed::Follow }
    }
}

const OFFSETS: [f32; 5] = [0.0, -4.0, 4.0, -8.0, 8.0];
const SPEEDS: [Speed; 4] = [Speed::Follow, Speed::Coast, Speed::Brake, Speed::Tuck];

fn oxygen_r1(s: &RiderState, on: &mut bool) -> bool {
    let v = &s.survival;
    if !v.low_oxygen || v.oxygen_tank <= 0.0 || v.oxygen >= OXYGEN_OFF {
        *on = false;
    } else if v.oxygen < OXYGEN_ON {
        *on = true;
    }
    *on && !s.in_air()
}

const PILOT_HEIGHT_GAIN: f32 = 0.1;
const PILOT_SPEED_GAIN: f32 = 0.02;

const GRIND_OFF_M: f32 = 6.0;

const OXYGEN_ON: f32 = 0.35;
const OXYGEN_OFF: f32 = 0.95;

fn policy(
    line: &CourseLine,
    cursor: &mut usize,
    oxygen: &mut bool,
    s: &RiderState,
    style: Style,
    flick: f32,
) -> (CourseControls, usize, f32, f32) {
    let mut c = CourseControls::default();
    let (k, along, off) = locate(line, cursor, s.traj);
    let speed = s.speed();
    let mps = speed * crate::TICKS_PER_SECOND;
    let lookahead = if s.state.is_wingsuit() { (mps * 0.8).clamp(15.0, 50.0) } else { (mps * 0.45).clamp(6.0, 25.0) };
    let (mut target, tk) = line.at(along + lookahead);
    if style.offset != 0.0 {
        let dir = (target - line.points[k]).with_y(0.0).normalize_or_zero();
        target += dir.cross(Vec3::Y) * style.offset;
    }
    let steer = steer_towards(target - s.traj, s.velocity, s.heading);
    let flying_here = (k..(tk + 1).min(line.points.len())).any(|i| line.wingsuit[i]);
    c.r1 = s.in_air() && (flying_here || (s.state.is_wingsuit() && line.wingsuit[k]));
    if s.state.is_wingsuit() {
        let (here, _) = line.at(along);
        let dy = here.y - s.traj.y;
        let dv = (speed - line.speed[k]) * crate::KMH_PER_M_PER_TICK;
        let bias = match style.speed {
            Speed::Follow => 0.0,
            Speed::Tuck => 0.5,
            Speed::Coast => -0.25,
            Speed::Brake => -0.5,
        };
        let pitch = (-PILOT_HEIGHT_GAIN * dy - PILOT_SPEED_GAIN * dv + bias).clamp(-1.0, 1.0);
        (c.input.steer, c.input.tuck, c.input.brake) = (steer, pitch.max(0.0), (-pitch).max(0.0));
        return (c, k, along, off);
    }
    if s.in_air() {
        return (c, k, along, off);
    }
    c.r1 = oxygen_r1(s, oxygen);
    if s.stopped {
        c.input.tuck = flick;
        return (c, k, along, off);
    }
    c.input.steer = steer;
    let rec = line.speed[k];
    match style.speed {
        Speed::Follow if speed < rec * 1.05 || speed < 0.1 => c.input.tuck = 1.0,
        Speed::Follow if speed > rec * 1.4 + 0.1 => c.input.brake = 0.4,
        Speed::Follow | Speed::Coast => {}
        Speed::Tuck => c.input.tuck = 1.0,
        Speed::Brake => c.input.brake = 0.6,
    }
    c.input.jump = line.jumps.iter().any(|&j| j > along && j - along < mps * JUMP_HOLD_S);
    if s.grind.is_grinding() && (line.in_air[k] || off > GRIND_OFF_M) {
        c.input.jump = flick > 0.5;
    }
    (c, k, along, off)
}

fn locate(line: &CourseLine, cursor: &mut usize, p: Vec3) -> (usize, f32, f32) {
    let n = line.points.len();
    let lo = cursor.saturating_sub(10);
    let (mut i, mut t, mut d) = line.project(p, lo..(*cursor + 60).min(n)).unwrap_or((*cursor, 0.0, f32::MAX));
    if d > 30.0
        && let Some(x) = line.project(p, 0..n)
    {
        (i, t, d) = x;
    }
    *cursor = i;
    let k = if t < 0.5 { i } else { (i + 1).min(n - 1) };
    (k, line.along_at(i, t), d)
}

#[derive(Clone, Debug)]
pub struct CoursePlayer {
    pub line: CourseLine,
    cursor: usize,
    style: Style,
    style_in_flight: bool,
    planned_at: f32,
    style_until: f32,
    best: f32,
    anchor: f32,
    best_at: f32,
    reanchor: bool,
    oxygen: bool,
    heli_jump_from: Option<f32>,
    off_since: Option<f32>,
    dead_since: Option<f32>,
    hold_left: Option<u32>,
    hold: u32,
    executions: u32,
    pub rewinds: u32,
    last_rewind: Option<f32>,
    streak: i32,
    pub rollout_steps: u64,
    pub searches: u32,
    pub log: Vec<String>,
    pub trace: f32,
    traced: f32,
}

const OFF_LINE_M: f32 = 45.0;
const OFF_LINE_AIR_M: f32 = 90.0;
const OFF_LINE_S: f32 = 1.5;
const STALL_S: f32 = 8.0;
const STALL_M: f32 = 15.0;
const DEAD_S: f32 = 0.5;
const REWIND_BACK_S: f32 = 3.0;
const STREAK_S: f32 = 15.0;
const PLAN_EVERY_S: f32 = 0.5;
const PLAN_HORIZON_S: f32 = 6.0;
const ROLLOUT_LOST_M: f32 = 60.0;
const ROLLOUT_STRAY_M: f32 = 15.0;
const ROLLOUT_STALL_M: f32 = 10.0;
const STYLE_HOLD_S: f32 = 1.0;
const FLIGHT_STYLE_HOLD_S: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rollout {
    survived: bool,
    along: f32,
    off: f32,
    max_off: f32,
    stalled: bool,
}

impl Rollout {
    fn score(&self) -> f32 {
        let stray = (self.max_off - ROLLOUT_STRAY_M).max(0.0);
        if self.survived { self.along - 0.5 * self.off - 2.0 * stray } else { self.along - 10_000.0 }
    }
}

impl CoursePlayer {
    pub fn new(line: CourseLine) -> Self {
        CoursePlayer {
            line,
            cursor: 0,
            style: Style::default(),
            style_in_flight: false,
            planned_at: f32::NEG_INFINITY,
            style_until: f32::NEG_INFINITY,
            best: 0.0,
            anchor: 0.0,
            best_at: 0.0,
            reanchor: false,
            oxygen: false,
            heli_jump_from: None,
            off_since: None,
            dead_since: None,
            hold_left: None,
            hold: 0,
            executions: 0,
            rewinds: 0,
            last_rewind: None,
            streak: 0,
            rollout_steps: 0,
            searches: 0,
            log: Vec::new(),
            trace: 0.0,
            traced: f32::NEG_INFINITY,
        }
    }

    fn trace_line(&mut self, time: f32, k: usize, off: f32, s: &RiderState, c: &CourseControls, what: &str) {
        if self.trace <= 0.0 || time - self.traced < self.trace - 1e-3 {
            return;
        }
        self.traced = time;
        let l = &self.line;
        self.log.push(format!(
            "{time:.2}: {what} at {:.0} m along ({off:.1} m off, line y {:.1} rider {:.1}), {:.0} km/h (recorded {:.0}{}{}), \
             stick ({:+.2}, {:+.2}){}, r1 {}, style {:+.0} m {:?}",
            l.along[k],
            l.points[k].y,
            s.traj,
            s.speed() * crate::KMH_PER_M_PER_TICK,
            l.speed[k] * crate::KMH_PER_M_PER_TICK,
            if l.in_air[k] { ", air" } else { "" },
            if l.wingsuit[k] { ", wingsuit" } else { "" },
            c.input.steer,
            c.input.tuck - c.input.brake,
            if c.input.jump { ", A" } else { "" },
            c.r1,
            self.style.offset,
            self.style.speed
        ));
    }

    fn rollout(&mut self, s: &RiderState, x: &Session, w: &World, style: Style, hold_s: f32) -> Rollout {
        let (mut s, mut x) = (s.clone(), x.clone());
        let (mut cursor, mut oxygen) = (self.cursor, self.oxygen);
        let mut out = Rollout { survived: true, along: 0.0, off: 0.0, max_off: 0.0, stalled: false };
        let mut start = None;
        let steps = (PLAN_HORIZON_S * crate::SIM_HZ) as u32;
        let held = (hold_s * crate::SIM_HZ) as u32;
        for i in 0..steps {
            let flick = if i % 2 == 0 { 1.0 } else { 0.0 };
            let st = if i < held { style } else { Style::default() };
            let (c, _, along, off) = policy(&self.line, &mut cursor, &mut oxygen, &s, st, flick);
            (out.along, out.off) = (along, off);
            start.get_or_insert(along);
            if !s.in_air() {
                out.max_off = out.max_off.max(off);
            }
            if !s.death.is_alive() || off > ROLLOUT_LOST_M || s.janked.janked {
                out.survived = false;
                break;
            }
            s.wingsuit.pad_r1 = c.r1;
            let e = x.execute(&mut s, w, &c.input, &c.tricks, 0.0, &RewindPad::default(), None);
            self.rollout_steps += 1;
            if e.rider_dead.is_some() {
                out.survived = false;
                break;
            }
            if x.event.track_finished {
                out.along += 1000.0;
                return out;
            }
        }
        let end = self.line.along.last().copied().unwrap_or(0.0);
        out.stalled =
            out.survived && out.along - start.unwrap_or(0.0) < ROLLOUT_STALL_M && out.along + ROLLOUT_STALL_M < end;
        out
    }

    fn plan(&mut self, s: &RiderState, x: &Session, w: &World, time: f32) {
        let flying = s.state.is_wingsuit();
        if time >= self.style_until || (self.style_in_flight && !flying) {
            (self.style, self.style_in_flight) = (Style::default(), false);
        }
        let airborne = s.in_air() && !flying;
        if time - self.planned_at < PLAN_EVERY_S || airborne || (!flying && self.style != Style::default()) {
            return;
        }
        self.planned_at = time;
        let hold = if flying { FLIGHT_STYLE_HOLD_S } else { STYLE_HOLD_S };
        let current = self.rollout(s, x, w, self.style, hold);
        if current.survived && !current.stalled && current.max_off <= ROLLOUT_STRAY_M {
            return;
        }
        self.searches += 1;
        let mut best = (current.score(), self.style);
        for &speed in &SPEEDS {
            for &offset in &OFFSETS {
                let style = Style { offset, speed };
                if style == self.style {
                    continue;
                }
                let r = self.rollout(s, x, w, style, hold);
                if r.score() > best.0 {
                    best = (r.score(), style);
                }
            }
        }
        if best.1 != self.style {
            self.log.push(format!(
                "{time:.2}: lookahead{}: the {} {} ({:.0} m along, {:.0} m off at most), {:+.0} m {:?} scores {:.0}",
                if flying { " in flight" } else { "" },
                if self.style == Style::default() { "line's own" } else { "style ridden" },
                if !current.survived {
                    "is lost"
                } else if current.stalled {
                    "stalls"
                } else {
                    "strays"
                },
                current.along,
                current.max_off,
                best.1.offset,
                best.1.speed,
                best.0
            ));
            (self.style, self.style_until, self.style_in_flight) = (best.1, time + hold, flying);
        }
    }

    pub fn controls(
        &mut self,
        s: &RiderState,
        x: &Session,
        w: &World,
        time: f32,
        go_clock: f32,
        rewind_idle: bool,
    ) -> CourseControls {
        let mut c = CourseControls::default();
        let flick = if ((time * crate::SIM_HZ).round() as i64) % 2 == 0 { 1.0 } else { 0.0 };
        if let Some(n) = self.hold_left {
            c.rewind = RewindPad { rewind_held: n > 0, takeover: n == 0, flip_accel: 0.0 };
            c.r1 = s.in_air() && (s.state.is_wingsuit() || s.wingsuit.signal);
            return c;
        }
        if let Some(l) = s.start.as_ref() {
            if l.idle() && go_clock >= 2.0 {
                (self.cursor, self.best, self.anchor, self.best_at, self.style) = (0, 0.0, 0.0, time, Style::default());
                c.input.tuck = flick;
            }
            return c;
        }
        let finished = x.event.track_finished || x.event_ended;
        let (_, along, off) = locate(&self.line, &mut self.cursor, s.traj);
        if std::mem::take(&mut self.reanchor) {
            (self.best, self.anchor, self.best_at) = (along, along, time);
        }
        self.best = self.best.max(along);
        if self.best >= self.anchor + STALL_M {
            (self.anchor, self.best_at) = (self.best, time);
        }
        self.dead_since = if s.death.is_alive() { None } else { self.dead_since.or(Some(time)) };
        let limit = if s.in_air() { OFF_LINE_AIR_M } else { OFF_LINE_M };
        self.off_since = if off > limit { self.off_since.or(Some(time)) } else { None };
        let end = self.line.along.last().copied().unwrap_or(0.0);
        let stalled = !s.in_air() && time - self.best_at > STALL_S && self.best + 5.0 < end;
        let reason = if s.janked.anim_complete {
            Some("janked")
        } else if self.dead_since.is_some_and(|t| time - t >= DEAD_S) {
            Some("dead")
        } else if self.off_since.is_some_and(|t| time - t >= OFF_LINE_S) {
            Some("off the line")
        } else if stalled {
            Some("no progress")
        } else {
            None
        };
        let count = &x.manager.count;
        let can_rewind = s.janked.janked || (count.enabled && (count.infinite || count.rewinds_left > 0));
        if let Some(why) = reason.filter(|_| !finished && rewind_idle && can_rewind) {
            self.streak = match self.last_rewind {
                Some(at) if time - at < STREAK_S => (self.streak + 1).min(4),
                _ => 0,
            };
            let back = (REWIND_BACK_S * 2f32.powi(self.streak)).min(15.0);
            self.hold = (30.0 * back + 12.0).round() as u32;
            (self.hold_left, self.executions, self.last_rewind) = (Some(self.hold), 0, Some(time));
            self.log.push(format!(
                "{time:.2}: rewind {} ({why}; {along:.0} m along the line of {end:.0}, {off:.0} m off it): {back:.0} s back",
                self.rewinds + 1
            ));
            c.rewind = RewindPad { rewind_held: true, takeover: false, flip_accel: 0.0 };
            return c;
        }
        if !finished {
            self.plan(s, x, w, time);
        }
        let mut cursor = self.cursor;
        let (mut c, k, _, off) = policy(&self.line, &mut cursor, &mut self.oxygen, s, self.style, flick);
        let intercept = x.heli_jump.as_ref().map(|h| h.intercept).filter(|i| i.valid);
        if intercept.is_none() || s.in_air() {
            self.heli_jump_from = None;
        }
        if let Some(i) = intercept
            && !s.in_air()
            && !s.stopped
        {
            let d = (i.position - s.traj).with_y(0.0);
            c.input.steer = steer_towards(d, s.velocity, s.heading);
            (c.input.tuck, c.input.brake) = (1.0, 0.0);
            let from = *self.heli_jump_from.get_or_insert(time);
            c.input.jump = time - from < JUMP_HOLD_S;
        }
        let what = if s.state.is_wingsuit() {
            "flying"
        } else if s.in_air() {
            "in the air"
        } else {
            "riding"
        };
        self.trace_line(time, k, off, s, &c, what);
        c
    }

    pub fn report(&self, p: Vec3) -> CourseReport {
        let mut cursor = self.cursor;
        let (_, along, off) = locate(&self.line, &mut cursor, p);
        CourseReport {
            id: self.line.id,
            line_m: self.line.along.last().copied().unwrap_or(0.0),
            best_m: self.best.max(along),
            along_m: along,
            off_m: off,
            rewinds: self.rewinds,
            searches: self.searches,
        }
    }

    pub fn after(&mut self, r: &RewindStep, time: f32) {
        let Some(n) = self.hold_left.as_mut() else { return };
        *n = n.saturating_sub(1);
        self.executions += 1;
        if r.terminating {
            self.hold_left = None;
            self.rewinds += 1;
            (self.reanchor, self.off_since, self.dead_since, self.planned_at) = (true, None, None, f32::NEG_INFINITY);
            (self.style, self.style_in_flight) = (Style::default(), false);
            self.log.push(format!("{time:.2}: hand-back after rewind {}", self.rewinds));
        } else if self.executions > self.hold + 400
            || (r.state == RewindState::Idle && self.executions > self.hold + 30)
        {
            self.hold_left = None;
            self.log.push(format!("{time:.2}: the rewind did not run"));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CourseReport {
    pub id: u32,
    pub line_m: f32,
    pub best_m: f32,
    pub along_m: f32,
    pub off_m: f32,
    pub rewinds: u32,
    pub searches: u32,
}

pub fn steer_towards(d: Vec3, v: Vec3, heading: Vec3) -> f32 {
    let d = d.with_y(0.0).normalize_or_zero();
    let f = if v.with_y(0.0).length() > 0.05 { v } else { heading }.with_y(0.0).normalize_or_zero();
    let side = d.dot(f.cross(Vec3::Y));
    (side.atan2(f.dot(d)) * 2.0).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[[f32; 3]]) -> CourseLine {
        let points: Vec<Vec3> = points.iter().map(|p| Vec3::from_array(*p)).collect();
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along.last().unwrap() + w[0].distance(w[1]));
        }
        let n = points.len();
        CourseLine {
            jumps: Vec::new(),
            id: 1,
            points,
            along,
            speed: vec![1.0; n],
            vy: vec![0.0; n],
            in_air: vec![false; n],
            wingsuit: vec![false; n],
        }
    }

    #[test]
    fn the_line_is_walked_by_distance() {
        let l = line(&[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [20.0, 0.0, 0.0], [30.0, 0.0, 0.0]]);
        assert!(l.at(15.0).0.distance(Vec3::new(15.0, 0.0, 0.0)) < 1e-4);
        assert_eq!(l.at(99.0).0, Vec3::new(30.0, 0.0, 0.0));
        assert_eq!(l.at(-5.0).0, Vec3::ZERO);
        let (i, t, d) = l.project(Vec3::new(13.5, 0.0, 4.0), 0..4).unwrap();
        assert_eq!(i, 1);
        assert!((t - 0.35).abs() < 1e-5 && (d - 4.0).abs() < 1e-5);
        assert!((l.along_at(i, t) - 13.5).abs() < 1e-4);
        assert_eq!(l.nearest_in(Vec3::new(29.0, 0.0, 1.0), 0..4).map(|x| x.0), Some(3));
    }

    #[test]
    fn steering_turns_towards_the_target() {
        assert!(steer_towards(Vec3::new(10.0, 0.0, 10.0), Vec3::X, Vec3::X) > 0.5);
        assert!(steer_towards(Vec3::new(10.0, 0.0, -10.0), Vec3::X, Vec3::X) < -0.5);
        assert!(steer_towards(Vec3::X, Vec3::X, Vec3::X).abs() < 1e-6);
    }
}
