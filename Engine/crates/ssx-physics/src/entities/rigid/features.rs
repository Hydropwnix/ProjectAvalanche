#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::manual_range_contains)]

use glam::Vec3;

use super::narrow::EPS30;

const EPS37: f32 = 1e-37;
const BIG37: f32 = 1e37;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Feature {
    Disc { center: Vec3, normal: Vec3, radius: f32 },
    Point(Vec3),
    Segment(Vec3, Vec3),
}

pub type Pairs = Vec<(Vec3, Vec3)>;

fn clamp01(x: f32) -> f32 {
    1.0f32.min(0.0f32.max(x))
}

fn recip(x: f32) -> f32 {
    1.0 / x
}

pub fn intersect(n: Vec3, a: &Feature, b: &[Vec3], kpar: f32) -> Pairs {
    let swap = |p: Pairs| p.into_iter().map(|(x, y)| (y, x)).collect();
    match (*a, b.len()) {
        (Feature::Disc { center, normal, radius }, 1) => disc_point(n, center, normal, radius, b[0]),
        (Feature::Disc { center, normal, radius }, 2) => disc_segment(n, center, normal, radius, b[0], b[1]),
        (Feature::Disc { center, normal, radius }, _) => disc_triangle(n, center, normal, radius, [b[0], b[1], b[2]]),
        (Feature::Point(p), 1) => vec![(p, b[0])],
        (Feature::Point(p), 2) => swap(segment_point(-n, b[0], b[1], p)),
        (Feature::Point(p), _) => swap(triangle_point(-n, [b[0], b[1], b[2]], p)),
        (Feature::Segment(s0, s1), 1) => segment_point(n, s0, s1, b[0]),
        (Feature::Segment(s0, s1), 2) => segment_segment(n, s0, s1, b[0], b[1], kpar),
        (Feature::Segment(s0, s1), _) => swap(triangle_segment(-n, [b[0], b[1], b[2]], s0, s1, kpar)),
    }
}

pub fn intersect_features(n: Vec3, a: &Feature, b: &Feature, kpar: f32) -> Pairs {
    let swap = |p: Pairs| p.into_iter().map(|(x, y)| (y, x)).collect();
    match (*a, *b) {
        (
            Feature::Disc { center: ca, normal: na, radius: ra },
            Feature::Disc { center: cb, normal: nb, radius: rb },
        ) => disc_disc(n, ca, na, ra, cb, nb, rb),
        (Feature::Disc { center, normal, radius }, Feature::Point(p)) => disc_point(n, center, normal, radius, p),
        (Feature::Disc { center, normal, radius }, Feature::Segment(s0, s1)) => {
            disc_segment(n, center, normal, radius, s0, s1)
        }
        (Feature::Point(p), Feature::Disc { center, normal, radius }) => {
            swap(disc_point(-n, center, normal, radius, p))
        }
        (Feature::Segment(s0, s1), Feature::Disc { center, normal, radius }) => {
            swap(disc_segment(-n, center, normal, radius, s0, s1))
        }
        (Feature::Point(p), Feature::Point(q)) => vec![(p, q)],
        (Feature::Point(p), Feature::Segment(s0, s1)) => swap(segment_point(-n, s0, s1, p)),
        (Feature::Segment(s0, s1), Feature::Point(p)) => segment_point(n, s0, s1, p),
        (Feature::Segment(a0, a1), Feature::Segment(b0, b1)) => segment_segment(n, a0, a1, b0, b1, kpar),
    }
}

pub fn cross_fused(a: Vec3, b: Vec3) -> Vec3 {
    let (ay, by) = (Vec3::new(a.y, a.z, a.x), Vec3::new(b.y, b.z, b.x));
    let t = |i: usize| (-ay[i]).mul_add(b[i], a[i] * by[i]);
    Vec3::new(t(1), t(2), t(0))
}

pub fn disc_disc(n: Vec3, ca: Vec3, na: Vec3, ra: f32, cb: Vec3, nb: Vec3, rb: f32) -> Pairs {
    let (inv_an, inv_bn) = (recip(na.dot(n)), recip(nb.dot(n)));
    let (rmax, rmin) = if ra > rb { (ra, rb) } else { (rb, ra) };
    let off = n.cross((ca - cb).cross(n));
    let cbp = cb + n * (na.dot(ca - cb) * inv_an);
    let e2 = off.length_squared();
    let e = if e2 == 0.0 { 0.0 } else { e2.sqrt() };
    let to_b = |p: Vec3| p + n * (nb.dot(cb - p) * inv_bn);
    if rmax - rmin * 0.5 > e {
        let (big, c) = if ra > rb { (ca, cbp) } else { (cbp, ca) };
        let x = cross_fused(na, nb);
        let u = if x.length_squared() > 0.0 {
            x.normalize()
        } else if e2 > 0.0 {
            off * recip(e)
        } else {
            let w = if na.x.abs() > na.y.abs() { Vec3::Y } else { Vec3::X };
            na.cross(w).normalize()
        } * rmin;
        let v = u.cross(na);
        return [c + u, c + v, c - u, c - v]
            .into_iter()
            .map(|p| {
                let q = p - big;
                let l2 = q.length_squared();
                let p = if l2 > rmax * rmax { big + q * (rmax / l2.sqrt()) } else { p };
                (p, to_b(p))
            })
            .collect();
    }
    if e >= ra + rb {
        let w = cbp - ca;
        let l2 = w.length_squared();
        let pa = if l2 > ra * ra { ca + w * (ra / l2.sqrt()) } else { cbp };
        let cap = ca + n * (nb.dot(cb - ca) * inv_bn);
        let w = cap - cb;
        let l2 = w.length_squared();
        let pb = if l2 > rb * rb { cb + w * (rb / l2.sqrt()) } else { cap };
        return vec![(pa, pb)];
    }
    let u = off * recip(e);
    let db = (e2 - ra * ra + rb * rb) * recip(e) * 0.5;
    let mut out = Vec::with_capacity(4);
    if rb * 0.999 > db {
        let m = u * db + cbp;
        let h2 = rb * rb - db * db;
        let h = if h2 == 0.0 { 0.0 } else { h2.sqrt() };
        let t = u.cross(n) * h;
        out.extend([m + t, m - t].map(|p| (p, to_b(p))));
    }
    out.extend([ca - u * (e - rb), ca - u * ra].map(|p| (p, to_b(p))));
    out
}

pub fn segment_point(n: Vec3, s0: Vec3, s1: Vec3, p: Vec3) -> Pairs {
    let e = s1 - s0;
    let w = p - s0;
    let e2 = e - n * n.dot(e);
    let w2 = w - n * n.dot(w);
    let ee = e2.length_squared();
    let inv = if ee > EPS30 { 1.0 / ee } else { 0.0 };
    let t = 1.0f32.min(0.0f32.max(w2.dot(e2) * inv));
    vec![(s0 + e * t, p)]
}

pub fn disc_point(n: Vec3, c: Vec3, nd: Vec3, r: f32, p: Vec3) -> Pairs {
    let lam = nd.dot(c - p) * recip(nd.dot(n));
    let x = p + n * lam;
    let v = x - c;
    let l2 = v.length_squared();
    let d = if l2 == 0.0 { 0.0 } else { l2.sqrt() };
    let x = if d > r { c + v * (r * recip(d)) } else { x };
    vec![(x, p)]
}

fn rim(c: Vec3, r: f32, q: Vec3) -> Vec3 {
    let v = q - c;
    let l2 = v.length_squared();
    let l = if l2 == 0.0 { 0.0 } else { l2.sqrt() };
    c + v * (r * recip(l))
}

pub fn disc_segment(n: Vec3, c: Vec3, nd: Vec3, r: f32, s0: Vec3, s1: Vec3) -> Pairs {
    let inv = recip(nd.dot(n));
    let q1 = s1 + n * (nd.dot(c - s1) * inv);
    let q0 = s0 + n * (nd.dot(c - s0) * inv);
    let d = q1 - q0;
    let c0 = c - q0;
    let dd = d.length_squared();
    let dc = d.dot(c0);
    let disc = dd * (r * r - c0.length_squared()) + dc * dc;
    let invd = if dd > EPS30 { 1.0 / dd } else { 0.0 };
    let tc = dc * invd;
    let e = s1 - s0;
    if 0.0 >= disc {
        let t = clamp01(tc);
        return vec![(rim(c, r, q0 + d * t), s0 + e * t)];
    }
    let h = disc.sqrt() * invd;
    let lo = (tc - h).max(0.0);
    let hi = (tc + h).min(1.0);
    if lo >= 1.0 {
        return vec![(rim(c, r, q1), s1)];
    }
    if 0.0 >= hi {
        return vec![(rim(c, r, q0), s0)];
    }
    vec![(q0 + d * hi, s0 + e * hi), (q0 + d * lo, s0 + e * lo)]
}

pub fn segment_segment(n: Vec3, a0: Vec3, a1: Vec3, b0: Vec3, b1: Vec3, kpar: f32) -> Pairs {
    let a = a1 - a0;
    let b = b1 - b0;
    let (aa, bb, ab) = (a.length_squared(), b.length_squared(), a.dot(b));
    let prod = aa * bb;
    let det = prod - ab * ab;
    if ab * ab >= prod * kpar {
        let (bs, be) = if 0.0 > ab { (b1, b0) } else { (b0, b1) };
        let bo = be - bs;
        let sm = a + bo;
        let d = sm - n * n.dot(sm);
        let (da0, dbe, da1, dbs) = (d.dot(a0), d.dot(be), d.dot(a1), d.dot(bs));
        if da0 > dbe {
            return vec![(a0, be)];
        }
        if dbs > da1 {
            return vec![(a1, bs)];
        }
        let lo = da0.max(dbs);
        let hi = da1.min(dbe);
        let ia = a * recip(da1 - da0);
        let ib = bo * recip(dbe - dbs);
        return vec![(a0 + ia * (lo - da0), bs + ib * (lo - dbs)), (a0 + ia * (hi - da0), bs + ib * (hi - dbs))];
    }
    let rv = a0 - b0;
    let (ar, br) = (a.dot(rv), b.dot(rv));
    let s_num = ab * br - bb * ar;
    let t_num = aa * br - ab * ar;
    let (mut s_n, mut t_n, mut t_den) = (s_num, t_num, det);
    if s_num > det {
        t_n = br + ab;
        s_n = det;
    }
    if s_num > det || 0.0 > s_num {
        t_den = bb;
    }
    if 0.0 > s_num {
        t_n = br;
        s_n = 0.0;
    }
    let mut s_den = det;
    let (t_hi, t_lo) = (t_n > t_den, 0.0 > t_n);
    if t_hi {
        s_n = ab - ar;
        t_n = t_den;
    }
    if t_hi || t_lo {
        s_den = aa;
    }
    if t_lo {
        s_n = -ar;
        t_n = 0.0;
    }
    let inv_t = if t_den > EPS30 { 1.0 / t_den } else { 0.0 };
    let t = t_n * inv_t;
    let mut s_c = if s_n > s_den { s_den } else { s_n };
    if 0.0 > s_n {
        s_c = 0.0;
    }
    let inv_s = if s_den > EPS30 { 1.0 / s_den } else { 0.0 };
    vec![(a0 + a * (s_c * inv_s), b0 + b * t)]
}

pub fn triangle_point(n: Vec3, t: [Vec3; 3], p: Vec3) -> Pairs {
    let e = [t[1] - t[0], t[2] - t[1], t[0] - t[2]];
    let nt = e[0].cross(e[1]);
    let q: [f32; 3] = std::array::from_fn(|i| (p - t[i]).dot(e[i]));
    let g = q.map(|x| x >= 0.0);
    let m = if n.dot(nt) > 0.0 { -n } else { n };
    let o: [bool; 3] = std::array::from_fn(|i| (p - t[i]).dot(m.cross(e[i])) >= 0.0);
    let lam = nt.dot(t[0] - p) * recip(nt.dot(n));
    let mut out = p + n * lam;
    for i in 0..3 {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        if o[i] && !(o[j] && g[j]) && (g[i] || !o[k]) {
            out = t[i] + e[i] * clamp01(q[i] * recip(e[i].length_squared()));
        }
    }
    vec![(out, p)]
}

fn argmin3(v: [f32; 3]) -> usize {
    if v[1] > v[0] && v[2] > v[0] {
        0
    } else if v[2] > v[1] {
        1
    } else {
        2
    }
}

fn argmax3(v: [f32; 3]) -> usize {
    if v[0] > v[1] && v[0] > v[2] {
        0
    } else if v[1] > v[2] {
        1
    } else {
        2
    }
}

pub fn triangle_segment(n: Vec3, t: [Vec3; 3], s0: Vec3, s1: Vec3, kpar: f32) -> Pairs {
    let e = [t[1] - t[0], t[2] - t[1], t[0] - t[2]];
    let s = s1 - s0;
    let se = e.map(|x| s.dot(x));
    let w = e.map(|x| n.cross(x));
    let h: [f32; 3] = std::array::from_fn(|i| (t[i] - s0).dot(w[i]));
    let g: [f32; 3] = std::array::from_fn(|i| s.dot(w[i]));
    let tan_lim = (1.0 - kpar).sqrt();
    let cand: [f32; 3] = std::array::from_fn(|i| {
        if (0.0 > h[i] || g[i] > h[i]) && se[i].abs() > EPS37 { (g[i] * recip(se[i])).abs() } else { 2.0 }
    });
    let k = argmin3(cand);
    if tan_lim > cand[k] {
        let (d, ps) = if 0.0 > s.dot(e[k]) { (-e[k], t[k] + e[k]) } else { (e[k], t[k]) };
        let inv = recip(d.length_squared());
        let u1 = d.dot(s1 - ps) * inv;
        let u0 = d.dot(s0 - ps) * inv;
        let (hi, lo) = (1.0f32.min(u1), 0.0f32.max(u0));
        if hi > lo {
            let iu = recip(u1 - u0);
            return vec![(ps + d * lo, s0 + s * ((lo - u0) * iu)), (ps + d * hi, s0 + s * ((hi - u0) * iu))];
        }
    }
    let outside = (0..3).any(|i| 0.0 > h[i] && g[i] > h[i]);
    if !outside {
        let tt: [f32; 3] = std::array::from_fn(|i| h[i] * recip(g[i]));
        let up: [f32; 3] = std::array::from_fn(|i| if g[i] > EPS37 { tt[i] } else { 1.0 });
        let lw: [f32; 3] = std::array::from_fn(|i| if -EPS37 > g[i] { tt[i] } else { 0.0 });
        let (ku, kl) = (argmin3(up), argmax3(lw));
        let (u, l) = (up[ku], lw[kl]);
        let hi = if 1.0 > u { u } else { 1.0 };
        let lo = if l > 0.0 { l } else { 0.0 };
        if hi > lo {
            let nt = e[0].cross(e[1]);
            let sn = s.cross(n);
            let inv_nn = recip(nt.dot(n));
            let a1 = if l > 0.0 {
                t[kl] + e[kl] * ((s0 - t[kl]).dot(sn) * recip(e[kl].dot(sn)))
            } else {
                s0 + n * (nt.dot(t[0] - s0) * inv_nn)
            };
            let a2 = if 1.0 > u {
                t[ku] + e[ku] * ((s0 - t[ku]).dot(sn) * recip(e[ku].dot(sn)))
            } else {
                s1 + n * (nt.dot(t[0] - s1) * inv_nn)
            };
            return vec![(a1, s0 + s * lo), (a2, s0 + s * hi)];
        }
    }
    let inv_s = recip(s.length_squared());
    let (mut best, mut a, mut b) = (BIG37, s, s);
    for i in 0..3 {
        let ie = recip(e[i].length_squared());
        let ea = t[i] + e[i] * clamp01(e[i].dot(s0 - t[i]) * ie);
        let eb = t[i] + e[i] * clamp01(e[i].dot(s1 - t[i]) * ie);
        let next = t[(i + 1) % 3];
        let sg = s0 + s * clamp01(s.dot(next - s0) * inv_s);
        for (pa, pb) in [(ea, s0), (eb, s1), (next, sg)] {
            let dd = (pa - pb).length_squared();
            if best > dd {
                (best, a, b) = (dd, pa, pb);
            }
        }
    }
    vec![(a, b)]
}

pub fn disc_triangle(n: Vec3, c: Vec3, nd: Vec3, r: f32, t: [Vec3; 3]) -> Pairs {
    let inv_nn = recip(n.dot(nd));
    let p = t.map(|x| x + n * (nd.dot(c - x) * inv_nn));
    let nt = (t[1] - t[0]).cross(t[2] - t[0]);
    let nt2 = nt.length_squared();
    let tn = if 1e-10 * r >= nt2 { n } else { nt / nt2.sqrt() };
    let inv_ntn = recip(n.dot(tn));
    let mut out = Pairs::new();
    let inside = p.map(|x| r * r > (x - c).length_squared());
    for i in 0..3 {
        if inside[i] {
            out.push((p[i], t[i]));
        }
    }
    let d = [p[1] - p[0], p[2] - p[1], p[0] - p[2]];
    let e = [t[1] - t[0], t[2] - t[1], t[0] - t[2]];
    let mut tc = [0.0f32; 3];
    let mut disc = [0.0f32; 3];
    for k in 0..3 {
        let cp = c - p[k];
        let inv = recip(d[k].length_squared());
        tc[k] = d[k].dot(cp) * inv;
        disc[k] = tc[k] * tc[k] + inv * (r * r - cp.length_squared());
    }
    let x: [bool; 3] = std::array::from_fn(|k| disc[k] > 0.0 && !(0.0 > tc[k] || tc[k] > 1.0));
    let entry: [bool; 3] = std::array::from_fn(|k| !inside[k] && (inside[(k + 1) % 3] || x[k]));
    let exit: [bool; 3] = std::array::from_fn(|k| !inside[(k + 1) % 3] && (inside[k] || x[k]));
    let sq = disc.map(|v| if v == 0.0 { 0.0 } else { v.abs().sqrt() });
    for k in 0..3 {
        if entry[k] {
            let u = tc[k] - sq[k];
            out.push((p[k] + d[k] * u, t[k] + e[k] * u));
        }
    }
    for k in 0..3 {
        if exit[k] {
            let u = tc[k] + sq[k];
            out.push((p[k] + d[k] * u, t[k] + e[k] * u));
        }
    }
    let tnn = tn.cross(nd);
    let tnn2 = tnn.length_squared();
    let a1 = if 1e-6 > tnn2 {
        let unit = if tn.x.abs() > tn.y.abs() { Vec3::Y } else { Vec3::X };
        let v = tn.cross(unit);
        v / v.length()
    } else {
        tnn / tnn2.sqrt()
    };
    let ra = a1 * r;
    let w = ra.cross(nd);
    let rims = [c + w, c + ra, c - w, c - ra];
    let en = d.map(|x| nd.cross(x));
    let in_tri = |q: Vec3| (0..3).all(|k| 0.0 >= (q - p[k]).dot(en[k]));
    let onto = |q: Vec3| q + n * (tn.dot(t[0] - q) * inv_ntn);
    let found = inside.iter().chain(&entry).chain(&exit).any(|&b| b);
    if found {
        for q in rims {
            if in_tri(q) {
                out.push((q, onto(q)));
            }
        }
        return out;
    }
    if in_tri(c) {
        return rims.iter().map(|&q| (q, onto(q))).collect();
    }
    let tcl = tc.map(|v| v.clamp(0.0, 1.0));
    let pp: [Vec3; 3] = std::array::from_fn(|k| p[k] + d[k] * tcl[k]);
    let oo: [Vec3; 3] = std::array::from_fn(|k| t[k] + e[k] * tcl[k]);
    let dd = pp.map(|q| (q - c).length_squared());
    let mut k = 0;
    if dd[0] > dd[1] {
        k = 1;
    }
    if dd[k] > dd[2] {
        k = 2;
    }
    let v = pp[k] - c;
    vec![(c + v * (r / v.length()), oo[k])]
}
