use super::*;

impl GlobeScene {
    pub fn rotation(&self) -> Quat {
        globe_rotation(self.pitch.x, self.yaw.x)
    }

    pub fn visible(&self) -> bool {
        self.globe_visible
    }

    pub fn frame(&self, aspect: f32) -> GlobeFrame {
        let cam = Camera::from_shot(&self.shots.current);
        let view = cam.view();
        let q = self.rotation();
        let world = qmat(q);
        let (w_inv, project_cam, globe_r, atmos_r) =
            render_object(self.data.globe.radius, self.data.globe.atmosphere_radius, q, cam.eye);
        let world_view = mmul(&world, &view);
        let hl = self.data.range(self.focus).map_or([0.0, 1.0, 0.0], |r| {
            let d = node_direction(r.lon_lat[1].to_radians(), r.lon_lat[0].to_radians());
            let v = transform(d, &world_view);
            let o = transform([0.0; 3], &world_view);
            normalize([v[0] - o[0], v[1] - o[1], v[2] - o[2]])
        });
        let game_time = self.time % 8388.608;
        let midground = self.midground_frame(&cam, aspect, game_time);
        GlobeFrame {
            visible: self.visible(),
            view,
            proj: cam.projection(aspect),
            view_inverse: inverse_rigid(&view),
            world_view,
            w_inv,
            camera_pos: cam.eye,
            project_cam,
            globe_r,
            atmos_r,
            highlight_view: hl,
            game_time,
            midground,
            midground_range: self.midground_range,
            lighting: self.lighting(),
            light: self.light.current(),
            postfx: self.postfx.weights(),
            fade: crate::game::fe::fade::Fader::default().presentation(),
            cloud: [self.cloud_in, self.cloud_out],
            clouds: if self.cloud_in > 0.0 || self.cloud_out > 0.0 {
                self.clouds.draws([self.cloud_in, self.cloud_out])
            } else {
                Vec::new()
            },
        }
    }

    pub fn lighting(&self) -> u32 {
        self.light.target.key
    }

    pub(super) fn midground_frame(&self, cam: &Camera, aspect: f32, game_time: f32) -> Option<MidgroundFrame> {
        if !self.midground_visible {
            return None;
        }
        let range = self.nodes.ranges.get(&self.midground_range)?;
        let lighting = self.lighting();
        let (s, c) = self.highlight_angle.sin_cos();
        Some(MidgroundFrame {
            range: range.key,
            world: range.world,
            view: cam.view(),
            proj: cam.projection(aspect),
            camera_pos: cam.eye,
            highlight: [self.highlight[0], self.highlight[1], self.highlight[2], self.highlight_off],
            highlight_xform: [[c, 0.0, -s, 0.0], [0.0, 1.0, 0.0, 0.0], [s, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]],
            lighting,
            game_time,
        })
    }

    pub fn entries(&self, stage: (f32, f32), aspect: f32) -> Vec<GlobeEntry> {
        match self.level {
            Some(4 | 5) => return self.list.clone(),
            None => return Vec::new(),
            _ if !self.visible() => return Vec::new(),
            _ => {}
        }
        let cam = Camera::from_shot(&self.shots.current);
        let q = self.rotation();
        let world_view = mmul(&qmat(q), &cam.view());
        let wvp = mmul(&world_view, &cam.projection(aspect));
        let r = self.data.globe.radius;
        let mut out = Vec::new();
        for range in &self.data.ranges {
            let focused = range.key == self.focus;
            if self.level == Some(2) && !focused {
                continue;
            }
            let (lat, lon) = (range.lon_lat[1].to_radians(), range.lon_lat[0].to_radians());
            let p = scale(node_direction(lat, lon), r);
            let c = transform(p, &wvp);
            let x = (c[0] / c[3] + 1.0) * stage.0 * 0.5;
            let y = (1.0 - c[1] / c[3]) * stage.1 * 0.5;
            let rz = p[0] * world_view[0][2] + p[1] * world_view[1][2] + p[2] * world_view[2][2];
            let scale_v = (rz / r).min(0.5);
            out.push(GlobeEntry {
                key: range.key,
                kind: 1,
                name: String::new(),
                x,
                y,
                opacity: if focused || self.steering { 1.0 } else { self.opacity(lat, lon) },
                scale: scale_v,
                win_condition: 7,
                selectable: true,
                selected: focused,
                ..Default::default()
            });
        }
        out
    }

    pub(super) fn opacity(&self, lat: f32, lon: f32) -> f32 {
        let pi = std::f32::consts::PI;
        let dlat = (self.pitch.x - lat).abs();
        let mut cur_lon = -self.yaw.x;
        if cur_lon < -pi {
            cur_lon += std::f32::consts::TAU;
        }
        let dlon = if lat <= -std::f32::consts::FRAC_PI_2 || lat >= std::f32::consts::FRAC_PI_2 {
            0.0
        } else {
            let mut d = (cur_lon - lon).abs();
            while d >= pi {
                d -= std::f32::consts::TAU;
            }
            d.abs()
        };
        let a = dlat / std::f32::consts::FRAC_PI_2;
        let b = dlon / 1.586_504;
        (1.0 - (a * a + b * b).sqrt()).max(0.0)
    }
}
