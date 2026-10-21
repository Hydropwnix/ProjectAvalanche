// TODO(guess): exact maths instead of the unreproducible VMX estimates (the vlogefp / vexptefp / vrefp tables are unknown): LUT entries of Levels / ColourBalance layers may differ by one step — the Levels power 0x834865F4 and the ColourBalance power 0x83485018 are unrefined vexptefp(e·vlogefp(x)), its 1/L1 an unrefined vrefp 0x8348508C;
// an error model from Xenia's recorded console results moves 3–9 of the 32 grid values per channel of such a layer by one step (up to 2): `dusk_orange_colorcube`, `dusk_purple_colorcube`, `default_colorcube` (an identity Levels); settle: the estimate tables (more console results than Xenia's recorded points).

use anyhow::{Context, Result, bail};

use super::xml::XmlLayer;

pub const N: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColourCube {
    pub texels: Vec<[u8; 3]>,
}

const INV_255: f32 = 1.0 / 255.0;
const LUMA: [f32; 3] = [0.299, 0.587, 0.114];

pub fn quantise(v: f32) -> u8 {
    if v.is_nan() { 0 } else { (v.clamp(0.0, 1.0) * 255.5) as u8 }
}

impl ColourCube {
    pub fn identity() -> Self {
        let byte = |i: usize| (i * 255 / (N - 1)) as u8;
        let mut texels = Vec::with_capacity(N * N * N);
        for b in 0..N {
            for g in 0..N {
                for r in 0..N {
                    texels.push([byte(r), byte(g), byte(b)]);
                }
            }
        }
        ColourCube { texels }
    }

    pub fn rgba8(&self) -> Vec<u8> {
        self.texels.iter().flat_map(|t| [t[0], t[1], t[2], 255]).collect()
    }

    pub fn max_deviation(&self) -> u8 {
        let id = ColourCube::identity();
        self.texels
            .iter()
            .zip(&id.texels)
            .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c])))
            .max()
            .unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
}

impl BlendMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "Normal" => BlendMode::Normal,
            "Multiply" => BlendMode::Multiply,
            "Screen" => BlendMode::Screen,
            "Overlay" => BlendMode::Overlay,
            _ => return None,
        })
    }

    fn apply(self, base: f32, layer: f32) -> f32 {
        match self {
            BlendMode::Normal => layer,
            BlendMode::Multiply => base * layer,
            BlendMode::Screen => base + layer - base * layer,
            BlendMode::Overlay if base > 0.5 => {
                let s = 2.0 * base - 1.0;
                s + layer - s * layer
            }
            BlendMode::Overlay => 2.0 * base * layer,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayerKind {
    LoadColourCube,
    BrightnessContrast { brightness: i32, contrast: i32 },
    Levels { input_mid: [f32; 3], input_min: [u8; 3], input_max: [u8; 3], output_min: [u8; 3], output_max: [u8; 3] },
    ColourBalance { shadows: [i32; 3], midtones: [i32; 3], highlights: [i32; 3], preserve_luminosity: bool },
    HueSaturation { hue: i32, saturation: i32, lightness: i32 },
}

fn param<'a>(l: &'a XmlLayer, name: &str) -> Result<&'a str> {
    l.parameters
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .with_context(|| format!("layer {:?} ({}) has no parameter {name}", l.name, l.type_name))
}

fn int(l: &XmlLayer, name: &str) -> Result<i32> {
    let v = param(l, name)?;
    v.trim().parse().with_context(|| format!("{name} = {v:?}"))
}

fn triple<T: std::str::FromStr>(l: &XmlLayer, name: &str) -> Result<[T; 3]> {
    let v = param(l, name)?;
    let parts: Vec<T> = v
        .split(',')
        .map(|p| p.trim().parse::<T>())
        .collect::<Result<_, _>>()
        .ok()
        .with_context(|| format!("{name} = {v:?}: expected three numbers"))?;
    <[T; 3]>::try_from(parts).ok().with_context(|| format!("{name} = {v:?}: expected three numbers"))
}

impl LayerKind {
    pub fn parse(l: &XmlLayer) -> Result<Self> {
        Ok(match l.type_name.as_str() {
            "LoadColourCube" => LayerKind::LoadColourCube,
            "BrightnessContrast" => {
                LayerKind::BrightnessContrast { brightness: int(l, "brightness")?, contrast: int(l, "contrast")? }
            }
            "Levels" => LayerKind::Levels {
                input_mid: triple(l, "inputMid")?,
                input_min: triple(l, "inputMin")?,
                input_max: triple(l, "inputMax")?,
                output_min: triple(l, "outputMin")?,
                output_max: triple(l, "outputMax")?,
            },
            "ColourBalance" => LayerKind::ColourBalance {
                shadows: triple(l, "shadows")?,
                midtones: triple(l, "midtones")?,
                highlights: triple(l, "highlights")?,
                preserve_luminosity: match param(l, "preserveLuminosity")? {
                    "true" => true,
                    "false" => false,
                    v => bail!("preserveLuminosity = {v:?}"),
                },
            },
            "HueSaturation" => LayerKind::HueSaturation {
                hue: int(l, "hueRotation")?,
                saturation: int(l, "saturation")?,
                lightness: int(l, "lightness")?,
            },
            other => bail!("layer {:?}: type {other} is not decoded", l.name),
        })
    }
}

enum Prepared {
    Copy,
    BrightnessContrast { b: f32, c: f32 },
    Levels { lo: [f32; 3], range: [f32; 3], mid: [f32; 3], out_lo: [f32; 3], out_range: [f32; 3] },
    ColourBalance { a: [f32; 3], e: [f32; 3], preserve: bool },
    Matrix([[f32; 3]; 3]),
}

fn mat_mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn inverse(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c = |r: usize, k: usize| m[(r + 1) % 3][(k + 1) % 3] * m[(r + 2) % 3][(k + 2) % 3];
    let cof = |r: usize, k: usize| c(r, k) - m[(r + 1) % 3][(k + 2) % 3] * m[(r + 2) % 3][(k + 1) % 3];
    let det: f64 = (0..3).map(|k| m[0][k] * cof(0, k)).sum();
    let mut inv = [[0.0; 3]; 3];
    for (r, row) in inv.iter_mut().enumerate() {
        for (k, v) in row.iter_mut().enumerate() {
            *v = cof(k, r) / det;
        }
    }
    inv
}

impl Prepared {
    fn new(kind: &LayerKind) -> Self {
        let bytes = |v: [u8; 3]| v.map(|b| b as f32 * INV_255);
        match *kind {
            LayerKind::LoadColourCube => Prepared::Copy,
            LayerKind::BrightnessContrast { brightness, contrast } => {
                Prepared::BrightnessContrast { b: 1.0 + brightness as f32 * 0.01, c: 1.0 + contrast as f32 * 0.01 }
            }
            LayerKind::Levels { input_mid, input_min, input_max, output_min, output_max } => {
                let (lo, hi, out_lo, out_hi) =
                    (bytes(input_min), bytes(input_max), bytes(output_min), bytes(output_max));
                Prepared::Levels {
                    lo,
                    range: [0, 1, 2].map(|c| hi[c] - lo[c]),
                    mid: input_mid,
                    out_lo,
                    out_range: [0, 1, 2].map(|c| out_hi[c] - out_lo[c]),
                }
            }
            LayerKind::ColourBalance { shadows, midtones, highlights, preserve_luminosity } => {
                let s = |v: i32| v as f32 * -0.01;
                let a = [0, 1, 2].map(|c| {
                    let p = |base: f64, v: i32| base.powf(f64::from(s(v))) as f32;
                    p(0.7, shadows[c]) * p(0.5, midtones[c]) * p(0.7, highlights[c])
                });
                let e = [0, 1, 2].map(|c| {
                    let recip = 1.0 / (1.0 - s(highlights[c]) * 0.5);
                    f64::from(recip).powf(f64::from(a[c])) as f32
                });
                Prepared::ColourBalance { a, e, preserve: preserve_luminosity }
            }
            LayerKind::HueSaturation { hue, saturation, lightness } => {
                let to_rgb = [[1.0, 1.0, 1.0], [0.0, -0.34414, 1.772], [1.402, -0.71414, 0.0]];
                let to_ycc = inverse(to_rgb).map(|r| r.map(|v| v as f32));
                let to_rgb = to_rgb.map(|r| r.map(|v| v as f32));
                let h = hue as f32 * 0.017_453_29;
                let (sin, cos) = h.sin_cos();
                let rot = [[1.0, 0.0, 0.0], [0.0, cos, sin], [0.0, -sin, cos]];
                let s = 1.0 + saturation as f32 * 0.01;
                let sat: [[f32; 3]; 3] =
                    [0, 1, 2].map(|i| [0, 1, 2].map(|j| if i == j { s } else { 0.0 } + (1.0 - s) * LUMA[i]));
                let l = 1.0 + lightness as f32 * 0.01;
                let light = [[l, 0.0, 0.0], [0.0, l, 0.0], [0.0, 0.0, l]];
                Prepared::Matrix(mat_mul(mat_mul(mat_mul(mat_mul(to_ycc, rot), to_rgb), sat), light))
            }
        }
    }

    fn eval(&self, x: [f32; 3]) -> [f32; 3] {
        match self {
            Prepared::Copy => x,
            Prepared::BrightnessContrast { b, c } => x.map(|v| (v * b - 0.5) * c + 0.5),
            Prepared::Levels { lo, range, mid, out_lo, out_range } => [0, 1, 2].map(|i| {
                let d = (x[i] - lo[i]).max(0.0);
                let t = if range[i] > 0.0 { (d / range[i]).min(1.0) } else { f32::from(u8::from(d > 0.0)) };
                out_lo[i] + out_range[i] * (mid[i] * t.log2()).exp2()
            }),
            Prepared::ColourBalance { a, e, preserve } => {
                let n = [0, 1, 2].map(|i| a[i] * (e[i] * x[i].log2()).exp2());
                if !preserve {
                    return n;
                }
                let dot = |v: [f32; 3]| v[0] * LUMA[0] + v[1] * LUMA[1] + v[2] * LUMA[2];
                let (l0, l1) = (dot(x), dot(n));
                if l1 > 0.0 { n.map(|v| v * l0 * (1.0 / l1)) } else { n }
            }
            Prepared::Matrix(m) => [0, 1, 2].map(|j| x[0] * m[0][j] + x[1] * m[1][j] + x[2] * m[2][j]),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub label: String,
    pub kind: LayerKind,
    pub blend: BlendMode,
    pub opacity: u32,
}

pub fn apply_stage(cube: &mut ColourCube, stage: &Stage) {
    if stage.opacity == 0 || stage.kind == LayerKind::LoadColourCube {
        return;
    }
    let f = Prepared::new(&stage.kind);
    let op256 = (stage.opacity << 8) / 255;
    let op = op256 as f32 / 256.0;
    for t in &mut cube.texels {
        let x = t.map(|b| b as f32 * INV_255);
        let y = f.eval(x);
        *t = match stage.blend {
            BlendMode::Normal if op256 == 256 => y.map(quantise),
            BlendMode::Normal => [0, 1, 2].map(|c| quantise(x[c] + op * (y[c] - x[c]))),
            mode => [0, 1, 2].map(|c| {
                let layer = quantise(y[c]) as f32 * INV_255;
                let blended = mode.apply(x[c], layer);
                quantise(if op256 == 256 { blended } else { x[c] + (blended - x[c]) * op })
            }),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(kind: LayerKind, blend: BlendMode, opacity: u32) -> Stage {
        Stage { label: String::new(), kind, blend, opacity }
    }

    fn q(v: f32) -> u8 {
        quantise(v)
    }

    #[test]
    fn identity_and_quantisation() {
        let id = ColourCube::identity();
        assert_eq!(id.texels[0], [0, 0, 0]);
        assert_eq!(id.texels[N * N * N - 1], [255, 255, 255]);
        assert_eq!(id.texels[1], [8, 0, 0]);
        assert_eq!(id.texels[5 * N], [0, 41, 0]);
        assert_eq!((q(0.5), q(1.0), q(-1.0), q(2.0), q(f32::NAN)), (127, 255, 0, 255, 0));
        assert!((0..=255u8).all(|b| q(b as f32 * INV_255) == b));
    }

    #[test]
    fn brightness_contrast_matches_closed_form() {
        let mut cube = ColourCube::identity();
        let id = cube.clone();
        apply_stage(
            &mut cube,
            &stage(LayerKind::BrightnessContrast { brightness: 0, contrast: 15 }, BlendMode::Normal, 255),
        );
        for (t, i) in cube.texels.iter().zip(&id.texels) {
            for c in 0..3 {
                assert_eq!(t[c], q((i[c] as f32 * INV_255 - 0.5) * (1.0 + 15.0 * 0.01) + 0.5));
            }
        }
    }

    #[test]
    fn identity_layers_keep_the_cube() {
        let id = ColourCube::identity();
        let layers = [
            LayerKind::Levels {
                input_mid: [1.0; 3],
                input_min: [0; 3],
                input_max: [255; 3],
                output_min: [0; 3],
                output_max: [255; 3],
            },
            LayerKind::HueSaturation { hue: 0, saturation: 0, lightness: 0 },
            LayerKind::BrightnessContrast { brightness: 0, contrast: 0 },
        ];
        for kind in layers {
            let mut cube = id.clone();
            apply_stage(&mut cube, &stage(kind.clone(), BlendMode::Normal, 255));
            assert_eq!(cube, id, "{kind:?}");
        }
    }

    #[test]
    fn colour_balance_coefficients() {
        let kind = LayerKind::ColourBalance {
            shadows: [15, 0, -23],
            midtones: [2, 0, 0],
            highlights: [0, 0, 0],
            preserve_luminosity: true,
        };
        let Prepared::ColourBalance { a, e, .. } = Prepared::new(&kind) else { panic!() };
        assert!((a[0] - 1.069685).abs() < 1e-5 && a[1] == 1.0 && (a[2] - 0.921239).abs() < 1e-5, "{a:?}");
        assert_eq!(e, [1.0; 3]);
        let kind = LayerKind::ColourBalance {
            shadows: [0; 3],
            midtones: [0; 3],
            highlights: [8, 0, 0],
            preserve_luminosity: false,
        };
        let Prepared::ColourBalance { a, e, .. } = Prepared::new(&kind) else { panic!() };
        assert!((e[0] - 1.04f32.powf(-a[0])).abs() < 1e-6);
        let p = Prepared::ColourBalance { a: [1.2, 1.0, 0.8], e: [1.0; 3], preserve: true };
        let y = p.eval([0.3, 0.5, 0.7]);
        let luma = |v: [f32; 3]| v[0] * LUMA[0] + v[1] * LUMA[1] + v[2] * LUMA[2];
        assert!((luma(y) - luma([0.3, 0.5, 0.7])).abs() < 1e-6);
    }

    #[test]
    fn blend_modes_and_opacity() {
        assert_eq!(BlendMode::Overlay.apply(0.25, 0.5), 0.25);
        assert_eq!(BlendMode::Overlay.apply(0.75, 0.5), 0.75);
        assert_eq!(BlendMode::Multiply.apply(0.5, 0.5), 0.25);
        assert_eq!(BlendMode::Screen.apply(0.5, 0.5), 0.75);
        assert_eq!((158u32 << 8) / 255, 158);
        assert_eq!((255u32 << 8) / 255, 256);
        let mut cube = ColourCube::identity();
        let kind = LayerKind::BrightnessContrast { brightness: 100, contrast: 0 };
        apply_stage(&mut cube, &stage(kind, BlendMode::Normal, 128));
        let x = 131.0 * INV_255;
        let op = ((128u32 << 8) / 255) as f32 / 256.0;
        assert_eq!(cube.texels[16][0], q(x + op * (2.0 * x - x)));
        let mut cube = ColourCube::identity();
        let kind = LayerKind::BrightnessContrast { brightness: 48, contrast: -29 };
        apply_stage(&mut cube, &stage(kind, BlendMode::Multiply, 255));
        let c = (20.0f32 * 255.0 / 31.0).floor() * INV_255;
        let (b, k) = (1.0 + 48.0 * 0.01, 1.0 - 29.0 * 0.01);
        assert_eq!(cube.texels[20][0], q(c * (q((c * b - 0.5) * k + 0.5) as f32 * INV_255)));
    }
}
