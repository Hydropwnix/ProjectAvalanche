use super::*;

#[derive(Clone, Copy, Debug)]
pub struct TriHit {
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

pub fn particle_triangle(p: Vec3, delta: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, Vec3, Vec3)> {
    let (e1, e2, s) = (b - a, c - a, p - a);
    let pv = delta.cross(e2);
    let det = e1.dot(pv);
    if det <= 0.0 {
        return None;
    }
    let qv = s.cross(e1);
    let (u_num, v_num) = (s.dot(pv), delta.dot(qv));
    let inv = 1.0 / det;
    let (w, u, v) = ((det - u_num - v_num) * inv, u_num * inv, v_num * inv);
    let t = e2.dot(qv) * inv;
    if (t - 0.5).abs() >= 0.5 || w < 0.0 || u < 0.0 || v < 0.0 {
        return None;
    }
    let n = e1.cross(e2);
    Some((t, p + delta * t, n / n.length()))
}

pub fn face_normal(a: Vec3, b: Vec3, c: Vec3) -> Option<Vec3> {
    (b - a).cross(c - a).try_normalize()
}

pub fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    const BARY_TOL: f32 = 5e-4;
    let (e1, e2) = (b - a, c - a);
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det <= 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if u < -BARY_TOL {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -BARY_TOL || 1.0 - u - v < -BARY_TOL {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some((t, u, v))
}

pub fn closest_point_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> (Vec3, [f32; 3]) {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, [1.0, 0.0, 0.0]);
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return (b, [0.0, 1.0, 0.0]);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (a + ab * v, [1.0 - v, v, 0.0]);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return (c, [0.0, 0.0, 1.0]);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (a + ac * w, [1.0 - w, 0.0, w]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (b + (c - b) * w, [0.0, 1.0 - w, w]);
    }
    let denom = va + vb + vc;
    if denom.abs() < f32::MIN_POSITIVE {
        return (a, [1.0, 0.0, 0.0]);
    }
    let (v, w) = (vb / denom, vc / denom);
    (a + ab * v + ac * w, [1.0 - v - w, v, w])
}

#[allow(clippy::too_many_arguments)]
pub fn sweep_sphere_triangle(
    o: Vec3,
    d: Vec3,
    max_t: f32,
    r: f32,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    two_sided: bool,
) -> Option<TriHit> {
    let (o, b0, c0) = (o - a, b - a, c - a);
    let a0 = Vec3::ZERO;
    let n = face_normal(a0, b0, c0)?;
    let dn = d.dot(n);
    if !two_sided && dn >= 0.0 {
        return None;
    }
    let raw = |center: Vec3, p: Vec3, side: f32| (center - p).try_normalize().unwrap_or(n * side);
    let s0 = o.dot(n);
    let side0 = if s0 >= 0.0 { 1.0 } else { -1.0 };
    let (cp0, _) = closest_point_triangle(o, a0, b0, c0);
    if (o - cp0).length_squared() <= r * r {
        return Some(TriHit { t: 0.0, point: cp0 + a, normal: raw(o, cp0, side0) });
    }
    if s0.abs() >= r {
        let toward = -side0 * dn;
        if toward > 0.0 {
            let t = (s0.abs() - r) / toward;
            if t <= max_t {
                let center = o + d * t;
                let p = center - n * (side0 * r);
                let (q, _) = closest_point_triangle(p, a0, b0, c0);
                if q.distance_squared(p) < 1e-10 * (1.0 + p.length_squared()) {
                    return Some(TriHit { t, point: p + a, normal: n * side0 });
                }
            }
        }
    }
    let mut best: Option<(f32, Vec3)> = None;
    let mut consider = |t: f32, p: Vec3| {
        if (0.0..=max_t).contains(&t) && best.is_none_or(|(bt, _)| t < bt) {
            best = Some((t, p));
        }
    };
    for v in [a0, b0, c0] {
        if let Some(t) = ray_sphere(o, d, v, r) {
            consider(t, v);
        }
    }
    for (p, q) in [(a0, b0), (b0, c0), (c0, a0)] {
        if let Some((t, s)) = ray_capsule_side(o, d, p, q, r) {
            consider(t, p + (q - p) * s);
        }
    }
    best.map(|(t, p)| TriHit { t, point: p + a, normal: raw(o + d * t, p, side0) })
}

pub(super) fn ray_sphere(o: Vec3, d: Vec3, center: Vec3, r: f32) -> Option<f32> {
    let m = o - center;
    let b = m.dot(d);
    let c = m.length_squared() - r * r;
    if c > 0.0 && b > 0.0 {
        return None;
    }
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    Some((-b - disc.sqrt()).max(0.0))
}

pub(super) fn ray_capsule_side(o: Vec3, d: Vec3, p: Vec3, q: Vec3, r: f32) -> Option<(f32, f32)> {
    let e = q - p;
    let len = e.length();
    if len < 1e-6 {
        return None;
    }
    let axis = e / len;
    let m = o - p;
    let m_perp = m - axis * m.dot(axis);
    let d_perp = d - axis * d.dot(axis);
    let a = d_perp.length_squared();
    if a < 1e-9 {
        return None;
    }
    let b = m_perp.dot(d_perp);
    let c = m_perp.length_squared() - r * r;
    if c > 0.0 && b > 0.0 {
        return None;
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    if t < 0.0 {
        return None;
    }
    let s = (m + d * t).dot(axis) / len;
    (0.0..=1.0).contains(&s).then_some((t, s))
}
