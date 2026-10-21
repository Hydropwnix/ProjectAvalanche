pub const NO_LANDING: f32 = 1e35;
const EPS: f32 = 1.192_092_9e-7;
const INPUT_DEAD_ZONE: f32 = 0.05;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Actuator {
    pub k: f32,
    pub vel: f32,
    pub vel_rate: f32,
    pub chosen_target: f32,
    pub k_unassisted: f32,
    pub vel_unassisted: f32,
    pub unassisted_rate: f32,
    pub requested: f32,
    pub input: f32,
    pub target_vel: f32,
    pub stick_scale: f32,
    pub max_vel: f32,
    pub max_assist_vel: f32,
    pub angle: f32,
}

fn decay(k: f32, t: f32) -> f32 {
    (-k * t).exp()
}

pub fn crit_damped(e0: f32, rate: f32, k: f32, t: f32) -> (f32, f32) {
    let ex = decay(k, t);
    let c = rate + k * e0;
    ((e0 + c * t) * ex, (rate - c * k * t) * ex)
}

pub fn wrap_angle(x: f32) -> f32 {
    let f = x as f64 / std::f64::consts::TAU;
    let frac = f - f.trunc();
    let r = if frac >= 0.5 { frac - 1.0 } else { frac } * std::f64::consts::TAU;
    (if frac <= -0.5 { r + std::f64::consts::TAU } else { r }) as f32
}

impl Actuator {
    pub fn new(
        strength: f32,
        angle: f32,
        vel: f32,
        input: f32,
        stick_scale: f32,
        max_vel: f32,
        max_assist: f32,
    ) -> Self {
        let k = strength.max(0.0).sqrt();
        let r = if stick_scale.abs() <= EPS { 0.0 } else { vel / stick_scale };
        let input = if input < 0.0 { r.max(input) } else { r.min(input) };
        Self {
            k,
            vel,
            vel_rate: 0.0,
            chosen_target: 0.0,
            k_unassisted: k,
            vel_unassisted: vel,
            unassisted_rate: 0.0,
            requested: 0.0,
            input,
            target_vel: vel,
            stick_scale,
            max_vel,
            max_assist_vel: max_assist,
            angle,
        }
    }

    pub fn set_input(&mut self, u: f32) {
        let u = u.clamp(-1.0, 1.0);
        if u.abs() <= INPUT_DEAD_ZONE {
            self.target_vel = 0.0;
            self.input = 0.0;
            return;
        }
        let p = self.input;
        if p.abs() > INPUT_DEAD_ZONE && u * p >= 0.0 {
            if (u - p).abs() < INPUT_DEAD_ZONE {
                return;
            }
            self.input = u;
            let (ua, pa, ta) = (u.abs(), p.abs(), self.target_vel.abs());
            let sign = if u >= 0.0 { 1.0 } else { -1.0 };
            self.target_vel = if ua < pa {
                (ta / pa).max(self.stick_scale) * ua * sign
            } else {
                sign * (ta + (ua - pa) * ((self.stick_scale - ta) / (1.0 - pa)).max(0.0))
            };
        } else {
            self.target_vel = self.stick_scale * u;
            self.input = u;
        }
    }

    pub fn predict_delta(&self, target: f32, t: f32) -> f32 {
        let k = self.k;
        if k <= EPS {
            return self.vel * t;
        }
        let ex = decay(k, t);
        let e0 = self.vel - target;
        let c = self.vel_rate + k * e0;
        target * t + ((k * e0 + c) * (1.0 - ex) - c * k * t * ex) / (k * k)
    }

    pub fn solve_target(&self, delta: f32, t: f32) -> f32 {
        let k = self.k;
        if k <= EPS {
            return delta / t.max(EPS);
        }
        let ex = decay(k, t);
        let (v, r) = (self.vel, self.vel_rate);
        let num = k * k * delta + (v * k * k + r * k) * ex * t + (2.0 * k * v + r) * (ex - 1.0);
        num / ((ex + 1.0) * k * k * t + 2.0 * k * (ex - 1.0))
    }

    pub fn update(&mut self, dt: f32, ticks_to_land: f32, offset: f32, assist: f32, period: f32) {
        let mut target = self.target_vel;
        if ticks_to_land < NO_LANDING {
            let final_ = self.angle + offset + self.predict_delta(self.target_vel, ticks_to_land);
            let n = (final_ / period).trunc();
            let half = (1.0 - assist) * period * 0.5;
            let (mut best_cost, mut best) = (NO_LANDING, self.target_vel);
            for i in -1..=1 {
                let c = (n + i as f32) * period;
                let goal = final_.max(c - half).min(c + half);
                let mut v = self.solve_target(goal - self.angle - offset, ticks_to_land);
                let cost = if v.abs() > self.max_assist_vel {
                    let cost = v.abs() + 100.0 - self.max_assist_vel;
                    v = self.max_assist_vel * if v >= 0.0 { 1.0 } else { -1.0 };
                    cost
                } else {
                    let d = v - self.target_vel;
                    if v.abs() > EPS && self.target_vel * v < 0.0 { 3.0 * d } else { d }
                };
                if cost.abs() < best_cost {
                    (best_cost, best) = (cost.abs(), v);
                }
            }
            target = best;
        }
        self.chosen_target = target;
        let (e, r) = crit_damped(self.vel - target, self.vel_rate, self.k, dt);
        (self.vel, self.vel_rate) = (e + target, r);
        self.angle = wrap_angle(self.angle + self.predict_delta(target, dt));
        self.requested = self.target_vel;
        let (e, r) = crit_damped(self.vel_unassisted - self.target_vel, self.unassisted_rate, self.k_unassisted, dt);
        (self.vel_unassisted, self.unassisted_rate) = (e + self.target_vel, r);
    }

    pub fn normalized(&self) -> f32 {
        if self.max_vel.abs() <= EPS { 0.0 } else { self.vel / self.max_vel }
    }

    pub fn normalized_unassisted(&self) -> f32 {
        if self.max_vel.abs() <= EPS { 0.0 } else { self.vel_unassisted / self.max_vel }
    }

    pub fn magnitude(&self) -> f32 {
        let (a, u) = (self.normalized(), self.normalized_unassisted());
        if a > 1.525_879e-5 {
            a.min(u.max(0.0))
        } else if a < -1.525_879e-5 {
            a.max(u.min(0.0))
        } else {
            0.0
        }
    }

    pub fn is_idle(&self, x: f32) -> bool {
        self.magnitude().abs() <= x
    }
}

pub fn off_multiple(a: f32, period: f32) -> f32 {
    let r = a.rem_euclid(period);
    r.min(period - r)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{PI, TAU};

    use super::*;

    const DEG: f32 = PI / 180.0;

    fn spin(vel: f32, angle: f32) -> Actuator {
        Actuator::new(0.015, angle, vel, 0.0, 6.0 * DEG, 6.0 * DEG, 0.314_159_27)
    }

    #[test]
    fn spin_up_is_critically_damped_with_k_sqrt_0_015() {
        let mut a = spin(0.0, 0.0);
        a.set_input(1.0);
        assert!((a.target_vel - 6.0 * DEG).abs() < 1e-7);
        let k = 0.015f32.sqrt();
        assert!((1.0 / k - 8.165).abs() < 1e-3, "time constant {} ticks", 1.0 / k);
        for n in 1..=30 {
            a.update(2.0, NO_LANDING, 0.0, 0.0, PI);
            let t = 2.0 * n as f32;
            let expect = 6.0 * DEG * (1.0 - (1.0 + k * t) * (-k * t).exp());
            assert!((a.vel - expect).abs() < 1e-6, "t = {t}: {} vs {expect}", a.vel);
        }
        let at = |t: f32| 1.0 - (1.0 + k * t) * (-k * t).exp();
        assert!((at(1.0 / k) - 0.264).abs() < 1e-3 && (at(30.0) - 0.881).abs() < 1e-3 && at(60.0) > 0.99);
        a.set_input(0.04);
        assert_eq!(a.target_vel, 0.0);
        (0..60).for_each(|_| a.update(2.0, NO_LANDING, 0.0, 0.0, PI));
        assert!(a.vel.abs() < 1e-4 * DEG);
    }

    #[test]
    fn predict_and_solve_are_inverse() {
        let mut a = spin(3.0 * DEG, 0.3);
        a.vel_rate = 0.01;
        for (target, t) in [(6.0 * DEG, 40.0), (-2.0 * DEG, 10.0), (0.2, 3.0)] {
            let d = a.predict_delta(target, t);
            assert!((a.solve_target(d, t) - target).abs() < 1e-5, "{target} over {t}");
        }
        let (target, t) = (6.0 * DEG, 40.0);
        let (mut e, mut r, mut sum) = (a.vel - target, a.vel_rate, 0.0);
        for _ in 0..40_000 {
            sum += (e + target) * 0.001;
            (e, r) = crit_damped(e, r, a.k, 0.001);
        }
        assert!((a.predict_delta(target, t) - sum).abs() < 1e-3);
    }

    #[test]
    fn spin_with_assist_lands_on_a_multiple_of_pi() {
        for ttl0 in [50.0f32, 36.0, 80.0, 120.0, 37.0] {
            let mut a = spin(6.0 * DEG, 0.0);
            a.set_input(1.0);
            let mut ttl = ttl0;
            let mut total = 0.0f32;
            while ttl > 0.0 {
                let before = a.angle;
                a.update(2.0, ttl.max(1.0), 0.0, 1.0, PI);
                total += wrap_angle(a.angle - before);
                ttl -= 2.0;
            }
            let off = off_multiple(total, PI);
            let tol = if ttl0 % 2.0 == 0.0 { 0.5 * DEG } else { a.vel.abs() + 0.5 * DEG };
            assert!(off < tol, "ttl {ttl0}: rotated {}°, {}° off a multiple of 180°", total / DEG, off / DEG);
            assert!(a.vel.abs() <= 0.314_159_27 + 1e-6);
        }
    }

    #[test]
    fn assist_window_and_velocity_cap() {
        let flip = |final_deg: f32| {
            let mut a = Actuator::new(0.015, 0.0, 0.0, 0.0, 6.0 * DEG, 6.0 * DEG, 0.314_159_27);
            let ttl = 40.0;
            a.target_vel = (TAU + final_deg * DEG) / a.predict_delta(1.0, ttl) * 1.0;
            let want = a.target_vel;
            let end = a.predict_delta(want, ttl);
            assert!((end - (TAU + final_deg * DEG)).abs() < 1e-4);
            a.update(2.0, ttl, 0.0, 0.8, TAU);
            (want, a.chosen_target, a)
        };
        let (want, chosen, _) = flip(20.0);
        assert!((chosen - want).abs() < 1e-6, "inside the window: {chosen} vs {want}");
        let (_, chosen, a) = flip(50.0);
        let mut b = a;
        (b.vel, b.vel_rate, b.angle) = (0.0, 0.0, 0.0);
        let end = b.predict_delta(chosen, 40.0);
        assert!(((end - TAU) / DEG - 36.0).abs() < 0.05, "pulled to {}°", (end - TAU) / DEG);
        let mut c = spin(0.0, 0.0);
        c.update(2.0, 4.0, 0.0, 1.0, PI);
        assert!(c.chosen_target.abs() < 1e-6);
        let mut c = spin(0.0, PI / 2.0);
        c.update(2.0, 2.0, 0.0, 1.0, PI);
        assert!((c.chosen_target.abs() - 0.314_159_27).abs() < 1e-6, "{}", c.chosen_target);
    }

    #[test]
    fn input_easing_keeps_the_speed_ratio() {
        let mut a = spin(0.0, 0.0);
        a.set_input(1.0);
        a.set_input(0.5);
        assert!((a.target_vel - 3.0 * DEG).abs() < 1e-7);
        a.set_input(0.52);
        assert!((a.target_vel - 3.0 * DEG).abs() < 1e-7);
        a.set_input(-1.0);
        assert!((a.target_vel + 6.0 * DEG).abs() < 1e-7);
        let b = Actuator::new(0.015, 0.0, 12.0 * DEG, 0.3, 6.0 * DEG, 6.0 * DEG, 0.3);
        assert!((b.input - 0.3).abs() < 1e-7 && (b.target_vel - 12.0 * DEG).abs() < 1e-7);
        assert!((wrap_angle(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-6 && (wrap_angle(-0.25) + 0.25).abs() < 1e-7);
    }
}
