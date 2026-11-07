use super::{Clip, Interpolation, Keys, Path};
use crate::hash::ContentHash;

pub const DOMAIN: &[u8] = b"project-avalanche/anim-sim/1\n";

struct Enc(blake3::Hasher);

impl Enc {
    fn u64(&mut self, x: u64) {
        self.0.update(&x.to_le_bytes());
    }
    fn text(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.0.update(s.as_bytes());
    }
    fn floats(&mut self, xs: &[f32]) {
        self.u64(xs.len() as u64);
        for x in xs {
            self.0.update(&x.to_bits().to_le_bytes());
        }
    }
    fn keys(&mut self, k: &Keys) {
        self.u64(match k.interpolation {
            Interpolation::Step => 0,
            Interpolation::Linear => 1,
            Interpolation::CubicSpline => 2,
        });
        self.floats(&k.times);
        self.floats(&k.values);
    }
}

pub fn sim_hash(clip: &Clip) -> ContentHash {
    let mut e = Enc(blake3::Hasher::new());
    e.0.update(DOMAIN);
    e.u64(u64::from(clip.additive));
    let mut joints: Vec<_> = clip.joints.iter().collect();
    joints.sort_by(|a, b| (&a.joint, a.path).cmp(&(&b.joint, b.path)));
    e.u64(joints.len() as u64);
    for t in joints {
        e.text(&t.joint);
        e.u64(match t.path {
            Path::Rotation => 0,
            Path::Translation => 1,
            Path::Scale => 2,
        });
        e.keys(&t.keys);
    }
    let mut floats: Vec<_> = clip.floats.iter().collect();
    floats.sort_by(|a, b| a.dof.cmp(&b.dof));
    e.u64(floats.len() as u64);
    for t in floats {
        e.text(&t.dof);
        e.keys(&t.keys);
    }
    ContentHash(*e.0.finalize().as_bytes())
}
