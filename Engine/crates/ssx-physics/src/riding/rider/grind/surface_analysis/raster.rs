use super::*;

pub(super) const CLUSTER_VERTICES: usize = 256;

pub(super) fn fused_ops() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

pub(super) fn rasterize_cluster(
    raster: &mut [u8],
    m: &Mat4,
    c: &CollisionWorld,
    cl: &crate::collision::Cluster,
    scratch: &mut [Vec3; CLUSTER_VERTICES],
    fused: bool,
) {
    #[cfg(target_arch = "x86_64")]
    if fused {
        unsafe { rasterize_cluster_fused(raster, m, c, cl, scratch) };
        return;
    }
    let _ = fused;
    rasterize_cluster_with(raster, m, c, cl, scratch);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
pub(super) fn rasterize_cluster_fused(
    raster: &mut [u8],
    m: &Mat4,
    c: &CollisionWorld,
    cl: &crate::collision::Cluster,
    scratch: &mut [Vec3; CLUSTER_VERTICES],
) {
    rasterize_cluster_with(raster, m, c, cl, scratch);
}

#[inline(always)]
pub(super) fn rasterize_cluster_with(
    raster: &mut [u8],
    m: &Mat4,
    c: &CollisionWorld,
    cl: &crate::collision::Cluster,
    scratch: &mut [Vec3; CLUSTER_VERTICES],
) {
    let vs = cl.vert_start as usize;
    let n = (cl.vert_count as usize).min(CLUSTER_VERTICES).min(c.vertices.len().saturating_sub(vs));
    for (s, &v) in scratch.iter_mut().zip(&c.vertices[vs..vs + n]) {
        *s = raster_vertex(m, v);
    }
    for t in &c.triangles[cl.tri_start as usize..cl.tri_start as usize + cl.tri_count as usize] {
        let v = t.map(|k| match usize::from(k) {
            k if k < n => scratch[k],
            k => raster_vertex(m, c.vertices[vs + k]),
        });
        rasterize_triangle(raster, v[0], v[1], v[2]);
    }
}

#[inline(always)]
pub(super) fn raster_vertex(m: &Mat4, p: Vec3) -> Vec3 {
    let (r0, r1, r2, r3) = (m.x_axis, m.y_axis, m.z_axis, m.w_axis);
    let f = |k: usize| r2[k].mul_add(p.z, r1[k].mul_add(p.y, r0[k].mul_add(p.x, r3[k])));
    Vec3::new(f(0), f(1), f(2))
}

#[inline(always)]
pub(super) fn recip(x: f32) -> f32 {
    let mut r = 1.0 / x;
    for _ in 0..2 {
        let e = -r.mul_add(x, -1.0);
        r = r.mul_add(e, r);
    }
    r
}

#[inline(always)]
pub(super) fn slope(dz: f32, dy: f32) -> f32 {
    if dy.abs() <= EPS { 0.0 } else { recip(dy) * dz }
}

#[inline(always)]
pub fn rasterize_triangle(raster: &mut [u8], v0: Vec3, v1: Vec3, v2: Vec3) {
    let mn = v0.min(v1).min(v2);
    let mx = v0.max(v1).max(v2);
    if (0..3).any(|k| mn[k].floor() >= 255.0 || 0.0 >= mx[k].ceil()) {
        return;
    }
    scan_triangle(raster, v0, v1, v2);
    ceiling_line(raster, v0, v1, v2);
}

#[inline(always)]
pub(super) fn scan_triangle(raster: &mut [u8], v0: Vec3, v1: Vec3, v2: Vec3) {
    let round = |v: Vec3| Vec3::new((v.x + 0.5).floor(), (v.y + 0.5).floor(), (v.z + 0.5).floor());
    let mut p = [round(v0), round(v1), round(v2)];
    p.sort_by(|a, b| a.y.total_cmp(&b.y));
    let [t, m, b] = p;
    if t.y == b.y {
        return;
    }
    let (yt, ym, yb) = (t.y as i64, m.y as i64, b.y as i64);
    let (xt, xm, xb) = (t.x as i64, m.x as i64, b.x as i64);
    let s_l = slope(b.z - t.z, b.y - t.y);
    let s_tm = slope(t.z - m.z, t.y - m.y);
    let s_mb = slope(b.z - m.z, b.y - m.y);
    let fy0 = t.y.max(0.0);
    let mut zl = s_l.mul_add(fy0, t.z - s_l * t.y);
    let mut ztm = s_tm.mul_add(fy0, m.z - s_tm * m.y);
    let mut zmb = s_mb.mul_add(fy0, m.z - s_mb * m.y);
    let neg = (xm - xt) * (yb - yt) - (ym - yt) * (xb - xt) < 0;
    let y0 = yt.max(0);
    let (mut el, mut e1, mut e2) =
        (EdgeWalk::new(t, b, y0, 0), EdgeWalk::new(t, m, y0, xm - xt), EdgeWalk::new(m, b, y0, xm - xb));
    for y in y0..=yb.min(255) {
        let (xl, x1, x2) = (el.x, e1.x, e2.x);
        (el, e1, e2) = (el.next(), e1.next(), e2.next());
        let pick_tm = if neg { x1 > x2 } else { x1 < x2 };
        let (xs, zs) = if pick_tm { (x1, ztm) } else { (x2, zmb) };
        let dzdx = slope(zs - zl, xs as f32 - xl as f32);
        let (lo, hi) = (xl.min(xs).max(0), xl.max(xs).min(255));
        if lo <= hi {
            let xa = lo & !15;
            let base = xa as f32 - xl as f32;
            let mut vals = [0.0f32; 4];
            for (lane, v) in vals.iter_mut().enumerate() {
                *v = (255.0 - dzdx.mul_add(base + lane as f32, zl)).floor();
            }
            let row: &mut [u8; GRID] =
                (&mut raster[y as usize * GRID..(y as usize + 1) * GRID]).try_into().expect("a raster row");
            #[cfg(target_arch = "x86_64")]
            unsafe {
                span_sse2(row, lo, hi, xa, vals, dzdx * -4.0)
            };
            #[cfg(not(target_arch = "x86_64"))]
            span(row, lo, hi, xa, vals, dzdx * -4.0);
        }
        zl += s_l;
        ztm += s_tm;
        zmb += s_mb;
    }
}

#[cfg(any(test, not(target_arch = "x86_64")))]
#[inline(always)]
pub(super) fn span(row: &mut [u8; GRID], lo: i64, hi: i64, xa: i64, mut vals: [f32; 4], step: f32) {
    let mut x = xa;
    while x <= hi {
        for (lane, v) in vals.iter_mut().enumerate() {
            let xx = x + lane as i64;
            if (lo..=hi).contains(&xx) {
                let val = trunc_sat(*v);
                if (0..=255).contains(&val) {
                    let cell = &mut row[xx as usize];
                    *cell = (*cell).max(val as u8);
                }
            }
            *v += step;
        }
        x += 4;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
pub(super) fn span_sse2(row: &mut [u8; GRID], lo: i64, hi: i64, xa: i64, vals: [f32; 4], step: f32) {
    use std::arch::x86_64::*;
    let (mut v, step4) = (_mm_set_ps(vals[3], vals[2], vals[1], vals[0]), _mm_set1_ps(step));
    let (z, top) = (_mm_setzero_si128(), _mm_set1_epi32(255));
    let mut x = xa;
    while x <= hi {
        if x >= lo && x + 3 <= hi {
            let at = x as usize;
            let i = _mm_cvttps_epi32(v);
            let kept = _mm_andnot_si128(_mm_or_si128(_mm_cmplt_epi32(i, z), _mm_cmpgt_epi32(i, top)), i);
            let bytes = _mm_packus_epi16(_mm_packs_epi32(kept, kept), z);
            let cells: &mut [u8; 4] = (&mut row[at..at + 4]).try_into().expect("four cells");
            let old = _mm_cvtsi32_si128(i32::from_le_bytes(*cells));
            *cells = _mm_cvtsi128_si32(_mm_max_epu8(old, bytes)).to_le_bytes();
        } else {
            let lanes = [
                _mm_cvtss_f32(v),
                _mm_cvtss_f32(_mm_shuffle_ps::<0b01_01_01_01>(v, v)),
                _mm_cvtss_f32(_mm_shuffle_ps::<0b10_10_10_10>(v, v)),
                _mm_cvtss_f32(_mm_shuffle_ps::<0b11_11_11_11>(v, v)),
            ];
            for (lane, &lv) in lanes.iter().enumerate() {
                let xx = x + lane as i64;
                if (lo..=hi).contains(&xx) {
                    let val = trunc_sat(lv);
                    if (0..=255).contains(&val) {
                        let cell = &mut row[xx as usize];
                        *cell = (*cell).max(val as u8);
                    }
                }
            }
        }
        v = _mm_add_ps(v, step4);
        x += 4;
    }
}

#[derive(Clone, Copy)]
pub(super) struct EdgeWalk {
    pub(super) x: i64,
    pub(super) r: i64,
    pub(super) dq: i64,
    pub(super) dr: i64,
    pub(super) dy: i64,
}

impl EdgeWalk {
    #[inline(always)]
    pub(super) fn new(a: Vec3, b: Vec3, y: i64, sign: i64) -> Self {
        let dy = (b.y - a.y) as i64;
        if dy == 0 {
            let x = if sign >= 0 { 1 << 20 } else { -(1 << 20) };
            return Self { x, r: 0, dq: 0, dr: 0, dy: 1 };
        }
        let (dx, num) = ((b.x - a.x) as i64, (y - a.y as i64) * (b.x - a.x) as i64);
        Self {
            x: a.x as i64 + num.div_euclid(dy),
            r: num.rem_euclid(dy),
            dq: dx.div_euclid(dy),
            dr: dx.rem_euclid(dy),
            dy,
        }
    }

    #[inline(always)]
    pub(super) fn next(self) -> Self {
        let (mut x, mut r) = (self.x + self.dq, self.r + self.dr);
        if r >= self.dy {
            (x, r) = (x + 1, r - self.dy);
        }
        Self { x, r, ..self }
    }
}

#[inline(always)]
pub(super) fn trunc_sat(x: f32) -> i64 {
    if x.is_nan() { 0 } else { (x as i32) as i64 }
}

#[inline(always)]
pub(super) fn ceiling_line(raster: &mut [u8], v0: Vec3, v1: Vec3, v2: Vec3) {
    let (mut neg, mut pos) = ([Vec3::ZERO; 3], [Vec3::ZERO; 3]);
    let (mut nn, mut np) = (0, 0);
    for v in [v0, v1, v2] {
        if 0.0 > v.z {
            (neg[nn], nn) = (v, nn + 1);
        } else {
            (pos[np], np) = (v, np + 1);
        }
    }
    if nn == 0 || np == 0 {
        return;
    }
    let (a0, b0) = (neg[0], pos[0]);
    let a1 = if nn == 1 { neg[0] } else { neg[1] };
    let b1 = if np == 1 { pos[0] } else { pos[1] };
    let cut = |a: Vec3, b: Vec3| {
        let d = b - a;
        let r = recip(d.z);
        let k = a.z.abs();
        let f = |i: usize| ((r * d[i]).mul_add(k, a[i]) + 0.5).floor() as i64;
        (f(0), f(1))
    };
    let ((ax, ay), (bx, by)) = (cut(a0, b0), cut(a1, b1));
    let (dx, dy) = ((bx - ax).abs(), (by - ay).abs());
    let (sx, sy) = (if ax < bx { 1 } else { -1 }, if ay < by { 1 } else { -1 });
    let (mut err, mut x, mut y) = (2 * (dx - dy), ax, ay);
    loop {
        if (0..GRID as i64).contains(&x) && (0..GRID as i64).contains(&y) {
            raster[y as usize * GRID + x as usize] = 255;
        }
        if x == bx && y == by {
            break;
        }
        let e2 = err;
        if err > -dy {
            err -= 2 * dy;
            x += sx;
        }
        if e2 < dx {
            err += 2 * dx;
            y += sy;
        }
    }
}

pub fn ridgiprocity(heights: &[u8], to_world: &Mat4) -> Vec<u8> {
    let t = RidgeTables::new(to_world);
    let mut out = vec![0u8; GRID * GRID];
    for (y, (row, o)) in heights.as_chunks::<GRID>().0.iter().zip(out.as_chunks_mut::<GRID>().0).enumerate() {
        *o = t.row(row, y);
    }
    out
}

pub(super) struct RidgeTables {
    pub(super) w: usize,
    pub(super) end: usize,
    pub(super) r1: Vec4,
    pub(super) by_x: [[f32; GRID]; 3],
    pub(super) by_h: [[f32; 256]; 3],
}

impl RidgeTables {
    pub(super) fn new(to_world: &Mat4) -> Self {
        let cell = to_world.x_axis.truncate().length();
        let w = (0.35 / cell + 0.5).floor().max(1.0) as usize;
        let (r0, r2, r3) = (to_world.x_axis, to_world.z_axis, to_world.w_axis);
        Self {
            w,
            end: GRID.saturating_sub(w),
            r1: to_world.y_axis,
            by_x: std::array::from_fn(|k| std::array::from_fn(|x| x as f32 * r0[k])),
            by_h: std::array::from_fn(|k| std::array::from_fn(|h| h as f32 * r2[k] + r3[k])),
        }
    }

    pub(super) fn row(&self, row: &[u8; GRID], y: usize) -> [u8; GRID] {
        let mut out = [0u8; GRID];
        #[cfg(target_arch = "x86_64")]
        if fused_ops() {
            unsafe { ridge_row_fused(self, row, y, &mut out) };
            return out;
        }
        ridge_row_with(self, row, y, &mut out);
        out
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
pub(super) fn ridge_row_fused(t: &RidgeTables, row: &[u8; GRID], y: usize, out: &mut [u8; GRID]) {
    ridge_row_with(t, row, y, out);
}

#[inline(always)]
pub(super) fn ridge_row_with(t: &RidgeTables, row: &[u8; GRID], y: usize, out: &mut [u8; GRID]) {
    let (w, end) = (t.w, t.end);
    if w >= end || row[w..end].iter().all(|&h| h == 0) {
        return;
    }
    let mut p = [[0.0f32; GRID]; 3];
    for (k, pk) in p.iter_mut().enumerate() {
        let yr = y as f32 * t.r1[k];
        let (bx, bh) = (&t.by_x[k], &t.by_h[k]);
        for x in 0..GRID {
            pk[x] = bx[x] + (yr + bh[usize::from(row[x])]);
        }
    }
    let [px, py, pz] = &p;
    let mut arm = [[0.0f32; GRID]; 3];
    let [ax, ay, az] = &mut arm;
    let n = end;
    let (pxa, pya, pza) = (&px[..n], &py[..n], &pz[..n]);
    let (pxb, pyb, pzb) = (&px[w..w + n], &py[w..w + n], &pz[w..w + n]);
    let (ax, ay, az) = (&mut ax[..n], &mut ay[..n], &mut az[..n]);
    for x in 0..n {
        let (dx, dy, dz) = (pxb[x] - pxa[x], pyb[x] - pya[x], pzb[x] - pza[x]);
        let r = 1.0 / ((dx * dx + dy * dy) + dz * dz).sqrt();
        (ax[x], ay[x], az[x]) = (dx * r, dy * r, dz * r);
    }
    let m = end - w;
    let (alx, aly, alz) = (&ax[..m], &ay[..m], &az[..m]);
    let (arx, ary, arz) = (&ax[w..w + m], &ay[w..w + m], &az[w..w + m]);
    let (yl, yc, yr) = (&py[..m], &py[w..w + m], &py[2 * w..2 * w + m]);
    let (hc, vc) = (&row[w..w + m], &mut out[w..w + m]);
    for i in 0..m {
        let c = ((-alx[i] * arx[i] + -aly[i] * ary[i]) + -alz[i] * arz[i]).clamp(-1.0, 1.0);
        let k = (1.0 + c).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
        let r = ((k - 0.25) / (0.5 - 0.25)).clamp(0.0, 1.0);
        let r = if 0.5 * (yl[i] + yr[i]) >= yc[i] { 0.0 } else { r };
        vc[i] = if hc[i] == 0 { 0 } else { (255.0 * r + 0.5).floor() as u8 };
    }
    for x in 0..GRID {
        if row[x] >= 240 {
            out[x.saturating_sub(w)..(x + w).min(GRID)].fill(0);
        }
    }
}

#[derive(Clone)]
pub(super) struct Ridges {
    pub(super) tables: std::sync::OnceLock<std::sync::Arc<RidgeTables>>,
    pub(super) rows: Box<[std::sync::OnceLock<[u8; GRID]>]>,
}

impl Default for Ridges {
    fn default() -> Self {
        Self { tables: std::sync::OnceLock::new(), rows: (0..GRID).map(|_| std::sync::OnceLock::new()).collect() }
    }
}

impl std::fmt::Debug for Ridges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ridges({} rows made)", self.rows.iter().filter(|r| r.get().is_some()).count())
    }
}
