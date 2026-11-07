use ssx_net_proto::{Mode, ReportKind, TICK_HZ};

use super::limits::*;
use super::track::{Pending, RiderValidation};
use super::{Finding, Measure};
use ssx_rules::course::SPLITS;

impl RiderValidation {
    pub(super) fn check_report(&mut self, p: &Pending, out: &mut Vec<Finding>) {
        let r = &p.report;
        let point = p.point;
        let k = point.exec;
        let clock = f64::from(r.clock_ms) * f64::from(TICK_HZ) / 1000.0;
        if !self.paused {
            self.check_speed(&point, out);
            if let Some(a) = self.sample_before(k)
                && a.epoch == point.epoch
                && !point.warped
                && !a.rewind
                && k > a.exec
            {
                self.check_step(&a, &point, out);
            }
        }
        if r.kind == ReportKind::Finish
            && self.limits.finish_at_end()
            && self.touch.is_none()
            && let Some(a) = self.sample_before(k)
            && a.epoch == point.epoch
            && !point.warped
            && !a.rewind
            && self.pass_gates(a.pos, point.pos)
            && let Some(d) = self.limits.end_segment_distance(a.pos, point.pos)
        {
            self.nearest_end = self.nearest_end.min(d);
            if d <= self.limits.touch_reach() {
                self.touch = Some((a.exec, k));
            }
        }
        let handover = self.start.handover;
        let steps = k - handover;
        let limit = clock_max_ticks(steps);
        self.finding(
            out,
            Measure::ClockStart,
            k,
            clock,
            limit,
            format!(
                "a {} at execution {k:.0} with the clock {} ms: {clock:.2} executions, at most {limit:.2}",
                r.kind.name(),
                r.clock_ms
            ),
        );
        let (lo, hi) = match self.touch {
            Some((a, _)) if self.limits.finish_at_end() && a <= k => (a, k),
            _ if matches!(r.kind, ReportKind::Finish | ReportKind::Failed) && self.limits.timed_out(r.clock_ms) => {
                (k - 1.0, k)
            }
            _ => (k, k),
        };
        let (s_lo, s_hi) = (lo - clock, hi - clock);
        let new_loop = r.loops > self.loops;
        match self.anchor {
            _ if new_loop => {}
            None => {
                if let Some(ready) = self.ready {
                    let slowed = self.slowed_in(0.0, hi);
                    let path = handover;
                    let by_ready = ready.elapsed + ready.horizon;
                    let rounding = clock_rounding_between_ticks(0.0, hi - handover);
                    let allow = path.min(by_ready) + slowed + CLOCK_BEHIND_TICKS + rounding;
                    self.finding(
                        out,
                        Measure::ClockReady,
                        k,
                        s_lo,
                        allow,
                        format!(
                            "the clock started after execution {s_lo:.2} (a {} at execution {k:.0} with {} ms): at most \
                             {allow:.2} ({} {path:.0}, `Ready` came {:.2} ticks after the start with the horizon \
                             {:.2}; slowed {slowed:.2}, the rounding {rounding:.3})",
                            r.kind.name(),
                            r.clock_ms,
                            self.limits.start_words().1,
                            ready.elapsed,
                            ready.horizon
                        ),
                    );
                }
            }
            Some((a_lo, a_hi, k0)) => {
                let slowed = self.slowed_in(k0, hi);
                let rounding = clock_rounding_between_ticks(k0 - handover, hi - handover);
                let detail = format!(
                    "the executions less the clock moved from {a_lo:.2}…{a_hi:.2} at execution {k0:.0} to \
                     {s_lo:.2}…{s_hi:.2} at {k:.0} (a {} with {} ms), the rider slowed {slowed:.2} executions, the \
                     rounding {rounding:.3}",
                    r.kind.name(),
                    r.clock_ms
                );
                let allow = slowed + CLOCK_BEHIND_TICKS + rounding;
                self.finding(out, Measure::ClockBehind, k, s_lo - a_hi, allow, detail);
            }
        }
        self.anchor = Some((s_lo, s_hi, hi));
        match r.kind {
            ReportKind::Split => self.check_split(p, out),
            ReportKind::Finish => {
                let at_end = self.limits.finish_at_end() && !self.limits.result_at_time_up(r.clock_ms);
                if at_end {
                    let last = self.split.map_or(-1, i32::from);
                    let missing = (SPLITS as i32 - 1 - last).max(0);
                    self.finding(
                        out,
                        Measure::SplitOrder,
                        k,
                        f64::from(missing),
                        0.0,
                        format!("a finish after split {:?}: {missing} of the course's splits never came", self.split),
                    );
                }
                if at_end && !self.limits.gates.is_empty() {
                    let total = self.limits.gates.len();
                    let missing = total.saturating_sub(self.gate);
                    let next = match self.limits.gates.get(self.gate) {
                        Some(v) => format!(
                            "; gate {} at {:.1}: the path's nearest {:.1} m from it, the touch reaches {:.2} m",
                            self.gate + 1,
                            v.centre,
                            self.gate_nearest,
                            self.limits.touch_reach()
                        ),
                        None => String::new(),
                    };
                    self.finding(
                        out,
                        Measure::Gates,
                        k,
                        missing as f64,
                        0.0,
                        format!(
                            "a finish at {:.1} with {} of the map's {total} gates passed{next}",
                            point.pos, self.gate
                        ),
                    );
                }
                if let Some(max) = self.limits.time_limit_finish_ms() {
                    self.finding(
                        out,
                        Measure::TimeLimit,
                        k,
                        f64::from(r.clock_ms),
                        max,
                        format!(
                            "a finish with {} ms on the clock, the map's time limit {} ms lets it read at most \
                             {max:.1}",
                            r.clock_ms,
                            self.limits.time_limit_ms.unwrap_or(0)
                        ),
                    );
                }
                if at_end {
                    let touched = self.touch.is_some_and(|(a, _)| a <= k);
                    let near = self.nearest_end;
                    self.finding(
                        out,
                        Measure::Finish,
                        k,
                        if touched { f64::from(near) } else { f64::INFINITY },
                        f64::from(self.limits.touch_reach()),
                        format!(
                            "a finish at {:.1}, the run never within {:.2} m of the `end` volume (nearest {near:.1} m)",
                            point.pos,
                            self.limits.touch_reach()
                        ),
                    );
                }
                self.over = true;
            }
            ReportKind::Failed => self.over = true,
            ReportKind::Points => {}
        }
        if self.mode == Mode::Trick && matches!(r.kind, ReportKind::Points | ReportKind::Finish) {
            let (before, at) = self.points;
            let gained = f64::from(r.points) - f64::from(before);
            let max = self.limits.trick_points_max(k - at);
            self.finding(
                out,
                Measure::Tricks,
                k,
                gained / max,
                1.0,
                format!(
                    "{gained:.0} points in {:.0} executions ({} → {}), the table allows {max:.0}",
                    k - at,
                    before,
                    r.points
                ),
            );
            self.points = (r.points, k);
        }
        self.loops = self.loops.max(r.loops);
    }

    fn check_split(&mut self, p: &Pending, out: &mut Vec<Finding>) {
        let r = &p.report;
        let t = p.point.exec;
        let Some(k) = r.split else { return };
        let new_loop = r.loops > self.loops;
        let expected = if new_loop { 0 } else { self.split.map_or(0, |s| s.saturating_add(1)) };
        self.finding(
            out,
            Measure::SplitOrder,
            t,
            f64::from(k.abs_diff(expected)),
            0.0,
            format!("split {k} where split {expected} comes next (the last {:?}, loop {})", self.split, r.loops),
        );
        if let Some(&h) = self.limits.splits.get(usize::from(k)) {
            let y = p.point.pos.y;
            let start = self.start.handover_place().y - self.start.margin;
            let line = if r.loops == 0 { h.min(start) } else { h };
            let above = f64::from(y - h);
            let below = f64::from(line - y) - f64::from(self.limits.split_depth);
            let depth = self.limits.split_depth;
            let detail = if line < h {
                format!(
                    "split {k} at the height {y:.2}, its line at {h:.2} above the run's start at {line:.2} (at most \
                     {depth:.2} m below the start)"
                )
            } else {
                format!("split {k} at the height {y:.2}, its line at {h:.2} (at most {depth:.2} m below it)")
            };
            self.finding(out, Measure::SplitHeight, t, above.max(below), -1e-6, detail);
        }
        self.split = Some(k);
        if new_loop && self.paused && self.limits.loops() {
            self.resume();
        }
    }
}
