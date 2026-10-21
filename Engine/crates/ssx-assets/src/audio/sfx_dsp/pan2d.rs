pub fn matrix_panner_attr(outputs: usize, channel: usize) -> Option<usize> {
    let row: &[usize] = match outputs {
        2 => &[0, 2],
        4 => &[0, 2, 3, 4],
        6 => &[0, 1, 2, 3, 4, 5],
        8 => &[0, 1, 2, 3, 4, 6, 7, 5],
        _ => return None,
    };
    row.get(channel).copied()
}

#[derive(Clone, Copy, Debug)]
pub struct PanParams {
    pub angle: f64,
    pub distance: f64,
    pub size: f64,
    pub twist: f64,
    pub center: f64,
    pub main: f64,
    pub source_front: f64,
    pub source_rear: f64,
}

impl Default for PanParams {
    fn default() -> Self {
        Self {
            angle: 0.0,
            distance: 1.0,
            size: 1.0,
            twist: 0.0,
            center: 1.0,
            main: 1.0,
            source_front: 30.0,
            source_rear: 110.0,
        }
    }
}

fn pan_point(x: f32, y: f32) -> (f32, f32, f32, f32) {
    let mut p = (x, y, x * x + y * y);
    if p.2 > 1.0 {
        let s = 1.0 / p.2.sqrt();
        p = (x * s, y * s, 1.0);
    } else if p.2 > 0.999 {
        p.2 = 1.0;
    }
    (p.0, p.1, p.2, p.1.atan2(p.0))
}

fn pair(u: (f32, f32), v: (f32, f32), c: f32, s: f32) -> (f32, f32) {
    let inv = 1.0 / u.0.mul_add(v.1, -(u.1 * v.0));
    (inv * (v.1 * c - v.0 * s), inv * (u.0 * s - u.1 * c))
}

pub fn pan2d_gains(
    p: &PanParams,
    inputs: usize,
    outputs: usize,
    norm: i32,
    front_deg: f32,
    rear_deg: f32,
) -> Vec<[f32; 8]> {
    let rad = 0.017_453_294_f64;
    let (front, rear) = (front_deg * 0.017_453_29, rear_deg * 0.017_453_29);
    let at = |a: f32| (a.cos(), a.sin());
    let (fl, fr, rl, rr) = (at(front), at(-front), at(rear), at(-rear));
    let (i_fl, i_fr, i_rl, i_rr) = if outputs > 4 { (0, 2, 3, 4) } else { (0, 1, 2, 3) };
    let theta = (p.angle * -rad) as f32;
    let twist = (p.twist * -rad) as f32;
    let (bx, by) = ((theta.cos() as f64 * p.distance) as f32, (theta.sin() as f64 * p.distance) as f32);
    let size = p.size as f32;
    let around = |a: f32| pan_point(a.cos().mul_add(size, bx), a.sin().mul_add(size, by));
    let (sf, sr) = ((p.source_front * rad) as f32, (p.source_rear * rad) as f32);
    let base = theta + twist;
    let points: Vec<(f32, f32, f32, f32)> = match inputs {
        1 => {
            let (x, y, r2, _) = pan_point(bx, by);
            let a = if p.distance <= 0.0 { theta + std::f32::consts::PI } else { theta };
            vec![(x, y, r2, a)]
        }
        2 => vec![around(base + sf), around(base - sf)],
        4 => vec![around(base + sf), around(base - sf), around(base + sr), around(base - sr)],
        6 => vec![around(base + sf), around(base), around(base - sf), around(base + sr), around(base - sr)],
        _ => Vec::new(),
    };
    let normf = match norm {
        1 => 1.0 / inputs.max(1) as f32,
        2 => 1.0 / (inputs.max(1) as f32).sqrt(),
        _ => 1.0,
    };
    let center = p.center as f32;
    let mut out = vec![[0.0f32; 8]; inputs];
    for (g, &(x, y, r2, a)) in out.iter_mut().zip(&points) {
        if r2 < 1.0 {
            let w = |u: (f32, f32)| -(((u.0 - x).powi(2) + (u.1 - y).powi(2)).sqrt().mul_add(0.5, -1.0));
            let (wl, wr) = (w(fl), w(fr));
            let mut ff = (x + 1.0) * 0.5;
            if ff.abs() < 0.0005 {
                ff = 0.0;
            }
            let mut fb = 1.0 - ff;
            if fb.abs() < 0.0005 {
                fb = 0.0;
            }
            let s = (-(r2 * r2 - 1.0)).sqrt();
            if outputs == 2 || outputs == 4 {
                let f = (ff / (wr * wr + wl * wl)).sqrt();
                g[i_fl] = s * f * wl;
                g[i_fr] = s * f * wr;
            } else {
                let wc = -((1.0 - x).hypot(y).mul_add(0.5, -1.0)) * center;
                let f = (ff / (wl * wl + wr * wr + wc * wc)).sqrt();
                g[i_fl] = s * f * wl;
                g[i_fr] = s * f * wr;
                g[1] = s * f * wc;
            }
            if outputs > 2 {
                let (wbl, wbr) = (w(rl), w(rr));
                let f = (fb / (wbl * wbl + wbr * wbr)).sqrt();
                g[i_rl] = s * wbl * f;
                g[i_rr] = s * wbr * f;
            }
        }
        let two_pi = std::f32::consts::TAU;
        let t = (a + front) * 0.159_154_94;
        let phi = (t - t.floor()) * 6.283_185_5 - front;
        if outputs == 2 {
            let (l, r) = if front > 1.308_997 {
                let l = (y + 1.0) * 0.5;
                (l, 1.0 - l)
            } else {
                let folded = if phi <= front {
                    phi
                } else if a > std::f32::consts::PI {
                    (1.0 - ((two_pi - phi) - front) / (std::f32::consts::PI - front)) * front
                } else {
                    (1.0 - (phi - front) / (std::f32::consts::PI - front)) * front
                };
                let (l, r) = pair(fl, fr, folded.cos(), folded.sin());
                let f = r2 / (l * l + r * r).sqrt();
                (l * f, r * f)
            };
            g[i_fl] += l;
            g[i_fr] += r;
        } else {
            let (c, s) = (phi.cos(), phi.sin());
            if phi < front {
                let (mut l, mut r) = pair(fl, fr, c, s);
                let mut cg = 0.0;
                if outputs > 5 {
                    let m = l.min(r) * center;
                    l -= m;
                    r -= m;
                    cg = front.cos() * 2.0 * m;
                }
                let f = r2 / (l * l + cg * cg + r * r).sqrt();
                g[i_fl] += l * f;
                g[i_fr] += r * f;
                if outputs > 5 {
                    g[1] += cg * f;
                }
            } else {
                let (u, v, iu, iv) = if phi < rear {
                    (fl, rl, i_fl, i_rl)
                } else if phi < two_pi - rear {
                    (rl, rr, i_rl, i_rr)
                } else {
                    (rr, fr, i_rr, i_fr)
                };
                let (gu, gv) = pair(u, v, c, s);
                let f = r2 / (gu * gu + gv * gv).sqrt();
                g[iu] += gu * f;
                g[iv] += gv * f;
            }
        }
        let level = (p.main as f32) * normf;
        let power = g.iter().map(|v| v * v).sum::<f32>();
        let k = if r2 < 1.0 && power > 0.0 { level / power.sqrt() } else { level };
        g.iter_mut().for_each(|v| *v *= k);
    }
    out
}
