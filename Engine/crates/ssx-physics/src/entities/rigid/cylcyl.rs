#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]

use glam::Vec3;

use super::features::{self, Feature, Pairs};
use super::narrow::{CylinderGp, EPS30, MIN_NORMAL_SQ, Manifold, PARALLEL, circle_segment, cyl_interval};

const FLT_MIN: f32 = f32::MIN_POSITIVE;

fn unit_or_raw(v: Vec3) -> Vec3 {
    let l2 = v.length_squared();
    if EPS30 > l2 { v } else { v / l2.sqrt() }
}

fn in_plane(v: Vec3, n: Vec3) -> Vec3 {
    v - n * v.dot(n)
}

pub fn circle_circle(c1: Vec3, n1: Vec3, r1: f32, c2: Vec3, n2: Vec3, r2: f32, h: Vec3) -> Option<Vec3> {
    let eps = r1.min(r2) * 1e-7;
    if FLT_MIN > eps {
        return None;
    }
    let p1 = c1 - h.cross(n1) * r1;
    let q = in_plane(p1 - c2, n2);
    if r2 * r2 > q.length_squared() {
        return None;
    }
    let p2 = c2 + h.cross(n2) * r2;
    let v57 = p2 - c1;
    let w = n1.cross(v57.cross(n1));
    if r1 * r1 > w.length_squared() {
        return None;
    }
    let d12 = c1 - c2;
    if eps > d12.dot(h).abs() {
        let x1 = c1 + unit_or_raw(in_plane(v57, n1)) * r1;
        let x2 = c2 + unit_or_raw(in_plane(x1 - c2, n2)) * r2;
        let d = x2 - x1;
        if d.length_squared() > FLT_MIN {
            return Some(d.normalize());
        }
        return Some((n1 + n2).cross(h).normalize());
    }
    let mut v29 = c1;
    let dd = d12.length_squared();
    if (r1 + r2) * (r1 + r2) > dd && dd > FLT_MIN {
        let dl = dd.sqrt();
        let s = (dl + r1 + r2) * 0.5;
        let a2 = s * (s - dl) * (s - r1) * (s - r2);
        let area = if a2 == 0.0 { 0.0 } else { a2.sqrt() };
        let sin = 2.0 * area / dl / r1;
        v29 = c1 + (p1 - c1) * sin;
    }
    let g = |y: Vec3, x: Vec3| (y - c2).dot(n2.cross(x - c2));
    let xa = c1 + unit_or_raw(in_plane(v57, n1)) * r1;
    let yb = c2 + unit_or_raw(in_plane(v29 - c2, n2)) * r2;
    let xc = c1 + unit_or_raw(in_plane(yb - c1, n1)) * r1;
    let (ga, gb) = (g(p2, xa), g(yb, xc));
    let mut a = (xa, p2, ga);
    let b = (xc, yb, gb);
    if ga * gb > 0.0 {
        let p2b = c2 + c2 - p2;
        let xa2 = c1 + unit_or_raw(in_plane(p2b - c1, n1)) * r1;
        let ga2 = g(p2b, xa2);
        if ga2 * gb > 0.0 {
            return None;
        }
        a = (xa2, p2b, ga2);
    }
    let ((mut xa, mut ya, mut ga), (mut xb, mut yb, mut gb)) = if a.2 > b.2 { (b, a) } else { (a, b) };
    let mut w = 1.0f32;
    let mut it = 0;
    let (x, y) = loop {
        if gb > -ga {
            if ga > -eps || it == 10 {
                break (xa, ya);
            }
        } else if eps > gb || it == 10 {
            break (xb, yb);
        }
        let lam = (gb / (gb - ga) + w * 0.5) / (w + 1.0);
        let yi = ya * lam + yb * (1.0 - lam);
        let yn = c2 + (yi - c2).normalize() * r2;
        let xn = c1 + unit_or_raw(in_plane(yn - c1, n1)) * r1;
        let gn = g(yn, xn);
        it += 1;
        w *= 0.5;
        if 0.0 > gn {
            (xa, ya, ga) = (xn, yn, gn);
        } else {
            (xb, yb, gb) = (xn, yn, gn);
        }
    };
    let d = y - x;
    if d.length_squared() > EPS30 {
        return Some(d.normalize());
    }
    let c = (x - c1).cross(n1).cross((y - c2).cross(n2));
    Some(if c.length_squared() > EPS30 { c.normalize() } else { n1 })
}

fn extent(d: Vec3, c: &CylinderGp) -> f32 {
    cyl_interval(d, c.axis, c.half_height, c.radius).1
}

fn score(d: Vec3, dab: Vec3, a: &CylinderGp, b: &CylinderGp) -> (f32, Vec3) {
    let (ea, eb) = (extent(d, a), extent(d, b));
    let s = dab.dot(d);
    let sep1 = (s - eb) - ea;
    let sep2 = -ea - (s + eb);
    if sep2 > sep1 { (sep2, d) } else { (sep1, -d) }
}

fn score_intervals(d: Vec3, dab: Vec3, a: &CylinderGp, b: &CylinderGp) -> (f32, Vec3) {
    let (ea, eb) = (extent(d, a), extent(d, b));
    let s = dab.dot(d);
    let x = -ea - (s + eb);
    let y = (s - eb) - ea;
    if x > y { (x, d) } else { (y, -d) }
}

fn rim_segment(axis: Vec3, hh: f32, r: f32, p0: Vec3, p1: Vec3) -> Vec3 {
    let e = p1 - p0;
    let n = if 0.0 > e.cross(e.cross(p0)).dot(axis) { axis } else { -axis };
    circle_segment(n * hh, n, r, p0, p1)
}

pub fn separating_axis(a: &CylinderGp, b: &CylinderGp) -> (f32, Vec3) {
    let (aa, ab) = (a.axis, b.axis);
    let dab = b.center - a.center;
    let (mut best, mut axis) = score(aa, dab, a, b);
    let take = |c: (f32, Vec3), best: &mut f32, axis: &mut Vec3| {
        if c.0 > *best {
            (*best, *axis) = c;
        }
    };
    take(score(ab, dab, a, b), &mut best, &mut axis);
    let cr = aa.cross(ab);
    if MIN_NORMAL_SQ > cr.length_squared() {
        let mut v = aa.cross(dab.cross(ab));
        if MIN_NORMAL_SQ > v.length_squared() {
            let u = if 0.5 > aa.x.abs() { Vec3::X } else { Vec3::Y };
            v = u.cross(aa);
        }
        take(score(v.normalize(), dab, a, b), &mut best, &mut axis);
        return (best, axis);
    }
    let n = cr.normalize();
    take(score(n, dab, a, b), &mut best, &mut axis);
    let (hb, ha) = (ab * b.half_height, aa * a.half_height);
    let d1 = rim_segment(aa, a.half_height, a.radius, dab + hb, dab - hb);
    take(score(d1, dab, a, b), &mut best, &mut axis);
    let d2 = -rim_segment(ab, b.half_height, b.radius, ha - dab, -dab - ha);
    take(score(d2, dab, a, b), &mut best, &mut axis);
    let pos = aa.dot(ab) > 0.0;
    let n1 = if pos { -aa } else { aa };
    let hint = if pos { -n } else { n };
    let from_p = dab - n1 * a.half_height;
    let from_m = dab + n1 * a.half_height;
    let combos = [
        (n1, from_p + hb, ab, hint),
        (-n1, from_m - hb, -ab, hint),
        (-n1, from_m + hb, ab, -hint),
        (n1, from_p - hb, -ab, -hint),
    ];
    for (k, (cn1, c2, cn2, h)) in combos.into_iter().enumerate() {
        let (u, v) = (n1.dot(c2), ab.dot(c2));
        let gate = match k {
            0 => u > 0.0 || 0.0 > v,
            1 => 0.0 > u || v > 0.0,
            2 => 0.0 > u && 0.0 > v,
            _ => u > 0.0 && v > 0.0,
        };
        if !gate {
            continue;
        }
        if let Some(d) = circle_circle(Vec3::ZERO, cn1, a.radius, c2, cn2, b.radius, h) {
            take(score_intervals(d, dab, a, b), &mut best, &mut axis);
        }
    }
    (best, axis)
}

pub fn feature_toward(d: Vec3, c: &CylinderGp) -> Feature {
    let ad = d.dot(c.axis);
    let cap_n = if 0.0 > ad { -c.axis } else { c.axis };
    let cap_c = c.center + cap_n * c.half_height;
    let k2 = ad * ad;
    if k2 >= PARALLEL {
        return Feature::Disc { center: cap_c, normal: cap_n, radius: c.radius };
    }
    let p = (d - c.axis * ad).normalize() * c.radius + cap_c;
    if (1.0 - PARALLEL) >= k2 {
        return Feature::Segment(p, p - cap_n * (2.0 * c.half_height));
    }
    Feature::Point(p)
}

pub fn cylinder_cylinder(a: &CylinderGp, b: &CylinderGp, tolerance: f32) -> Option<Manifold> {
    let pad = tolerance + a.fat + b.fat;
    let (best, d) = separating_axis(a, b);
    if best > pad {
        return None;
    }
    let fa = feature_toward(-d, a);
    let fb = feature_toward(d, b);
    let pairs: Pairs = features::intersect_features(d, &fa, &fb, PARALLEL);
    let (mut n, mut sep) = (d, best);
    if pairs.len() < 3
        && let Some(&(pa, pb)) = pairs.first()
    {
        let mut delta = pa - pb;
        let mut dd = delta.dot(d);
        if let Some(&(pa1, pb1)) = pairs.get(1) {
            let d1 = pa1 - pb1;
            let dd1 = d1.dot(d);
            if dd > dd1 {
                (dd, delta) = (dd1, d1);
            }
        }
        if dd > 0.0 {
            sep = dd;
            let l2 = delta.length_squared();
            if l2 > MIN_NORMAL_SQ {
                let l = l2.sqrt();
                if l > dd {
                    (sep, n) = (l, delta / l);
                }
            }
        }
    }
    if sep > pad {
        return None;
    }
    let points = pairs.iter().map(|&(pa, pb)| (pa - n * a.fat, pb + n * b.fat)).collect();
    Some(Manifold { normal: n, points })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cyl(center: Vec3, axis: Vec3) -> CylinderGp {
        CylinderGp { center, axis, half_height: 0.18, radius: 0.04, fat: 0.0 }
    }

    #[test]
    fn flares_touch_as_the_game_s() {
        let a = cyl(Vec3::ZERO, Vec3::X);
        let m = cylinder_cylinder(&a, &cyl(Vec3::new(0.0, 0.07, 0.0), Vec3::X), 0.1).unwrap();
        assert!(m.normal.abs_diff_eq(-Vec3::Y, 1e-6));
        assert_eq!(m.points.len(), 2);
        for (pa, pb) in &m.points {
            assert!((pa.x.abs() - 0.18).abs() < 1e-5 && (pa.y - 0.04).abs() < 1e-5 && (pb.y - 0.03).abs() < 1e-5);
        }
        let m = cylinder_cylinder(&a, &cyl(Vec3::new(0.0, 0.07, 0.0), Vec3::Z), 0.1).unwrap();
        assert_eq!(m.points.len(), 1);
        assert!((m.points[0].0 - Vec3::new(0.0, 0.04, 0.0)).length() < 1e-5);
        let m = cylinder_cylinder(&a, &cyl(Vec3::new(0.35, 0.0, 0.0), Vec3::X), 0.1).unwrap();
        assert!(m.normal.abs_diff_eq(-Vec3::X, 1e-6));
        assert_eq!(m.points.len(), 4);
        assert!(m.points.iter().all(|(pa, pb)| (pa.x - 0.18).abs() < 1e-5 && (pb.x - 0.17).abs() < 1e-5));
        assert!(cylinder_cylinder(&a, &cyl(Vec3::new(0.0, 0.5, 0.0), Vec3::X), 0.1).is_none());
    }

    #[test]
    fn a_skew_pair_takes_the_cross_axis() {
        let a = cyl(Vec3::ZERO, Vec3::X);
        let b = cyl(Vec3::new(0.1, 0.06, 0.02), Vec3::new(0.6, 0.8, 0.0));
        let m = cylinder_cylinder(&a, &b, 0.1).unwrap();
        assert!(m.normal.abs_diff_eq(-Vec3::Z, 1e-5), "{m:?}");
        assert_eq!(m.points.len(), 1);
        assert!((m.points[0].0 - Vec3::new(0.055, 0.0, 0.04)).length() < 1e-4, "{m:?}");
        assert!((m.points[0].1 - Vec3::new(0.055, 0.0, -0.02)).length() < 1e-4, "{m:?}");
    }
}
