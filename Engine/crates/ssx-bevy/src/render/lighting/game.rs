use crate::render::reload::Recipe;
use std::path::Path;

use anyhow::{Result, ensure};
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::{
    App, Assets, Commands, FixedUpdate, Handle, Image, IntoScheduleConfigs, Plugin, Reflect, Res, ResMut, Resource,
    StandardMaterial, Update, Vec3, Vec4, warn,
};
use bevy::render::render_resource::{
    Extent3d, ShaderType, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::shader::load_shader_library;

use crate::render::lighting::GameLighting;

pub const LIGHTING_STATUS: &str = "\
lighting (game formulas): key light prelighting PS3 (ramp/rim/bounce, Phong), material passes, AO = maxAOAttenuation, \
fog per vertex (desaturate + fog_color*F), 8-bit L-buffer\n\
  shadow s = ss_shadow over the game's 3 ortho cascades (5x bilinear PCF, edge fade), MIN the 128² character map; \
SSAO never runs (ao=1); contact shadow ss_contact_shadow;\n  local lights (point/spot/box/projection) multiply into the 8-bit L-buffer where the material binds them; \
output = colour / globalLightingMult (the post chain applies it)";

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct GameLightUniform {
    pub ramp_lit: Vec4,
    pub ramp_shade: Vec4,
    pub ramp_bounce: Vec4,
    pub light_dir: Vec4,
    pub bounce_dir: Vec4,
    pub rim_shadow: Vec4,
    pub specular: Vec4,
    pub fog_color: Vec4,
    pub fog_data: Vec4,
    pub sparkle: Vec4,
    pub misc: Vec4,
}

const FLOAT_FIELDS: [&str; 16] = [
    "rampIntensity",
    "rampRimFalloff",
    "rampRimInLight",
    "rampRimInShadow",
    "rampBounceAtt",
    "rampBounceSpread",
    "maxShadowAttenuation",
    "maxContactShadowAttenuation",
    "maxAOAttenuation",
    "globalLightingMult",
    "snowSparkleTile",
    "snowSparklePower",
    "snowSparkleIntensity",
    "snowSparkleRampMin",
    "snowSparkleRampMax",
    "localLightsIntensity",
];
const VECTOR_FIELDS: [&str; 8] = [
    "rampColor0",
    "rampColor1",
    "rampColor2",
    "dirLight_Direction",
    "dirLight_Colour",
    "globalSpecularColour",
    "fog_color",
    "fog_data",
];

impl GameLightUniform {
    pub fn from_preset(l: &GameLighting) -> Self {
        let (f, v) = (|n: &str| l.f(n), |n: &str| l.v(n));
        let spec_strength = v("dirLight_Direction").w * v("dirLight_Colour").w;
        GameLightUniform {
            ramp_lit: v("rampColor0").truncate().extend(f("rampIntensity")),
            ramp_shade: v("rampColor1").truncate().extend(f("rampRimFalloff")),
            ramp_bounce: v("rampColor2").truncate().extend(f("rampBounceAtt")),
            light_dir: l.sun_offset.extend(spec_strength),
            bounce_dir: l.bounce.normalize_or_zero().extend(f("rampBounceSpread")),
            rim_shadow: Vec4::new(
                f("rampRimInLight"),
                f("rampRimInShadow"),
                f("maxShadowAttenuation"),
                f("maxContactShadowAttenuation"),
            ),
            specular: v("globalSpecularColour").truncate().extend(f("maxAOAttenuation")),
            fog_color: v("fog_color"),
            fog_data: v("fog_data"),
            sparkle: Vec4::new(
                f("snowSparkleTile"),
                f("snowSparklePower"),
                f("snowSparkleIntensity"),
                f("snowSparkleRampMin"),
            ),
            misc: Vec4::new(f("snowSparkleRampMax"), f("globalLightingMult"), f("localLightsIntensity"), 0.0),
        }
    }

    pub fn viewer_default() -> Self {
        GameLightUniform {
            ramp_lit: Vec4::new(1.0, 1.0, 1.0, 1.0),
            ramp_shade: Vec4::new(0.25, 0.25, 0.25, 1.0),
            ramp_bounce: Vec4::ZERO,
            light_dir: Vec3::new(0.35, 0.5, 0.78).normalize().extend(1.0),
            bounce_dir: Vec4::new(0.0, -1.0, 0.0, 0.0),
            rim_shadow: Vec4::new(0.0, 0.0, 0.9, 0.0),
            specular: Vec4::new(1.0, 1.0, 1.0, 1.0),
            fog_color: Vec4::new(0.0, 0.0, 0.0, 1.0),
            fog_data: Vec4::new(0.0, 1.0, 1.0, 0.0),
            sparkle: Vec4::ZERO,
            misc: Vec4::new(1.0, 1.0, 0.0, 0.0),
        }
    }
}

pub fn missing_fields(l: &GameLighting) -> Vec<&'static str> {
    let floats = FLOAT_FIELDS.into_iter().filter(|n| l.preset.float(n).is_none());
    floats.chain(VECTOR_FIELDS.into_iter().filter(|n| l.preset.vector(n).is_none())).collect()
}

#[derive(Resource, Clone, Debug)]
pub struct GameLightAssets {
    pub uniform: GameLightUniform,
    pub reflect_cube: Handle<Image>,
    pub white: Handle<Image>,
    pub flat_normal: Handle<Image>,
    pub neutral_rgb: Handle<Image>,
    pub summary: String,
}

pub const GAME_VERTEX_PATH: &str = "embedded://ssx_bevy/render/lighting/game_vertex.wesl";

pub fn specialize_game_vertex(
    descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
    light_binding: u32,
) {
    descriptor.vertex.shader_defs.push(bevy::shader::ShaderDefVal::UInt("SSX_LIGHT_BINDING".into(), light_binding));
    let binds_local_lights = descriptor
        .layout
        .iter()
        .any(|group| group.entries.iter().any(|e| e.binding == crate::render::lighting::local::LOCAL_LIGHTS_BINDING));
    if binds_local_lights {
        descriptor.vertex.shader_defs.push("SSX_LIGHT_BUFFER".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("SSX_LOCAL_LIGHTS".into());
        }
    }
}

pub struct GameLightingPlugin;

impl Plugin for GameLightingPlugin {
    fn build(&self, app: &mut App) {
        use crate::render::lighting::fade::{self, LightingFadeSet};
        load_shader_library!(app, "game_lighting.wesl");
        bevy::asset::embedded_asset!(app, "game_vertex.wesl");
        app.add_systems(crate::shell::level::LevelPreStartup, setup_light_assets);
        crate::shell::level::scoped::<GameLightAssets>(app);
        app.add_systems(FixedUpdate, fade::read_ride.after(crate::ride::RideStepSet))
            .add_systems(
                Update,
                (fade::read_ride, fade::frame).chain().after(crate::ride::RideStepSet).in_set(LightingFadeSet),
            )
            .add_systems(
                Update,
                (
                    follow_uniform,
                    (
                        follow_light::<crate::render::objects::foreground::ForegroundExtension>,
                        follow_light::<crate::ride::helicopter::HeliTranslucentExtension>,
                    ),
                )
                    .chain()
                    .after(LightingFadeSet)
                    .run_if(fade::faded),
            );
    }
}

pub trait GameLit: bevy::pbr::MaterialExtension {
    fn light_mut(&mut self) -> &mut GameLightUniform;
}

macro_rules! game_lit {
    ($($extension:ty),+) => {
        $(impl GameLit for $extension {
            fn light_mut(&mut self) -> &mut GameLightUniform {
                &mut self.light
            }
        })+
    };
}
game_lit!(crate::render::objects::foreground::ForegroundExtension, crate::ride::helicopter::HeliTranslucentExtension);

fn follow_uniform(lighting: Res<GameLighting>, mut assets: ResMut<GameLightAssets>) {
    assets.uniform = GameLightUniform::from_preset(&lighting);
}

fn follow_light<E: GameLit>(
    assets: Res<GameLightAssets>,
    materials: Option<ResMut<Assets<bevy::pbr::ExtendedMaterial<StandardMaterial, E>>>>,
) {
    let Some(mut materials) = materials else { return };
    for (_, m) in materials.iter_mut() {
        *m.extension.light_mut() = assets.uniform;
    }
}

pub fn refresh_light_assets(world: &mut bevy::prelude::World) {
    let Some(lighting) = world.get_resource::<GameLighting>() else { return };
    let uniform = GameLightUniform::from_preset(lighting);
    let cube = reflection_image(lighting);
    let Some(mut assets) = world.get_resource_mut::<GameLightAssets>() else { return };
    assets.uniform = uniform;
    let handle = assets.reflect_cube.clone();
    if let (Some(cube), Some(mut images)) = (cube, world.get_resource_mut::<Assets<Image>>())
        && let Err(e) = images.insert(handle.id(), cube)
    {
        warn!("reflection cube not replaced: {e}");
    }
}

fn reflection_image(l: &GameLighting) -> Option<Image> {
    if let ssx_assets::content::level::LevelSky::Cubemap { sky, .. } = &l.level.sky
        && sky.def.reflection
        && let Some(c) = &sky.cube
    {
        let (size, mips, faces) = crate::render::sky::package::reflection_faces(c, &sky.def);
        return Some(cube_image(size, mips, faces));
    }
    let (root, id) = l.reflect_cube.as_ref()?;
    decode_cube(root, id).inspect_err(|e| warn!("reflection cube: {e:#}")).ok()
}

fn setup_light_assets(mut commands: Commands, lighting: Option<Res<GameLighting>>, mut images: ResMut<Assets<Image>>) {
    let black_cube = || solid_cube([0, 0, 0, 255]);
    let (uniform, cube, summary) = match lighting.as_deref() {
        Some(l) => {
            let missing = missing_fields(l);
            if !missing.is_empty() {
                warn!("{}: lighting fields missing, read as 0: {missing:?}", l.preset.name);
            }
            let sky_cube = match &l.level.sky {
                ssx_assets::content::level::LevelSky::Cubemap { id, sky, .. } if sky.def.reflection => {
                    sky.cube.as_ref().map(|c| (id.clone(), crate::render::sky::package::reflection_faces(c, &sky.def)))
                }
                _ => None,
            };
            let (cube, cube_note) = match (&l.reflect_cube, sky_cube) {
                (_, Some((id, (size, mips, faces)))) => {
                    ((cube_image(size, mips, faces), None), format!("the sky {id}"))
                }
                (Some((big_root, id)), None) => match decode_cube(big_root, id) {
                    Ok(image) => {
                        let (root, cube_id) = (big_root.clone(), id.clone());
                        let recipe = Recipe::new(vec![id.clone()], move || decode_cube(&root, &cube_id));
                        ((image, Some(recipe)), stem(id))
                    }
                    Err(e) => {
                        warn!("reflection cube: {e:#}");
                        ((black_cube(), None), format!("{} undecodable (no reflection)", stem(id)))
                    }
                },
                (None, None) => ((black_cube(), None), "none (no reflection)".to_string()),
            };
            let unset = if missing.is_empty() { String::new() } else { format!(" | unset (0): {}", missing.join(" ")) };
            (GameLightUniform::from_preset(l), cube, format!("reflectCubeTexture {cube_note}{unset}"))
        }
        None => (
            GameLightUniform::viewer_default(),
            (black_cube(), None),
            "no lighting preset: VIEWER DEFAULT key light, no fog/sparkle/reflection".to_string(),
        ),
    };
    commands.insert_resource(GameLightAssets {
        uniform,
        reflect_cube: crate::render::reload::add(&mut images, cube.0, cube.1),
        white: images.add(solid_2d([255; 4], TextureFormat::Rgba8Unorm)),
        flat_normal: images.add(solid_2d([128, 128, 0, 0], TextureFormat::Rg8Unorm)),
        neutral_rgb: images.add(solid_2d([128, 128, 128, 255], TextureFormat::Rgba8Unorm)),
        summary,
    });
}

fn stem(id: &ssx_content::ContentId) -> String {
    id.path().rsplit('/').next().unwrap_or_default().to_string()
}

fn solid_2d(texel: [u8; 4], format: TextureFormat) -> Image {
    let bytes = format.block_copy_size(None).unwrap_or(4) as usize;
    let size = Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
    let mut image = Image::new_uninit(size, TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(texel[..bytes].to_vec());
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..Default::default()
    });
    image
}

fn solid_cube(texel: [u8; 4]) -> Image {
    cube_image(1, 1, (0..6).map(|_| texel.to_vec()).collect())
}

fn cube_image(size: u32, mips: u32, faces: Vec<Vec<u8>>) -> Image {
    let extent = Extent3d { width: size, height: size, depth_or_array_layers: 6 };
    let mut image =
        Image::new_uninit(extent, TextureDimension::D2, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(faces.concat());
    image.texture_descriptor.mip_level_count = mips;
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..Default::default() });
    let states = crate::render::fx_states::sampler(
        "terrain_default",
        "prelight_material_2BaseSurfaces_Snow",
        "reflectCubeTextureSampler",
    );
    image.sampler = match states.and_then(|s| s.image_sampler()) {
        Ok(sampler) => sampler,
        Err(e) => {
            warn!("reflection cube sampler: {e:#}; clamp + trilinear");
            ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::ClampToEdge,
                address_mode_v: ImageAddressMode::ClampToEdge,
                address_mode_w: ImageAddressMode::ClampToEdge,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                ..Default::default()
            })
        }
    };
    image
}

pub fn decode_cube(big_root: &Path, id: &ssx_content::ContentId) -> Result<Image> {
    let found = ssx_assets::content::GameContent::of(big_root).texture(id, &ssx_content::texture::Want::CUBE)?;
    let levels = found.texture.levels;
    let size = levels.first().map_or(0, |l| l.width);
    ensure!(size > 0, "{id}: no level 0");
    let mips = levels
        .iter()
        .enumerate()
        .take_while(|(i, l)| l.images.len() == 6 && l.width == (size >> i).max(1) && l.height == l.width)
        .count();
    ensure!(mips > 0, "{id}: not a cube with six square faces");
    let faces =
        (0..6).map(|face| levels[..mips].iter().flat_map(|l| l.images[face].iter().copied()).collect()).collect();
    Ok(cube_image(size, mips as u32, faces))
}
