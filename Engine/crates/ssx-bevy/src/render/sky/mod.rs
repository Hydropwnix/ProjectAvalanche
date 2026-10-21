mod midground;
mod render;

use std::path::Path;

use anyhow::{Context, Result};
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};

use crate::render::lighting::GameLighting;

const SKY_DISTANCE_KM: f32 = 400.0;
const CLOUD_DISTANCE_SCALE: f32 = 0.5;
const K_RAYLEIGH: f32 = f32::from_bits(0x0DC9_7207);
const K_RAYLEIGH_DASH: f32 = f32::from_bits(0x0BC0_5DBF);
const K_MIE: f32 = f32::from_bits(0x277F_2301);
const K_MIE_DASH: f32 = f32::from_bits(0x2622_6CD2);
const WAVELENGTH_SCALE: f32 = f32::from_bits(0x3586_37BD);

pub struct SkyPlugin;

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct SkyUniform {
    pub beta_r: Vec4,
    pub beta_dash_r: Vec4,
    pub beta_m: Vec4,
    pub beta_dash_m: Vec4,
    pub heights: Vec4,
    pub multipliers: Vec4,
    pub light_dir: Vec4,
    pub light_colour: Vec4,
    pub cloud_ramp0: Vec4,
    pub cloud_ramp1: Vec4,
    pub cloud: Vec4,
    pub sun: Vec4,
}

#[derive(Resource, Clone, ExtractResource)]
pub struct SkySettings {
    pub uniform: SkyUniform,
    pub cloud_density: Handle<Image>,
    pub cloud_normal: Handle<Image>,
    pub light_texture: Handle<Image>,
    pub midground: Option<midground::MidgroundSettings>,
}

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sky.wgsl");
        embedded_asset!(app, "midground.wgsl");
        render::build(app);
        crate::shell::level::scoped::<SkySettings>(app);
        use crate::render::lighting::fade::{LightingFadeSet, faded};
        app.add_systems(Update, follow_lighting.after(LightingFadeSet).run_if(faded));
    }
}

fn follow_lighting(lighting: Res<GameLighting>, settings: Option<ResMut<SkySettings>>) {
    let Some(mut settings) = settings else { return };
    settings.uniform = sky_uniform(&lighting);
    for part in settings.midground.iter_mut().flat_map(|m| &mut m.parts) {
        part.uniform = midground::relit(&lighting, &part.uniform);
    }
}

fn recip(x: f32) -> f32 {
    if x == 0.0 { 1e30 } else { 1.0 / x }
}

fn scattering(l: &GameLighting) -> [Vec3; 4] {
    let colour = l.v("atmosRayleighColour").truncate();
    let lambda = (Vec3::ONE - colour).abs() * WAVELENGTH_SCALE;
    let l2 = lambda * lambda;
    let l4 = l2 * l2;
    let (ray, mie) = (l.f("atmosRayleighMultiplier"), l.f("atmosMieMultiplier"));
    let mie_colour = l.v("atmosMieColour").truncate();
    [
        Vec3::splat(K_RAYLEIGH) / l4 * ray,
        Vec3::splat(K_RAYLEIGH_DASH) / l4 * ray,
        Vec3::splat(K_MIE) / l2 * mie_colour * mie,
        Vec3::splat(K_MIE_DASH) / l2 * mie,
    ]
}

fn henyey_greenstein(l: &GameLighting) -> f32 {
    l.preset.float("atmosHenyeyGreensteinParam").or_else(|| l.preset.float("0xceafcae0")).unwrap_or(0.0)
}

fn sky_uniform(l: &GameLighting) -> SkyUniform {
    let [beta_r, beta_dash_r, beta_m, beta_dash_m] = scattering(l);
    let glm = l.f("globalLightingMult");
    let use_light_texture = f32::from(u8::from(l.preset.textures.contains_key("dirLightTexture")));
    let ramp = |name: &str, w: f32| l.v(name).truncate().extend(w);
    SkyUniform {
        beta_r: beta_r.extend(henyey_greenstein(l)),
        beta_dash_r: beta_dash_r.extend(l.f("atmosPlanetRadius")),
        beta_m: beta_m.extend(SKY_DISTANCE_KM),
        beta_dash_m: beta_dash_m.extend(CLOUD_DISTANCE_SCALE),
        heights: Vec4::new(
            recip(l.f("atmosRayleighHeightFalloff")),
            l.f("atmosRayleighHeightOffset"),
            recip(l.f("atmosMieHeightFalloffSky")),
            l.f("atmosMieHeightOffsetSky"),
        ),
        multipliers: Vec4::new(
            l.f("atmosExtinctionMultiplier"),
            l.f("atmosInScatteringMultiplier"),
            glm,
            use_light_texture,
        ),
        light_dir: l.v("dirLight_Direction").truncate().normalize_or_zero().extend(l.f("dirLightTextureUVScale")),
        light_colour: l.v("dirLight_Colour").truncate().extend(l.f("cloudTextureOffset")),
        cloud_ramp0: ramp("cloudRampColor0", l.f("cloudRampIntensity")),
        cloud_ramp1: ramp("cloudRampColor1", l.f("cloudDensityMultiplier")),
        cloud: Vec4::new(
            l.f("cloudDensityRimLevel"),
            recip(l.f("cloudDensityRimSize")),
            l.f("skySunRimIntensity"),
            l.f("skySunRimFalloff"),
        ),
        sun: Vec4::new(l.f("skySunIntensity"), l.f("skySunFalloff"), recip(glm), 0.0),
    }
}

fn load_texture(big_root: &Path, rel: &str) -> Result<Image> {
    let path = ssx_assets::track_objects::asset_file(big_root, &format!("{rel}.xpr"))
        .with_context(|| format!("{rel}.xpr not found under {}", big_root.display()))?;
    let chain = crate::render::terrain::textures::decode_xpr(&path)?;
    Ok(crate::render::terrain::textures::texture_2d(
        &chain,
        crate::render::terrain::textures::Texels::Linear,
        default(),
    ))
}

fn texel(rgba: [u8; 4]) -> Image {
    let size = Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
    Image::new(size, TextureDimension::D2, rgba.to_vec(), TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD)
}

pub struct SkyLoad {
    uniform: SkyUniform,
    images: [Image; 3],
    midground: Option<midground::MidgroundLoad>,
    pub status: String,
}

pub fn load(lighting: &GameLighting, root: &Path) -> Result<SkyLoad> {
    if crate::render::post::env_is("SSX_GAME_SKY", "off") {
        anyhow::bail!("disabled by SSX_GAME_SKY=off");
    }
    let tex = |name: &str| lighting.preset.textures.get(name).cloned();
    let density = tex("cloudDensityTexture").context("preset has no cloudDensityTexture")?;
    let normal = tex("cloudDensityNormalTexture").context("preset has no cloudDensityNormalTexture")?;
    let moon = tex("dirLightTexture");
    let density_img = load_texture(root, &density)?;
    let normal_img = load_texture(root, &normal)?;
    let light_img = match &moon {
        Some(rel) => load_texture(root, rel)?,
        None => texel([0, 0, 0, 0]),
    };
    let uniform = sky_uniform(lighting);
    let name = |rel: &str| rel.rsplit('\\').next().unwrap_or(rel).to_string();
    let midground = midground::load(lighting, root)
        .inspect_err(|e| warn!("mid-ground mountains off: {e:#}"))
        .map_err(|e| format!("mid-ground: off ({e:#})"));
    let midground_status = match &midground {
        Ok(Some(m)) => m.status.clone(),
        Ok(None) => "mid-ground: none placed by the track".to_string(),
        Err(e) => e.clone(),
    };
    let status = format!(
        "sky: sky.fxo port (scattering to {SKY_DISTANCE_KM} km, HG g {}, clouds {} / {}{}, sun disc) | written / gLM; \
         shader atan2/acos polynomials; samplers + depth/stencil from sky.fxo",
        uniform.beta_r.w,
        name(&density),
        name(&normal),
        moon.as_deref().map_or_else(String::new, |m| format!(", moon {}", name(m))),
    ) + "\n"
        + &midground_status;
    let midground = midground.ok().flatten();
    Ok(SkyLoad { uniform, images: [density_img, normal_img, light_img], midground, status })
}

pub fn insert(world: &mut World, load: SkyLoad) -> Result<()> {
    let [density, normal, light] = load.images;
    let mut images = world.get_resource_mut::<Assets<Image>>().context("no image assets")?;
    let settings = SkySettings {
        uniform: load.uniform,
        cloud_density: images.add(density),
        cloud_normal: images.add(normal),
        light_texture: images.add(light),
        midground: load.midground.map(|m| m.into_settings(&mut images)),
    };
    world.insert_resource(settings);
    Ok(())
}
