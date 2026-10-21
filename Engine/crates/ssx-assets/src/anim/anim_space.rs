use crate::anim_eval::{Controller, Env, GameState, Window, instantiate};
use crate::anim_graph::{Ctrl, CtrlId};
use crate::anim_pose::AnimPose;

#[allow(clippy::too_many_arguments)]
pub fn rate_limit(
    cur: f32,
    target: f32,
    dt: f32,
    inc: f32,
    dec: f32,
    acc: f32,
    lo: f32,
    hi: f32,
    vel: &mut f32,
) -> f32 {
    const EPS: f32 = 1.525_878_9e-5;
    let inv = if dt.abs() >= EPS { 1.0 / dt } else { 1000.0 };
    let (mut next, sign) =
        if target - cur < 0.0 { ((cur - dt * dec).max(target), -1.0) } else { ((cur + dt * inc).min(target), 1.0) };
    let mut v = (next - cur) * inv;
    let a = (v - *vel) * inv;
    if acc < a.abs() && (next - cur).abs() > EPS {
        let s = if a < 0.0 { -1.0 } else { 1.0 };
        v = s * dt * acc + *vel;
        next = v * dt + cur;
        if sign * target < sign * next {
            next = target;
        }
    }
    *vel = v;
    next.max(lo).min(hi)
}

fn barycentric(p: [f32; 2], t: [[f32; 2]; 3]) -> [f32; 3] {
    let [a, b, c] = t;
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-12 {
        return [1.0, 0.0, 0.0];
    }
    let w0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let w1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    [w0, w1, 1.0 - w0 - w1]
}

fn closest_on_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = dx * dx + dy * dy;
    let t = if l > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l).clamp(0.0, 1.0) } else { 0.0 };
    [a[0] + t * dx, a[1] + t * dy]
}

pub struct Diagram<'a> {
    pub items: &'a [[f32; 2]],
    pub triangles: &'a [[usize; 3]],
    pub inverse_area: &'a [f32],
    pub hull: &'a [([f32; 2], [f32; 2], usize)],
    pub snap: &'a [bool],
    pub snap_tolerance: f32,
}

fn cross(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[1] - a[1] * b[0]
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn len(a: [f32; 2]) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

pub fn item_weights(p: [f32; 2], d: &Diagram) -> Vec<f32> {
    let mut w = vec![0.0; d.items.len()];
    let pos = |i: usize| d.items.get(i).copied().unwrap_or([0.0, 0.0]);
    let tri = |t: &[usize; 3]| t.map(pos);
    let inside = |t: &[usize; 3]| barycentric(p, tri(t)).iter().all(|&b| b >= -1e-5);
    let pick = match d.triangles.iter().position(inside) {
        Some(k) => Some((k, p)),
        None => {
            let mut best: Option<(f32, usize, [f32; 2])> = None;
            for &(a, b, t) in d.hull {
                let q = closest_on_segment(p, a, b);
                let dd = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
                if t < d.triangles.len() && best.is_none_or(|(bd, ..)| dd < bd) {
                    best = Some((dd, t, q));
                }
            }
            best.map(|(_, t, q)| (t, q))
        }
    };
    let Some((k, q)) = pick else {
        if let Some(x) = w.first_mut() {
            *x = 1.0;
        }
        return w;
    };
    let t = d.triangles[k];
    let [a, b, c] = tri(&t);
    let inv = d.inverse_area.get(k).copied().unwrap_or(0.0);
    let mut tw = if inv == 0.0 {
        let l = len(sub(a, b));
        let wa = if l > 0.0 { (len(sub(q, b)) / l).clamp(0.0, 1.0) } else { 1.0 };
        [wa, 1.0 - wa, 0.0]
    } else {
        let wa = (0.5 * cross(sub(b, q), sub(q, c)) * inv).clamp(0.0, 1.0);
        let mut wc = (0.5 * cross(sub(a, q), sub(q, b)) * inv).clamp(0.0, 1.0);
        let mut wa = wa;
        let mut wb = 1.0 - wa - wc;
        if wb < 0.0 {
            if wa >= -wb {
                wa += wb;
            } else {
                wc += wb;
            }
            wb = 0.0;
        }
        [wa, wb, wc]
    };
    let count = if inv == 0.0 { 2 } else { 3 };
    let snap = |i: usize| d.snap.get(i).copied().unwrap_or(false);
    if t[..count].iter().all(|&i| snap(i)) {
        let best = (0..count).fold(0, |m, i| if tw[i] > tw[m] { i } else { m });
        tw = [0.0; 3];
        tw[best] = 1.0;
    }
    for x in tw.iter_mut().take(count) {
        if *x < d.snap_tolerance {
            *x = 0.0;
        }
    }
    let sum: f32 = tw[..count].iter().sum();
    for (&i, &x) in t.iter().zip(&tw).take(count) {
        if let Some(o) = w.get_mut(i)
            && sum > 0.0
        {
            *o += x / sum;
        }
    }
    w
}

pub struct BlendSpaceInst {
    ctrl: CtrlId,
    point: [f32; 2],
    rate: [f32; 2],
    items: Vec<Option<Box<dyn Controller>>>,
}

impl BlendSpaceInst {
    pub fn new(env: &Env, gs: &GameState, ctrl: CtrlId) -> Self {
        let Ctrl::BlendSpace { x, y, items, .. } = &env.g.ctrls[ctrl].1 else { unreachable!("not a blend space") };
        let read = |s: &Option<crate::anim_graph::SigId>| s.map_or(0.0, |s| gs.get(s));
        let items = items.iter().map(|(_, c)| c.map(|c| instantiate(env, gs, c))).collect();
        BlendSpaceInst { ctrl, point: [read(x), read(y)], rate: [0.0; 2], items }
    }

    fn weights(&self, env: &Env) -> Vec<f32> {
        let Ctrl::BlendSpace { items, triangles, inverse_area, hull, snap, snap_tolerance, .. } =
            &env.g.ctrls[self.ctrl].1
        else {
            return Vec::new();
        };
        let pos: Vec<[f32; 2]> = items.iter().map(|i| i.0).collect();
        let d = Diagram { items: &pos, triangles, inverse_area, hull, snap, snap_tolerance: *snap_tolerance };
        item_weights(self.point, &d)
    }
}

impl Controller for BlendSpaceInst {
    fn update(&mut self, env: &Env, gs: &mut GameState, dt: f32) {
        if let Ctrl::BlendSpace { x, y, axes, .. } = &env.g.ctrls[self.ctrl].1 {
            for (k, s) in [x, y].into_iter().enumerate() {
                let target = s.map_or(0.0, |s| gs.get(s));
                let [dec, inc, acc, lo, hi] = axes[k];
                self.point[k] = rate_limit(self.point[k], target, dt, inc, dec, acc, lo, hi, &mut self.rate[k]);
            }
        }
        for i in self.items.iter_mut().flatten() {
            i.update(env, gs, dt);
        }
    }

    fn pose(&self, env: &Env, gs: &GameState, out: &mut AnimPose) {
        let w = self.weights(env);
        let mut acc: Option<(AnimPose, f32)> = None;
        for (item, &wi) in self.items.iter().zip(&w) {
            let Some(item) = item.as_ref().filter(|_| wi > 1e-4) else { continue };
            let mut p = AnimPose::empty(env.bind);
            item.pose(env, gs, &mut p);
            acc = Some(match acc {
                None => (p, wi),
                Some((mut a, sum)) => {
                    a.blend(&p, wi / (sum + wi));
                    (a, sum + wi)
                }
            });
        }
        if let Some((a, _)) = acc {
            *out = a;
        }
    }

    fn time(&self) -> f32 {
        self.items.iter().flatten().next().map_or(0.0, |i| i.time())
    }

    fn length(&self) -> f32 {
        self.items.iter().flatten().next().map_or(0.0, |i| i.length())
    }

    fn windows(&self, _env: &Env) -> Vec<Window> {
        Vec::new()
    }

    fn label(&self, env: &Env, weight: f32, out: &mut Vec<(String, f32)>) {
        let w = self.weights(env);
        for (item, &wi) in self.items.iter().zip(&w) {
            if let Some(i) = item.as_ref().filter(|_| wi > 1e-4) {
                i.label(env, weight * wi, out);
            }
        }
    }

    fn delta(&self, env: &Env, gs: &GameState) -> crate::anim_pose::Delta {
        use crate::anim_pose::Delta;
        let w = self.weights(env);
        let mut acc: Option<(Delta, f32)> = None;
        for (item, &wi) in self.items.iter().zip(&w) {
            let Some(item) = item.as_ref().filter(|_| wi > 1e-4) else { continue };
            let d = item.delta(env, gs);
            acc = Some(match acc {
                None => (d, wi),
                Some((a, sum)) => (Delta::blend(a, d, wi / (sum + wi), env.physics), sum + wi),
            });
        }
        acc.map_or(Delta::None, |a| a.0)
    }

    fn exit(&mut self, env: &Env, gs: &GameState) {
        for i in self.items.iter_mut().flatten() {
            i.exit(env, gs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_moves_at_the_rates() {
        let mut v = 0.0;
        let x = rate_limit(0.0, 1.0, 2.0, 0.1, 0.2, 100.0, -1.0, 1.0, &mut v);
        assert!((x - 0.2).abs() < 1e-6 && (v - 0.1).abs() < 1e-6);
        let y = rate_limit(x, -1.0, 2.0, 0.1, 0.2, 100.0, -1.0, 1.0, &mut v);
        assert!((y + 0.2).abs() < 1e-6);
        let mut v = 0.0;
        assert_eq!(rate_limit(0.0, 0.05, 2.0, 0.1, 0.2, 100.0, -1.0, 1.0, &mut v), 0.05);
    }

    fn diagram<'a>(items: &'a [[f32; 2]], inv: &'a [f32], snap: &'a [bool], tol: f32) -> Diagram<'a> {
        const TRIS: [[usize; 3]; 1] = [[0, 1, 2]];
        const HULL: [([f32; 2], [f32; 2], usize); 1] = [([0.0, 1.0], [1.0, 0.0], 0)];
        Diagram { items, triangles: &TRIS, inverse_area: inv, hull: &HULL, snap, snap_tolerance: tol }
    }

    #[test]
    fn weights_inside_and_outside() {
        let items = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]];
        let d = diagram(&items, &[2.0], &[false; 3], 0.0);
        let w = item_weights([0.25, 0.25], &d);
        assert!((w[0] - 0.5).abs() < 1e-5 && (w[1] - 0.25).abs() < 1e-5 && (w[2] - 0.25).abs() < 1e-5);
        let w = item_weights([1.0, 1.0], &d);
        assert!((w[1] - 0.5).abs() < 1e-5 && (w[2] - 0.5).abs() < 1e-5 && w[0].abs() < 1e-5);
    }

    #[test]
    fn small_weights_drop_below_the_tolerance() {
        let items = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]];
        let d = diagram(&items, &[2.0], &[false; 3], 0.03);
        let w = item_weights([0.02, 0.5], &d);
        assert!(w[2] == 0.0 && (w[0] + w[1] - 1.0).abs() < 1e-5 && (w[0] - 0.48 / 0.98).abs() < 1e-4);
        let d = diagram(&items, &[2.0], &[true; 3], 0.03);
        assert_eq!(item_weights([0.1, 0.6], &d), vec![0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_degenerate_triangle_interpolates_its_first_edge() {
        let items = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        let d = Diagram {
            items: &items,
            triangles: &[[0, 1, 2]],
            inverse_area: &[0.0],
            hull: &[],
            snap: &[false; 3],
            snap_tolerance: 0.0,
        };
        let w = item_weights([0.25, 0.0], &d);
        assert!((w[0] - 0.75).abs() < 1e-5 && (w[1] - 0.25).abs() < 1e-5 && w[2] == 0.0);
    }
}
