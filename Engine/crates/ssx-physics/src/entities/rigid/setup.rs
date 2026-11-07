use glam::{Mat3, Quat, Vec3};
use ssx_formats::vmx::{add, eq, madd, mul, nmsub, refp, sub};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sym3 {
    pub xx: f32,
    pub xy: f32,
    pub xz: f32,
    pub yy: f32,
    pub yz: f32,
    pub zz: f32,
}

impl Sym3 {
    pub fn to_array(self) -> [f32; 6] {
        [self.xx, self.xy, self.xz, self.yy, self.yz, self.zz]
    }

    pub fn to_mat3(self) -> Mat3 {
        Mat3::from_cols(
            Vec3::new(self.xx, self.xy, self.xz),
            Vec3::new(self.xy, self.yy, self.yz),
            Vec3::new(self.xz, self.yz, self.zz),
        )
    }
}

pub fn world_inv_inertia(q: Quat, diag: Vec3, im: f32) -> Sym3 {
    let (w2, x2, y2, z2) = (add(q.w, q.w), add(q.x, q.x), add(q.y, q.y), add(q.z, q.z));
    let (xx, yy, zz) = (mul(x2, q.x), mul(y2, q.y), mul(z2, q.z));
    let (xy, xz, yz) = (mul(x2, q.y), mul(x2, q.z), mul(y2, q.z));
    let (wx, wy, wz) = (mul(w2, q.x), mul(w2, q.y), mul(w2, q.z));
    let one_zz = sub(1.0, zz);
    let r = [
        [sub(one_zz, yy), sub(xy, wz), add(xz, wy)],
        [add(xy, wz), sub(one_zz, xx), sub(yz, wx)],
        [sub(xz, wy), add(yz, wx), sub(sub(1.0, xx), yy)],
    ];
    let s = [mul(im, diag.x), mul(im, diag.y), mul(im, diag.z)];
    let e = |i: usize, j: usize| {
        madd(r[i][2], mul(s[2], r[j][2]), madd(r[i][0], mul(s[0], r[j][0]), mul(r[i][1], mul(s[1], r[j][1]))))
    };
    Sym3 { xx: e(0, 0), xy: e(0, 1), xz: e(0, 2), yy: e(1, 1), yz: e(1, 2), zz: e(2, 2) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Side {
    pub inv_mass: f32,
    pub inv_inertia: Sym3,
    pub r: Vec3,
}

fn cross_term(s: &Side) -> [f32; 6] {
    let (r, i) = (s.r, s.inv_inertia);
    let q00 = sub(mul(r.y, i.xz), mul(r.z, i.xy));
    let q01 = sub(mul(r.z, i.xx), mul(r.x, i.xz));
    let q02 = sub(mul(r.x, i.xy), mul(r.y, i.xx));
    let q10 = sub(mul(r.y, i.yz), mul(r.z, i.yy));
    let q11 = sub(mul(r.z, i.xy), mul(r.x, i.yz));
    let q12 = sub(mul(r.x, i.yy), mul(r.y, i.xy));
    let q20 = sub(mul(r.y, i.zz), mul(r.z, i.yz));
    let q21 = sub(mul(r.z, i.xz), mul(r.x, i.zz));
    [
        sub(mul(r.y, q20), mul(r.z, q10)),
        sub(mul(r.z, q00), mul(r.x, q20)),
        sub(mul(r.x, q10), mul(r.y, q00)),
        sub(mul(r.z, q01), mul(r.x, q21)),
        sub(mul(r.x, q11), mul(r.y, q01)),
        sub(mul(r.x, q12), mul(r.y, q02)),
    ]
}

pub fn contact_mass(n: Vec3, a: &Side, b: &Side) -> Sym3 {
    let (ka, kb) = (cross_term(a), cross_term(b));
    let m = add(a.inv_mass, b.inv_mass);
    let k00 = add(m, add(ka[0], kb[0]));
    let k10 = add(ka[1], kb[1]);
    let k20 = add(ka[2], kb[2]);
    let k11 = add(m, add(ka[3], kb[3]));
    let k21 = add(ka[4], kb[4]);
    let k22 = add(m, add(ka[5], kb[5]));
    let kn_x = madd(k20, n.z, madd(k10, n.y, mul(k00, n.x)));
    let kn_y = madd(k21, n.z, madd(k11, n.y, mul(k10, n.x)));
    let kn_z = madd(k22, n.z, madd(k21, n.y, mul(k20, n.x)));
    let s = madd(kn_z, n.z, madd(kn_x, n.x, mul(kn_y, n.y)));
    let (w_x, w_y, w_z) = (sub(kn_x, mul(s, n.x)), sub(kn_y, mul(s, n.y)), sub(kn_z, mul(s, n.z)));
    let twice = |p: f32| add(p, p);
    let a00 = sub(k00, twice(mul(w_x, n.x)));
    let b10 = sub(k10, add(mul(w_x, n.y), mul(w_y, n.x)));
    let c20 = sub(k20, add(mul(w_x, n.z), mul(w_z, n.x)));
    let d11 = sub(k11, twice(mul(w_y, n.y)));
    let e21 = sub(k21, add(mul(w_y, n.z), mul(w_z, n.y)));
    let f22 = sub(k22, twice(mul(w_z, n.z)));
    let adj = Sym3 {
        xx: sub(mul(d11, f22), mul(e21, e21)),
        xy: sub(mul(e21, c20), mul(b10, f22)),
        xz: sub(mul(b10, e21), mul(d11, c20)),
        yy: sub(mul(f22, a00), mul(c20, c20)),
        yz: sub(mul(c20, b10), mul(e21, a00)),
        zz: sub(mul(a00, d11), mul(b10, b10)),
    };
    let det = madd(adj.xz, c20, madd(adj.xx, a00, mul(adj.xy, b10)));
    let y = refp(det);
    let y = madd(y, nmsub(y, det, 1.0), y);
    let inv = if eq(det, 0.0) { 0.0 } else { y };
    Sym3 {
        xx: mul(inv, adj.xx),
        xy: mul(inv, adj.xy),
        xz: mul(inv, adj.xz),
        yy: mul(inv, adj.yy),
        yz: mul(inv, adj.yz),
        zz: mul(inv, adj.zz),
    }
}
