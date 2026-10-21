pub mod overlay;
mod render;

use std::f32::consts::PI;

use anyhow::{Context, Result};
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use ssx_assets::postfx;

use crate::render::lighting::GameLighting;

pub struct PostPlugin;

#[derive(Component, Clone, Copy, Debug, ExtractComponent)]
pub struct GamePostCamera;

#[derive(Resource, Clone, Debug)]
#[allow(dead_code)]
pub struct PostStatus(pub String);

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct PostUniform {
    pub hable_a: Vec4,
    pub hable_b: Vec4,
    pub exposure: Vec4,
    pub bloom: Vec4,
    pub blur_h_samples: Vec4,
    pub blur_h_offsets: Vec4,
    pub blur_v_samples: Vec4,
    pub blur_v_offsets: Vec4,
    pub sizes: UVec4,
}

#[derive(Resource, Clone, ExtractResource)]
pub struct PostSettings {
    pub uniform: PostUniform,
    pub lut: Handle<Image>,
    pub scene_ready: bool,
    pub converge: bool,
}

impl Plugin for PostPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "post_common.wgsl");
        embedded_asset!(app, "post_downsample.wgsl");
        embedded_asset!(app, "post_adapt.wgsl");
        embedded_asset!(app, "post_bloom.wgsl");
        embedded_asset!(app, "post_final.wgsl");
        embedded_asset!(app, "post_fxaa.wgsl");
        app.add_systems(Update, (tick.run_if(resource_exists::<PostSettings>), msaa_off));
        use crate::render::lighting::fade::{LightingFadeSet, faded};
        app.add_systems(Update, follow_lighting.after(LightingFadeSet).run_if(faded));
        overlay::build(app);
        render::build(app);
        crate::shell::level::scoped::<PostSettings>(app);
        crate::shell::level::scoped::<overlay::ScreenEffects>(app);
        app.add_systems(crate::shell::level::LevelStartup, |status: Option<Res<PostStatus>>| {
            if let Some(s) = status {
                info!("{}", s.0);
            }
        });
    }
}

pub fn reset_level(world: &mut World) {
    overlay::reset(world);
}

pub fn env_is(key: &str, value: &str) -> bool {
    std::env::var(key).is_ok_and(|v| v.eq_ignore_ascii_case(value))
}

fn hable(x: f32, [a, b, c, d, e, f]: [f32; 6]) -> f32 {
    ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * f)) - e / f
}

fn gaussian_taps(sigma: f32) -> (Vec4, Vec4) {
    let mut w = [0.0f32; 9];
    for (i, v) in w.iter_mut().enumerate() {
        let x = i as f32 - 4.0;
        *v = (-x * x / (2.0 * sigma * sigma)).exp() / (2.0 * PI * sigma * sigma);
    }
    let sum: f32 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= sum);
    let outer = 3.0 + w[0] / (w[0] + w[1]);
    let inner = 1.0 + w[2] / (w[2] + w[3]);
    (Vec4::new(w[0] + w[1], w[2] + w[3], w[4], 0.0), Vec4::new(-outer, -inner, inner, outer))
}

fn lut_image(cube: &postfx::ColourCube) -> Image {
    let n = postfx::CUBE_SIZE as u32;
    let size = Extent3d { width: n, height: n, depth_or_array_layers: n };
    Image::new(size, TextureDimension::D3, cube.rgba8(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD)
}

pub struct PostLoad {
    uniform: PostUniform,
    lut: Image,
    converge: bool,
    pub status: String,
}

pub fn load(lighting: &GameLighting, root: &std::path::Path, converge: bool) -> Result<PostLoad> {
    if env_is("SSX_GAME_POST", "off") {
        anyhow::bail!("disabled by SSX_GAME_POST=off");
    }
    let name = lighting.postfx.clone().context("the drop names no post-FX preset")?;
    let (preset, build) = postfx::load_presets(root, &[name.as_str()])?.pop().context("preset not built")?;
    let l = lighting;
    let ([a, b, c, d, e, f], white) = curve(l);
    let blur = preset.bloom_blur().context("preset has no bloom_blur_x/y")?;
    let (hs, ho) = gaussian_taps(blur[0]);
    let (vs, vo) = gaussian_taps(blur[1]);
    let glm = l.f("globalLightingMult");
    let mut uniform = PostUniform {
        bloom: Vec4::new(0.0, 0.0, f32::from(u8::from(preset.bloom_active())), 1.0),
        blur_h_samples: hs,
        blur_h_offsets: ho,
        blur_v_samples: vs,
        blur_v_offsets: vo,
        ..Default::default()
    };
    lighting_constants(l, &mut uniform);
    let identity = env_is("SSX_POST_LUT", "identity");
    let cube = if identity { postfx::ColourCube::identity() } else { build.cube.clone() };
    let lut = lut_image(&cube);

    let groups: Vec<&str> = preset.colour_cubes.iter().map(|c| c.group.as_str()).collect();
    let second = if build.needs_second_lut { " (depth-blended 2nd LUT NOT built)" } else { "" };
    let status = format!(
        "post: game HDR chain, preset {name} | x gLM {glm} | key {} rate {} | Hable A{a} B{b} C{c} D{d} E{e} F{f} W{white}\n\
         bloom {} thr {} stops w {} sigma {:?} | colour cube {}{second} (max delta {}){}\n\
         as the game: L16 luminance chain (BlitL16 mips), FXAA, no MSAA; samplers from hdr/postfx/fxaa.fxo",
        uniform.exposure.y,
        uniform.exposure.z,
        if preset.bloom_active() { "on" } else { "off" },
        uniform.bloom.x,
        uniform.bloom.y,
        blur,
        groups.join("+"),
        build.cube.max_deviation(),
        if identity { " BYPASSED (SSX_POST_LUT=identity)" } else { "" },
    );
    Ok(PostLoad { uniform, lut, converge, status })
}

fn curve(l: &GameLighting) -> ([f32; 6], f32) {
    let curve = [
        l.f("toneMappingShoulderStrength"),
        l.f("toneMappingLinearStrength"),
        l.f("toneMappingLinearAngle"),
        l.f("toneMappingToeStrength"),
        l.f("toneMappingToeNumerator"),
        l.f("toneMappingToeDenominator"),
    ];
    (curve, l.f("toneMappingLinearWhite"))
}

fn lighting_constants(l: &GameLighting, u: &mut PostUniform) {
    let (curve, white) = curve(l);
    let [a, b, c, d, e, f] = curve;
    let h_white = hable(white, curve);
    u.hable_a = Vec4::new(a, b, b * c, d * e);
    u.hable_b = Vec4::new(d * f, e / f, 1.0 / h_white, 0.0);
    u.exposure = Vec4::new(
        l.f("globalLightingMult"),
        l.f("dynamicExposureKeyValue"),
        l.f("dynamicExposureAdaptionRate"),
        u.exposure.w,
    );
    (u.bloom.x, u.bloom.y) = (l.f("bloom_threshold"), l.f("bloom_weight"));
}

fn follow_lighting(lighting: Res<GameLighting>, settings: Option<ResMut<PostSettings>>) {
    if let Some(mut settings) = settings {
        lighting_constants(&lighting, &mut settings.uniform);
    }
}

pub fn insert(world: &mut World, load: PostLoad) -> Result<()> {
    let lut = world.get_resource_mut::<Assets<Image>>().context("no image assets")?.add(load.lut);
    world.insert_resource(PostSettings { uniform: load.uniform, lut, scene_ready: false, converge: load.converge });
    Ok(())
}

fn tick(time: Res<Time>, meshes: Query<(), With<Mesh3d>>, mut settings: ResMut<PostSettings>) {
    settings.uniform.exposure.w = time.delta_secs();
    settings.scene_ready = !meshes.is_empty();
}

fn msaa_off(mut cameras: Query<&mut Msaa, Added<GamePostCamera>>) {
    for mut msaa in &mut cameras {
        *msaa = Msaa::Off;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_taps_sum_to_one_and_mirror() {
        let (s, o) = gaussian_taps(3.0);
        assert!((2.0 * s.x + 2.0 * s.y + s.z - 1.0).abs() < 1e-6);
        assert!(o.x < -3.0 && o.x > -4.0 && o.y < -1.0 && o.y > -2.0 && o.z == -o.y && o.w == -o.x);
    }

    #[test]
    fn hable_is_zero_at_zero() {
        let curve = [0.22, 0.3, 0.1, 0.2, 0.01, 0.3];
        assert!(hable(0.0, curve).abs() < 1e-7);
        assert!(hable(3.0, curve) > hable(1.0, curve));
    }
}
