use ssx_formats::gfx::types::{ColorTransform, Matrix, Rect, Rgba};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat2d {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Default for Mat2d {
    fn default() -> Self {
        Mat2d::IDENTITY
    }
}

impl Mat2d {
    pub const IDENTITY: Mat2d = Mat2d { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub fn from_swf(m: &Matrix) -> Mat2d {
        Mat2d { a: m.a, b: m.b, c: m.c, d: m.d, tx: m.tx as f32, ty: m.ty as f32 }
    }

    pub fn scale(sx: f32, sy: f32) -> Mat2d {
        Mat2d { a: sx, d: sy, ..Mat2d::IDENTITY }
    }

    pub fn translate(tx: f32, ty: f32) -> Mat2d {
        Mat2d { tx, ty, ..Mat2d::IDENTITY }
    }

    pub fn mul(&self, o: &Mat2d) -> Mat2d {
        Mat2d {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    pub fn inverse(&self) -> Option<Mat2d> {
        let det = self.determinant();
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Mat2d { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) })
    }

    pub fn x_scale(&self) -> f32 {
        self.a.hypot(self.b)
    }

    pub fn y_scale(&self) -> f32 {
        self.c.hypot(self.d)
    }

    pub fn max_scale(&self) -> f32 {
        self.x_scale().max(self.y_scale())
    }

    pub fn rotation(&self) -> f32 {
        self.b.atan2(self.a)
    }

    pub fn as_array(&self) -> [f32; 6] {
        [self.a, self.b, self.c, self.d, self.tx, self.ty]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cxform {
    pub mult: [f32; 4],
    pub add: [f32; 4],
}

impl Default for Cxform {
    fn default() -> Self {
        Cxform::IDENTITY
    }
}

impl Cxform {
    pub const IDENTITY: Cxform = Cxform { mult: [1.0; 4], add: [0.0; 4] };

    pub fn from_swf(c: &ColorTransform) -> Cxform {
        Cxform { mult: c.mult.map(|m| f32::from(m) / 256.0), add: c.add.map(|a| f32::from(a) / 255.0) }
    }

    pub fn concat(&self, o: &Cxform) -> Cxform {
        Cxform {
            mult: std::array::from_fn(|i| self.mult[i] * o.mult[i]),
            add: std::array::from_fn(|i| self.mult[i] * o.add[i] + self.add[i]),
        }
    }

    pub fn apply(&self, c: [f32; 4]) -> [f32; 4] {
        std::array::from_fn(|i| (c[i] * self.mult[i] + self.add[i]).clamp(0.0, 1.0))
    }

    pub fn is_identity(&self) -> bool {
        *self == Cxform::IDENTITY
    }
}

pub fn rgba(c: Rgba) -> [f32; 4] {
    [c.r, c.g, c.b, c.a].map(|v| f32::from(v) / 255.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Default for RectF {
    fn default() -> Self {
        RectF::EMPTY
    }
}

impl RectF {
    pub const EMPTY: RectF = RectF { x0: f32::MAX, y0: f32::MAX, x1: f32::MIN, y1: f32::MIN };

    pub fn from_swf(r: &Rect) -> RectF {
        RectF { x0: r.x_min as f32, y0: r.y_min as f32, x1: r.x_max as f32, y1: r.y_max as f32 }
    }

    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> RectF {
        RectF { x0, y0, x1, y1 }
    }

    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1 || self.y0 > self.y1
    }

    pub fn width(&self) -> f32 {
        if self.is_empty() { 0.0 } else { self.x1 - self.x0 }
    }

    pub fn height(&self) -> f32 {
        if self.is_empty() { 0.0 } else { self.y1 - self.y0 }
    }

    pub fn add_point(&mut self, x: f32, y: f32) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
    }

    pub fn union(&self, o: &RectF) -> RectF {
        if o.is_empty() {
            return *self;
        }
        RectF { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    pub fn transform(&self, m: &Mat2d) -> RectF {
        if self.is_empty() {
            return RectF::EMPTY;
        }
        let mut r = RectF::EMPTY;
        for (x, y) in [(self.x0, self.y0), (self.x1, self.y0), (self.x0, self.y1), (self.x1, self.y1)] {
            let (px, py) = m.apply(x, y);
            r.add_point(px, py);
        }
        r
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_inverse_and_concat() {
        let m = Mat2d { a: 2.0, b: 0.5, c: -0.25, d: 1.5, tx: 10.0, ty: -4.0 };
        let i = m.inverse().unwrap();
        let p = m.mul(&i).apply(3.0, 7.0);
        assert!((p.0 - 3.0).abs() < 1e-4 && (p.1 - 7.0).abs() < 1e-4);
        let t = Mat2d::translate(5.0, 0.0).mul(&Mat2d::scale(2.0, 2.0));
        assert_eq!(t.apply(1.0, 1.0), (7.0, 2.0));
    }

    #[test]
    fn cxform_concat_matches_nesting() {
        let parent = Cxform { mult: [0.5, 1.0, 1.0, 0.5], add: [0.1, 0.0, 0.0, 0.0] };
        let child = Cxform { mult: [1.0, 1.5, 1.0, 0.8], add: [0.2, 0.0, 0.0, 0.1] };
        let c = [0.4, 0.3, 0.2, 1.0];
        let nested = parent.apply(child.apply(c));
        let once = parent.concat(&child).apply(c);
        for k in 0..4 {
            assert!((nested[k] - once[k]).abs() < 1e-6, "{k}: {nested:?} {once:?}");
        }
    }
}
