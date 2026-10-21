use super::*;

#[derive(Clone, Copy, Debug)]
pub struct SegmentQuery {
    pub start: Vec3,
    pub end: Vec3,
    pub radius: f32,
    pub two_sided: bool,
    pub class_mask: u32,
}

impl SegmentQuery {
    pub fn line(start: Vec3, end: Vec3, class_mask: u32) -> Self {
        Self { start, end, radius: 0.0, two_sided: false, class_mask }
    }

    pub fn sphere(start: Vec3, end: Vec3, radius: f32, class_mask: u32) -> Self {
        Self { start, end, radius, two_sided: false, class_mask }
    }
}

pub const SEGMENT_MIN_DELTA: f32 = 1.525_878_9e-5;

impl CollisionWorld {
    pub fn ray_cast(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Option<Hit> {
        self.ray_cast_masked(origin, dir, max_t, 0)
    }

    pub fn ray_cast_masked(&self, origin: Vec3, dir: Vec3, max_t: f32, class_mask: u32) -> Option<Hit> {
        self.segment_query(&SegmentQuery::line(origin, origin + dir * max_t, class_mask))
    }

    pub fn sphere_cast(&self, origin: Vec3, dir: Vec3, max_t: f32, radius: f32) -> Option<Hit> {
        self.sphere_cast_masked(origin, dir, max_t, radius, 0)
    }

    pub fn sphere_cast_masked(&self, origin: Vec3, dir: Vec3, max_t: f32, radius: f32, class_mask: u32) -> Option<Hit> {
        self.segment_query(&SegmentQuery::sphere(origin, origin + dir * max_t, radius, class_mask))
    }

    pub fn segment_query(&self, q: &SegmentQuery) -> Option<Hit> {
        self.segment_query_with(q, true)
    }

    pub(super) fn segment_query_with(&self, q: &SegmentQuery, reject: bool) -> Option<Hit> {
        const SEGMENT_BOX_MARGIN: f32 = 0.05;
        let delta = q.end - q.start;
        if delta.abs().max_element() <= SEGMENT_MIN_DELTA {
            return None;
        }
        let len = delta.length();
        let dir = delta / len;
        let inv = dir.recip();
        let r = q.radius;
        let best = std::cell::Cell::new(f32::INFINITY);
        let mut hit: Option<(f32, TriHit, u32, u32)> = None;
        let reach = |b: f32| b.min(len + r);
        let end = q.start + dir * len;
        let grow = Vec3::splat(r + SEGMENT_BOX_MARGIN);
        let (box_lo, box_hi) = (q.start.min(end).min(q.end) - grow, q.start.max(end).max(q.end) + grow);
        self.bvh.traverse(
            |lo, hi| segment_hits_box(q.start, inv, reach(best.get()), lo, hi, r),
            |ci| {
                let c = &self.clusters[ci as usize];
                if !class_passes(q.class_mask, c.class)
                    || !segment_hits_box(q.start, inv, reach(best.get()), c.min, c.max, r)
                {
                    return;
                }
                for tri in c.tri_start as usize..(c.tri_start + c.tri_count as u32) as usize {
                    let [a, b, cc] = self.corners(c, tri);
                    let (lo, hi) = (a.min(b).min(cc), a.max(b).max(cc));
                    let slack = Vec3::splat((hi - lo).max_element() * 0.01);
                    if reject && ((lo - slack).cmpgt(box_hi).any() || (hi + slack).cmplt(box_lo).any()) {
                        continue;
                    }
                    let found = if r > 0.0 {
                        sweep_sphere_triangle(q.start, dir, len, r, a, b, cc, q.two_sided)
                    } else {
                        ray_triangle(q.start, dir, a, b, cc).filter(|t| t.0 <= len).map(|(t, _, _)| {
                            let n = face_normal(a, b, cc).unwrap_or(-dir);
                            TriHit { t, point: q.start + dir * t, normal: n }
                        })
                    };
                    if let Some(h) = found {
                        let score = h.t + h.point.distance(q.start + dir * h.t);
                        if score < best.get() {
                            best.set(score);
                            hit = Some((score, h, ci, tri as u32));
                        }
                    }
                }
            },
        );
        let (_, h, ci, tri) = hit?;
        let [a, b, cc] = self.corners(&self.clusters[ci as usize], tri as usize);
        let (_, bary) = closest_point_triangle(h.point, a, b, cc);
        Some(Hit {
            t: h.t,
            point: h.point,
            normal: self.smooth_normal(ci, tri, bary, h.normal),
            contact_normal: h.normal,
            cluster: ci,
            tri,
            bary,
        })
    }

    pub fn particle_sweep(&self, start: Vec3, delta: Vec3) -> Option<(Vec3, Vec3)> {
        let len = delta.length();
        if len <= SEGMENT_MIN_DELTA {
            return None;
        }
        let inv = (delta / len).recip();
        let mut best: Option<(f32, Vec3, Vec3)> = None;
        self.bvh.traverse(
            |lo, hi| segment_hits_box(start, inv, len, lo, hi, 0.0),
            |ci| {
                let c = &self.clusters[ci as usize];
                if !segment_hits_box(start, inv, len, c.min, c.max, 0.0) {
                    return;
                }
                let mut last = None;
                for tri in c.tri_start as usize..(c.tri_start + c.tri_count as u32) as usize {
                    let [a, b, cc] = self.corners(c, tri);
                    if let Some(h) = particle_triangle(start, delta, a, b, cc) {
                        last = Some(h);
                    }
                }
                // TODO(guess): the nearest cluster result is kept (game: per job part the first cluster with a hit, the merge 0x82C665C8 the last part — order from `FUN_82C6F738`'s gather and the four-part split 0x83454580 / 0x83454228); settle: the RW world's candidate order and the job runtime's part assignment.
                if last.is_some() && best.is_some() {
                    crate::guess_hit!();
                }
                if let Some((t, p, n)) = last
                    && best.is_none_or(|b| t < b.0)
                {
                    best = Some((t, p, n));
                }
            },
        );
        best.map(|(_, p, n)| (p, n))
    }

    pub fn overlap_sphere(&self, center: Vec3, radius: f32, out: &mut Vec<Contact>) {
        out.clear();
        self.bvh.traverse(
            |lo, hi| sphere_hits_box(center, radius, lo, hi),
            |ci| {
                let c = &self.clusters[ci as usize];
                if !sphere_hits_box(center, radius, c.min, c.max) {
                    return;
                }
                for tri in c.tri_start as usize..(c.tri_start + c.tri_count as u32) as usize {
                    let [a, b, cc] = self.corners(c, tri);
                    let Some(n) = face_normal(a, b, cc) else { continue };
                    if (center - a).dot(n) < 0.0 {
                        continue;
                    }
                    let (point, bary) = closest_point_triangle(center, a, b, cc);
                    let d = center.distance(point);
                    if d >= radius {
                        continue;
                    }
                    let normal = if d > 1e-6 { (center - point) / d } else { n };
                    out.push(Contact { point, normal, depth: radius - d, cluster: ci, tri: tri as u32, bary });
                }
            },
        );
    }

    pub fn surface_weights(&self, cluster: u32, tri: u32, bary: [f32; 3], normal_y: f32) -> Weights {
        let c = &self.clusters[cluster as usize];
        if c.is_terrain() && c.surface_id == TERRAIN_SURFACE_ID {
            let v = self.triangle_vertices(cluster, tri);
            surface::terrain_contact_weights(v.map(|i| self.nibbles[i]), bary)
        } else {
            surface::object_contact_weights(c.surface_id, normal_y)
        }
    }

    pub fn smooth_normal(&self, cluster: u32, tri: u32, bary: [f32; 3], contact: Vec3) -> Vec3 {
        if !self.clusters[cluster as usize].is_terrain() {
            return contact;
        }
        let v = self.triangle_vertices(cluster, tri);
        let n = v.iter().zip(bary).fold(Vec3::ZERO, |acc, (&i, b)| acc + self.normals[i] * b);
        match n.try_normalize() {
            Some(n) if n.dot(contact) < 0.0 => -n,
            Some(n) => n,
            None => contact,
        }
    }
}
