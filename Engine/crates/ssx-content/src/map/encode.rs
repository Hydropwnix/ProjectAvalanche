use super::*;
use crate::hash::ContentHash;

pub const DOMAIN: &[u8] = b"project-avalanche/map-sim/1\n";

struct Enc(blake3::Hasher);

impl Enc {
    fn u64(&mut self, x: u64) {
        self.0.update(&x.to_le_bytes());
    }
    fn f(&mut self, x: f32) {
        self.0.update(&x.to_bits().to_le_bytes());
    }
    fn fs(&mut self, xs: &[f32]) {
        xs.iter().for_each(|&x| self.f(x));
    }
    fn text(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.0.update(s.as_bytes());
    }
    fn id(&mut self, id: &Option<ContentId>) {
        self.text(&id.as_ref().map(ToString::to_string).unwrap_or_default());
    }
    fn placement(&mut self, p: &Placement) {
        self.fs(&p.scale);
        self.fs(&p.rotation);
        self.fs(&p.position);
    }
    fn volume(&mut self, v: &Volume) {
        self.text(&v.name);
        self.u64(u64::from(v.order));
        match v.shape {
            VolumeShape::Box { centre, rotation, half_extents } => {
                self.u64(1);
                self.fs(&centre);
                self.fs(&rotation);
                self.fs(&half_extents);
            }
            VolumeShape::Sphere { centre, radius } => {
                self.u64(2);
                self.fs(&centre);
                self.f(radius);
            }
        }
    }
    fn collision(&mut self, list: &[CollisionMesh]) {
        self.u64(list.len() as u64);
        for c in list {
            self.u64(c.triangles.len() as u64);
            for t in &c.triangles {
                t.corners.iter().for_each(|p| self.fs(p));
                self.u64(u64::from(t.surface_id));
                self.u64(u64::from(t.group));
            }
        }
    }
}

pub fn sim_hash(map: &Map) -> ContentHash {
    let mut e = Enc(blake3::Hasher::new());
    e.0.update(DOMAIN);
    let t = &map.toml;
    e.id(&t.base);
    e.u64(t.modes.len() as u64);
    t.modes.iter().for_each(|m| e.text(m.name()));
    e.f(t.time_limit_s.unwrap_or(0.0));
    e.u64(u64::from(t.intro == Intro::Heli));
    e.id(&t.lighting);
    let d = &map.definition;
    e.u64(d.terrain.len() as u64);
    for m in &d.terrain {
        e.u64(m.positions.len() as u64);
        m.positions.iter().for_each(|p| e.fs(p));
        e.u64(m.triangles.len() as u64);
        m.triangles.iter().flatten().for_each(|&v| e.u64(u64::from(v)));
        m.weights.iter().for_each(|w| e.fs(w));
    }
    e.collision(&d.collision);
    e.collision(&d.rails);
    let objects: Vec<&Prop> = d.props.iter().filter(|p| matches!(p.look, PropLook::Object(_))).collect();
    e.u64(objects.len() as u64);
    for p in objects {
        if let PropLook::Object(id) = &p.look {
            e.text(&id.to_string());
        }
        e.placement(&p.placement);
        e.f(p.ao);
    }
    e.u64(d.spawns.len() as u64);
    for s in &d.spawns {
        e.u64(u64::from(s.slot));
        e.fs(&s.position);
        e.fs(&s.rotation);
    }
    for list in [&d.checkpoints, &d.triggers, &d.deaths] {
        e.u64(list.len() as u64);
        list.iter().for_each(|v| e.volume(v));
    }
    e.u64(u64::from(d.finish.is_some()));
    if let Some(f) = &d.finish {
        e.volume(f);
    }
    let area = d.boundary.as_ref().map_or(&[][..], |b| b.triangles.as_slice());
    e.u64(u64::from(d.boundary.is_some()));
    e.u64(area.len() as u64);
    area.iter().flatten().for_each(|p| e.fs(p));
    e.u64(d.heli_path.len() as u64);
    d.heli_path.iter().for_each(|p| {
        e.fs(&p.position);
        e.fs(&p.rotation);
    });
    ContentHash(*e.0.finalize().as_bytes())
}
