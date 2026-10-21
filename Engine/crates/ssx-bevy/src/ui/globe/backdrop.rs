use ssx_ui::game::fe::globe_look::LightState;
use ssx_ui::game::fe::globe_scene::{GlobeFrame, M4};

const SKY_DISTANCE_KM: f32 = 400.0;
const CLOUD_DISTANCE_SCALE: f32 = 0.5;
const BACKGROUND_INTENSITY: f32 = 1.0;

pub const UNIFORM_FLOATS: usize = 100;

fn recip(x: f32) -> f32 {
    if x == 0.0 { 1e30 } else { 1.0 / x }
}

fn scattering(l: &LightState) -> [[f32; 3]; 4] {
    const K_RAYLEIGH: f32 = f32::from_bits(0x0DC9_7207);
    const K_RAYLEIGH_DASH: f32 = f32::from_bits(0x0BC0_5DBF);
    const K_MIE: f32 = f32::from_bits(0x277F_2301);
    const K_MIE_DASH: f32 = f32::from_bits(0x2622_6CD2);
    const WAVELENGTH_SCALE: f32 = f32::from_bits(0x3586_37BD);
    let v = |n: &str| l.vector(n).unwrap_or([0.0; 4]);
    let f = |n: &str| l.float(n).unwrap_or(0.0);
    let (colour, mie_colour) = (v("atmosRayleighColour"), v("atmosMieColour"));
    let (ray, mie) = (f("atmosRayleighMultiplier"), f("atmosMieMultiplier"));
    let mut out = [[0.0; 3]; 4];
    for i in 0..3 {
        let lambda = (1.0 - colour[i]).abs() * WAVELENGTH_SCALE;
        let (l2, l4) = (lambda * lambda, lambda * lambda * lambda * lambda);
        out[0][i] = K_RAYLEIGH / l4 * ray;
        out[1][i] = K_RAYLEIGH_DASH / l4 * ray;
        out[2][i] = K_MIE / l2 * mie_colour[i] * mie;
        out[3][i] = K_MIE_DASH / l2 * mie;
    }
    out
}

fn projection_inverse(p: &M4) -> M4 {
    let (xs, ys, a, b) = (p[0][0], p[1][1], p[2][2], p[3][2]);
    [[1.0 / xs, 0.0, 0.0, 0.0], [0.0, 1.0 / ys, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0 / b], [0.0, 0.0, -1.0, a / b]]
}

pub fn uniform(frame: &GlobeFrame, viewport: [u32; 2]) -> Vec<f32> {
    let l = &frame.light;
    let f = |n: &str| l.float(n).unwrap_or(0.0);
    let v3 = |n: &str| {
        let v = l.vector(n).unwrap_or([0.0; 4]);
        [v[0], v[1], v[2]]
    };
    let [beta_r, beta_dash_r, beta_m, beta_dash_m] = scattering(l);
    let ext = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
    let use_light_texture = if l.textures.contains_key("dirLightTexture") { 1.0 } else { 0.0 };
    let vectors: [[f32; 4]; 13] = [
        ext(frame.camera_pos, BACKGROUND_INTENSITY),
        ext(beta_r, f("atmosHenyeyGreensteinParam")),
        ext(beta_dash_r, f("atmosPlanetRadius")),
        ext(beta_m, SKY_DISTANCE_KM),
        ext(beta_dash_m, CLOUD_DISTANCE_SCALE),
        [
            recip(f("atmosRayleighHeightFalloff")),
            f("atmosRayleighHeightOffset"),
            recip(f("atmosMieHeightFalloffSky")),
            f("atmosMieHeightOffsetSky"),
        ],
        [f("atmosExtinctionMultiplier"), f("atmosInScatteringMultiplier"), f("globalLightingMult"), use_light_texture],
        ext(v3("dirLight_Direction"), f("dirLightTextureUVScale")),
        ext(v3("dirLight_Colour"), f("cloudTextureOffset")),
        ext(v3("cloudRampColor0"), f("cloudRampIntensity")),
        ext(v3("cloudRampColor1"), f("cloudDensityMultiplier")),
        [f("cloudDensityRimLevel"), recip(f("cloudDensityRimSize")), f("skySunRimIntensity"), f("skySunRimFalloff")],
        [f("skySunIntensity"), f("skySunFalloff"), 1.0 / viewport[0].max(1) as f32, 1.0 / viewport[1].max(1) as f32],
    ];
    let mut out = Vec::with_capacity(UNIFORM_FLOATS);
    for m in [&projection_inverse(&frame.proj), &frame.view_inverse, &frame.view] {
        out.extend(m.iter().flatten());
    }
    out.extend(vectors.iter().flatten());
    out
}

pub fn sky_textures(l: &LightState) -> [Option<String>; 3] {
    ["cloudDensityTexture", "cloudDensityNormalTexture", "dirLightTexture"].map(|n| l.textures.get(n).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_projection_inverse_undoes_the_projection() {
        let (n, f, ys, xs) = (10.0f32, 100_000.0f32, 2.0f32, 1.2f32);
        let p: M4 =
            [[xs, 0.0, 0.0, 0.0], [0.0, ys, 0.0, 0.0], [0.0, 0.0, f / (n - f), -1.0], [0.0, 0.0, n * f / (n - f), 0.0]];
        let inv = projection_inverse(&p);
        let v = [3.0f32, -2.0, -50.0, 1.0];
        let c: [f32; 4] = std::array::from_fn(|j| (0..4).map(|k| v[k] * p[k][j]).sum());
        let back: [f32; 4] = std::array::from_fn(|j| (0..4).map(|k| c[k] * inv[k][j]).sum());
        for i in 0..3 {
            assert!((back[i] / back[3] - v[i]).abs() < 1e-3, "{back:?}");
        }
    }
}
