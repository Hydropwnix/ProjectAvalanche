use super::*;

#[derive(Clone, Debug)]
pub struct Curve {
    pub interp: u8,
    pub wrap: u8,
    pub uniform: bool,
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
    pub n_segments: u16,
    pub stride: u8,
    pub outputs: Vec<(Ref, Ref)>,
    pub xs: Vec<f32>,
    pub points: Vec<f32>,
}

impl Curve {
    pub(super) fn parse(body: &[u8], pos: usize) -> Result<Self> {
        let start = pos.next_multiple_of(16) - pos;
        if body.len() < start + 0x20 || body[..start].iter().any(|&b| b != 0) {
            return invalid("Curve padding / size");
        }
        let b = &body[start..];
        let mut r = Reader::be(b);
        let (z0, interp, wrap, uniform) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let (min_x, max_x, min_y, max_y) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
        let (o_xs, o_pts, n_segments, n_out, stride) =
            (r.u32()? as usize, r.u32()? as usize, r.u16()?, r.u8()? as usize, r.u8()?);
        let np = n_segments as usize + 1;
        let after_outputs = 0x20 + 16 * n_out;
        let xs_ok = if uniform != 0 {
            o_xs == 0 && o_pts == after_outputs
        } else {
            o_xs == after_outputs && o_pts == o_xs + 4 * np
        };
        let end = o_pts + 4 * np * stride as usize;
        if z0 != 0 || uniform > 1 || interp > 3 || wrap > 2 || n_segments == 0 || stride == 0 || !xs_ok || end > b.len()
        {
            return invalid(format!(
                "Curve header {interp} {wrap} {uniform} {o_xs:#x} {o_pts:#x} {n_segments} {stride}"
            ));
        }
        let mut outputs = Vec::with_capacity(n_out);
        for _ in 0..n_out {
            if f64::from_bits(r.u64()?) != f64::INFINITY {
                return invalid("Curve input cache is not +inf");
            }
            outputs.push((Ref(r.u32()?), Ref(r.u32()?)));
        }
        let floats = |off: usize, n: usize| -> Result<Vec<f32>> {
            let mut r = at(b, off)?;
            (0..n).map(|_| r.f32()).collect()
        };
        let xs = if uniform != 0 { Vec::new() } else { floats(o_xs, np)? };
        let points = floats(o_pts, np * stride as usize)?;
        if b[end..].iter().any(|&x| x != 0) {
            return invalid("Curve tail not zero");
        }
        Ok(Self {
            interp,
            wrap,
            uniform: uniform == 1,
            min_x,
            max_x,
            min_y,
            max_y,
            n_segments,
            stride,
            outputs,
            xs,
            points,
        })
    }
}

impl Curve {
    pub fn eval(&self, x: f64) -> f64 {
        let x = self.wrap(f64::from(x as f32));
        let seg = self.segment(x);
        let (x0, y0, x1, y1) = self.end_points(seg);
        let (x0, x1, y0, y1) = (f64::from(x0), f64::from(x1), f64::from(y0), f64::from(y1));
        let span = f64::from((x1 - x0) as f32);
        let mut fallback = f64::from((f64::from(self.max_y - self.min_y) * 0.5) as f32);
        if x0 == x1 {
            fallback = if seg == 0 { y0 } else { y1 };
        }
        let mut y = y0;
        if x0 < x {
            y = if x < x1 { fallback } else { y1 };
            if x < x1 {
                let t = (f64::from((x - x0) as f32) / span) as f32;
                match self.interp {
                    1 => y = f64::from((f64::from(t) * f64::from((y1 - y0) as f32) + y0) as f32),
                    2 => {
                        let (_, a, _, b) = self.controls(seg);
                        let (y0f, y1f, a3, b3) = (y0 as f32, y1 as f32, (f64::from(a) * 3.0) as f32, b * 3.0);
                        let c1 = a3 - y0f * 3.0;
                        let c3 = (a3 - y0f) - b3 + y1f;
                        let c2 = -(a * 6.0 - y0f * 3.0) + b3;
                        y = f64::from((f64::from(c1 * t + c3 * t * t * t + c2 * t * t) + y0) as f32);
                    }
                    0 if ((x1 - x) as f32) <= ((x - x0) as f32) => y = y1,
                    0 => y = y0,
                    3 => y = self.bezier(seg, x, fallback),
                    _ => {}
                }
            }
        }
        y.max(f64::from(self.min_y)).min(f64::from(self.max_y))
    }

    pub(super) fn wrap(&self, x: f64) -> f64 {
        let (lo, hi) = (f64::from(self.min_x), f64::from(self.max_x));
        let range = f64::from((hi - lo) as f32);
        let mut r = if range <= 0.0 { lo } else { x };
        match self.wrap {
            0 => {
                if x < lo {
                    r = lo;
                }
                if hi < x {
                    r = hi;
                }
            }
            1 => {
                let m = f64::from((x - lo) as f32) % range;
                r = f64::from((f64::from(m as f32) + lo) as f32);
                if r < lo {
                    r = f64::from((r + range) as f32);
                }
            }
            _ => {
                let m = f64::from((f64::from((x - lo) as f32) + range) as f32) % f64::from((range * 2.0) as f32);
                r = f64::from(((f64::from(m as f32).abs() - range).abs() as f32 as f64 + lo) as f32);
            }
        }
        r
    }

    fn segment(&self, x: f64) -> usize {
        let n = usize::from(self.n_segments);
        if self.uniform {
            let step = (self.max_x - self.min_x) / n as f32;
            let s = (((x - f64::from(self.min_x)) as f32) / step) as u32 as usize;
            return if s >= n { s - 1 } else { s };
        }
        let xs = &self.xs;
        let at = |i: usize| f64::from(xs.get(i).copied().unwrap_or(self.max_x));
        let (mut lo, mut hi) = (0usize, n);
        let mut mid;
        loop {
            loop {
                mid = lo + (hi - lo) / 2;
                if x >= at(mid) || mid == lo {
                    break;
                }
                hi = mid;
            }
            lo = mid + 1;
            if at(mid + 1) >= x || lo >= n {
                break;
            }
        }
        while mid > 0 && at(mid - 1) == x {
            mid -= 1;
        }
        if at(mid) == x || mid + 1 >= n {
            return mid.min(n - 1);
        }
        if at(mid + 2) == x { mid + 1 } else { mid }
    }

    fn end_points(&self, seg: usize) -> (f32, f32, f32, f32) {
        let n = usize::from(self.n_segments);
        let (x0, x1) = if self.uniform {
            let step = (self.max_x - self.min_x) / n as f32;
            let x0 = (seg as f32).mul_add(step, self.min_x);
            (x0, if seg + 1 == n { self.max_x } else { x0 + step })
        } else {
            (self.xs[seg], self.xs[seg + 1])
        };
        let s = usize::from(self.stride);
        (x0, self.points[seg * s], x1, self.points[(seg + 1) * s])
    }

    fn point(&self, i: isize) -> (f32, f32) {
        let n = isize::from(self.n_segments as i16);
        let s = usize::from(self.stride);
        let step = (self.max_x - self.min_x) / n as f32;
        let xs = |k: usize| self.xs.get(k).copied().unwrap_or(0.0);
        if (0..=n).contains(&i) {
            let x = if !self.uniform {
                xs(i as usize)
            } else if i < n {
                (i as f32).mul_add(step, self.min_x)
            } else {
                self.max_x
            };
            return (x, self.points[i as usize * s]);
        }
        let nn = n as usize;
        let last = self.points[(nn - 1) * s];
        if i == -1 {
            let mut x = self.min_x - step;
            match self.wrap {
                0 | 2 => {
                    if !self.uniform {
                        x = self.min_x - (xs(1) - self.min_x);
                    }
                    (x, self.points[0])
                }
                _ => {
                    if !self.uniform {
                        x = self.min_x - (xs(nn) - xs(nn - 1));
                    }
                    (x, last)
                }
            }
        } else {
            let mut x = self.max_x + step;
            match self.wrap {
                0 | 2 => {
                    if !self.uniform {
                        x = (xs(nn) - xs(nn - 1)) + self.max_x;
                    }
                    (x, last)
                }
                _ => {
                    if !self.uniform {
                        x = (xs(1) - self.min_x) + self.max_x;
                    }
                    (x, self.points[1])
                }
            }
        }
    }

    pub(super) fn controls(&self, seg: usize) -> (f32, f32, f32, f32) {
        let s = usize::from(self.stride);
        let p = &self.points;
        if s == 5 {
            return (p[seg * 5 + 3], p[seg * 5 + 4], p[(seg + 1) * 5 + 1], p[(seg + 1) * 5 + 2]);
        }
        let i = seg as isize;
        let ((xm, ym), (x0, y0), (x1, y1), (x2, y2)) =
            (self.point(i - 1), self.point(i), self.point(i + 1), self.point(i + 2));
        let dx = x1 - x0;
        let dx2 = x2 - x1;
        let ax = dx.mul_add(0.333_333_34, x0);
        let bx = dx.mul_add(0.666_666_7, x0);
        match s {
            1 => {
                let m = (1.0 / dx) * (y1 - y0);
                let ay = (m + (y0 - ym) / (x0 - xm)) * 0.5 * (ax - x0) + y0;
                let by = -((m + (y2 - y1) / dx2) * 0.5 * (x1 - bx) - y1);
                (ax, ay, bx, by)
            }
            3 => (ax, p[seg * 3 + 2], bx, p[(seg + 1) * 3 + 1]),
            _ => {
                let ay = p[seg * 2 + 1];
                let (yb, t) = (p[(seg + 1) * 2], p[(seg + 1) * 2 + 1]);
                let d = yb - t;
                let by = if self.uniform {
                    d + yb
                } else if dx2 > 0.0 {
                    (dx / dx2) * d + yb
                } else if d < 0.0 {
                    f32::MIN_POSITIVE
                } else if d > 0.0 {
                    f32::MAX
                } else {
                    yb
                };
                (ax, ay, bx, by)
            }
        }
    }

    fn bezier(&self, seg: usize, x: f64, fallback: f64) -> f64 {
        let (x0, y0, x1, y1) = self.end_points(seg);
        let (ax, ay, bx, by) = self.controls(seg);
        let (x0, y0, x1, y1) = (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1));
        let (mut lo, mut hi, mut t) = (0.0f64, 1.0f64, 0.5f64);
        loop {
            let u = 1.0 - t;
            let (t2, u2) = (t * t, u * u);
            let xt = x0 * u2 * u + (f64::from(bx) * t2 * u + f64::from(ax) * u2 * t) * 3.0 + x1 * t2 * t;
            let mut next_hi = t;
            if xt <= x + 0.0005 {
                next_hi = hi;
                lo = t;
                if x - 0.0005 <= xt {
                    return f64::from(
                        (y0 * u2 * u + (f64::from(by) * t2 * u + f64::from(ay) * u2 * t) * 3.0 + y1 * t2 * t) as f32,
                    );
                }
            }
            t = (next_hi - lo) * 0.5 + lo;
            hi = next_hi;
            if (xt - x).abs() <= 0.0005 {
                return fallback;
            }
        }
    }
}
