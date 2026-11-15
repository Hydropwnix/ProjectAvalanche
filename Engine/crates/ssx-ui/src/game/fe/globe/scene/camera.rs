use super::*;

impl GlobeScene {
    pub(super) fn node_targets(&self) -> Option<(f32, f32)> {
        let r = self.data.range(self.focus)?;
        let (lat, lon) = (r.lon_lat[1].to_radians(), r.lon_lat[0].to_radians());
        let mut yaw = self.yaw.target;
        if lat > -std::f32::consts::FRAC_PI_2 && lat < std::f32::consts::FRAC_PI_2 {
            yaw = -lon;
            if yaw < 0.0 {
                yaw += std::f32::consts::TAU;
            }
        }
        Some((lat.clamp(-PITCH_CLAMP, PITCH_CLAMP), yaw))
    }

    pub fn set_focus(&mut self, range: u32, snap: bool) {
        self.focus = range;
        if snap && let Some((p, y)) = self.node_targets() {
            self.pitch = Spring { omega: 42f32.sqrt(), x: p, v: 0.0, target: p };
            self.yaw = Spring { omega: 42f32.sqrt(), x: y, v: 0.0, target: y };
        }
    }

    pub fn wrap_yaw(&mut self) {
        let tau = std::f32::consts::TAU;
        let f = (self.yaw.x - std::f32::consts::PI) / tau;
        let frac = f - f.trunc();
        let mut w = if frac >= 0.5 { frac - 1.0 } else { frac };
        if w <= -0.5 {
            w += 1.0;
        }
        self.yaw.x = w * tau + std::f32::consts::PI;
    }

    pub(crate) fn camera_targets(&mut self, dt: f32, targets: (f32, f32)) {
        let mut targets = targets;
        if let Some((p, y)) = self.node_targets().filter(|_| self.follow_focus) {
            self.pitch.omega = 42f32.sqrt();
            self.yaw.omega = 42f32.sqrt();
            targets = (p, y);
        }
        self.pitch.target = targets.0.clamp(-PITCH_CLAMP, PITCH_CLAMP);
        let turns = (targets.1 - self.yaw.x) / std::f32::consts::TAU;
        let frac = turns - turns.trunc();
        let wrap = if frac > 0.5 {
            frac - 1.0
        } else if frac < -0.5 {
            frac + 1.0
        } else {
            frac
        };
        self.yaw.target = self.yaw.x + wrap * std::f32::consts::TAU;
        self.pitch.step(dt);
        self.yaw.step(dt);
    }

    pub(crate) fn camera_idle(&mut self, dt: f32) {
        self.wrap_yaw();
        self.pitch.omega = 2.0;
        self.yaw.omega = 2.0;
        let yaw = self.yaw.x + 0.1;
        self.camera_targets(dt, (0.0, yaw));
    }

    pub(crate) fn camera_menu(&mut self, dt: f32) {
        if self.data.range(self.focus).is_some() {
            self.follow_focus = true;
            self.wrap_yaw();
            let held = (self.pitch.target, self.yaw.target);
            self.camera_targets(dt, held);
        } else {
            self.follow_focus = false;
            self.camera_idle(dt);
        }
    }

    pub(crate) fn camera_range(&mut self, dt: f32, input: bool, flow: &dyn GlobeFlow) -> Option<u32> {
        let mut picked = None;
        let mut targets = (self.pitch.x, self.yaw.x);
        self.steering = false;
        let (x, y) = if input { self.stick } else { (0.0, 0.0) };
        if y.abs() > 0.25 {
            targets.0 += y * y * y;
            self.steering = true;
        }
        if x.abs() > 0.25 {
            targets.1 -= x * x * x;
            self.steering = true;
        }
        let mut allow = false;
        if self.steering {
            self.follow_focus = false;
            if self.repeat >= 0.0 {
                self.repeat -= dt;
            }
            allow = self.repeat <= 0.0;
        } else {
            self.repeat = -1.0;
        }
        self.pitch.omega = 6.0;
        self.yaw.omega = 6.0;
        if allow {
            picked = self.pick().filter(|&k| flow.range_valid(k));
            if let Some(p) = picked {
                self.focus = p;
                self.repeat = 0.225;
            }
        }
        self.camera_targets(dt, targets);
        picked
    }

    pub(crate) fn pick(&self) -> Option<u32> {
        let stage = crate::game::fe::globe::STAGE;
        let aspect = stage.0 / stage.1;
        let cam = Camera::from_shot(&self.shots.current);
        let vp = mmul(&cam.view(), &cam.projection(aspect));
        let wvp = mmul(&qmat(self.rotation()), &vp);
        let to_screen = |p: V3, m: &M4| {
            let c = transform(p, m);
            ([(c[0] / c[3] + 1.0) * stage.0 * 0.5, (1.0 - c[1] / c[3]) * stage.1 * 0.5], c[2] / c[3])
        };
        let screen = |p: V3| to_screen(p, &wvp);
        let r = self.data.globe.radius;
        let (centre, centre_z) = to_screen([0.0; 3], &vp);
        let (edge, _) = to_screen([r, 0.0, 0.0], &vp);
        let reach = (0.975 * (edge[0] - centre[0]).abs()).powi(2);
        let focus_dir = self.data.range(self.focus).map_or_else(
            || node_direction(self.pitch.x, -self.yaw.x),
            |f| node_direction(f.lon_lat[1].to_radians(), f.lon_lat[0].to_radians()),
        );
        let (focus, _) = screen(scale(focus_dir, r));
        let (sx, sy) = self.stick;
        let sl = (sx * sx + sy * sy).sqrt().max(1e-6);
        let dir = [sx / sl, -sy / sl];
        let (mut best, mut best_cos, mut nearest, mut nearest_d) = (None, 0.7f32, None, reach);
        for range in &self.data.ranges {
            let p = scale(node_direction(range.lon_lat[1].to_radians(), range.lon_lat[0].to_radians()), r);
            let (s, z) = screen(p);
            if centre_z <= z || z.is_nan() {
                continue;
            }
            let d2 = (s[0] - centre[0]).powi(2) + (s[1] - centre[1]).powi(2);
            if d2 >= reach {
                continue;
            }
            if range.key == self.focus {
                nearest_d = 0.0;
                nearest = Some(range.key);
            } else if d2 < nearest_d {
                nearest_d = d2;
                nearest = Some(range.key);
            }
            let v = [s[0] - focus[0], s[1] - focus[1]];
            let l2 = v[0] * v[0] + v[1] * v[1];
            if l2 > 0.0001 {
                let c = (v[0] * dir[0] + v[1] * dir[1]) / l2.sqrt();
                if c > best_cos {
                    best_cos = c;
                    best = Some(range.key);
                }
            }
        }
        best.or(nearest).filter(|&k| k != self.focus)
    }
}
