const C1: i32 = 64277;
const C2: i32 = 60547;
const C3: i32 = 54491;
const C4: i32 = 46341;
const C5: i32 = 36410;
const C6: i32 = 25080;
const C7: i32 = 12785;

#[inline(always)]
fn mul(c: i32, v: i32) -> i32 {
    ((c as u32).wrapping_mul(v as u32) as i32) >> 16
}

#[inline(always)]
fn idct_1d(i: [i32; 8], bias: i32) -> [i32; 8] {
    let a = mul(C1, i[1]) + mul(C7, i[7]);
    let b = mul(C7, i[1]) - mul(C1, i[7]);
    let c = mul(C3, i[3]) + mul(C5, i[5]);
    let d = mul(C3, i[5]) - mul(C5, i[3]);
    let ad = mul(C4, a - c);
    let bd = mul(C4, b - d);
    let cd = a + c;
    let dd = b + d;
    let e = mul(C4, i[0] + i[4]) + bias;
    let f = mul(C4, i[0] - i[4]) + bias;
    let g = mul(C2, i[2]) + mul(C6, i[6]);
    let h = mul(C6, i[2]) - mul(C2, i[6]);
    let ed = e - g;
    let gd = e + g;
    let add = f + ad;
    let bdd = bd - h;
    let fd = f - ad;
    let hd = bd + h;
    [gd + cd, add + hd, add - hd, ed + dd, ed - dd, fd + bdd, fd - bdd, gd - cd]
}

#[inline(always)]
fn clip(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

pub fn idct(coef: &mut [i16; 64], dst: &mut [u8], stride: usize, add: bool) {
    for r in 0..8 {
        let row = &mut coef[r * 8..r * 8 + 8];
        if row.iter().all(|&v| v == 0) {
            continue;
        }
        let o = idct_1d(std::array::from_fn(|k| i32::from(row[k])), 0);
        for (dst, v) in row.iter_mut().zip(o) {
            *dst = v as i16;
        }
    }
    let level = if add { 0 } else { 16 * 128 };
    for x in 0..8 {
        let col: [i32; 8] = std::array::from_fn(|r| i32::from(coef[r * 8 + x]));
        let out: [i32; 8] = if col[1..].iter().all(|&v| v == 0) {
            if add && col[0] == 0 {
                continue;
            }
            let v = (C4 * col[0] + (8 << 16)) >> 20;
            [v + level / 16; 8]
        } else {
            idct_1d(col, 8 + level).map(|v| v >> 4)
        };
        for (r, v) in out.into_iter().enumerate() {
            let p = &mut dst[r * stride + x];
            *p = if add { clip(i32::from(*p) + v) } else { clip(v) };
        }
    }
    *coef = [0; 64];
}

pub fn bilinear(dst: &mut [u8], dp: usize, src: &[u8], sp: usize, fx: i32, fy: i32) {
    let (wa, wb, wc, wd) = ((8 - fx) * (8 - fy), fx * (8 - fy), (8 - fx) * fy, fx * fy);
    for y in 0..8 {
        let s0 = &src[y * sp..];
        let s1 = &src[(y + 1) * sp..];
        for x in 0..8 {
            let v =
                wa * i32::from(s0[x]) + wb * i32::from(s0[x + 1]) + wc * i32::from(s1[x]) + wd * i32::from(s1[x + 1]);
            dst[y * dp + x] = ((v + 32) >> 6) as u8;
        }
    }
}

pub fn bilinear_2pass(dst: &mut [u8], dp: usize, src: &[u8], sp: usize, fx: i32, fy: i32) {
    let mut tmp = [0u8; 9 * 8];
    for y in 0..9 {
        let s = &src[y * sp..];
        for x in 0..8 {
            let v = (8 - fx) * 8 * i32::from(s[x]) + fx * 8 * i32::from(s[x + 1]);
            tmp[y * 8 + x] = ((v + 32) >> 6) as u8;
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            let v = (8 - fy) * 8 * i32::from(tmp[y * 8 + x]) + fy * 8 * i32::from(tmp[(y + 1) * 8 + x]);
            dst[y * dp + x] = ((v + 32) >> 6) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(coef: &[i16; 64]) -> [f64; 64] {
        let mut out = [0f64; 64];
        for y in 0..8 {
            for x in 0..8 {
                let mut s = 0.0;
                for v in 0..8 {
                    for u in 0..8 {
                        let cu = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                        let cv = if v == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
                        let basis = ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.0).cos()
                            * ((2 * y + 1) as f64 * v as f64 * std::f64::consts::PI / 16.0).cos();
                        s += cu * cv * f64::from(coef[v * 8 + u]) * basis;
                    }
                }
                out[y * 8 + x] = s / 8.0;
            }
        }
        out
    }

    #[test]
    fn dc_only_put_and_add() {
        let mut coef = [0i16; 64];
        coef[0] = 160;
        let mut dst = [0u8; 64];
        idct(&mut coef, &mut dst, 8, false);
        assert!(dst.iter().all(|&p| p == 133), "{dst:?}");
        assert!(coef.iter().all(|&c| c == 0));
        coef[0] = -160;
        idct(&mut coef, &mut dst, 8, true);
        assert!(dst.iter().all(|&p| p == 128));
    }

    #[test]
    fn matches_float_reference() {
        let mut seed = 99u32;
        for _ in 0..200 {
            let mut coef = [0i16; 64];
            for c in coef.iter_mut().take(20) {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *c = ((seed >> 16) % 160) as i16 - 80;
            }
            let reference = reference(&coef);
            let mut dst = [0u8; 64];
            idct(&mut coef, &mut dst, 8, false);
            for (p, r) in dst.iter().zip(reference) {
                let want = (r / 2.0 + 128.0).clamp(0.0, 255.0);
                assert!((f64::from(*p) - want).abs() <= 1.5, "{p} vs {want}");
            }
        }
    }

    #[test]
    fn bilinear_offsets() {
        let src: Vec<u8> = (0..12 * 12).map(|i| ((i % 12) * 10 + (i / 12)) as u8).collect();
        let mut dst = [0u8; 64];
        bilinear(&mut dst, 8, &src, 12, 0, 0);
        assert_eq!(dst[9], src[13]);
        bilinear(&mut dst, 8, &src, 12, 4, 0);
        assert_eq!(dst[0], 5);
        bilinear_2pass(&mut dst, 8, &src, 12, 4, 4);
        assert_eq!(dst[0], 6);
    }
}
