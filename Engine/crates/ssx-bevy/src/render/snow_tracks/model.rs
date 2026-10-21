use std::collections::VecDeque;

use bevy::math::{IVec2, Mat3, Quat, Vec2, Vec3, Vec4};
use bevy::tasks::{ComputeTaskPool, TaskPool};

pub const TEXELS: usize = 128;
pub const MAX_TILES: usize = 256;
pub const TILE_WORLD: f32 = 3.0;
pub const LOOKUP_MAX: i32 = 0x1000;
pub const NO_SLICE: u16 = 0xFFFF;
pub const STAMPS: usize = 16;
pub const BOX_NEAR: f32 = 0.3;
pub const BOX_FAR: f32 = 2.0;
pub const DISP_OFFSET: f32 = -0.3;
pub const DISP_SCALE: f32 = 1.0;
pub const VELOCITY_SCALE: f32 = 0.3;
pub const VELOCITY_RADIUS: i32 = 5;
pub const NORMAL_SCALE: f32 = 1.0;
pub const COVERED: f32 = 0.85;
pub const SPRAY_SCALE: f32 = 100.0;
pub const NOISE_DEPTH_GAIN: f32 = 2.0;
pub const NOISE_DEPTH_BIAS: f32 = 0.1;
pub const NOISE_SPRAY: f32 = 0.1;
pub const NOISE_MOTION: f32 = 0.05;

pub fn unorm8(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

pub fn unorm(b: u8) -> f32 {
    f32::from(b) / 255.0
}

pub const UNTOUCHED: [u8; 4] = [128, 128, 128, 191];
pub const VELDISP_CLEAR: [u8; 4] = [128, 128, 128, 255];

#[derive(Clone, Copy, Debug)]
pub struct Caster {
    pub orientation: Quat,
    pub prev_orientation: Quat,
    pub contact: Vec3,
    pub prev_contact: Vec3,
    pub normal: Vec3,
    pub depth: f32,
    pub spray: Vec3,
}

pub fn ground_quat(n: Vec3) -> Quat {
    let n = n.normalize();
    let reference = if n.z >= n.x { Vec3::X } else { Vec3::Z };
    let z = reference.cross(n).normalize();
    let x = n.cross(z);
    Quat::from_mat3(&bevy::math::Mat3::from_cols(x, n, z))
}

fn hamilton(a: Quat, b: Quat) -> Quat {
    a * b
}

#[derive(Clone, Copy, Debug)]
pub struct CasterTransforms {
    contact: Vec3,
    ground: Mat3,
    ground_prev: Mat3,
    contact_delta: Vec3,
    depth: f32,
}

impl CasterTransforms {
    pub fn new(c: &Caster) -> Self {
        let qn_inv = ground_quat(c.normal).inverse();
        let delta_rot = hamilton(c.orientation.inverse(), c.prev_orientation).normalize();
        CasterTransforms {
            contact: c.contact,
            ground: Mat3::from_quat(qn_inv),
            ground_prev: Mat3::from_quat(delta_rot * qn_inv),
            contact_delta: c.prev_contact - c.contact,
            depth: c.depth,
        }
    }

    pub fn ground(&self, p: Vec3) -> Vec3 {
        let v = self.ground * (p - self.contact);
        Vec3::new(v.x, v.z, v.y + 1.0 - self.depth)
    }

    pub fn ground_prev(&self, p: Vec3) -> Vec3 {
        let v = self.ground_prev * (p - self.contact) + self.contact_delta;
        Vec3::new(v.x, v.z, v.y + 1.0 - self.depth)
    }

    pub fn view(&self, p: Vec3) -> Vec3 {
        let d = p - self.contact;
        let half = TILE_WORLD * 0.5;
        let x = (d.x / half + 1.0) * 0.5 * TEXELS as f32;
        let y = (1.0 - d.z / half) * 0.5 * TEXELS as f32;
        Vec3::new(x, y, (d.y - (BOX_NEAR - 1.0)) / (BOX_FAR - BOX_NEAR))
    }

    pub fn texel(&self, p: Vec3) -> Vec4 {
        let o0 = self.ground(p);
        let v = ((o0 - self.ground_prev(p)) * VELOCITY_SCALE).clamp(Vec3::splat(-1.0), Vec3::ONE);
        (v * 0.5 + 0.5).extend((o0.z + DISP_OFFSET) / DISP_SCALE)
    }
}

#[derive(Clone)]
pub struct Map128 {
    pub texels: Vec<[u8; 4]>,
}

impl Map128 {
    pub fn filled(v: [u8; 4]) -> Self {
        Map128 { texels: vec![v; TEXELS * TEXELS] }
    }

    #[cfg(test)]
    pub fn at(&self, x: i32, y: i32) -> [u8; 4] {
        let c = |v: i32| v.clamp(0, TEXELS as i32 - 1) as usize;
        self.texels[c(y) * TEXELS + c(x)]
    }
}

pub fn disp_map(vel: &Map128) -> Map128 {
    disp_map_with(vel, true)
}

fn disp_map_with(vel: &Map128, skip: bool) -> Map128 {
    const AXIS: f32 = 0.717_878_16;
    const DIAG: f32 = 1.015_228_4 * 0.577_35;
    const BIAS: f32 = 0.015_228_425;
    let means = corner_means(vel);
    let n = TEXELS as i32;
    let tap = |x: i32, y: i32| {
        let m = means[y.clamp(0, n) as usize * (TEXELS + 1) + x.clamp(0, n) as usize];
        (2.0 * m.x - 1.0, 2.0 * m.y - 1.0, m.w < COVERED)
    };
    let r = VELOCITY_RADIUS;
    let (mut lo, mut hi) = ((n, n), (-1, -1));
    for (i, m) in means.iter().enumerate() {
        if m.w < COVERED {
            let (x, y) = ((i % (TEXELS + 1)) as i32, (i / (TEXELS + 1)) as i32);
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
    }
    let reaches = |x: i32, y: i32| {
        let c = |v: i32| v.clamp(0, n);
        c(x - r) <= hi.0 && c(x + r) >= lo.0 && c(y - r) <= hi.1 && c(y + r) >= lo.1
    };
    let mut out = Map128::filled([0; 4]);
    for y in 0..TEXELS as i32 {
        for x in 0..TEXELS as i32 {
            let h = unorm8(means[y as usize * (TEXELS + 1) + x as usize].w);
            if skip && !reaches(x, y) {
                out.texels[y as usize * TEXELS + x as usize] = [unorm8(0.0), h, unorm8(0.0), h];
                continue;
            }
            let mut sum = 0.0;
            let mut add = |(vx, vz, covered): (f32, f32, bool), term: fn(f32, f32) -> f32| {
                if covered {
                    sum += term(vx, vz);
                }
            };
            add(tap(x + r, y), |vx, _| -AXIS * vx - BIAS);
            add(tap(x - r, y), |vx, _| AXIS * vx - BIAS);
            add(tap(x, y - r), |_, vz| -AXIS * vz - BIAS);
            add(tap(x, y + r), |_, vz| AXIS * vz - BIAS);
            add(tap(x + r, y - r), |vx, vz| -DIAG * (vz + vx) - BIAS);
            add(tap(x - r, y + r), |vx, vz| DIAG * (vz + vx) - BIAS);
            add(tap(x - r, y - r), |vx, vz| DIAG * (vx - vz) - BIAS);
            add(tap(x + r, y + r), |vx, vz| DIAG * (vz - vx) - BIAS);
            let rim = unorm8(sum.max(0.0));
            out.texels[y as usize * TEXELS + x as usize] = [rim, h, rim, h];
        }
    }
    out
}

fn corner_means(m: &Map128) -> Vec<Vec4> {
    let f: Vec<Vec4> = m.texels.iter().map(|t| Vec4::new(unorm(t[0]), unorm(t[1]), unorm(t[2]), unorm(t[3]))).collect();
    let last = TEXELS as i32 - 1;
    let at = |x: i32, y: i32| f[y.clamp(0, last) as usize * TEXELS + x.clamp(0, last) as usize];
    let mut out = Vec::with_capacity((TEXELS + 1) * (TEXELS + 1));
    for j in 0..=TEXELS as i32 {
        for i in 0..=TEXELS as i32 {
            out.push((at(i - 1, j - 1) + at(i, j - 1) + at(i - 1, j) + at(i, j)) * 0.25);
        }
    }
    out
}

pub struct Noise {
    pub size: usize,
    pub texels: Vec<f32>,
}

impl Noise {
    pub fn tile_sum(&self) -> Vec<f32> {
        let n = TEXELS as f32;
        (0..TEXELS * TEXELS)
            .map(|i| {
                let uv = Vec2::new((i % TEXELS) as f32, (i / TEXELS) as f32) / n;
                self.sample(uv.x * 3.0, uv.y * 3.0) + self.sample(uv.x, uv.y)
            })
            .collect()
    }

    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let n = self.size as i32;
        let fx = u * n as f32 - 0.5;
        let fy = v * n as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = (fx - x0, fy - y0);
        let w = |x: i32, y: i32| self.texels[(y.rem_euclid(n) * n + x.rem_euclid(n)) as usize];
        let (x0, y0) = (x0 as i32, y0 as i32);
        let top = w(x0, y0) + (w(x0 + 1, y0) - w(x0, y0)) * ax;
        let bottom = w(x0, y0 + 1) + (w(x0 + 1, y0 + 1) - w(x0, y0 + 1)) * ax;
        top + (bottom - top) * ay
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamp {
    pub col0: Vec4,
    pub col1: Vec4,
}

fn yaw(q: Quat) -> f32 {
    let (x, y, z, w) = (q.x, q.y, q.z, q.w);
    let m00 = 1.0 - 2.0 * (y * y + z * z);
    let m01 = 2.0 * (x * y + z * w);
    let m02 = 2.0 * (x * z - y * w);
    (-m02).atan2((m00 * m00 + m01 * m01).sqrt())
}

fn slerp(a: Quat, b: Quat, t: f32) -> Quat {
    let mut b = b;
    let mut d = a.dot(b);
    if d < 0.0 {
        b = -b;
        d = -d;
    }
    if d > 0.996_194_7 {
        return (a * (1.0 - t) + b * t).normalize();
    }
    let theta = d.clamp(-1.0, 1.0).acos();
    let s = theta.sin();
    a * (((1.0 - t) * theta).sin() / s) + b * ((t * theta).sin() / s)
}

pub fn stamps(delta_rot: Quat, prev: Vec3, contact: Vec3, tile_origin: Vec2) -> [Stamp; STAMPS] {
    let px = TEXELS as f32;
    std::array::from_fn(|i| {
        let t = i as f32 / STAMPS as f32;
        let pos = prev + (contact - prev) * t;
        let a = yaw(slerp(delta_rot, Quat::IDENTITY, t));
        let (s, c) = a.sin_cos();
        let off = (tile_origin - Vec2::new(pos.x, pos.z) / TILE_WORLD) * px;
        let x0 = c * off.x - s * off.y;
        let y0 = s * off.x + c * off.y;
        Stamp {
            col0: Vec4::new(c / px, -s / px, 0.0, x0 / px + 0.5),
            col1: Vec4::new(s / px, c / px, 0.0, y0 / px + 0.5),
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Rect {
    pub const FULL: Rect = Rect { x0: 0, y0: 0, x1: TEXELS, y1: TEXELS };

    fn from_f32(lo: Vec2, hi: Vec2) -> Option<Rect> {
        let c = |v: f32| v.clamp(0.0, TEXELS as f32);
        let r = Rect {
            x0: c(lo.x.floor()) as usize,
            y0: c(lo.y.floor()) as usize,
            x1: c(hi.x.ceil()) as usize,
            y1: c(hi.y.ceil()) as usize,
        };
        (r.x0 < r.x1 && r.y0 < r.y1).then_some(r)
    }

    fn union(self, o: Rect) -> Rect {
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    fn grow(self, n: usize) -> Rect {
        Rect {
            x0: self.x0.saturating_sub(n),
            y0: self.y0.saturating_sub(n),
            x1: (self.x1 + n).min(TEXELS),
            y1: (self.y1 + n).min(TEXELS),
        }
    }
}

#[derive(Default)]
pub struct Changes {
    pub lookup: Vec<(usize, u16)>,
    pub slices: Vec<(u16, Rect)>,
}

pub struct TileSystem {
    pub lookup: Vec<u16>,
    pub size: IVec2,
    pub offset: IVec2,
    pub slices: Vec<[u8; 4]>,
    owner: Vec<i32>,
    lru: VecDeque<u16>,
    pending: Vec<(usize, u16)>,
}

impl TileSystem {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        let third = 1.0 / TILE_WORLD;
        let w = (((max.x - min.x) * third) as i32).clamp(1, LOOKUP_MAX);
        let h = (((max.z - min.z) * third) as i32).clamp(1, LOOKUP_MAX);
        let offset = IVec2::new(w - (max.x * third) as i32, -((min.z * third) as i32));
        TileSystem {
            lookup: vec![NO_SLICE; (w * h) as usize],
            size: IVec2::new(w, h),
            offset,
            slices: vec![UNTOUCHED; MAX_TILES * TEXELS * TEXELS],
            owner: vec![-1; MAX_TILES],
            lru: (0..MAX_TILES as u16).rev().collect(),
            pending: Vec::new(),
        }
    }

    pub fn begin_frame(&mut self, changes: &mut Changes) {
        for (i, v) in self.pending.drain(..) {
            self.lookup[i] = v;
            changes.lookup.push((i, v));
        }
    }

    pub fn slices_in_use(&self) -> usize {
        self.owner.iter().filter(|&&o| o >= 0).count()
    }

    pub fn tiles_around(&self, p: Vec3) -> [IVec2; 4] {
        let t = IVec2::new(
            (p.x / TILE_WORLD - 0.5).floor() as i32 + self.offset.x,
            (p.z / TILE_WORLD - 0.5).floor() as i32 + self.offset.y,
        );
        [t, t + IVec2::X, t + IVec2::Y, t + IVec2::ONE]
    }

    fn slice_for(&mut self, tile: IVec2, changes: &mut Changes) -> Option<u16> {
        if tile.x < 0 || tile.y < 0 || tile.x >= self.size.x || tile.y >= self.size.y {
            return None;
        }
        let index = (tile.y * self.size.x + tile.x) as usize;
        let current = self.lookup[index];
        if current != NO_SLICE {
            if let Some(at) = self.lru.iter().position(|&s| s == current) {
                self.lru.remove(at);
            }
            self.lru.push_front(current);
            return Some(current);
        }
        let slice = self.lru.pop_back()?;
        self.lru.push_front(slice);
        let old = self.owner[slice as usize];
        if old >= 0 {
            let base = slice as usize * TEXELS * TEXELS;
            self.slices[base..base + TEXELS * TEXELS].fill(UNTOUCHED);
            changes.slices.push((slice, Rect::FULL));
            self.pending.push((old as usize, NO_SLICE));
        }
        self.owner[slice as usize] = index as i32;
        self.pending.push((index, slice));
        Some(slice)
    }
}

fn active_uv(disp: &Map128) -> Option<Option<(Vec2, Vec2)>> {
    let (mut lo, mut hi) = ((TEXELS, TEXELS), (0, 0));
    for (i, t) in disp.texels.iter().enumerate() {
        if t[0] != 0 || t[3] != 255 {
            let (x, y) = (i % TEXELS, i / TEXELS);
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x + 1), hi.1.max(y + 1));
        }
    }
    if hi.0 == 0 {
        return None;
    }
    if lo.0 == 0 || lo.1 == 0 || hi.0 == TEXELS || hi.1 == TEXELS {
        return Some(None);
    }
    let n = TEXELS as f32;
    Some(Some((
        Vec2::new(lo.0 as f32 - 0.5, lo.1 as f32 - 0.5) / n,
        Vec2::new(hi.0 as f32 + 0.5, hi.1 as f32 + 0.5) / n,
    )))
}

impl Stamp {
    fn u(&self, p: Vec2) -> f32 {
        self.col0.x * p.x + self.col0.y * p.y + self.col0.w
    }

    fn v_pre(&self, p: Vec2) -> f32 {
        self.col1.x * p.x + self.col1.y * p.y + self.col1.w
    }

    fn inverse(&self, u: f32, v_pre: f32) -> Vec2 {
        let n = TEXELS as f32;
        let (c, s) = (self.col0.x * n, self.col1.x * n);
        let (du, dv) = ((u - self.col0.w) * n, (v_pre - self.col1.w) * n);
        Vec2::new(c * du + s * dv, -s * du + c * dv)
    }

    fn footprint(&self, lo: Vec2, hi: Vec2) -> Option<Rect> {
        let corners = [(lo.x, lo.y), (hi.x, lo.y), (lo.x, hi.y), (hi.x, hi.y)].map(|(u, v)| self.inverse(u, 1.0 - v));
        let min = corners.iter().fold(Vec2::splat(f32::MAX), |a, &b| a.min(b));
        let max = corners.iter().fold(Vec2::splat(f32::MIN), |a, &b| a.max(b));
        Rect::from_f32(min - 1.0, max + 1.0)
    }
}

struct DispF {
    x: Vec<f32>,
    w: Vec<f32>,
}

impl DispF {
    fn new(m: &Map128) -> Self {
        DispF { x: m.texels.iter().map(|t| unorm(t[0])).collect(), w: m.texels.iter().map(|t| unorm(t[3])).collect() }
    }

    fn sample(&self, u: f32, v: f32) -> (f32, f32) {
        let n = TEXELS as f32;
        let (fx, fy) = (u * n - 0.5, v * n - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (ax, ay) = (fx - x0, fy - y0);
        let last = TEXELS as i32 - 1;
        let (xa, ya) = (x0 as i32, y0 as i32);
        let (xi0, xi1) = (xa.clamp(0, last) as usize, (xa + 1).clamp(0, last) as usize);
        let (yi0, yi1) = (ya.clamp(0, last) as usize * TEXELS, (ya + 1).clamp(0, last) as usize * TEXELS);
        let bl = |c: &[f32]| {
            let top = c[yi0 + xi0] + (c[yi0 + xi1] - c[yi0 + xi0]) * ax;
            let bottom = c[yi1 + xi0] + (c[yi1 + xi1] - c[yi1 + xi0]) * ax;
            top + (bottom - top) * ay
        };
        (bl(&self.x), bl(&self.w))
    }
}

const BAND: usize = 16;

struct TileJob {
    slice: u16,
    region: Rect,
    stamps: [Stamp; STAMPS],
    reach: [Option<Rect>; STAMPS],
}

#[derive(Clone, Copy)]
struct NoiseParams {
    scale: f32,
    offset: Vec2,
    intensity: f32,
}

impl TileSystem {
    pub fn update(&mut self, c: &Caster, disp: &Map128, noise: &[f32], changes: &mut Changes) {
        let delta_rot = hamilton(c.orientation.inverse(), c.prev_orientation).normalize();
        let motion = c.contact - c.prev_contact;
        let spray = -c.spray * SPRAY_SCALE;
        let np = NoiseParams {
            scale: c.depth * NOISE_DEPTH_GAIN - NOISE_DEPTH_BIAS,
            offset: Vec2::new(spray.x + motion.x, spray.z + motion.z),
            intensity: NOISE_SPRAY * Vec2::new(spray.x, spray.z).length()
                + NOISE_MOTION * Vec2::new(motion.x, motion.z).length(),
        };
        let active = active_uv(disp);
        let mut jobs = Vec::new();
        for tile in self.tiles_around(c.contact) {
            let Some(slice) = self.slice_for(tile, changes) else { continue };
            let st = stamps(delta_rot, c.prev_contact, c.contact, (tile - self.offset).as_vec2());
            let reach: [Option<Rect>; STAMPS] = std::array::from_fn(|k| match active {
                Some(Some((lo, hi))) => st[k].footprint(lo, hi),
                Some(None) => Some(Rect::FULL),
                None => None,
            });
            let mut region = reach.iter().flatten().copied().reduce(Rect::union);
            if np.intensity * np.scale != 0.0 {
                let centre = st[STAMPS - 1].inverse((1.5 - np.offset.x) / 3.0, (1.5 - np.offset.y) / 3.0);
                let r = TEXELS as f32 / TILE_WORLD + 1.0;
                if let Some(d) = Rect::from_f32(centre - r, centre + r) {
                    region = Some(region.map_or(d, |q| q.union(d)));
                }
            }
            if let Some(region) = region {
                jobs.push(TileJob { slice, region, stamps: st, reach });
            }
        }
        let disp = DispF::new(disp);
        let mut chunks: Vec<Option<&mut [[u8; 4]]>> = self.slices.chunks_mut(TEXELS * TEXELS).map(Some).collect();
        let mut slots: Vec<(&TileJob, &mut [[u8; 4]])> =
            jobs.iter().filter_map(|j| chunks[j.slice as usize].take().map(|d| (j, d))).collect();
        let disp = &disp;
        let pool = ComputeTaskPool::get_or_init(TaskPool::default);
        pool.scope(|scope| {
            for (job, data) in slots.iter_mut() {
                let job: &TileJob = job;
                for (b, band) in data.chunks_mut(TEXELS * BAND).enumerate() {
                    let rows = (b * BAND, (b + 1) * BAND);
                    if rows.1 > job.region.y0 && rows.0 < job.region.y1 {
                        scope.spawn(async move { accumulate(band, rows.0, job, disp, noise, np) });
                    }
                }
            }
        });
        let heights: Vec<Vec<f32>> = slots.iter().map(|(_, d)| d.iter().map(|t| unorm(t[3])).collect()).collect();
        pool.scope(|scope| {
            for ((job, data), acc) in slots.iter_mut().zip(&heights) {
                let r = job.region.grow(1);
                for (b, band) in data.chunks_mut(TEXELS * BAND).enumerate() {
                    let rows = (b * BAND, (b + 1) * BAND);
                    if rows.1 > r.y0 && rows.0 < r.y1 {
                        scope.spawn(async move { normals(band, rows.0, acc, r) });
                    }
                }
            }
        });
        changes.slices.extend(jobs.iter().map(|j| (j.slice, j.region.grow(1))));
    }
}

fn accumulate(band: &mut [[u8; 4]], y_base: usize, job: &TileJob, disp: &DispF, noise: &[f32], np: NoiseParams) {
    let r = job.region;
    let inside =
        |q: &Option<Rect>, x: usize, y: usize| q.is_some_and(|q| x >= q.x0 && x < q.x1 && y >= q.y0 && y < q.y1);
    for y in r.y0.max(y_base)..r.y1.min(y_base + band.len() / TEXELS) {
        for x in r.x0..r.x1 {
            let p = Vec2::new(x as f32, y as f32);
            let s = noise[y * TEXELS + x] * np.scale;
            let t = &mut band[(y - y_base) * TEXELS + x];
            let mut acc = unorm(t[3]);
            for (stamp, reach) in job.stamps.iter().zip(&job.reach) {
                if inside(reach, x, y) {
                    let (sx, sw) = disp.sample(stamp.u(p), 1.0 - stamp.v_pre(p));
                    acc = acc.min(sw) + s * sx;
                } else {
                    acc = acc.min(1.0);
                }
            }
            let last = &job.stamps[STAMPS - 1];
            let d = Vec2::new(last.u(p) * 3.0 - 1.5, last.v_pre(p) * 3.0 - 1.5) + np.offset;
            *t = [unorm8(acc + np.intensity * s * (1.0 - d.length()).clamp(0.0, 1.0)); 4];
        }
    }
}

fn normals(band: &mut [[u8; 4]], y_base: usize, acc: &[f32], r: Rect) {
    let last = TEXELS as i32 - 1;
    let a = |x: i32, y: i32| acc[y.clamp(0, last) as usize * TEXELS + x.clamp(0, last) as usize];
    let (y0, y1) = (r.y0.max(y_base), r.y1.min(y_base + band.len() / TEXELS));
    for y in y0 as i32..y1 as i32 {
        for x in r.x0 as i32..r.x1 as i32 {
            let gx = 2.0 * a(x + 1, y) + a(x + 1, y - 1) + a(x + 1, y + 1)
                - a(x - 1, y - 1)
                - 2.0 * a(x - 1, y)
                - a(x - 1, y + 1);
            let gy = 2.0 * a(x, y + 1) + a(x - 1, y + 1) + a(x + 1, y + 1)
                - a(x - 1, y - 1)
                - 2.0 * a(x, y - 1)
                - a(x + 1, y - 1);
            let nrm = Vec3::new(-gx, -gy, 1.0 / NORMAL_SCALE).normalize() * 0.5 + 0.5;
            let t = &mut band[(y as usize - y_base) * TEXELS + x as usize];
            t[..3].copy_from_slice(&[unorm8(nrm.x), unorm8(nrm.y), unorm8(nrm.z)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(m: [[f32; 3]; 4], p: Vec3) -> Vec3 {
        Vec3::from(m[0]) * p.x + Vec3::from(m[1]) * p.y + Vec3::from(m[2]) * p.z + Vec3::from(m[3])
    }

    fn near(a: Vec3, b: Vec3, tol: f32) -> bool {
        (a - b).abs().max_element() <= tol
    }

    #[test]
    fn caster_transforms_match_the_code() {
        let c = Caster {
            orientation: Quat::from_xyzw(0.0, 0.342_897_8, 0.0, 0.939_372_7),
            prev_orientation: Quat::from_xyzw(0.005_330_251, 0.366_272_53, -0.007_267_84, 0.930_507_6),
            contact: Vec3::new(10.0, 5.0, -7.0),
            prev_contact: Vec3::new(9.8, 5.02, -7.3),
            normal: Vec3::new(0.2, 0.95, -0.1),
            depth: 0.12,
            spray: Vec3::ZERO,
        };
        let t = CasterTransforms::new(&c);
        let gr = [
            [0.021109, -0.97855, 0.204926],
            [0.100265, 0.20601, 0.9734],
            [0.994737, 0.0, -0.102463],
            [6.250747, 8.755445, -6.753501],
        ];
        let grp = [
            [-0.025624, -0.975252, 0.219605],
            [0.120509, 0.215062, 0.969137],
            [0.992381, -0.051297, -0.112016],
            [6.400362, 8.018131, -6.925848],
        ];
        for p in [Vec3::new(10.5, 5.2, -6.1), Vec3::new(9.0, 4.4, -8.0), Vec3::new(11.3, 5.9, -7.7)] {
            assert!(near(t.ground(p), rows(gr, p), 2e-4), "{:?} vs {:?}", t.ground(p), rows(gr, p));
            assert!(near(t.ground_prev(p), rows(grp, p), 2e-4));
            let v = t.view(p);
            let clip = Vec3::new((p.x - 10.0) * 0.666_667, (p.z + 7.0) * 0.666_667, p.y * 0.588_235 - 2.529_412);
            let expect = Vec3::new((clip.x + 1.0) * 64.0, (1.0 - clip.y) * 64.0, clip.z);
            assert!(near(v, expect, 2e-4));
        }
    }

    #[test]
    fn stamps_match_the_code() {
        let q = Quat::from_xyzw(0.020_899_984, 0.104_499_92, 0.010_449_992, 0.994_250_35);
        let prev = Vec3::new(10.2, 3.0, -7.4);
        let st = stamps(q, prev, prev + Vec3::new(0.3, 0.02, 0.1), Vec2::new(3.0, -3.0));
        let expect = [
            (0, [0.007_642_7, -0.001_62, 0.219_286_8], [0.001_62, 0.007_642_7, -0.104_685_5]),
            (1, [0.007_663_1, -0.001_520_4, 0.205_713_6], [0.001_520_4, 0.007_663_1, -0.104_239_6]),
            (7, [0.007_758_5, -0.000_916_9, 0.123_622_2], [0.000_916_9, 0.007_758_5, -0.096_21]),
            (15, [0.007_811_8, -0.000_102_2, 0.013_68], [0.000_102_2, 0.007_811_8, -0.070_995_9]),
        ];
        for (i, c0, c1) in expect {
            let s = st[i];
            assert!(near(Vec3::new(s.col0.x, s.col0.y, s.col0.w), Vec3::from(c0), 2e-6), "stamp {i} {s:?}");
            assert!(near(Vec3::new(s.col1.x, s.col1.y, s.col1.w), Vec3::from(c1), 2e-6), "stamp {i} {s:?}");
        }
    }

    #[test]
    fn ground_frame_maps_up_to_the_normal() {
        for n in [Vec3::Y, Vec3::new(0.0, 0.8, 0.6), Vec3::new(0.6, 0.8, 0.0), Vec3::new(-0.5, 0.38, -0.78)] {
            let q = ground_quat(n);
            assert!(near(q * Vec3::Y, n.normalize(), 1e-5));
        }
        let q = ground_quat(Vec3::new(0.6, 0.8, 0.0));
        let e = Quat::from_xyzw(-0.223_61, -0.670_82, -0.223_61, 0.670_82);
        assert!(q.dot(e).abs() > 0.999_99);
    }

    #[test]
    fn tiles_are_recycled_least_recent_first() {
        let mut ts = TileSystem::new(Vec3::new(-300.0, 0.0, -300.0), Vec3::new(300.0, 10.0, 300.0));
        let mut ch = Changes::default();
        let a = ts.slice_for(IVec2::new(10, 10), &mut ch).unwrap();
        assert_eq!(a, 0);
        ts.begin_frame(&mut ch);
        assert_eq!(ts.slice_for(IVec2::new(10, 10), &mut ch), Some(0));
        for i in 0..MAX_TILES as i32 {
            assert!(ts.slice_for(IVec2::new(i % 100, 30 + i / 100), &mut ch).is_some());
        }
        ts.begin_frame(&mut ch);
        assert_eq!(ts.lookup[(10 * ts.size.x + 10) as usize], NO_SLICE);
        assert_eq!(ts.slice_for(IVec2::new(-1, 0), &mut ch), None);
    }

    #[test]
    fn technique0_fetches_at_texel_corners() {
        let mut vel = Map128::filled(VELDISP_CLEAR);
        for (x, y) in [(64, 64), (65, 64), (64, 65), (65, 65)] {
            vel.texels[y * TEXELS + x] = [255, 128, 128, unorm8(0.45)];
        }
        let disp = disp_map(&vel);
        let r = (2.0 + 2.0 * unorm(128)) / 4.0;
        let expect = unorm8(0.717_878_16 * (2.0 * r - 1.0) - 0.015_228_425);
        assert_eq!(disp.texels[65 * TEXELS + 69][0], expect);
        let a = (unorm(unorm8(0.45)) + 3.0) / 4.0;
        assert_eq!(disp.texels[64 * TEXELS + 64][3], unorm8(a));
        assert_eq!(disp.texels[65 * TEXELS + 65][3], unorm8(unorm(unorm8(0.45))));
    }

    #[test]
    fn skipping_unreached_texels_changes_nothing() {
        let mut seed = 777u32;
        let mut rnd = |n: u32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) % n
        };
        for blob in [(40, 50, 20), (0, 0, 6), (120, 3, 8), (70, 125, 3)] {
            let mut vel = Map128::filled(VELDISP_CLEAR);
            for _ in 0..200 {
                let (x, y) = (blob.0 + rnd(blob.2) as usize, blob.1 + rnd(blob.2) as usize);
                if x < TEXELS && y < TEXELS {
                    vel.texels[y * TEXELS + x] = [rnd(256) as u8, rnd(256) as u8, 128, rnd(200) as u8];
                }
            }
            assert!(disp_map_with(&vel, true).texels == disp_map_with(&vel, false).texels, "{blob:?}");
        }
    }

    #[test]
    fn neutral_dispmap_changes_nothing() {
        let vel = Map128::filled(VELDISP_CLEAR);
        let disp = disp_map(&vel);
        assert!(disp.texels.iter().all(|t| *t == [0, 255, 0, 255]));
        assert!(active_uv(&disp).is_none());
    }
}
