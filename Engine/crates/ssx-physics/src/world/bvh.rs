use glam::Vec3;

pub const LEAF_SIZE: usize = 4;

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub min: Vec3,
    pub max: Vec3,
    pub first: u32,
    pub count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Bvh {
    pub nodes: Vec<Node>,
    pub items: Vec<u32>,
}

impl Bvh {
    pub fn build(boxes: &[(Vec3, Vec3)]) -> Self {
        let mut bvh = Bvh { nodes: Vec::with_capacity(boxes.len() / 2 + 1), items: (0..boxes.len() as u32).collect() };
        if boxes.is_empty() {
            return bvh;
        }
        let centroids: Vec<Vec3> = boxes.iter().map(|(lo, hi)| (*lo + *hi) * 0.5).collect();
        bvh.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
        let mut stack = vec![(0usize, 0usize, boxes.len())];
        while let Some((node, start, end)) = stack.pop() {
            let items = &mut bvh.items[start..end];
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            let (mut clo, mut chi) = (lo, hi);
            for &i in items.iter() {
                let (bl, bh) = boxes[i as usize];
                lo = lo.min(bl);
                hi = hi.max(bh);
                clo = clo.min(centroids[i as usize]);
                chi = chi.max(centroids[i as usize]);
            }
            let ext = chi - clo;
            if items.len() <= LEAF_SIZE || ext.max_element() <= 0.0 {
                bvh.nodes[node] = Node { min: lo, max: hi, first: start as u32, count: items.len() as u32 };
                continue;
            }
            let axis = if ext.x >= ext.y && ext.x >= ext.z {
                0
            } else if ext.y >= ext.z {
                1
            } else {
                2
            };
            let mid = items.len() / 2;
            items.select_nth_unstable_by(mid, |&a, &b| {
                centroids[a as usize][axis].total_cmp(&centroids[b as usize][axis])
            });
            let left = bvh.nodes.len();
            bvh.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
            bvh.nodes.push(Node { min: Vec3::ZERO, max: Vec3::ZERO, first: 0, count: 0 });
            bvh.nodes[node] = Node { min: lo, max: hi, first: left as u32, count: 0 };
            stack.push((left, start, start + mid));
            stack.push((left + 1, start + mid, end));
        }
        bvh
    }

    pub fn traverse(&self, mut enter: impl FnMut(Vec3, Vec3) -> bool, mut visit: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack: Vec<u32> = Vec::with_capacity(64);
        stack.push(0);
        while let Some(n) = stack.pop() {
            let node = self.nodes[n as usize];
            if !enter(node.min, node.max) {
                continue;
            }
            if node.count > 0 {
                for &item in &self.items[node.first as usize..(node.first + node.count) as usize] {
                    visit(item);
                }
            } else {
                stack.push(node.first + 1);
                stack.push(node.first);
            }
        }
    }

    pub fn memory_bytes(&self) -> usize {
        self.nodes.len() * std::mem::size_of::<Node>() + self.items.len() * 4
    }
}

pub fn segment_hits_box(origin: Vec3, inv_dir: Vec3, max_t: f32, min: Vec3, max: Vec3, pad: f32) -> bool {
    let t0 = (min - Vec3::splat(pad) - origin) * inv_dir;
    let t1 = (max + Vec3::splat(pad) - origin) * inv_dir;
    let tmin = t0.min(t1).max_element().max(0.0);
    let tmax = t0.max(t1).min_element().min(max_t);
    tmin <= tmax
}

pub fn sphere_hits_box(center: Vec3, radius: f32, min: Vec3, max: Vec3) -> bool {
    center.clamp(min, max).distance_squared(center) <= radius * radius
}
