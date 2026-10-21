#![allow(clippy::neg_cmp_op_on_partial_ord)]

use glam::{Vec2, Vec3, Vec4};

use super::surface_analysis::{GRID, SurfaceAnalysis};

const TINY: f32 = 1.525_879e-5;
const EPS: f32 = 1.192_092_9e-7;
const NEW: u8 = 0;
const GOAL: u8 = 1;
const OPEN: u8 = 2;
const CLOSED: u8 = 3;
const EXCLUDED: f32 = 2e20;
const CUTOFF: f32 = 1e20;
const LATERAL_WEIGHT: f32 = 2.0;
const RIDGE_WEIGHT: f32 = 80.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathPoints {
    pub dir_a: Vec3,
    pub pos_a: Vec3,
    pub dir_b: Vec3,
    pub pos_b: Vec3,
    pub peak_a: f32,
    pub peak_b: f32,
}

#[derive(Clone, Copy, Default)]
struct Node {
    state: u8,
    parent: u32,
    g: f32,
}

const NODES: usize = GRID * GRID + 1;

struct NodeGrid {
    nodes: Vec<Node>,
    stamps: Vec<u32>,
    generation: u32,
}

thread_local! {
    static NODE_GRIDS: std::cell::RefCell<Vec<NodeGrid>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl NodeGrid {
    fn take() -> Self {
        let mut g = NODE_GRIDS.with(|p| p.borrow_mut().pop()).unwrap_or_else(|| NodeGrid {
            nodes: vec![Node::default(); NODES],
            stamps: vec![0; NODES],
            generation: 0,
        });
        g.generation = g.generation.wrapping_add(1);
        if g.generation == 0 {
            g.stamps.fill(0);
            g.generation = 1;
        }
        g
    }

    fn get(&self, i: u32) -> Node {
        let i = i as usize;
        if self.stamps[i] == self.generation { self.nodes[i] } else { Node::default() }
    }

    fn set(&mut self, i: u32, n: Node) {
        let i = i as usize;
        (self.stamps[i], self.nodes[i]) = (self.generation, n);
    }

    fn set_state(&mut self, i: u32, state: u8) {
        let n = self.get(i);
        self.set(i, Node { state, ..n });
    }
}

pub struct Solver<'a> {
    sa: &'a SurfaceAnalysis,
    start: Vec4,
    target: Vec4,
    velg: Vec4,
    dir: Vec4,
    sin: f32,
    cos: f32,
    step: f32,
    nodes: Option<NodeGrid>,
    heap: Vec<(u32, f32)>,
    pub path: Vec<u32>,
}

impl Drop for Solver<'_> {
    fn drop(&mut self) {
        if let Some(g) = self.nodes.take() {
            NODE_GRIDS.with(|p| p.borrow_mut().push(g));
        }
    }
}

fn cell_of(idx: u32) -> (i32, i32) {
    (((idx - 1) % 256) as i32, ((idx - 1) / 256) as i32)
}

impl<'a> Solver<'a> {
    pub fn solve(sa: &'a SurfaceAnalysis, pos: Vec3, vel: Vec3, step: f32, k: f32, angle: f32) -> Self {
        let pg = sa.grid(pos).extend(1.0);
        let vg = sa.grid_dir(vel).extend(0.0);
        let l2 = vg.x * vg.x + vg.y * vg.y;
        let l3 = vg.truncate().length();
        let dir = if l2 < TINY { Vec4::new(0.0, 1.0, 0.0, 0.0) } else { vg * (1.0 / l2.sqrt()) };
        let velg = dir * l3;
        let start = (pg + 0.5).floor();
        let target = velg * step * k + start;
        let mut s = Solver {
            sa,
            start,
            target,
            velg,
            dir,
            sin: angle.sin(),
            cos: angle.cos(),
            step,
            nodes: Some(NodeGrid::take()),
            heap: Vec::new(),
            path: Vec::new(),
        };
        let (cx, cy) = (start.x, start.y);
        let moving = velg.x.abs() > TINY || velg.y.abs() > TINY;
        let inside = |v: f32| (0.0..GRID as f32).contains(&v);
        if moving && inside(cx) && inside(cy) {
            s.push(cy as u32 * 256 + cx as u32 + 1, 0.0);
            s.run();
        }
        s
    }

    fn grid(&self) -> &NodeGrid {
        self.nodes.as_ref().expect("the node grid (until dropped)")
    }

    fn grid_mut(&mut self) -> &mut NodeGrid {
        self.nodes.as_mut().expect("the node grid (until dropped)")
    }

    fn dot2(&self, x: f32, y: f32) -> f32 {
        x * self.dir.x + y * self.dir.y
    }

    fn run(&mut self) {
        loop {
            let Some(&(idx, _)) = self.heap.first() else {
                self.path.clear();
                return;
            };
            let n = self.grid().get(idx);
            if n.state == GOAL {
                self.heap.clear();
                let mut i = idx;
                while i != 0 {
                    self.path.push(i);
                    i = self.grid().get(i).parent;
                }
                self.path.reverse();
                return;
            }
            self.grid_mut().set_state(idx, CLOSED);
            let last = self.heap.pop().unwrap_or((0, 0.0));
            if !self.heap.is_empty() {
                let len = self.heap.len();
                self.down(len, 0, last);
            }
            let (x, y) = cell_of(idx);
            if !((2..254).contains(&x) && y < 254) {
                continue;
            }
            let base = 256 * (y as u32 + 2) + x as u32;
            for nb in [base - 1, base + 1, base + 3] {
                self.relax(x, y, idx, n.g, nb);
            }
        }
    }
}

impl Solver<'_> {
    fn relax(&mut self, fx: i32, fy: i32, from: u32, g: f32, nb: u32) {
        let (nx, ny) = cell_of(nb);
        let (cost, goal) = self.edge(fx, fy, nx, ny);
        if cost >= CUTOFF {
            return;
        }
        let g2 = g + cost.max(TINY);
        let n = self.grid().get(nb);
        if n.state != NEW && !(n.g > g2) {
            return;
        }
        self.grid_mut().set(nb, Node { parent: from, g: g2, ..n });
        if goal {
            self.push(nb, g2);
            self.grid_mut().set_state(nb, GOAL);
        } else {
            let h = self.dot2(self.target.x, self.target.y) - self.dot2(nx as f32, ny as f32);
            self.push(nb, g2 + h);
        }
    }

    fn edge(&self, fx: i32, fy: i32, nx: i32, ny: i32) -> (f32, bool) {
        let (s, d) = (self.start, self.dir);
        let (dn, df) = (Vec2::new(nx as f32 - s.x, ny as f32 - s.y), Vec2::new(fx as f32 - s.x, fy as f32 - s.y));
        let lat = |a: Vec2| {
            let p = self.dot2(a.x, a.y);
            Vec2::new(a.x - d.x * p, a.y - d.y * p).length()
        };
        let goal = self.dot2(nx as f32, ny as f32) >= self.dot2(self.target.x, self.target.y) || ny >= 255;
        if self.outside(dn) {
            return (EXCLUDED, goal);
        }
        let r = f32::from(self.sa.ridge(nx as usize, ny as usize)) / 256.0;
        let step = Vec2::new((nx - fx) as f32, (ny - fy) as f32).length();
        let cost = LATERAL_WEIGHT * (lat(dn) - lat(df)).max(0.0) + step + RIDGE_WEIGHT * (1.0 - r);
        (cost, goal)
    }

    fn outside(&self, d: Vec2) -> bool {
        let dir = self.dir;
        let off = (d.y * dir.x / dir.y - d.x).abs() > 1.0;
        let inv = 1.0 / d.length();
        off && self.cos >= d.x * inv * dir.x + d.y * inv * dir.y
    }

    fn push(&mut self, idx: u32, key: f32) {
        let st = self.grid().get(idx).state;
        if st == OPEN || st == GOAL {
            if let Some(i) = self.heap.iter().position(|e| e.0 == idx) {
                self.heap[i] = (idx, key);
                let n = self.heap.len();
                let last = self.heap[n - 1];
                self.heap[n - 1] = self.heap[i];
                self.down(n - 1, i, last);
                let moved = self.heap[n - 1];
                self.up(n - 1, moved);
            }
        } else {
            self.heap.push((idx, key));
            let n = self.heap.len() - 1;
            self.up(n, (idx, key));
        }
        self.grid_mut().set_state(idx, OPEN);
    }

    fn up(&mut self, mut hole: usize, val: (u32, f32)) {
        while hole > 0 {
            let parent = (hole - 1) >> 1;
            if !(self.heap[parent].1 > val.1) {
                break;
            }
            self.heap[hole] = self.heap[parent];
            hole = parent;
        }
        self.heap[hole] = val;
    }

    fn down(&mut self, len: usize, mut hole: usize, val: (u32, f32)) {
        let mut child = 2 * hole + 2;
        while child < len {
            if self.heap[child].1 > self.heap[child - 1].1 {
                child -= 1;
            }
            self.heap[hole] = self.heap[child];
            hole = child;
            child = 2 * child + 2;
        }
        if child == len {
            self.heap[hole] = self.heap[child - 1];
            hole = child - 1;
        }
        self.up(hole, val);
    }
}

impl Solver<'_> {
    fn refine(&self, x: i32, y: i32) -> (f32, f32, f32) {
        let sa = self.sa;
        let (h, r) = (|x: i32| i32::from(sa.height(x as usize, y as usize)), |x: i32| sa.ridge(x as usize, y as usize));
        let (dx, dy) = (x as f32 - self.start.x, y as f32 - self.start.y);
        let l = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = if l > 0.0 { (dx / l, dy / l) } else { (0.0, 0.0) };
        let (c, s) = (self.cos, self.sin);
        let (pxp, pyp) = (c * ux - s * uy, s * ux + c * uy);
        let (pxm, pym) = (c * ux + s * uy, -s * ux + c * uy);
        let simple = (x as f32, y as f32, f32::from(r(x)) / 256.0);
        if !(dy > 0.0 && !(pxp > pxm)) {
            return simple;
        }
        let ep = pxp * (1.0 / pyp) * dy;
        let em = pxm * (1.0 / pym) * dy;
        let l0 = self.start.x + if pyp > 0.0 { ep } else { -256.0 };
        let r0 = self.start.x + if pym > 0.0 { em } else { 256.0 };
        let (lw, rw) = ((l0 + 0.5).floor() as i32, (r0 + 0.5).floor() as i32);
        let clamp = |v: i32| (x + 32).min(255).min((x - 32).max(0).max(v));
        let (lo, hi) = (clamp(lw), clamp(rw));
        let (h0, q0) = (h(x), i32::from(r(x)));
        let scan = |from: i32, to: i32, step: i32| {
            let mut cnt = 0;
            let mut i = from;
            while i != to {
                if h0 - h(i) > 48 {
                    return i - step;
                }
                if q0 - i32::from(r(i)) > 192 {
                    cnt += 1;
                    if cnt > 12 {
                        return i - step;
                    }
                } else {
                    cnt = 0;
                }
                i += step;
            }
            to
        };
        let left = if x > lo { scan(x, lo, -1) } else { x };
        let right = if x < hi { scan(x, hi, 1) } else { x };
        let (mut sum, mut sum_x, mut peak) = (0.0f32, 0.0f32, 0.0f32);
        for i in left..=right {
            let v = f32::from(r(i)) / 256.0;
            sum += v;
            sum_x += v * i as f32;
            peak = peak.max(v);
        }
        if sum.abs() <= EPS {
            return simple;
        }
        ((sum_x / sum + 0.5).floor(), y as f32, peak)
    }

    pub fn extract(&self) -> Option<PathPoints> {
        if self.path.len() < 3 {
            return None;
        }
        let sv = self.velg * self.step;
        let r2 = (sv.x * sv.x + sv.y * sv.y).floor() as i32;
        let (sx, sy) = (self.start.x.floor() as i32, self.start.y.floor() as i32);
        let cells: Vec<(i32, i32)> = self.path.iter().map(|&i| cell_of(i)).collect();
        let &(fx, fy) = cells.iter().find(|&&(x, y)| (x - sx).pow(2) + (y - sy).pow(2) >= r2)?;
        let &(xl, yl) = cells.last()?;
        let (a, b) = (self.refine(fx, fy), self.refine(xl, yl));
        let d1 = Vec2::new(a.0 - self.start.x, a.1 - self.start.y);
        if d1.x.abs() <= EPS && d1.y.abs() <= EPS {
            return None;
        }
        let spd = Vec2::new(self.velg.x, self.velg.y).length();
        let mut q = (0, 0);
        for &(x, y) in &cells {
            if (yl - y).pow(2) + (xl - x).pow(2) < r2 {
                break;
            }
            q = (x, y);
        }
        let e = Vec2::new(b.0 - q.0 as f32, b.1 - q.1 as f32);
        if e.x.abs() <= EPS && e.y.abs() <= EPS {
            return None;
        }
        let sa = self.sa;
        let lin = |v: Vec2| sa.world_dir(Vec3::new(v.x, v.y, 0.0));
        let pt = |p: (f32, f32, f32)| {
            let h = sa.height(p.0.floor() as usize, p.1.floor() as usize);
            sa.world(Vec3::new(p.0, p.1, f32::from(h)))
        };
        Some(PathPoints {
            dir_a: lin(d1 / d1.length() * spd),
            pos_a: pt(a),
            dir_b: lin(e / e.length() * spd),
            pos_b: pt(b),
            peak_a: a.2,
            peak_b: b.2,
        })
    }
}

pub fn carving_path(sa: &SurfaceAnalysis, joint: Vec3, global_velocity: Vec3, dt: f32) -> super::state::PathFrame {
    let v = global_velocity;
    let k = if v.abs().max_element() <= TINY { 2.0 } else { (1.0 / v.length()).max(2.0) };
    let s = Solver::solve(sa, joint, v, dt.max(1.0), k, std::f32::consts::FRAC_PI_2);
    match s.extract() {
        Some(p) => super::state::PathFrame {
            valid: true,
            dir_a: p.dir_a,
            pos_a: p.pos_a,
            dir_b: p.dir_b,
            pos_b: p.pos_b,
            peak_a: p.peak_a,
            peak_b: p.peak_b,
            ridge_a: sa.ridgiprocity_at(p.pos_a),
        },
        None => super::state::PathFrame {
            valid: false,
            dir_a: v,
            pos_a: joint,
            dir_b: v,
            pos_b: joint,
            ..Default::default()
        },
    }
}

pub fn max_ridge_on_segment(sa: &SurfaceAnalysis, a: Vec3, b: Vec3) -> (Vec3, f32) {
    let cell = |p: Vec3| {
        let g = sa.grid(p);
        ((g.x + 0.5).floor() as i32, (g.y + 0.5).floor() as i32)
    };
    let ((x0, y0), (x1, y1)) = (cell(a), cell(b));
    let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut err, mut x, mut y) = (2 * (dx - dy), x0, y0);
    let (mut best, mut bx, mut by) = (0u8, x0, y0);
    let inside = |v: i32| (0..GRID as i32).contains(&v);
    loop {
        if inside(x) && inside(y) {
            let v = sa.ridge(x as usize, y as usize);
            if v > best {
                (best, bx, by) = (v, x, y);
            }
        }
        if x == x1 && y == y1 {
            break;
        }
        let e = err;
        if err > -dy {
            err -= 2 * dy;
            x += sx;
        }
        if e < dx {
            err += 2 * dx;
            y += sy;
        }
    }
    let h = if inside(bx) && inside(by) { sa.height(bx as usize, by as usize) } else { 0 };
    (sa.world(Vec3::new(bx as f32, by as f32, f32::from(h))), f32::from(best) / 256.0)
}

pub fn nearest_ridge(sa: &SurfaceAnalysis, x0: i32, y0: i32, nx: i32, ny: i32, threshold: f32) -> Vec3 {
    let t = (threshold * 255.0 + 0.5).floor() as i32;
    let cl = |v: i32| v.clamp(0, 255);
    let (mut rx, mut ry) = (cl(x0), cl(y0));
    let span = cl(x0 - (nx - 1).max(0)) as usize..=cl(x0 + (nx - 1).max(0)) as usize;
    let above = |row: &[u8; super::surface_analysis::GRID]| row[span.clone()].iter().any(|&v| i32::from(v) > t);
    'rows: for dy in 0..ny {
        let (ya, yb) = (cl(y0 - dy), cl(y0 + dy));
        let (ra, rb) = (sa.ridge_row(ya as usize), sa.ridge_row(yb as usize));
        if nx <= 0 || !(above(ra) || above(rb)) {
            continue;
        }
        for dx in 0..nx {
            let (xl, xr) = (cl(x0 - dx), cl(x0 + dx));
            for (x, y, row) in [(xl, ya, ra), (xr, ya, ra), (xl, yb, rb), (xr, yb, rb)] {
                if i32::from(row[x as usize]) > t {
                    (rx, ry) = (x, y);
                    break 'rows;
                }
            }
        }
    }
    sa.cell_world(rx as usize, ry as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nearest_ridge_cells(sa: &SurfaceAnalysis, x0: i32, y0: i32, nx: i32, ny: i32, threshold: f32) -> Vec3 {
        let t = (threshold * 255.0 + 0.5).floor() as i32;
        let cl = |v: i32| v.clamp(0, 255);
        let r = |x: i32, y: i32| i32::from(sa.ridge(x as usize, y as usize));
        let (mut rx, mut ry) = (cl(x0), cl(y0));
        'rows: for dy in 0..ny {
            let (ya, yb) = (cl(y0 - dy), cl(y0 + dy));
            for dx in 0..nx {
                let (xl, xr) = (cl(x0 - dx), cl(x0 + dx));
                for (x, y) in [(xl, ya), (xr, ya), (xl, yb), (xr, yb)] {
                    if r(x, y) > t {
                        (rx, ry) = (x, y);
                        break 'rows;
                    }
                }
            }
        }
        sa.cell_world(rx as usize, ry as usize)
    }

    #[test]
    fn the_row_wise_ridge_search_is_the_cell_scan() {
        let mut seed = 0x9abc_def0_1234_5678u64;
        let mut next = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 33) % m
        };
        let rows = [
            Vec4::new(15.9375, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 31.875, 0.0),
            Vec4::new(0.0, 15.9375, 0.0, 0.0),
            Vec4::new(127.5, 127.5, 100.0, 1.0),
        ];
        let mut hits = 0;
        for case in 0..40 {
            let mut sa = SurfaceAnalysis::from_rows(rows);
            for y in 0..GRID {
                let ridge_at = (next(4) == 0).then(|| next(GRID as u64) as i32);
                for x in 0..GRID {
                    sa.heights[y * GRID + x] = match ridge_at {
                        Some(c) => (120 - 3 * (x as i32 - c).abs()).max(1) as u8,
                        None => 100,
                    };
                }
            }
            for _ in 0..20 {
                let (x0, y0) = (next(300) as i32 - 20, next(300) as i32 - 20);
                let (nx, ny) = (next(140) as i32, next(140) as i32);
                let th = [0.1, 0.25, 0.6][case % 3];
                let found = nearest_ridge(&sa, x0, y0, nx, ny, th);
                assert_eq!(
                    found,
                    nearest_ridge_cells(&sa, x0, y0, nx, ny, th),
                    "case {case}: ({x0}, {y0}) {nx} × {ny}"
                );
                hits += usize::from(found != sa.cell_world(x0.clamp(0, 255) as usize, y0.clamp(0, 255) as usize));
            }
        }
        assert!(hits > 100, "{hits} searches found a ridge");
    }
}
