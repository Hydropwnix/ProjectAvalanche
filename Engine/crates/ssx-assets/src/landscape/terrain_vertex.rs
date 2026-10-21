use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_formats::subd::SubdMesh;

pub const LAYER_USED_THRESHOLD: f32 = 1.525_878_9e-5;
pub const SHADER_LAYERS: usize = 4;
pub const LAYER_NAMES: [&str; 6] = ["Windswept", "Crusty", "Wet", "Groomed", "Crud", "Powder"];

pub fn normalise_layer_weights(a: &mut [f32; 8]) {
    let sum = a[4] + a[0] + a[1] + a[2] + a[3];
    if sum > 1.0 {
        let r = 1.0 / sum;
        for w in &mut a[..5] {
            *w *= r;
        }
    }
}

pub fn layer_weights(a: &[f32; 8]) -> [f32; 6] {
    [a[0], a[1], a[2], a[3], a[4], 1.0 - ((((a[4] + a[3]) + a[2]) + a[1]) + a[0])]
}

pub fn used_layers(mesh: &SubdMesh) -> Vec<u8> {
    let mut used = [false; 6];
    for face in mesh.faces.iter().filter(|f| f.unk_a & 1 == 0) {
        for v in mesh.face_vertices(face) {
            let Some(vertex) = mesh.vertices.get(v as usize) else { continue };
            let mut a = vertex.attributes;
            normalise_layer_weights(&mut a);
            for (k, w) in layer_weights(&a).into_iter().enumerate() {
                used[k] |= w > LAYER_USED_THRESHOLD;
            }
        }
    }
    (0..6u8).filter(|&k| used[k as usize]).collect()
}

fn trunc255(x: f32) -> i32 {
    (x * 255.0) as i32
}

fn or_lanes(l: [i32; 4]) -> [u8; 4] {
    ((l[0] as u32) | ((l[1] as u32) << 8) | ((l[2] as u32) << 16) | ((l[3] as u32) << 24)).to_le_bytes()
}

pub fn pack_colors(a: &[f32; 8], layers: &[u8]) -> ([u8; 4], [u8; 4]) {
    let w = layer_weights(a);
    let mut c0 = [0i32; 4];
    for (lane, &k) in c0.iter_mut().zip(layers.iter().take(SHADER_LAYERS)) {
        *lane = trunc255(w[k as usize]);
    }
    (or_lanes(c0), or_lanes([trunc255(a[5]), trunc255(a[6]), trunc255(a[7]), 0]))
}

pub fn quantized_attributes(a: &[f32; 8]) -> [f32; 8] {
    let q = |x: f32| (((x * 15.0) as i32) & 0xF) as f32 * (1.0 / 15.0);
    let mut out = a.map(q);
    out[5] = q((2.0 * a[5]).clamp(0.0, 1.0));
    out
}

pub fn displacement(a: &[f32; 8], snow_depth: &[f32; 6]) -> f32 {
    let q = quantized_attributes(a);
    let q5 = q[5];
    let mut m = [0.0f32; 6];
    for k in 0..5 {
        m[k] = q[k] * q5;
    }
    m[5] = (1.0 - (q[0] + q[1] + q[2] + q[3] + q[4]).clamp(0.0, 1.0)) * q5;
    q5 * m.iter().zip(snow_depth).fold(0.0, |s, (m, d)| s + d * m)
}

pub fn displaced_position(p: [f32; 3], n: [f32; 3], a: &[f32; 8], snow_depth: &[f32; 6]) -> [f32; 3] {
    let d = displacement(a, snow_depth);
    [p[0] + n[0] * d, p[1] + n[1] * d, p[2] + n[2] * d]
}

pub fn snow_depths(big_root: &Path) -> Result<[f32; 6]> {
    let path = big_root.join("cache0/data/common/anim/anim_all.cba");
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let stream = ssx_formats::ant::Stream::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    let mut out = [0.0f32; 6];
    for (k, layer) in LAYER_NAMES.iter().enumerate() {
        let name = format!("SP_{layer}");
        let asset = stream
            .assets_of_class("SurfacePropertiesAsset")
            .find(|a| a.object.name(&stream.schema) == Some(name.as_str()))
            .with_context(|| format!("no SurfacePropertiesAsset {name} in {}", path.display()))?;
        match asset.object.get(&stream.schema, "SnowDepth") {
            Some(ssx_formats::ant::Value::Float(d)) => out[k] = *d,
            other => bail!("{name}.SnowDepth is {other:?}"),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_powder_sign_extends_into_higher_lanes() {
        let a = [0.6, 0.0, 0.0, 0.0, 0.5, 1.0, 0.0, 1.0];
        let (c0, c1) = pack_colors(&a, &[0, 5]);
        assert_eq!(c0[0], 153);
        assert_eq!(c0[1], (-25i32) as u8);
        assert_eq!(&c0[2..], &[0xFF, 0xFF]);
        assert_eq!(c1, [255, 0, 255, 0]);
    }

    #[test]
    fn layer_weights_are_normalised_only_above_one() {
        let mut a = [0.6, 0.0, 0.0, 0.0, 0.5, 1.0, 0.0, 1.0];
        normalise_layer_weights(&mut a);
        assert!(((a[0] + a[4]) - 1.0).abs() < 1e-6 && (a[0] / a[4] - 1.2).abs() < 1e-5);
        assert_eq!(&a[5..], &[1.0, 0.0, 1.0]);
        let mut b = [0.2, 0.1, 0.0, 0.0, 0.3, 0.5, 1.0, 0.4];
        let before = b;
        normalise_layer_weights(&mut b);
        assert_eq!(b, before);
    }

    #[test]
    fn displacement_scales_with_coverage_squared() {
        let depth = [0.15, 0.15, 0.15, 0.15, 0.15, 0.3];
        let mut a = [0.0f32; 8];
        a[5] = 1.0;
        assert!((displacement(&a, &depth) - 0.3).abs() < 1e-6);
        a[5] = 0.25;
        let q5 = 7.0 / 15.0;
        assert!((displacement(&a, &depth) - 0.3 * q5 * q5).abs() < 1e-6);
        a[5] = 0.0;
        assert_eq!(displacement(&a, &depth), 0.0);
    }
}
