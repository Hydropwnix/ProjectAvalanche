use crate::geom::{Mat2d, RectF};
use crate::tess::Mesh;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice9 {
    pub grid: RectF,
    pub bounds: RectF,
    pub sx: f32,
    pub sy: f32,
}

fn axis(v: f32, b0: f32, b1: f32, g0: f32, g1: f32, s: f32) -> f32 {
    let g0 = g0.clamp(b0, b1);
    let g1 = g1.clamp(g0, b1);
    let (left, right) = (g0 - b0, b1 - g1);
    let total = (b1 - b0) * s;
    let k = if left + right > total && left + right > 0.0 { total / (left + right) } else { 1.0 };
    let (o0, o1) = (b0 * s, b1 * s);
    if v <= g0 {
        o0 + (v - b0) * k
    } else if v >= g1 {
        o1 - (b1 - v) * k
    } else {
        let (m0, m1) = (o0 + left * k, o1 - right * k);
        m0 + (v - g0) / (g1 - g0).max(1e-6) * (m1 - m0)
    }
}

impl Slice9 {
    pub fn map(&self, x: f32, y: f32) -> (f32, f32) {
        let (g, b) = (&self.grid, &self.bounds);
        (axis(x, b.x0, b.x1, g.x0, g.x1, self.sx), axis(y, b.y0, b.y1, g.y0, g.y1, self.sy))
    }

    pub fn split(m: &Mat2d) -> (f32, f32, Mat2d) {
        let (sx, sy) = (m.x_scale().max(1e-6), m.y_scale().max(1e-6));
        (sx, sy, m.mul(&Mat2d::scale(1.0 / sx, 1.0 / sy)))
    }

    pub fn slice(&self, mesh: &Mesh, xf: &Mat2d) -> Mesh {
        let mut out = Mesh::default();
        let mut paint = Vec::new();
        for t in mesh.indices.as_chunks::<3>().0 {
            let poly: Vec<([f32; 2], [f32; 2])> = t
                .iter()
                .map(|&i| {
                    let v = mesh.vertices[i as usize];
                    let (x, y) = xf.apply(v[0], v[1]);
                    ([x, y], mesh.paint_at(i as usize))
                })
                .collect();
            let mut pieces = vec![poly];
            for (axis, at) in [(0, self.grid.x0), (0, self.grid.x1), (1, self.grid.y0), (1, self.grid.y1)] {
                pieces = pieces.into_iter().flat_map(|p| split(&p, axis, at)).filter(|p| p.len() >= 3).collect();
            }
            for p in pieces {
                let base = out.vertices.len() as u32;
                for (pos, uv) in &p {
                    let (x, y) = self.map(pos[0], pos[1]);
                    out.vertices.push([x, y]);
                    paint.push(*uv);
                }
                for k in 1..p.len() as u32 - 1 {
                    out.indices.extend_from_slice(&[base, base + k, base + k + 1]);
                }
            }
        }
        out.paint = paint;
        out
    }
}

fn split(poly: &[([f32; 2], [f32; 2])], axis: usize, at: f32) -> Vec<Vec<([f32; 2], [f32; 2])>> {
    let (mut lo, mut hi) = (Vec::new(), Vec::new());
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (a.0[axis] - at, b.0[axis] - at);
        if da <= 0.0 {
            lo.push(a);
        }
        if da >= 0.0 {
            hi.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            let t = da / (da - db);
            let lerp = |p: [f32; 2], q: [f32; 2]| [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
            let c = (lerp(a.0, b.0), lerp(a.1, b.1));
            lo.push(c);
            hi.push(c);
        }
    }
    vec![lo, hi]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_keep_their_size() {
        let s = Slice9 {
            grid: RectF::new(10.0, 10.0, 90.0, 90.0),
            bounds: RectF::new(0.0, 0.0, 100.0, 100.0),
            sx: 3.0,
            sy: 1.0,
        };
        assert_eq!(s.map(0.0, 0.0), (0.0, 0.0));
        assert_eq!(s.map(10.0, 50.0), (10.0, 50.0));
        assert_eq!(s.map(90.0, 50.0), (290.0, 50.0));
        assert_eq!(s.map(100.0, 100.0), (300.0, 100.0));
        assert_eq!(s.map(50.0, 0.0).0, 150.0);
        let t = Slice9 { sx: 0.1, ..s };
        assert!((t.map(10.0, 0.0).0 - 5.0).abs() < 1e-4 && (t.map(100.0, 0.0).0 - 10.0).abs() < 1e-4);
        let u = Slice9 { grid: RectF::new(-200.0, 10.0, -150.0, 90.0), sx: 1.0, ..s };
        assert_eq!(u.map(37.0, 0.0).0, 37.0);
    }

    #[test]
    fn slicing_keeps_area_and_paint() {
        let quad = Mesh {
            vertices: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            paint: Vec::new(),
        };
        let s = Slice9 {
            grid: RectF::new(10.0, 10.0, 90.0, 90.0),
            bounds: RectF::new(0.0, 0.0, 100.0, 100.0),
            sx: 2.0,
            sy: 2.0,
        };
        let m = s.slice(&quad, &Mat2d::IDENTITY);
        assert!((m.area() - 200.0 * 200.0).abs() < 1e-2, "{}", m.area());
        let i = m.vertices.iter().position(|v| *v == [200.0, 200.0]).unwrap();
        assert_eq!(m.paint[i], [100.0, 100.0]);
    }
}
