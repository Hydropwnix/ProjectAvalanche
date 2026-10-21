#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    pub name: String,
    pub points: Vec<[f32; 2]>,
}

impl Curve {
    pub fn new(points: Vec<[f32; 2]>) -> Self {
        Self { name: String::new(), points }
    }

    pub fn eval(&self, x: f32) -> f32 {
        let p = &self.points;
        match p.len() {
            0 => 0.0,
            1 => p[0][1],
            n => {
                let x = x.clamp(p[0][0], p[n - 1][0].max(p[0][0]));
                let i = (0..n - 1).find(|&i| p[i][0] <= x && x < p[i + 1][0]).unwrap_or(n - 2);
                let ([x0, y0], [x1, y1]) = (p[i], p[i + 1]);
                if x1 == x0 { y1 } else { y0 + (y1 - y0) * (x - x0) / (x1 - x0) }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_with_clamping() {
        let c = Curve::new(vec![[0.0, 0.15], [0.463, 0.2], [0.926, 0.22]]);
        assert_eq!(c.eval(-1.0), 0.15);
        assert_eq!(c.eval(5.0), 0.22);
        assert!((c.eval(0.2315) - 0.175).abs() < 1e-6);
        assert_eq!(c.eval(0.926), 0.22);
        assert_eq!(Curve::default().eval(1.0), 0.0);
        assert_eq!(Curve::new(vec![[3.0, 7.0]]).eval(1.0), 7.0);
    }
}
