use std::path::Path;

use anyhow::{Context, Result};
use ssx_assets::career::globe::GlobeData;
use ssx_assets::vault_db::Material;
use ssx_ui::game::fe::globe_look::LightState;
use ssx_ui::game::fe::globe_scene::{M4, MidgroundFrame};

pub const TEXTURES: [&str; 7] = [
    "normalTexture",
    "ambOcclusionTexture",
    "maskTexture",
    "redChannelTexture",
    "greenChannelTexture",
    "baseChannelTexture",
    "fePeakHighlightTexture",
];

#[derive(Clone, Debug, Default)]
pub struct MidgroundMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct MipTexture {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
    pub blocks: Option<Blocks>,
}

#[derive(Clone, Debug)]
pub struct Blocks {
    pub format: ssx_formats::xenos::TextureFormat,
    pub levels: Vec<Vec<u8>>,
}

pub(super) fn blocks_of(
    data: &[u8],
    xpr: &ssx_formats::xpr::XprFile,
    res: &ssx_formats::xpr::XprResource,
    (width, height): (u32, u32),
    levels: usize,
) -> Option<Blocks> {
    let (format, found) = ssx_assets::texture::resource_blocks(data, xpr, res).ok()??;
    if width % 4 != 0 || height % 4 != 0 || found.len() < levels {
        return None;
    }
    let shapes_match = found
        .iter()
        .take(levels)
        .enumerate()
        .all(|(i, l)| (l.width, l.height) == ((width >> i).max(1), (height >> i).max(1)));
    shapes_match.then(|| Blocks { format, levels: found.into_iter().take(levels).map(|l| l.blocks).collect() })
}

#[derive(Clone, Debug, Default)]
pub struct MidgroundPart {
    pub mesh: MidgroundMesh,
    pub material: Material,
    pub textures: Vec<MipTexture>,
}

pub const WAVE_TEXTURES: [&str; 3] = ["normalTexture", "diffuseTexture", "specularTexture"];

#[derive(Clone, Debug, Default)]
pub struct WavePart {
    pub mesh: MidgroundMesh,
    pub colours: Vec<[u8; 4]>,
    pub material: Material,
    pub textures: Vec<MipTexture>,
}

#[derive(Clone, Debug, Default)]
pub struct MidgroundAssets {
    pub range: u32,
    pub parts: Vec<MidgroundPart>,
    pub waves: Vec<WavePart>,
}

pub(super) fn mip_texture(big_root: &Path, rel: &str) -> Result<MipTexture> {
    let rel = rel.replace('\\', "/");
    let path = ssx_assets::track_objects::asset_file(big_root, &format!("{rel}.xpr"))
        .with_context(|| format!("{rel}.xpr not found"))?;
    mip_texture_at(&path)
}

pub(super) fn mip_texture_at(path: &Path) -> Result<MipTexture> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let xpr = ssx_formats::xpr::XprFile::parse(&data)?;
    let res = xpr.resources.first().context("no texture resource")?;
    let levels = ssx_assets::texture::decode_resource(&data, &xpr, res, true, ssx_assets::texture::DxnExport::Raw)?;
    let mut levels: Vec<_> = levels.into_iter().filter(|l| l.face == 0).collect();
    levels.sort_by_key(|l| l.level);
    let first = levels.first().context("no texture level")?;
    let (width, height) = (first.image.width, first.image.height);
    let mut out = MipTexture { width, height, levels: Vec::new(), blocks: None };
    for (i, l) in levels.iter().enumerate() {
        let (w, h) = ((width >> i).max(1), (height >> i).max(1));
        if l.image.width != w || l.image.height != h {
            break;
        }
        out.levels.push(l.image.rgba.clone());
    }
    out.blocks = blocks_of(&data, &xpr, res, (width, height), out.levels.len());
    Ok(out)
}

pub fn texture_bytes<'a>(textures: impl IntoIterator<Item = (&'a [Vec<u8>], Option<&'a Blocks>)>) -> [usize; 4] {
    let mut out = [0; 4];
    for (rgba, blocks) in textures {
        let all: usize = rgba.iter().map(Vec::len).sum();
        out[3] += all;
        match blocks {
            Some(b) => {
                out[0] += 1;
                out[1] += b.levels.iter().map(Vec::len).sum::<usize>();
            }
            None => out[2] += all,
        }
    }
    out
}

pub fn bytes_line(n: usize, [k, bc, rgba, all]: [usize; 4]) -> String {
    let mb = |b: usize| b as f64 / 1048576.0;
    format!(
        "{k} of {n} textures with the disc's blocks ({:.1} MB), the rest RGBA8 only ({:.1} MB); all as RGBA8 {:.1} MB",
        mb(bc),
        mb(rgba),
        mb(all)
    )
}

impl MidgroundAssets {
    pub fn load(big_root: &Path, data: &GlobeData, range: u32) -> Result<Self> {
        let meshes = data.midground_meshes.get(&range).with_context(|| format!("range {range:#x}: no mid-ground"))?;
        let mut parts = Vec::new();
        for m in meshes.iter().filter(|m| m.material.effect.as_deref() == Some("terrain_midground_fe.fx")) {
            let rel = m.geometry.replace('\\', "/");
            let path =
                ssx_assets::track_objects::asset_file(big_root, &rel).with_context(|| format!("{rel} not found"))?;
            let o = ssx_assets::object_mesh::load(&path)?;
            let n = o.positions.len();
            let mesh = MidgroundMesh {
                normals: o.normals.clone().unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; n]),
                tangents: o.tangents.clone().unwrap_or_else(|| vec![[1.0, 0.0, 0.0]; n]),
                uvs: o.uv0.clone().unwrap_or_else(|| vec![[0.0, 0.0]; n]),
                positions: o.positions,
                indices: o.indices,
            };
            let textures = TEXTURES
                .iter()
                .map(|p| {
                    let rel = m.material.textures.get(*p).with_context(|| format!("material without {p}"))?;
                    mip_texture(big_root, rel)
                })
                .collect::<Result<Vec<_>>>()?;
            parts.push(MidgroundPart { mesh, material: m.material.clone(), textures });
        }
        let mut waves = Vec::new();
        for m in meshes.iter().filter(|m| m.material.effect.as_deref() == Some("terrain_oceanwave_fe.fx")) {
            let rel = m.geometry.replace('\\', "/");
            let path =
                ssx_assets::track_objects::asset_file(big_root, &rel).with_context(|| format!("{rel} not found"))?;
            let o = ssx_assets::object_mesh::load(&path)?;
            let n = o.positions.len();
            let colours = o.colours.clone().unwrap_or_else(|| vec![[255; 4]; n]);
            let mesh = MidgroundMesh {
                normals: o.normals.clone().unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; n]),
                tangents: o.tangents.clone().unwrap_or_else(|| vec![[1.0, 0.0, 0.0]; n]),
                uvs: o.uv0.clone().unwrap_or_else(|| vec![[0.0, 0.0]; n]),
                positions: o.positions,
                indices: o.indices,
            };
            let textures = WAVE_TEXTURES
                .iter()
                .map(|p| {
                    let rel = m.material.textures.get(*p).with_context(|| format!("material without {p}"))?;
                    mip_texture(big_root, rel)
                })
                .collect::<Result<Vec<_>>>()?;
            waves.push(WavePart { mesh, colours, material: m.material.clone(), textures });
        }
        Ok(MidgroundAssets { range, parts, waves })
    }
}

pub const UNIFORM_FLOATS: usize = 152;

fn scattering(l: &LightState) -> [[f32; 3]; 4] {
    const K_RAYLEIGH: f32 = f32::from_bits(0x0DC9_7207);
    const K_RAYLEIGH_DASH: f32 = f32::from_bits(0x0BC0_5DBF);
    const K_MIE: f32 = f32::from_bits(0x277F_2301);
    const K_MIE_DASH: f32 = f32::from_bits(0x2622_6CD2);
    const WAVELENGTH_SCALE: f32 = f32::from_bits(0x3586_37BD);
    let v = |n: &str| l.vector(n).unwrap_or([0.0; 4]);
    let f = |n: &str| l.float(n).unwrap_or(0.0);
    let colour = v("atmosRayleighColour");
    let mie_colour = v("atmosMieColour");
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

fn recip(x: f32) -> f32 {
    if x == 0.0 { 1e30 } else { 1.0 / x }
}

pub fn uniform(frame: &MidgroundFrame, m: &Material, light: &LightState, lod0: bool) -> Vec<f32> {
    let f = |n: &str| m.float(n).unwrap_or(0.0);
    let mv = |n: &str, i: usize| m.vectors.get(n).and_then(|v| v.get(i).copied()).unwrap_or(0.0);
    let lf = |n: &str| light.float(n).unwrap_or(0.0);
    let lv = |n: &str| light.vector(n).unwrap_or([0.0; 4]);
    let d = lv("dirLight_Direction");
    let (doff, bounce) = (light.sun_offset, light.bounce);
    let v3 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
    let [br, bdr, bm, bdm] = scattering(light);
    let vectors: [[f32; 4]; 22] = [
        v3(frame.camera_pos, 1.0),
        d,
        lv("dirLight_Colour"),
        v3(super::to_view(doff, &frame.view), 0.0),
        v3(super::to_view(bounce, &frame.view), 0.0),
        lv("rampColor0"),
        lv("rampColor1"),
        lv("rampColor2"),
        [lf("rampIntensity"), lf("rampRimFalloff"), lf("rampRimInLight"), lf("rampRimInShadow")],
        [lf("rampBounceAtt"), lf("rampBounceSpread"), lf("globalLightingMult"), frame.game_time],
        v3(br, lf("atmosHenyeyGreensteinParam")),
        v3(bdr, lf("atmosPlanetRadius")),
        v3(bm, 0.0),
        v3(bdm, 0.0),
        [
            recip(lf("atmosRayleighHeightFalloff")),
            lf("atmosRayleighHeightOffset"),
            recip(lf("atmosMieHeightFalloff")),
            lf("atmosMieHeightOffset"),
        ],
        [lf("atmosExtinctionMultiplier"), lf("atmosInScatteringMultiplier"), 0.0, 0.0],
        [mv("baseChannelTile", 0), mv("redChannelTile", 0), mv("greenChannelTile", 0), f("snowBias")],
        [mv("aoTile", 0), mv("aoTile", 1), if lod0 { 1.0 } else { 0.0 }, 0.0],
        frame.highlight,
        [mv("fePeakColour", 0), mv("fePeakColour", 1), mv("fePeakColour", 2), mv("fePeakColour", 3)],
        [f("fePeakCoverage"), f("fePeakLinesSpacing"), f("fePeakLinesWidth"), f("fePeakLinesSharpness")],
        [f("fePeakLinesStrength"), f("fePeakPulseFrequency"), f("fePeakPulseStrength"), 0.0],
    ];
    let mut out = Vec::with_capacity(UNIFORM_FLOATS);
    let mats: [&M4; 4] = [&frame.world, &frame.view, &frame.proj, &frame.highlight_xform];
    for mat in mats {
        out.extend(mat.iter().flatten());
    }
    out.extend(vectors.iter().flatten());
    out
}

pub const WAVE_UNIFORM_FLOATS: usize = 132;

pub fn wave_uniform(frame: &MidgroundFrame, m: &Material, light: &LightState) -> Vec<f32> {
    let f = |n: &str| m.float(n).unwrap_or(0.0);
    let mv = |n: &str, i: usize| m.vectors.get(n).and_then(|v| v.get(i).copied()).unwrap_or(0.0);
    let lf = |n: &str| light.float(n).unwrap_or(0.0);
    let lv = |n: &str| light.vector(n).unwrap_or([0.0; 4]);
    let v3 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
    let [br, bdr, bm, bdm] = scattering(light);
    let [a, b, c, d, e, ff] = [
        "toneMappingShoulderStrength",
        "toneMappingLinearStrength",
        "toneMappingLinearAngle",
        "toneMappingToeStrength",
        "toneMappingToeNumerator",
        "toneMappingToeDenominator",
    ]
    .map(lf);
    let white = lf("toneMappingLinearWhite");
    let hable = |x: f32| ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * ff)) - e / ff;
    let vectors: [[f32; 4]; 21] = [
        v3(frame.camera_pos, frame.game_time),
        lv("dirLight_Direction"),
        lv("dirLight_Colour"),
        v3(super::to_view(light.sun_offset, &frame.view), 0.0),
        v3(super::to_view(light.bounce, &frame.view), 0.0),
        lv("rampColor0"),
        lv("rampColor1"),
        lv("rampColor2"),
        [lf("rampIntensity"), lf("rampRimFalloff"), lf("rampRimInLight"), lf("rampRimInShadow")],
        [lf("rampBounceAtt"), lf("rampBounceSpread"), lf("globalLightingMult"), 0.0],
        lv("globalSpecularColour"),
        v3(br, lf("atmosHenyeyGreensteinParam")),
        v3(bdr, lf("atmosPlanetRadius")),
        v3(bm, 0.0),
        v3(bdm, 0.0),
        [
            recip(lf("atmosRayleighHeightFalloff")),
            lf("atmosRayleighHeightOffset"),
            recip(lf("atmosMieHeightFalloff")),
            lf("atmosMieHeightOffset"),
        ],
        [
            lf("atmosExtinctionMultiplier"),
            lf("atmosInScatteringMultiplier"),
            f("specularIntensity"),
            f("specularEccentricity"),
        ],
        [mv("UVspeed", 0), mv("UVspeed", 1), mv("diffuseTile", 0), mv("diffuseTile", 1)],
        [mv("normalTile", 0), mv("normalTile", 1), mv("specularTile", 0), mv("specularTile", 1)],
        [a, b, b * c, d * e],
        [d * ff, e / ff, 1.0 / hable(white), lf("dynamicExposureKeyValue")],
    ];
    let view_proj = ssx_ui::game::fe::globe_scene::mmul(&frame.view, &frame.proj);
    let mut out = Vec::with_capacity(WAVE_UNIFORM_FLOATS);
    for mat in [&frame.world, &view_proj, &frame.view] {
        out.extend(mat.iter().flatten());
    }
    out.extend(vectors.iter().flatten());
    out
}
