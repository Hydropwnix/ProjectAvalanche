use super::pan3d::Packet;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f32 {
    a[2].mul_add(b[2], a[1].mul_add(b[1], a[0] * b[0]))
}

fn scale(a: V3, s: f32) -> V3 {
    a.map(|v| v * s)
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn audio_frame(v: V3) -> V3 {
    [v[0], v[1], if v[2] != 0.0 { -v[2] } else { v[2] }]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub pos: V3,
    pub forward: V3,
    pub up: V3,
}

impl Listener {
    pub fn from_camera(eye: V3, look: V3) -> Self {
        let (pos, f) = (audio_frame(eye), audio_frame(look));
        let f = scale(f, 1.0 / dot(f, f).sqrt());
        let u0 = if f[1] >= 0.999 {
            [-1.0, 0.0, 0.0]
        } else if -0.999 >= f[1] {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let r = cross(u0, f);
        let r = scale(r, 1.0 / dot(r, r).sqrt());
        Self { pos, forward: f, up: cross(f, r) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmitterSettings {
    pub center: f32,
    pub proximity: f32,
    pub hf_scale: f32,
    pub rolloff: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub cut_distance: f32,
    pub offset: V3,
    pub max_delay: f32,
    pub doppler: f32,
}

pub const BOARD_CENTER: EmitterSettings = EmitterSettings {
    center: 0.5,
    proximity: 1.0,
    hf_scale: 1.0,
    rolloff: 0.0115,
    min_distance: 1.0,
    max_distance: 200.0,
    cut_distance: 200.0,
    offset: [0.0, -1.0, 0.0],
    max_delay: 0.2,
    doppler: 1.0,
};

pub const RIDER_VOICE: EmitterSettings = EmitterSettings { center: 1.0, ..BOARD_CENTER };

pub const AVALANCHE: EmitterSettings = EmitterSettings {
    center: 0.3,
    proximity: 1.0,
    hf_scale: 1.0,
    rolloff: 0.0001,
    min_distance: 1.0,
    max_distance: 50.0,
    cut_distance: 200.0,
    offset: [0.0, 0.0, 0.0],
    max_delay: 0.2,
    doppler: 1.0,
};

pub const HELICOPTER_EMITTERS: [(&str, V3); 5] = [
    ("Helicopter_Center_BUS", [-2.0, 0.0, 0.0]),
    ("Helicopter_Left_BUS", [1.0, 4.0, 0.0]),
    ("Helicopter_Head_BUS", [7.0, 0.0, 0.0]),
    ("Helicopter_Tail_BUS", [-18.0, 0.0, 0.0]),
    ("Helicopter_Right_BUS", [2.0, -6.0, 0.0]),
];

pub const HELICOPTER: EmitterSettings = EmitterSettings {
    center: 0.3,
    proximity: 1.0,
    hf_scale: 1.0,
    rolloff: 0.0028,
    min_distance: 1.0,
    max_distance: 3000.0,
    cut_distance: 3100.0,
    offset: [0.0, 0.0, 0.0],
    max_delay: 0.2,
    doppler: 1.0,
};

pub const SPEED_OF_SOUND: f32 = 1125.0;

const CUTOFF: [u32; 128] = [
    0x00000000, 0x43B4F791, 0x442979E3, 0x4469B088, 0x448E9CAA, 0x44A343EA, 0x44B3E62E, 0x44C16A48, 0x44CC8991,
    0x44D5D4CE, 0x44DDBB57, 0x44E49418, 0x44EA9F74, 0x44F00F2F, 0x44F509A0, 0x44F9AAF4, 0x44FE07EB, 0x4501189A,
    0x45031949, 0x45050AE0, 0x4506F09C, 0x4508CD57, 0x450AA30B, 0x450C7366, 0x450E3FB0, 0x451008EC, 0x4511CFEB,
    0x45139568, 0x451559FB, 0x45171E27, 0x4518E262, 0x451AA714, 0x451C6C98, 0x451E3356, 0x451FFB90, 0x4521C5A2,
    0x452391DC, 0x4525607A, 0x452731C9, 0x45290612, 0x452ADD9D, 0x452CB8B5, 0x452E979E, 0x45307A9E, 0x453261F6,
    0x45344DE5, 0x45363EA5, 0x45383496, 0x453A3035, 0x453C3165, 0x453E3891, 0x4540466C, 0x45425A77, 0x4544761C,
    0x45469885, 0x4548C332, 0x454AF527, 0x454D307C, 0x454F73E1, 0x4551C0D0, 0x45541783, 0x4556779C, 0x4558E2E6,
    0x455B5944, 0x455DDA9A, 0x45606783, 0x4563026C, 0x4565AA1C, 0x45685F32, 0x456B225D, 0x456DF453, 0x4570D756,
    0x4573CB0D, 0x4576D004, 0x4579E722, 0x457D115B, 0x458027DB, 0x4581D1A5, 0x4583869F, 0x45854767, 0x458714A7,
    0x4588EF13, 0x458AD76E, 0x458CCE83, 0x458ED531, 0x4590EC64, 0x45931518, 0x4595505C, 0x45979F4F, 0x459A0324,
    0x459C7D22, 0x459F0EA4, 0x45A1BA08, 0x45A48441, 0x45A76C20, 0x45AA73A6, 0x45AD9CF8, 0x45B0EA52, 0x45B45E02,
    0x45B8038A, 0x45BBDC85, 0x45BFE893, 0x45C42B5B, 0x45C8A875, 0x45CD7875, 0x45D29C44, 0x45D81261, 0x45DDE086,
    0x45E4272E, 0x45EB0199, 0x45F260D2, 0x45FA4D9B, 0x46019265, 0x460679F2, 0x460BD725, 0x4611C793, 0x4618C4F3,
    0x4620A029, 0x46295F58, 0x46347A9A, 0x4641A5F4, 0x4650E502, 0x4666A946, 0x46822B74, 0x46948AF4, 0x46BED89F,
    0x47179E4C, 0x477A0000,
];

pub fn cutoff(h: f32) -> f32 {
    if h >= 1.0 {
        return f32::from_bits(CUTOFF[127]);
    }
    if h <= 0.0 {
        return 0.0;
    }
    let x = h * 127.0;
    let i = x.floor();
    let f = x - i;
    let i = i as usize;
    let (a, b) = (f32::from_bits(CUTOFF[i]), f32::from_bits(CUTOFF[i + 1]));
    a.mul_add(1.0 - f, b * f)
}

pub fn direct_path(l: &Listener, pos: V3, velocity: V3, s: &EmitterSettings) -> Packet {
    let d = sub(pos, l.pos);
    let r2 = dot(d, d);
    // TODO(guess): exact f32 arithmetic stands for the job's vrsqrtefp / vrefp estimates, XMVectorACos (0x8249EF98) and the vlogefp / vexptefp pow (the direct path 0x83576990: distance, the rolloff gain 0x83573AE8, the air absorption 10^(−0.0054·d) 0x83572218, the pan angle); settle: port the estimate tables (precision only).
    let dist = if r2 == 0.0 { 0.0 } else { r2.sqrt() };
    let a = s.min_distance;
    let clamped = dist.min(s.max_distance).max(a);
    let gain = a * (1.0 / s.rolloff.mul_add(clamped - a, a));
    let mut hf = 10f32.powf(f32::from_bits(0xBBB0_F27C) * clamped);
    if s.hf_scale > 0.0 {
        hf *= 1.0 / s.hf_scale;
    } else {
        hf = 1.0;
    }
    let (mut g, mut h) = if dist > s.cut_distance { (0.0, 0.0) } else { (gain, hf) };
    // TODO(guess): the occlusion by the track's audio geometry (0x83575F88 on the `_3dGeometry` assets) is not ported — factor 1; settle: port the geometry and its rays.
    (g, h) = (g.clamp(0.0, 1.0), h.clamp(0.0, 1.0));
    let (angle, distance) = pan(l, pos);
    let shelf = if a > dist { s.proximity.mul_add(1.0 - dist * (1.0 / a), 1.0) } else { 1.0 };
    let (delay, delay_rate) = delay_path(d, dist, velocity, s);
    Packet {
        center: s.center,
        angle,
        distance,
        gain: g,
        cutoff: cutoff(h),
        shelf,
        delay,
        delay_rate,
        ..Default::default()
    }
}

// TODO(guess): the listener record's factor (+0x64) and velocity (+0x30) taken as 1 and 0 (the emitter defaults 0x83571D34 / 0x8387F160; commit 0x835601B0, update 0x83560FE8 from +0x110, no writer found in 0x83550000…0x83580000 but 0x83560A80; camera listener 0x830B8480); settle: the listener's setter and the record's +0x64 initialiser.
fn delay_path(d: V3, dist: f32, velocity: V3, s: &EmitterSettings) -> (f32, f32) {
    let k = s.doppler;
    let inv_c = 1.0 / SPEED_OF_SOUND;
    let delay = (dist * inv_c * k).min(s.max_delay);
    if dist < 0.001 {
        return (delay, 0.0);
    }
    let n = scale(d, 1.0 / dist);
    let rate = (dot(velocity, n) * inv_c * k).clamp(-0.99, 0.99);
    (delay, rate)
}

pub fn pan(l: &Listener, pos: V3) -> (f32, f32) {
    let d = sub(pos, l.pos);
    let h = dot(d, l.up);
    let p = sub(pos, scale(l.up, h));
    let a = sub(p, l.pos);
    let dir = if a.iter().all(|v| v.abs() < 0.001) { l.forward } else { scale(a, 1.0 / dot(a, a).sqrt()) };
    let c = dot(l.forward, dir).clamp(-1.0, 1.0);
    let theta = c.acos();
    let side = dot(cross(dir, l.forward), l.up);
    let theta = if side > 0.0 { f32::from_bits(0x40C9_0FDB) - theta } else { theta };
    let angle = theta * f32::from_bits(0x4265_2EE0);
    let horizontal = dot(a, a).sqrt();
    (angle, horizontal * (1.0 / (1.0 + h.abs())))
}

pub fn emitter_position(base: V3, q: [f32; 4], offset: V3) -> V3 {
    let u = [q[0], q[1], q[2]];
    let t = cross(u, offset).map(|x| 2.0 * x);
    let c = cross(u, t);
    [0, 1, 2].map(|i| base[i] + (offset[i] + q[3] * t[i] + c[i]))
}

#[derive(Clone, Copy, Debug)]
pub struct Emitter {
    pub bus: &'static str,
    pub settings: EmitterSettings,
    pub base: V3,
    pub rotation: [f32; 4],
    pub velocity: V3,
}

#[derive(Clone, Copy, Debug, Default)]
struct Memory {
    pos: V3,
    gain: f32,
}

#[derive(Debug, Default)]
pub struct Renderer {
    listener: Option<Listener>,
    listener_pos: V3,
    memory: Vec<(&'static str, Memory)>,
    script_listener: Option<Listener>,
    script_listener_pos: V3,
}

fn hear(listener: &mut Option<Listener>, last: &mut V3, camera: Option<(V3, V3)>) -> (Option<Listener>, bool) {
    let mut jumped = false;
    if let Some((eye, look)) = camera {
        let l = Listener::from_camera(eye, look);
        jumped = moved(l.pos, *last);
        *last = l.pos;
        *listener = Some(l);
    }
    (*listener, jumped)
}

fn moved(a: V3, b: V3) -> bool {
    let d = sub(a, b);
    dot(d, d).sqrt() > 10.0
}

impl Renderer {
    pub fn frame(&mut self, camera: Option<(V3, V3)>, emitters: &[Emitter], dt: f32) -> Vec<(&'static str, Packet)> {
        let (listener, jumped) = hear(&mut self.listener, &mut self.listener_pos, camera);
        let Some(l) = listener else { return Vec::new() };
        self.packets(&l, jumped, emitters, dt)
    }

    pub fn frame_scripts(
        &mut self,
        camera: Option<(V3, V3)>,
        emitters: &[Emitter],
        dt: f32,
    ) -> Vec<(&'static str, Packet)> {
        let (listener, jumped) = hear(&mut self.script_listener, &mut self.script_listener_pos, camera);
        let Some(l) = listener else { return Vec::new() };
        self.packets(&l, jumped, emitters, dt)
    }

    fn packets(&mut self, l: &Listener, jumped: bool, emitters: &[Emitter], dt: f32) -> Vec<(&'static str, Packet)> {
        let mut out = Vec::with_capacity(emitters.len());
        for e in emitters {
            let pos = audio_frame(emitter_position(e.base, e.rotation, e.settings.offset));
            let k = match self.memory.iter().position(|(b, _)| *b == e.bus) {
                Some(k) => k,
                None => {
                    self.memory.push((e.bus, Memory::default()));
                    self.memory.len() - 1
                }
            };
            let m = &mut self.memory[k].1;
            let mut p = direct_path(l, pos, audio_frame(e.velocity), &e.settings);
            p.interval = dt;
            p.active = true;
            p.reset = jumped || moved(pos, m.pos) || ((p.gain == 0.0) != (m.gain == 0.0));
            (m.pos, m.gain) = (pos, p.gain);
            out.push((e.bus, p));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener() -> Listener {
        Listener::from_camera([0.0; 3], [0.0, 0.0, -1.0])
    }

    #[test]
    fn listener_frame_flips_z_and_keeps_world_up() {
        let l = listener();
        assert_eq!(l.forward, [0.0, 0.0, 1.0]);
        assert!((l.up[1] - 1.0).abs() < 1e-6 && l.up[0].abs() < 1e-6 && l.up[2].abs() < 1e-6);
        let down = Listener::from_camera([0.0; 3], [0.0, -1.0, 0.0]);
        assert!((dot(down.up, down.forward)).abs() < 1e-6 && (dot(down.up, down.up) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn angles_follow_the_listener_and_the_radius_shrinks_with_height() {
        let l = listener();
        let at = |game: V3| pan(&l, audio_frame(game));
        let (a, r) = at([0.0, 0.0, -5.0]);
        assert!(a.abs() < 1e-3 || (a - 360.0).abs() < 1e-3);
        assert!((r - 5.0).abs() < 1e-4);
        assert!((at([0.0, 0.0, 5.0]).0 - 180.0).abs() < 1e-3);
        assert!((at([1.0, 0.0, -1.0]).0 - 45.0).abs() < 1e-2);
        assert!((at([-1.0, 0.0, -1.0]).0 - 315.0).abs() < 1e-2);
        assert!((at([1.0, 0.0, 0.0]).0 - 90.0).abs() < 1e-2);
        assert!((at([0.0, -3.0, -4.0]).1 - 1.0).abs() < 1e-5);
    }

    fn stereo_energy(game: V3) -> (f32, f32) {
        let mut r = Renderer::default();
        let mut p = super::super::pan3d::Pan3d::new(6, 0.0, 30.0, 110.0);
        let e = Emitter {
            bus: "Avalanche_Center_BUS",
            settings: AVALANCHE,
            base: game,
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
        };
        let cam = Some(([0.0; 3], [0.0, 0.0, -1.0]));
        for _ in 0..2 {
            for (_, packet) in r.frame(cam, &[e], 1.0 / 30.0) {
                p.send(Some(&packet));
            }
        }
        let mut out = vec![0.0; 2 * super::super::BLOCK];
        for k in 0..12 {
            let mut buf = vec![vec![1.0; super::super::BLOCK]];
            p.process(k == 0, 1.0, &mut buf);
            out.fill(0.0);
            crate::sfx_bus::downmix(&buf, &mut out);
        }
        let energy = |c: usize| out.iter().skip(c).step_by(2).map(|x| x * x).sum::<f32>();
        (energy(0), energy(1))
    }

    #[test]
    fn a_source_on_the_cameras_right_is_louder_on_the_right() {
        let (l, r) = stereo_energy([5.0, 0.0, 0.0]);
        assert!(r > 0.0 && l < 1e-6 * r, "straight right: L {l} R {r}");
        let (l, r) = stereo_energy([3.0, 0.0, -3.0]);
        assert!(r > 10.0 * l, "right front: L {l} R {r}");
        let (l, r) = stereo_energy([-5.0, 0.0, 0.0]);
        assert!(l > 0.0 && r < 1e-6 * l, "straight left: L {l} R {r}");
        let (l, r) = stereo_energy([0.0, 0.0, -5.0]);
        assert!((l - r).abs() < 1e-3 * l, "ahead: L {l} R {r}");
    }

    #[test]
    fn rolloff_absorption_and_cut() {
        let l = listener();
        let p = direct_path(&l, audio_frame([0.0, 0.0, -11.0]), [0.0; 3], &BOARD_CENTER);
        assert!((p.gain - 1.0 / (1.0 + 0.0115 * 10.0)).abs() < 1e-5);
        let h = 10f32.powf(-0.0054 * 11.0);
        assert!((p.cutoff - cutoff(h)).abs() < 1e-2 && p.cutoff > 7_900.0 && p.cutoff < 8_000.0);
        assert_eq!(p.shelf, 1.0);
        let near = direct_path(&l, audio_frame([0.0, 0.0, -0.5]), [0.0; 3], &BOARD_CENTER);
        assert!((near.shelf - 1.5).abs() < 1e-5 && (near.gain - 1.0).abs() < 1e-6);
        let far = direct_path(&l, audio_frame([0.0, 0.0, -250.0]), [0.0; 3], &AVALANCHE);
        assert_eq!((far.gain, far.cutoff), (0.0, 0.0));
        assert_eq!(cutoff(1.0), 64_000.0);
        assert_eq!(cutoff(0.0), 0.0);
        let half = cutoff(0.5);
        assert!(half > f32::from_bits(CUTOFF[63]) && half < f32::from_bits(CUTOFF[64]));
    }

    #[test]
    fn delay_and_doppler_rate() {
        let l = listener();
        let p = direct_path(&l, audio_frame([0.0, 0.0, -112.5]), [0.0; 3], &HELICOPTER);
        assert!((p.delay - 0.1).abs() < 1e-6 && p.delay_rate == 0.0);
        let far = direct_path(&l, audio_frame([0.0, 0.0, -1000.0]), [0.0; 3], &HELICOPTER);
        assert_eq!(far.delay, 0.2);
        let away = direct_path(&l, audio_frame([0.0, 0.0, -50.0]), audio_frame([0.0, 0.0, -11.25]), &HELICOPTER);
        assert!((away.delay_rate - 0.01).abs() < 1e-6, "{}", away.delay_rate);
        let fast = direct_path(&l, audio_frame([0.0, 0.0, -50.0]), audio_frame([0.0, 0.0, 5000.0]), &HELICOPTER);
        assert_eq!(fast.delay_rate, -0.99);
    }

    #[test]
    fn emitter_offset_turns_with_the_owner_and_restarts_follow_jumps() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let p = emitter_position([1.0, 2.0, 3.0], [h, 0.0, 0.0, h], [0.0, -1.0, 0.0]);
        assert!((p[0] - 1.0).abs() < 1e-6 && (p[1] - 2.0).abs() < 1e-6 && (p[2] - 2.0).abs() < 1e-6);
        let mut r = Renderer::default();
        let e = Emitter {
            bus: "Board_Center_BUS",
            settings: BOARD_CENTER,
            base: [0.0, 1.0, -5.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
        };
        assert!(r.frame(None, &[e], 1.0 / 30.0).is_empty());
        let cam = Some(([0.0; 3], [0.0, 0.0, -1.0]));
        assert!(r.frame(cam, &[e], 1.0 / 30.0)[0].1.reset, "first packet restarts (gain from 0)");
        assert!(!r.frame(cam, &[e], 1.0 / 30.0)[0].1.reset);
        let far = Emitter { base: [20.0, 1.0, -5.0], ..e };
        assert!(r.frame(cam, &[far], 1.0 / 30.0)[0].1.reset, "a jump of more than 10 m");
    }

    #[test]
    fn script_frames_leave_the_game_listener_alone() {
        let board = Emitter {
            bus: "Board_Center_BUS",
            settings: BOARD_CENTER,
            base: [0.0, 0.0, -5.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
        };
        let script = Emitter { bus: "Script_3D_0_BUS", base: [3.0, 0.0, -2.0], ..board };
        let look = [0.0, 0.0, -1.0];
        let cams = [([0.0; 3], look), ([0.0, 0.0, -50.0], look), ([0.0, 0.0, -51.0], look)];
        let (mut alone, mut shared) = (Renderer::default(), Renderer::default());
        for (k, cam) in cams.into_iter().enumerate() {
            let a = alone.frame(Some(cam), &[board], 1.0 / 60.0);
            let s = shared.frame_scripts(Some(([100.0, 0.0, 0.0], look)), &[script], 1.0 / 60.0);
            assert_eq!(s.len(), 1);
            let b = shared.frame(Some(cam), &[board], 1.0 / 60.0);
            assert_eq!(a, b, "the game's frame {k}");
        }
    }
}
