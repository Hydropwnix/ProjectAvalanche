use half::f16;

use super::{ColorEncoding, FloatImage};
use crate::texture::Level;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CubeLevel {
    pub size: u32,
    pub faces: [Vec<u16>; 6],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkyCube {
    pub levels: Vec<CubeLevel>,
}

impl SkyCube {
    pub fn size(&self) -> u32 {
        self.levels.first().map_or(0, |l| l.size)
    }

    pub fn bytes(&self) -> u64 {
        self.levels.iter().map(|l| l.faces.iter().map(|f| f.len() as u64 * 2).sum::<u64>()).sum()
    }

    pub fn texel(&self, face: usize, x: u32, y: u32) -> [f32; 4] {
        let l = &self.levels[0];
        let at = ((y * l.size + x) * 4) as usize;
        let f = &l.faces[face];
        [0, 1, 2, 3].map(|k| f16::from_bits(f[at + k]).to_f32())
    }
}

pub fn cube_bytes(face: u32) -> u64 {
    if face == 0 {
        return 0;
    }
    crate::texture::chain_bytes(face, face) * 6 * 2
}

pub fn face_direction(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
    let s = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
    let t = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
    let d = match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    };
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    d.map(|c| c / len)
}

pub fn equirect_texel(d: [f32; 3], width: u32, height: u32) -> (f32, f32) {
    let u = 0.5 + d[0].atan2(d[2]) / std::f32::consts::TAU;
    let v = d[1].clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    (u * width as f32 - 0.5, v * height as f32 - 0.5)
}

fn bilinear(img: &FloatImage, x: f32, y: f32) -> [f32; 3] {
    let (w, h) = (img.width as i64, img.height as i64);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: i64, yi: i64| {
        let xi = xi.rem_euclid(w);
        let yi = yi.clamp(0, h - 1);
        img.rgb[(yi * w + xi) as usize]
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    [0, 1, 2].map(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (d[k] - c[k]) * fx;
        top + (bottom - top) * fy
    })
}

fn pack(rgba: [f32; 4], out: &mut Vec<u16>) {
    out.extend(rgba.map(|v| f16::from_f32(v.min(f32::from(f16::MAX))).to_bits()));
}

pub fn equirect_to_cube(img: &FloatImage, face: u32) -> SkyCube {
    let n = face as usize;
    let faces: [Vec<u16>; 6] = std::array::from_fn(|f| {
        let mut out = Vec::with_capacity(n * n * 4);
        for y in 0..face {
            for x in 0..face {
                let (u, v) = equirect_texel(face_direction(f, x, y, face), img.width, img.height);
                let [r, g, b] = bilinear(img, u, v);
                pack([r, g, b, 1.0], &mut out);
            }
        }
        out
    });
    with_mips(CubeLevel { size: face, faces })
}

fn decode8(v: u8, color: ColorEncoding) -> f32 {
    let c = f32::from(v) / 255.0;
    match color {
        ColorEncoding::Linear => c,
        ColorEncoding::Srgb if c <= 0.04045 => c / 12.92,
        ColorEncoding::Srgb => ((c + 0.055) / 1.055).powf(2.4),
    }
}

fn level_from_rgba8(size: u32, images: &[Vec<u8>], color: ColorEncoding) -> Result<CubeLevel, String> {
    if images.len() != 6 {
        return Err(format!("{} faces, not 6", images.len()));
    }
    let want = size as usize * size as usize * 4;
    let mut faces: [Vec<u16>; 6] = Default::default();
    for (f, img) in images.iter().enumerate() {
        if img.len() != want {
            return Err(format!("face {f}: {} bytes for a {size}² face", img.len()));
        }
        let out = &mut faces[f];
        out.reserve(want);
        for t in img.as_chunks::<4>().0 {
            pack([decode8(t[0], color), decode8(t[1], color), decode8(t[2], color), f32::from(t[3]) / 255.0], out);
        }
    }
    Ok(CubeLevel { size, faces })
}

pub fn from_rgba8_levels(levels: &[Level], color: ColorEncoding) -> Result<SkyCube, String> {
    let l = levels.first().ok_or("no level")?;
    if l.width != l.height {
        return Err(format!("{}×{} faces (a cube's are square)", l.width, l.height));
    }
    Ok(with_mips(level_from_rgba8(l.width, &l.images, color)?))
}

pub fn from_faces(faces: Vec<Level>, color: ColorEncoding) -> Result<SkyCube, String> {
    let size = faces.first().map_or(0, |l| l.width);
    let mut images = Vec::with_capacity(6);
    for (k, f) in faces.into_iter().enumerate() {
        if f.width != size || f.height != size {
            return Err(format!("face {k} is {}×{}; the faces are squares of one size ({size})", f.width, f.height));
        }
        images.extend(f.images.into_iter().take(1));
    }
    Ok(with_mips(level_from_rgba8(size, &images, color)?))
}

fn with_mips(level0: CubeLevel) -> SkyCube {
    let mut levels = vec![level0];
    while let Some(last) = levels.last().filter(|l| l.size > 1) {
        let (src, half) = (last.size as usize, (last.size / 2) as usize);
        let faces: [Vec<u16>; 6] = std::array::from_fn(|f| {
            let s = &last.faces[f];
            let mut out = Vec::with_capacity(half * half * 4);
            for y in 0..half {
                for x in 0..half {
                    let mut acc = [0f32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let at = ((2 * y + dy) * src + 2 * x + dx) * 4;
                        for k in 0..4 {
                            acc[k] += f16::from_bits(s[at + k]).to_f32();
                        }
                    }
                    pack(acc.map(|v| v * 0.25), &mut out);
                }
            }
            out
        });
        levels.push(CubeLevel { size: half as u32, faces });
    }
    SkyCube { levels }
}
