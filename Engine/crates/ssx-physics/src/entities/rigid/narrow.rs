#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]

use glam::Vec3;

use super::features::{self, Feature, Pairs};
use super::fixup;

pub const PARALLEL: f32 = 0.999;
pub const MIN_NORMAL_SQ: f32 = 1e-8;
pub const EPS30: f32 = 1e-30;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CylinderGp {
    pub center: Vec3,
    pub axis: Vec3,
    pub half_height: f32,
    pub radius: f32,
    pub fat: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleGp {
    pub v: [Vec3; 3],
    pub radius: f32,
    pub flags: u32,
    pub edge_cos: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifold {
    pub normal: Vec3,
    pub points: Vec<(Vec3, Vec3)>,
}

pub(super) fn rsqrt(x: f32) -> f32 {
    if x == 0.0 { f32::INFINITY } else { 1.0 / x.sqrt() }
}

pub(super) fn scale_nan(v: Vec3, s: f32) -> Vec3 {
    let m = |x: f32| if s.is_infinite() && x == 0.0 { f32::NAN } else { x * s };
    Vec3::new(m(v.x), m(v.y), m(v.z))
}

pub(super) fn tri_interval(d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (f32, f32) {
    let p = [d.dot(a), d.dot(b), d.dot(c)];
    (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]))
}

pub(super) fn cyl_interval(d: Vec3, axis: Vec3, hh: f32, r: f32) -> (f32, f32) {
    let k = axis.dot(d).abs();
    let s2 = (1.0 - k * k).max(0.0);
    let e = if s2 == 0.0 { 0.0 } else { s2.sqrt() } * r + k * hh;
    (-e, e)
}

fn unit_or_raw(v: Vec3) -> Vec3 {
    let l2 = v.length_squared();
    if EPS30 > l2 { v } else { v / l2.sqrt() }
}

fn circle_point(c: Vec3, n: Vec3, r: f32, p: Vec3) -> Vec3 {
    let d = p - c;
    c + unit_or_raw(d - n * d.dot(n)) * r
}

fn circle_out(c: Vec3, n: Vec3, e: Vec3, x: Vec3, p: Vec3) -> Vec3 {
    let px = p - x;
    let l2 = px.length_squared();
    if l2 > EPS30 {
        return px / l2.sqrt();
    }
    let w = (x - c).cross(n).cross(e);
    let w2 = w.length_squared();
    if w2 > EPS30 { w / w2.sqrt() } else { n }
}

pub(super) fn circle_segment(c: Vec3, n: Vec3, r: f32, p0: Vec3, p1: Vec3) -> Vec3 {
    let e = p1 - p0;
    let (p_high, p_low) = if 0.0 > e.dot(n) { (p0, p1) } else { (p1, p0) };
    let m = n.cross(e).cross(n);
    let d_low = p_low - c;
    let proj_low = d_low - n * d_low.dot(n);
    let em = e.dot(m);
    let inv = if em > 0.0 { 1.0 / em } else { em };
    let t = (c - p0).dot(m) * inv;
    let q = if t > 0.0 && 1.0 > t { p0 + e * t } else { p_high };
    let dq = q - c;
    let hq = dq.dot(n);
    let xq = c + unit_or_raw(dq - n * hq) * r;
    let x_low = c + unit_or_raw(proj_low) * r;
    let gq = (xq - q).dot(e);
    let g_low = (x_low - p_low).dot(e);
    if gq * g_low > 0.0 {
        let (mut x, mut p) = (x_low, p_low);
        if 0.0 > hq {
            let s_vec = p_low - q;
            let s2 = s_vec.length_squared();
            if s2 > 0.0 {
                let inv2 = 1.0 / s2;
                let mut prev = 2.0f32;
                let mut ok = false;
                for _ in 0..20 {
                    let s = (x - q).dot(s_vec) * inv2;
                    if !(s >= 0.0 && 1.0 >= s) {
                        ok = false;
                        break;
                    }
                    p = q + s_vec * s;
                    ok = true;
                    if 1e-4 > (s - prev).abs() {
                        break;
                    }
                    prev = s;
                    x = circle_point(c, n, r, p);
                }
                if ok {
                    return circle_out(c, n, e, x, p);
                }
            }
        }
        if g_low.abs() > gq.abs() {
            (x, p) = (xq, q);
        }
        return circle_out(c, n, e, x, p);
    }
    let (mut p_max, mut x_max, mut g_max, mut p_min, mut x_min, mut g_min) =
        if g_low > gq { (p_low, x_low, g_low, q, xq, gq) } else { (q, xq, gq, p_low, x_low, g_low) };
    let mut w = 1.0f32;
    for _ in 0..10 {
        if !(g_max > g_min) {
            break;
        }
        let den1 = g_max - g_min;
        let den2 = w + 1.0;
        w *= 0.5;
        let lam = (g_max / den1 + w) / den2;
        let p = p_min * lam + p_max * (1.0 - lam);
        let x = circle_point(c, n, r, p);
        let g = (x - p).dot(e);
        if 0.0 > g {
            (p_min, x_min, g_min) = (p, x, g);
        } else {
            (p_max, x_max, g_max) = (p, x, g);
        }
    }
    if g_max > -g_min { circle_out(c, n, e, x_min, p_min) } else { circle_out(c, n, e, x_max, p_max) }
}

fn edge_axis(a: Vec3, e: Vec3, pa: Vec3, pb: Vec3) -> Vec3 {
    let w = a.cross(e);
    let w2 = w.length_squared();
    if !(MIN_NORMAL_SQ > w2) {
        let u = a.cross(w);
        let s = -u.dot(pa) / u.dot(e);
        if !(s > 1.0 || 0.0 > s) {
            return scale_nan(w, rsqrt(w2));
        }
        let p = if s > 1.0 { pb } else { pa };
        let rad = a.cross(p.cross(a));
        return scale_nan(rad, rsqrt(rad.length_squared()));
    }
    let mut v = a.cross(pa.cross(e));
    if MIN_NORMAL_SQ > v.length_squared() {
        let unit = if 0.5 > a.x.abs() { Vec3::X } else { Vec3::Y };
        v = unit.cross(a);
    }
    scale_nan(v, rsqrt(v.length_squared()))
}

fn pick(lo: f32, hi: f32, d: Vec3) -> (f32, Vec3) {
    if lo > hi { (lo, d) } else { (hi, -d) }
}

pub fn separating_axis(a: Vec3, hh: f32, r: f32, t: [Vec3; 3]) -> (f32, Vec3) {
    let [a0, a1, a2] = t;
    let ts = [a.dot(a1), a.dot(a0), a.dot(a2)];
    let tmax = ts[0].max(ts[1]).max(ts[2]);
    let tmin = ts[0].min(ts[1]).min(ts[2]);
    let (mut best, mut axis) = pick(-hh - tmax, tmin - hh, a);
    let (e0, e1, e2) = (a1 - a0, a2 - a1, a0 - a2);
    let n = e0.cross(e1);
    let n2 = n.length_squared();
    if n2 > MIN_NORMAL_SQ {
        let nn = n / n2.sqrt();
        let an = a.dot(nn);
        let (pmin, pmax) = tri_interval(nn, a0, a1, a2);
        let s2 = (1.0 - an * an).max(0.0);
        let ext = if s2 == 0.0 { 0.0 } else { s2.sqrt() } * r + an.abs() * hh;
        let (s, d) = pick(-ext - pmax, pmin - ext, nn);
        if s > best {
            (best, axis) = (s, d);
        }
    }
    for (e, pa, pb) in [(e0, a0, a1), (e1, a1, a2), (e2, a2, a0)] {
        let d = edge_axis(a, e, pa, pb);
        let (tmin, tmax) = tri_interval(d, a0, a1, a2);
        let (s, d) = pick(-r - tmax, tmin - r, d);
        if s > best {
            (best, axis) = (s, d);
        }
    }
    for cap in [1.0f32, -1.0] {
        let nc = a * cap;
        let cc = nc * hh;
        for (p0, p1) in [(a0, a1), (a1, a2), (a2, a0)] {
            let d = circle_segment(cc, nc, r, p0, p1);
            let (tmin, tmax) = tri_interval(d, a0, a1, a2);
            let (cmin, cmax) = cyl_interval(d, a, hh, r);
            let (s, d) = pick(cmin - tmax, tmin - cmax, d);
            if s > best {
                (best, axis) = (s, d);
            }
        }
    }
    (best, axis)
}

fn cylinder_feature(d: Vec3, c: &CylinderGp) -> Feature {
    let nd = -d;
    let ad = nd.dot(c.axis);
    let cap_n = if 0.0 > ad { -c.axis } else { c.axis };
    let cap_c = c.center + cap_n * c.half_height;
    let k2 = ad * ad;
    if k2 >= PARALLEL {
        return Feature::Disc { center: cap_c, normal: cap_n, radius: c.radius };
    }
    let v = nd - c.axis * ad;
    let p = scale_nan(v, rsqrt(v.length_squared())) * c.radius + cap_c;
    if (1.0 - PARALLEL) >= k2 {
        return Feature::Segment(p, p - cap_n * (2.0 * c.half_height));
    }
    Feature::Point(p)
}

fn triangle_feature(d: Vec3, v: [Vec3; 3]) -> Vec<Vec3> {
    let [v0, v1, v2] = v;
    let (e0, e1, e2) = (v1 - v0, v2 - v1, v0 - v2);
    let nt = e0.cross(e1);
    let tol = 1.0 - PARALLEL;
    if tol * nt.length_squared() > nt.cross(d).length_squared() {
        return if 0.0 > nt.dot(d) { vec![v0, v2, v1] } else { vec![v0, v1, v2] };
    }
    let (d0, d1, d2) = (e0.dot(d), e1.dot(d), e2.dot(d));
    let (q0, q1, q2) = (d0 * d0, d1 * d1, d2 * d2);
    let (m0, m1, m2) = (tol * e0.length_squared(), tol * e1.length_squared(), tol * e2.length_squared());
    if m0 >= q0 && q1 >= q0 && q2 >= q0 && 0.0 >= d1 {
        return vec![v0, v1];
    }
    if m1 >= q1 && q2 >= q1 && q0 >= q1 && 0.0 >= d2 {
        return vec![v1, v2];
    }
    if m2 >= q2 && q0 >= q2 && q1 >= q2 && 0.0 >= d0 {
        return vec![v2, v0];
    }
    if 0.0 >= d0 && !(0.0 >= d2) {
        return vec![v0];
    }
    if 0.0 >= d1 && !(0.0 >= d0) {
        return vec![v1];
    }
    vec![v2]
}

pub fn cylinder_triangle(c: &CylinderGp, t: &TriangleGp, tolerance: f32) -> Option<Manifold> {
    if t.flags & 0x10 != 0 && t.flags & 0x100 != 0 && fixup::early_reject(c, t) {
        return None;
    }
    let pad = (tolerance + c.fat) + t.radius;
    let rel = t.v.map(|x| x - c.center);
    let (best, d) = separating_axis(c.axis, c.half_height, c.radius, rel);
    if best > pad {
        return None;
    }
    let fa = cylinder_feature(d, c);
    let fb = triangle_feature(d, t.v);
    let pairs: Pairs = features::intersect(d, &fa, &fb, PARALLEL);
    let (mut n, mut sep) = (d, best);
    if pairs.len() < 3 {
        let (mut delta, mut dd) = (Vec3::ZERO, 0.0f32);
        if let Some(&(cp, tp)) = pairs.first() {
            delta = cp - tp;
            dd = delta.dot(d);
            if let Some(&(cp1, tp1)) = pairs.get(1) {
                let d1 = cp1 - tp1;
                let dd1 = d1.dot(d);
                if dd > dd1 {
                    (dd, delta) = (dd1, d1);
                }
            }
        }
        if dd > 0.0 {
            sep = dd;
            let l2 = delta.length_squared();
            if l2 > MIN_NORMAL_SQ {
                let l = if l2 == 0.0 { 0.0 } else { l2.sqrt() };
                if l > dd {
                    sep = l;
                    n = delta / l;
                }
            }
        }
    }
    if sep > pad {
        return None;
    }
    let m = -n;
    if !fixup::accept(m, t) {
        return None;
    }
    let points = pairs.iter().map(|&(cp, tp)| (tp + n * t.radius, cp - n * c.fat)).collect();
    Some(Manifold { normal: m, points })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRI: [Vec3; 3] = [Vec3::new(-50.0, 0.0, -50.0), Vec3::new(-50.0, 0.0, 50.0), Vec3::new(50.0, 0.0, 0.0)];

    fn query(center: Vec3, axis: Vec3) -> Option<Manifold> {
        let c = CylinderGp { center, axis, half_height: 0.18, radius: 0.04, fat: 0.0 };
        let t = TriangleGp { v: TRI, radius: 0.0, flags: 0x12, edge_cos: [0.0; 3] };
        cylinder_triangle(&c, &t, 0.1)
    }

    #[test]
    fn lying_tilted_and_standing_cylinders_touch_as_the_game_s() {
        let m = query(Vec3::new(0.0, 0.09, 0.0), Vec3::X).unwrap();
        assert_eq!(m.points.len(), 2);
        assert!(m.normal.abs_diff_eq(-Vec3::Y, 1e-6));
        for (t, c) in &m.points {
            assert!((t.x.abs() - 0.18).abs() < 1e-5 && t.y.abs() < 1e-6 && (c.y - 0.05).abs() < 1e-5);
        }
        let a = Vec3::new(20f32.to_radians().cos(), 20f32.to_radians().sin(), 0.0);
        let ext = a.y * 0.18 + (1.0 - a.y * a.y).sqrt() * 0.04;
        let m = query(Vec3::new(0.0, ext, 0.0), a).unwrap();
        assert_eq!(m.points.len(), 1);
        assert!((m.points[0].0.x + 0.1555).abs() < 1e-3);
        let m = query(Vec3::new(0.0, 0.18, 0.0), Vec3::Y).unwrap();
        assert_eq!(m.points.len(), 4);
        assert!(query(Vec3::new(0.0, 0.2, 0.0), Vec3::X).is_none(), "0.16 m above: farther than 0.1");
    }

    #[test]
    fn a_one_sided_triangle_rejects_a_cylinder_beneath_it() {
        assert!(query(Vec3::new(0.0, -0.09, 0.0), Vec3::X).is_none());
    }
}
