#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]

use glam::Vec3;

use super::narrow::{CylinderGp, TriangleGp, rsqrt};

pub const FACE_COS: f32 = 0.99985;
pub const EDGE_COS: f32 = 0.05;

pub fn region(m: Vec3, v: [Vec3; 3]) -> [bool; 3] {
    let e = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    let nt = e[0].cross(e[1]);
    let nt2 = nt.length_squared();
    let mn = m.dot(nt);
    if mn * mn > FACE_COS * FACE_COS * nt2 {
        return [true; 3];
    }
    let perp2 = (nt2 - mn * mn) * EDGE_COS * EDGE_COS;
    let s: [f32; 3] = std::array::from_fn(|i| {
        let d = m.dot(e[i]);
        d * d.abs() * nt2
    });
    let l: [f32; 3] = std::array::from_fn(|i| e[i].length_squared() * perp2);
    let vert: [bool; 3] = std::array::from_fn(|i| s[i] > l[i] && -s[(i + 2) % 3] > l[(i + 2) % 3]);
    if vert.iter().any(|&b| b) {
        return vert;
    }
    let ee = e.map(|x| x.length_squared());
    let t: [bool; 3] = std::array::from_fn(|i| -s[(i + 2) % 3] * ee[i] > s[i] * ee[(i + 2) % 3]);
    let u: [bool; 3] = std::array::from_fn(|i| t[i] && !t[(i + 1) % 3]);
    std::array::from_fn(|i| u[i] || u[(i + 2) % 3])
}

pub fn accept(m: Vec3, t: &TriangleGp) -> bool {
    let v = t.v;
    let reg = region(m, v);
    let flags = t.flags;
    let one_sided = flags & 0x10 != 0;
    let convex: [bool; 3] = std::array::from_fn(|i| flags & (0x20 << i) != 0);
    let vdis: [bool; 3] = std::array::from_fn(|i| flags & (0x200 << i) != 0);
    let e = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    let nt = e[0].cross(e[1]);
    let mn = m.dot(nt);
    let face = reg.iter().all(|&b| b);
    let edge: [bool; 3] = std::array::from_fn(|i| reg[i] && reg[(i + 1) % 3]);
    let any_edge = edge.iter().any(|&b| b);
    if flags & 0x100 == 0 {
        if one_sided && mn > 0.0 {
            return false;
        }
        if face {
            return true;
        }
        if (0..3).any(|i| edge[i] && !convex[i]) {
            return false;
        }
        if any_edge {
            return true;
        }
        return !(0..3).any(|i| reg[i] && vdis[i]);
    }
    // TODO(guess): the fix-up's two-sided branches with edge cosines (0x82E3E410 …) are not ported (no such triangle reaches a flare: every triangle a flare meets is one-sided — the terrain's flags 0x12, without cosines, and every `.pob` mesh's 0x113, ClusteredMesh flags 0x10 on all 273); settle: port those branches if one ever does.
    if !one_sided {
        crate::guess_hit!();
        return true;
    }
    if face {
        return !(mn > 0.0);
    }
    let vert = if any_edge { [false; 3] } else { reg };
    if (0..3).any(|i| vert[i] && vdis[i]) {
        return false;
    }
    let nt2 = nt.length_squared();
    let sq = |x: f32| x * x.abs();
    let outward = |i: usize| 0.0 > m.dot(e[i].cross(nt));
    if let Some(i) = (0..3).find(|&i| edge[i]) {
        if !convex[i] {
            return false;
        }
        let c = t.edge_cos[i];
        return !(sq(mn) > -sq(c) * nt2 && outward(i));
    }
    for (vi, pair) in [(0usize, [0usize, 2]), (1, [0, 1]), (2, [2, 1])] {
        if vert[vi] {
            for i in pair {
                let ee = e[i].length_squared();
                let cone_out = if convex[i] {
                    let me = m.dot(e[i]);
                    sq(mn) * ee > sq(t.edge_cos[i]) * (me * me - ee) * nt2
                } else {
                    true
                };
                if cone_out && outward(i) {
                    return false;
                }
            }
            return true;
        }
    }
    true
}

pub fn early_reject(c: &CylinderGp, t: &TriangleGp) -> bool {
    let v = t.v;
    let e = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    let nt = e[0].cross(e[1]);
    let ec = t.edge_cos;
    let mut sw = ec;
    if e[0].dot(e[1]) > 0.0 {
        sw = [ec[1], ec[0], ec[2]];
    }
    if e[2].dot(e[0]) > 0.0 {
        sw = [ec[2], ec[1], ec[0]];
    }
    if e[1].dot(e[2]) > 0.0 {
        sw = [ec[0], ec[2], ec[1]];
    }
    let cs: [f32; 3] = std::array::from_fn(|i| ec[i].min(sw[i]));
    let mut planes = [(nt, v[0]); 4];
    for (k, (i, p)) in [(0usize, v[0]), (1, v[2]), (2, v[2])].into_iter().enumerate() {
        let x = e[i].length_squared() * (1.0 - cs[i] * cs[i]);
        let sn = if x == 0.0 { 0.0 } else { x * rsqrt(x) };
        planes[k + 1] = (nt * sn - e[i].cross(nt) * cs[i], p);
    }
    planes.iter().any(|&(nk, p)| {
        let an = c.axis.dot(nk);
        let w = nk.length_squared() - an;
        let ext = if w == 0.0 { 0.0 } else { w * rsqrt(w) } * c.radius + (c.half_height * an).abs();
        nk.dot(p - c.center) > ext
    })
}
