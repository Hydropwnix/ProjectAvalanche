use glam::Vec3;

const FLAG_IN_AIR: u8 = 0x08;
const FLAG_WINGSUIT: u8 = 0x40;

#[derive(Clone, Debug)]
pub struct CourseLine {
    pub id: u32,
    pub points: Vec<Vec3>,
    pub along: Vec<f32>,
    pub speed: Vec<f32>,
    pub vy: Vec<f32>,
    pub in_air: Vec<bool>,
    pub wingsuit: Vec<bool>,
    pub jumps: Vec<f32>,
}

impl CourseLine {
    pub fn from_replay(r: &ssx_assets::ghosts::GhostReplay) -> Self {
        let all: Vec<Vec3> = r.poses.iter().map(|p| Vec3::from_array(p.trajectory_position)).collect();
        let keep = forward_samples(&all);
        let points: Vec<Vec3> = keep.iter().map(|&i| all[i]).collect();
        let mut along = Vec::with_capacity(points.len());
        let mut d = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                d += p.distance(points[i - 1]);
            }
            along.push(d);
        }
        let poses: Vec<&ssx_assets::ghosts::GhostPose> = keep.iter().map(|&i| &r.poses[i]).collect();
        let mut line = CourseLine {
            jumps: Vec::new(),
            id: r.id,
            along,
            speed: poses.iter().map(|p| Vec3::from_array(p.velocity).length()).collect(),
            vy: poses.iter().map(|p| p.velocity[1]).collect(),
            in_air: poses.iter().map(|p| p.flags & FLAG_IN_AIR != 0).collect(),
            wingsuit: poses.iter().map(|p| p.flags & FLAG_WINGSUIT != 0).collect(),
            points,
        };
        line.jumps = line.find_jumps();
        line
    }

    pub fn nearest_disc(big_root: &std::path::Path, start: Vec3, end: Vec3) -> anyhow::Result<(Self, f32, f32)> {
        let csv = ssx_assets::content::data::events::ghost_db_text(big_root)?;
        let mut ids: Vec<u32> = crate::event::parse_ghost_db(&csv).iter().map(|(_, r)| r.ghost_id as u32).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut best: Option<((u32, f32), Self, f32, f32)> = None;
        for id in ids {
            let Ok(r) = ssx_assets::ghosts::GhostReplay::load(big_root, id) else { continue };
            let line = Self::from_replay(&r);
            let Some((is, ds)) = line.nearest_in(start, 0..line.points.len()) else { continue };
            let Some((_, de)) = line.nearest_in(end, is..line.points.len()) else { continue };
            if ds > 80.0 || de > 150.0 {
                continue;
            }
            let key = (r.header.build, -(ds + de));
            if best.as_ref().is_some_and(|b| (b.0.0, b.0.1) >= key) {
                continue;
            }
            best = Some((key, line, ds, de));
        }
        best.map(|(_, l, ds, de)| (l, ds, de)).ok_or_else(|| anyhow::anyhow!("no disc replay passes the level start"))
    }

    pub(super) fn nearest_in(&self, p: Vec3, range: std::ops::Range<usize>) -> Option<(usize, f32)> {
        range
            .map(|i| (i, self.points[i].distance_squared(p)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, d)| (i, d.sqrt()))
    }

    pub(super) fn at(&self, d: f32) -> (Vec3, usize) {
        let n = self.points.len();
        let k = self.along.partition_point(|&a| a < d).min(n - 1);
        if k == 0 {
            return (self.points[0], 0);
        }
        let (a0, a1) = (self.along[k - 1], self.along[k]);
        let t = if a1 > a0 { ((d - a0) / (a1 - a0)).clamp(0.0, 1.0) } else { 1.0 };
        let (p0, p1, p2, p3) =
            (self.points[k.saturating_sub(2)], self.points[k - 1], self.points[k], self.points[(k + 1).min(n - 1)]);
        let (t2, t3) = (t * t, t * t * t);
        let q = 0.5
            * (2.0 * p1
                + (p2 - p0) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3);
        (q, k)
    }

    pub(super) fn project(&self, p: Vec3, range: std::ops::Range<usize>) -> Option<(usize, f32, f32)> {
        let n = self.points.len();
        range
            .filter(|&i| i + 1 < n)
            .map(|i| {
                let (a, b) = (self.points[i], self.points[i + 1]);
                let ab = b - a;
                let l2 = ab.length_squared();
                let t = if l2 > 1e-6 { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
                (i, t, (a + ab * t).distance(p))
            })
            .min_by(|x, y| x.2.total_cmp(&y.2))
    }

    pub(super) fn along_at(&self, i: usize, t: f32) -> f32 {
        let next = self.along.get(i + 1).copied().unwrap_or(self.along[i]);
        self.along[i] + (next - self.along[i]) * t
    }

    pub(super) fn find_jumps(&self) -> Vec<f32> {
        (0..self.points.len().saturating_sub(1))
            .filter(|&i| !self.in_air[i] && self.in_air[i + 1] && self.vy[i + 1] - self.vy[i] > JUMP_VY)
            .map(|i| 0.5 * (self.along[i] + self.along[i + 1]))
            .collect()
    }
}

fn forward_samples(p: &[Vec3]) -> Vec<usize> {
    let mut keep: Vec<usize> = Vec::with_capacity(p.len());
    let mut along: Vec<f32> = Vec::with_capacity(p.len());
    for (i, &q) in p.iter().enumerate() {
        if let Some(&last) = keep.last() {
            let step = q.distance(p[last]);
            if step > WARP_M {
                break;
            }
            let end = along.last().copied().unwrap_or(0.0);
            let back =
                (0..keep.len().saturating_sub(1)).rev().filter(|&j| along[j + 1] < end - RETRACE_BACK_M).find(|&j| {
                    let (a, b) = (p[keep[j]], p[keep[j + 1]]);
                    let ab = b - a;
                    let t = if ab.length_squared() > 1e-6 {
                        ((q - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    (a + ab * t).distance(q) < RETRACE_M
                });
            if let Some(j) = back {
                keep.truncate(j + 1);
                along.truncate(j + 1);
            }
            let end = along.last().copied().unwrap_or(0.0);
            along.push(end + q.distance(p[*keep.last().unwrap_or(&i)]));
        } else {
            along.push(0.0);
        }
        keep.push(i);
    }
    keep
}

const RETRACE_M: f32 = 3.0;
const RETRACE_BACK_M: f32 = 10.0;
const WARP_M: f32 = 200.0;

const JUMP_VY: f32 = 0.08;
