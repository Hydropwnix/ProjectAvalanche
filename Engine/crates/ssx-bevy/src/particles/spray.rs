use bevy::prelude::*;
use ssx_assets::particles::SprayKind;

const EPS: f32 = 1.192_092_9e-7;

#[derive(Clone, Copy, Debug, Default)]
pub struct SprayInputs {
    pub is_awesome: bool,
    pub velocity: Vec3,
    pub spray_velocity: Vec3,
    pub ice_spray: Vec3,
    pub snow_spray: Vec3,
    pub landing_impact: f32,
    pub ground_contact: bool,
    pub in_air: bool,
    pub depth_below_snow: f32,
    pub grinding: bool,
    pub powder: f32,
    pub packed: [f32; 5],
    pub ice: f32,
    pub rock: f32,
    pub backwards_rewind: bool,
    pub ground_normal: Vec3,
    pub snow_coverage: f32,
    pub ao: f32,
    pub is_super_awesome: bool,
    pub track_started: bool,
    pub wipeout: bool,
    pub ice_axe_active: bool,
    pub ice_axe_blend: f32,
    pub lean: f32,
    pub downwash_visual_strength: f32,
}

#[derive(Clone, Debug)]
pub struct EffectRecord {
    pub scale: Vec3,
    pub rotation: Quat,
    pub position: Vec3,
    pub ao: f32,
    pub blend: Vec<(u8, f32)>,
}

pub type Switch = (usize, usize, bool);

#[derive(Clone, Debug)]
pub struct SnowSprayDriver {
    pub lod: bool,
    enabled: bool,
    is_awesome: bool,
    kickoff_armed: bool,
    on: [[bool; 6]; 3],
    spray: Vec3,
    velocity: Vec3,
    surface_offset: Vec3,
    spray_velocity_len: f32,
    spray_len: f32,
    landing_impact: f32,
    depth: f32,
    ao: f32,
    contact_coverage: f32,
    trail_scale: i32,
}

impl SnowSprayDriver {
    pub fn new(lod: bool) -> Self {
        Self {
            lod,
            enabled: true,
            is_awesome: false,
            kickoff_armed: false,
            on: [[false; 6]; 3],
            spray: Vec3::ZERO,
            velocity: Vec3::ZERO,
            surface_offset: Vec3::ZERO,
            spray_velocity_len: 0.0,
            spray_len: 0.0,
            landing_impact: 0.0,
            depth: 0.0,
            ao: 0.0,
            contact_coverage: 0.0,
            trail_scale: 10,
        }
    }

    pub fn is_on(&self, s: usize, k: usize) -> bool {
        self.on[s][k]
    }
}

impl SnowSprayDriver {
    pub fn update(&mut self, inp: &SprayInputs, present: &[[bool; 6]; 3]) -> Vec<Switch> {
        self.is_awesome = inp.is_awesome;
        self.velocity = inp.velocity;
        self.spray_velocity_len = inp.spray_velocity.length();
        self.spray = inp.snow_spray + inp.ice_spray;
        self.spray_len = self.spray.length() - 0.001;
        self.landing_impact = inp.landing_impact;
        let contact = inp.ground_contact;
        let in_air = inp.in_air;
        let (mut grinding, mut rewind, mut rock_ok, mut row) = (false, false, true, 0i32);
        if !self.lod {
            self.depth = inp.depth_below_snow;
            grinding = inp.grinding;
            rewind = inp.backwards_rewind;
            rock_ok = inp.rock <= 0.09;
            let packed: f32 = inp.packed.iter().sum();
            let (powder, ice) = (inp.powder, inp.ice);
            row = if !contact && packed + ice + powder < 0.9 {
                -1
            } else if packed > powder && packed > ice {
                1
            } else if ice > packed && ice > powder {
                2
            } else {
                0
            };
        }
        if contact {
            self.surface_offset = inp.ground_normal * self.depth;
        }
        self.ao = inp.ao;
        let mut switches = Vec::new();
        let awesome = self.is_awesome as usize;
        let blocked = grinding || rewind || !rock_ok;
        for s in 0..3 {
            let mine = row == s as i32;
            let carve = mine && self.spray_len > EPS && !blocked;
            self.switch(s, SprayKind::Carve as usize, carve, present, &mut switches);
            let impact = mine && self.landing_impact > EPS && !blocked;
            self.switch(s, 1 + awesome, impact, present, &mut switches);
            let kickoff =
                in_air && self.velocity.length() > EPS && self.kickoff_armed && self.contact_coverage > EPS && !rewind;
            self.switch(s, 3 + awesome, kickoff, present, &mut switches);
            let trail = mine
                && self.depth > 0.1
                && self.spray_velocity_len > 0.01
                && !in_air
                && !grinding
                && !rewind
                && rock_ok;
            self.switch(s, SprayKind::Trail as usize, trail, present, &mut switches);
        }
        self.kickoff_armed = inp.snow_coverage > 0.0 && row != -1 && rock_ok;
        self.contact_coverage = if contact { inp.snow_coverage } else { 0.0 };
        switches
    }

    fn switch(&mut self, s: usize, k: usize, want: bool, present: &[[bool; 6]; 3], out: &mut Vec<Switch>) {
        if !present[s][k] {
            return;
        }
        if self.enabled && !self.on[s][k] && want {
            self.on[s][k] = true;
            out.push((s, k, true));
        } else if self.on[s][k] && !want {
            self.on[s][k] = false;
            out.push((s, k, false));
        }
    }

    pub fn apply(&self, k: usize, rec: &mut EffectRecord) {
        let awesome = self.is_awesome as usize;
        let clamp01 = |x: f32| x.clamp(0.0, 1.0);
        let w = if k == SprayKind::Carve as usize {
            self.align_to_spray(rec);
            clamp01(self.spray_len * 20.0)
        } else if k == 1 + awesome {
            clamp01(self.landing_impact * 80.0)
        } else if k == 3 + awesome {
            clamp01(self.velocity.length() * self.contact_coverage)
        } else if k == SprayKind::Trail as usize {
            rec.position += self.surface_offset;
            let w = clamp01((self.spray_velocity_len - 0.01) * (self.depth - 0.1) * 10.0);
            w * self.trail_scale as f32 * 0.1
        } else {
            return;
        };
        rec.ao = self.ao;
        rec.blend = vec![(1, 1.0 - w), (2, w)];
    }

    fn align_to_spray(&self, rec: &mut EffectRecord) {
        let q = rec.rotation;
        let (x, z) = (q * Vec3::X, q * Vec3::Z);
        let xh = x.normalize();
        let sh = (self.spray / self.spray.length()).normalize();
        let mut angle = xh.dot(sh).clamp(-1.0, 1.0).acos();
        if xh.cross(sh).dot(z) < 0.0 {
            angle = std::f32::consts::TAU - angle;
        }
        let (sin, cos) = (angle * 0.5).sin_cos();
        let r = Quat::from_xyzw(z.x * sin, z.y * sin, z.z * sin, cos);
        if r.is_finite() {
            rec.rotation = r * q;
        }
        let along = z * self.velocity.dot(z);
        if (along / along.length()).is_finite() {
            rec.position += self.surface_offset;
        }
    }
}
