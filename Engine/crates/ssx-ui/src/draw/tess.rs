#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub paint: Vec<[f32; 2]>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn area(&self) -> f64 {
        self.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.vertices[i as usize]);
                let cross = (f64::from(b[0]) - f64::from(a[0])) * (f64::from(c[1]) - f64::from(a[1]))
                    - (f64::from(b[1]) - f64::from(a[1])) * (f64::from(c[0]) - f64::from(a[0]));
                cross.abs() / 2.0
            })
            .sum()
    }

    pub fn append(&mut self, o: &Mesh) {
        let base = self.vertices.len() as u32;
        if !self.paint.is_empty() || !o.paint.is_empty() {
            let own = if self.paint.is_empty() { self.vertices.clone() } else { std::mem::take(&mut self.paint) };
            self.paint = own;
            self.paint.extend_from_slice(if o.paint.is_empty() { &o.vertices } else { &o.paint });
        }
        self.vertices.extend_from_slice(&o.vertices);
        self.indices.extend(o.indices.iter().map(|i| i + base));
    }

    pub fn paint_at(&self, i: usize) -> [f32; 2] {
        self.paint.get(i).copied().unwrap_or(self.vertices[i])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    EvenOdd,
    NonZero,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub from: [f32; 2],
    pub to: [f32; 2],
}

pub fn quad_segments(p0: [f32; 2], c: [f32; 2], p1: [f32; 2], tolerance: f32) -> usize {
    let dd = (p0[0] - 2.0 * c[0] + p1[0]).hypot(p0[1] - 2.0 * c[1] + p1[1]);
    let n = (dd / (4.0 * tolerance.max(1e-6))).sqrt().ceil();
    if n.is_finite() { (n as usize).clamp(1, 256) } else { 1 }
}

pub fn flatten_quad(p0: [f32; 2], c: [f32; 2], p1: [f32; 2], tolerance: f32, out: &mut Vec<[f32; 2]>) {
    let n = quad_segments(p0, c, p1, tolerance);
    for i in 1..n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        out.push([
            mt * mt * p0[0] + 2.0 * mt * t * c[0] + t * t * p1[0],
            mt * mt * p0[1] + 2.0 * mt * t * c[1] + t * t * p1[1],
        ]);
    }
    out.push(p1);
}

#[derive(Clone, Copy, Debug)]
struct Span {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    winding: i32,
}

impl Span {
    fn x_at(&self, y: f64) -> f64 {
        if y <= self.y0 {
            return self.x0;
        }
        if y >= self.y1 {
            return self.x1;
        }
        self.x0 + (y - self.y0) * (self.x1 - self.x0) / (self.y1 - self.y0)
    }
}

struct MeshBuilder {
    mesh: Mesh,
    index: std::collections::HashMap<(u32, u32), u32>,
}

impl MeshBuilder {
    fn new() -> Self {
        MeshBuilder { mesh: Mesh::default(), index: std::collections::HashMap::new() }
    }

    fn vertex(&mut self, x: f64, y: f64) -> u32 {
        let p = [x as f32, y as f32];
        let key = (p[0].to_bits(), p[1].to_bits());
        if let Some(&i) = self.index.get(&key) {
            return i;
        }
        let i = self.mesh.vertices.len() as u32;
        self.mesh.vertices.push(p);
        self.index.insert(key, i);
        i
    }

    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        if a != b && b != c && a != c {
            self.mesh.indices.extend_from_slice(&[a, b, c]);
        }
    }

    fn trapezoid(&mut self, top: f64, xt0: f64, xt1: f64, bottom: f64, xb0: f64, xb1: f64) {
        let a = self.vertex(xt0, top);
        let b = self.vertex(xt1, top);
        let c = self.vertex(xb1, bottom);
        let d = self.vertex(xb0, bottom);
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }
}

pub fn fill(edges: &[Edge], rule: FillRule) -> Mesh {
    let mut spans: Vec<Span> = edges
        .iter()
        .filter_map(|e| {
            let [x0, y0, x1, y1] = [e.from[0], e.from[1], e.to[0], e.to[1]].map(f64::from);
            if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) || y0 == y1 {
                return None;
            }
            Some(if y0 < y1 {
                Span { x0, y0, x1, y1, winding: 1 }
            } else {
                Span { x0: x1, y0: y1, x1: x0, y1: y0, winding: -1 }
            })
        })
        .collect();
    if spans.is_empty() {
        return Mesh::default();
    }
    spans.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    let mut ys: Vec<f64> = spans.iter().flat_map(|s| [s.y0, s.y1]).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut out = MeshBuilder::new();
    let mut active: Vec<Span> = Vec::new();
    let mut next = 0;
    let mut cuts: Vec<f64> = Vec::new();
    let mut xs: Vec<(f64, f64, f64, i32)> = Vec::new();
    for w in ys.windows(2) {
        let (top, bottom) = (w[0], w[1]);
        active.retain(|s| s.y1 > top);
        while next < spans.len() && spans[next].y0 <= top {
            active.push(spans[next]);
            next += 1;
        }
        if active.len() < 2 {
            continue;
        }
        cuts.clear();
        cuts.push(top);
        cuts.push(bottom);
        let mut ends: Vec<(f64, f64)> = active.iter().map(|s| (s.x_at(top), s.x_at(bottom))).collect();
        ends.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        if ends.windows(2).any(|p| p[0].1 > p[1].1) {
            for i in 0..ends.len() {
                for j in i + 1..ends.len() {
                    let ((ti, bi), (tj, bj)) = (ends[i], ends[j]);
                    let denom = (bi - ti) - (bj - tj);
                    if (ti - tj) * (bi - bj) < 0.0 && denom != 0.0 {
                        let u = (tj - ti) / denom;
                        let y = top + u * (bottom - top);
                        if y > top && y < bottom {
                            cuts.push(y);
                        }
                    }
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup();
        }
        for c in cuts.windows(2) {
            let (t, b) = (c[0], c[1]);
            if b - t <= 1e-9 * (1.0 + t.abs()) {
                continue;
            }
            let mid = (t + b) / 2.0;
            xs.clear();
            xs.extend(active.iter().map(|s| (s.x_at(mid), s.x_at(t), s.x_at(b), s.winding)));
            xs.sort_by(|a, b| a.0.total_cmp(&b.0).then((a.1 + a.2).total_cmp(&(b.1 + b.2))));
            let mut wind = 0;
            for k in 0..xs.len() - 1 {
                wind += xs[k].3;
                let inside = match rule {
                    FillRule::EvenOdd => (k + 1) % 2 == 1,
                    FillRule::NonZero => wind != 0,
                };
                if inside {
                    out.trapezoid(t, xs[k].1, xs[k + 1].1, b, xs[k].2, xs[k + 1].2);
                }
            }
        }
    }
    out.mesh
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cap {
    Round,
    None,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    Round,
    Bevel,
    Miter(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub start_cap: Cap,
    pub end_cap: Cap,
    pub join: Join,
}

fn polygon(edges: &mut Vec<Edge>, pts: &[[f32; 2]]) {
    if pts.len() < 3 {
        return;
    }
    let area: f32 = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let n = pts.len();
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        edges.push(if area >= 0.0 { Edge { from: a, to: b } } else { Edge { from: b, to: a } });
    }
}

fn disc(edges: &mut Vec<Edge>, c: [f32; 2], r: f32, tolerance: f32) {
    let ratio = (1.0 - tolerance / r.max(1e-6)).clamp(-1.0, 1.0);
    let n = (std::f32::consts::PI / ratio.acos().max(1e-3)).ceil().clamp(8.0, 128.0) as usize;
    let pts: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect();
    polygon(edges, &pts);
}

pub fn stroke(polylines: &[Vec<[f32; 2]>], style: &StrokeStyle, tolerance: f32) -> Mesh {
    let hw = style.width / 2.0;
    if hw <= 0.0 || !hw.is_finite() {
        return Mesh::default();
    }
    let mut edges = Vec::new();
    for pl in polylines {
        let mut pts: Vec<[f32; 2]> = Vec::with_capacity(pl.len());
        for &p in pl {
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
        if pts.len() == 1 {
            if style.start_cap == Cap::Round {
                disc(&mut edges, pts[0], hw, tolerance);
            }
            continue;
        }
        let closed = pts.len() > 2 && pts.first() == pts.last();
        let dirs: Vec<[f32; 2]> = pts
            .windows(2)
            .map(|w| {
                let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                let l = dx.hypot(dy);
                [dx / l, dy / l]
            })
            .collect();
        for (w, d) in pts.windows(2).zip(&dirs) {
            let n = [-d[1] * hw, d[0] * hw];
            let (a, b) = (w[0], w[1]);
            polygon(
                &mut edges,
                &[
                    [a[0] + n[0], a[1] + n[1]],
                    [b[0] + n[0], b[1] + n[1]],
                    [b[0] - n[0], b[1] - n[1]],
                    [a[0] - n[0], a[1] - n[1]],
                ],
            );
        }
        let joints: Vec<(usize, usize)> =
            (1..dirs.len()).map(|i| (i - 1, i)).chain(closed.then_some((dirs.len() - 1, 0))).collect();
        for (i0, i1) in joints {
            let p = pts[i1];
            let (d0, d1) = (dirs[i0], dirs[i1]);
            let cross = d0[0] * d1[1] - d0[1] * d1[0];
            let s = if cross > 0.0 { -1.0 } else { 1.0 };
            let n0 = [-d0[1] * hw * s, d0[0] * hw * s];
            let n1 = [-d1[1] * hw * s, d1[0] * hw * s];
            match style.join {
                Join::Round => disc(&mut edges, p, hw, tolerance),
                Join::Bevel => polygon(&mut edges, &[p, [p[0] + n0[0], p[1] + n0[1]], [p[0] + n1[0], p[1] + n1[1]]]),
                Join::Miter(limit) => {
                    let half = ((1.0 + d0[0] * d1[0] + d0[1] * d1[1]) / 2.0).max(1e-6).sqrt();
                    let m = [(n0[0] + n1[0]) / 2.0, (n0[1] + n1[1]) / 2.0];
                    let ml = m[0].hypot(m[1]).max(1e-6);
                    let len = hw / half;
                    let tip = [p[0] + m[0] / ml * len, p[1] + m[1] / ml * len];
                    let pa = [p[0] + n0[0], p[1] + n0[1]];
                    let pb = [p[0] + n1[0], p[1] + n1[1]];
                    if 1.0 / half <= limit.max(1.0) {
                        polygon(&mut edges, &[p, pa, tip, pb]);
                    } else {
                        polygon(&mut edges, &[p, pa, pb]);
                    }
                }
            }
        }
        if !closed {
            for (p, d, cap) in [
                (pts[0], [-dirs[0][0], -dirs[0][1]], style.start_cap),
                (pts[pts.len() - 1], dirs[dirs.len() - 1], style.end_cap),
            ] {
                match cap {
                    Cap::Round => disc(&mut edges, p, hw, tolerance),
                    Cap::Square => {
                        let n = [-d[1] * hw, d[0] * hw];
                        let e = [d[0] * hw, d[1] * hw];
                        polygon(
                            &mut edges,
                            &[
                                [p[0] + n[0], p[1] + n[1]],
                                [p[0] + n[0] + e[0], p[1] + n[1] + e[1]],
                                [p[0] - n[0] + e[0], p[1] - n[1] + e[1]],
                                [p[0] - n[0], p[1] - n[1]],
                            ],
                        );
                    }
                    Cap::None => {}
                }
            }
        }
    }
    fill(&edges, FillRule::NonZero)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(pts: &[[f32; 2]]) -> Vec<Edge> {
        (0..pts.len()).map(|i| Edge { from: pts[i], to: pts[(i + 1) % pts.len()] }).collect()
    }

    #[test]
    fn square_with_hole_even_odd() {
        let mut e = poly(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]]);
        e.extend(poly(&[[25.0, 25.0], [75.0, 25.0], [75.0, 75.0], [25.0, 75.0]]));
        let m = fill(&e, FillRule::EvenOdd);
        assert!((m.area() - (10_000.0 - 2_500.0)).abs() < 1e-3, "{}", m.area());
        assert!((fill(&e, FillRule::NonZero).area() - 10_000.0).abs() < 1e-3);
    }

    #[test]
    fn shared_interior_edges_cancel() {
        let mut e = poly(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        e.extend(poly(&[[10.0, 0.0], [20.0, 0.0], [20.0, 10.0], [10.0, 10.0]]));
        assert!((fill(&e, FillRule::EvenOdd).area() - 200.0).abs() < 1e-3);
    }

    #[test]
    fn bow_tie_crossing_is_split() {
        let e = poly(&[[0.0, 0.0], [10.0, 10.0], [10.0, 0.0], [0.0, 10.0]]);
        let m = fill(&e, FillRule::EvenOdd);
        assert!((m.area() - 50.0).abs() < 1e-3, "{}", m.area());
    }

    #[test]
    fn triangle_and_degenerate_input() {
        let m = fill(&poly(&[[0.0, 0.0], [40.0, 0.0], [0.0, 30.0]]), FillRule::EvenOdd);
        assert!((m.area() - 600.0).abs() < 1e-3);
        assert!(fill(&poly(&[[0.0, 0.0], [10.0, 0.0]]), FillRule::EvenOdd).is_empty());
        assert!(fill(&[], FillRule::NonZero).is_empty());
    }

    #[test]
    fn curve_flattening_meets_tolerance() {
        let (p0, c, p1) = ([0.0, 0.0], [50.0, 100.0], [100.0, 0.0]);
        let mut pts = Vec::new();
        flatten_quad(p0, c, p1, 0.5, &mut pts);
        assert_eq!(pts.last(), Some(&p1));
        let top = pts.iter().map(|p| p[1]).fold(0.0f32, f32::max);
        assert!(50.0 - top <= 0.5 + 1e-4, "{top}");
        assert_eq!(quad_segments(p0, [50.0, 0.0], p1, 0.5), 1);
    }

    #[test]
    fn stroke_union_has_no_overlap() {
        let style = StrokeStyle { width: 10.0, start_cap: Cap::None, end_cap: Cap::None, join: Join::Bevel };
        let m = stroke(&[vec![[0.0, 0.0], [100.0, 0.0]]], &style, 0.25);
        assert!((m.area() - 1000.0).abs() < 1e-2, "{}", m.area());
        let m = stroke(&[vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]]], &style, 0.25);
        let expected = 100.0 * 10.0 + 100.0 * 10.0 - 25.0 + 12.5;
        assert!((m.area() - expected).abs() < 1e-2, "{} vs {expected}", m.area());
        let round = StrokeStyle { start_cap: Cap::Round, end_cap: Cap::Round, join: Join::Round, ..style };
        let dot = stroke(&[vec![[5.0, 5.0], [5.0, 5.0]]], &round, 0.01);
        assert!((dot.area() - std::f64::consts::PI * 25.0).abs() < 0.5, "{}", dot.area());
    }
}
