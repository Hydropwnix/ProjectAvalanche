pub fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h =
        (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

pub fn value(x: f32, y: f32, seed: u32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (tx, ty) = (smooth(x - xi), smooth(y - yi));
    let (xi, yi) = (xi as i32, yi as i32);
    let top = hash(xi, yi, seed) * (1.0 - tx) + hash(xi + 1, yi, seed) * tx;
    let bottom = hash(xi, yi + 1, seed) * (1.0 - tx) + hash(xi + 1, yi + 1, seed) * tx;
    top * (1.0 - ty) + bottom * ty
}

pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 0.5, 1.0, 0.0);
    for o in 0..octaves {
        sum += amp * value(x * freq, y * freq, seed.wrapping_add(o * 131));
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

pub fn zigzag(t: f32, seed: u32) -> f32 {
    let i = t.floor();
    let f = t - i;
    hash(i as i32, 0, seed) * (1.0 - f) + hash(i as i32 + 1, 0, seed) * f - 0.5
}

pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    smooth(((x - a) / (b - a)).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for i in 0..200 {
            let (x, y) = (i as f32 * 0.37 - 20.0, i as f32 * 0.11);
            let (v, z) = (fbm(x, y, 4, 7), zigzag(x, 7));
            assert!((0.0..=1.0).contains(&v) && (-0.5..=0.5).contains(&z), "{v} {z}");
            assert_eq!(v, fbm(x, y, 4, 7));
        }
        assert_ne!(value(1.5, 2.5, 1), value(1.5, 2.5, 2));
    }
}
