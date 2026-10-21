use std::path::Path;

use anyhow::{Context, Result};
use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use ssx_assets::postfx_overlay::{OverlayDrop, OverlayParams, SurvivalOverlay, UV_SCALE_16_9, texture_path};

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct OverlayUniform {
    pub tint0: Vec4,
    pub tint1: Vec4,
    pub fadeoffbounds: Vec4,
    pub uvscale: Vec4,
}

impl OverlayUniform {
    pub fn set(&mut self, params: Option<OverlayParams>) {
        match params {
            Some(p) => {
                self.tint0 = Vec4::from_array(p.tint_intensity[0]);
                self.tint1 = Vec4::from_array(p.tint_intensity[1]);
                self.fadeoffbounds = Vec3::from_array(p.fadeoffbounds).extend(1.0);
            }
            None => self.fadeoffbounds.w = 0.0,
        }
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct PostOverlay {
    pub uniform: OverlayUniform,
    pub maps: [Handle<Image>; 2],
}

#[derive(Resource)]
pub struct ScreenEffects {
    pub overlay: SurvivalOverlay,
}

fn blank() -> Image {
    let size = Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
    let mut image =
        Image::new_uninit(size, TextureDimension::D2, TextureFormat::Rgba8Unorm, RenderAssetUsages::RENDER_WORLD);
    image.data = Some(vec![255; 4]);
    image
}

pub(super) fn build(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<PostOverlay>::default());
    reset(app.world_mut());
}

pub(super) fn reset(world: &mut World) {
    let Some(mut images) = world.get_resource_mut::<Assets<Image>>() else { return };
    let maps = [images.add(blank()), images.add(blank())];
    let uniform = OverlayUniform { uvscale: Vec4::from_array(UV_SCALE_16_9), ..default() };
    world.insert_resource(PostOverlay { uniform, maps });
}

fn overlay_image(big_root: &Path, name: &str) -> Result<Image> {
    let rel = texture_path(name);
    let path = ssx_assets::track_objects::asset_file(big_root, &rel).with_context(|| format!("{rel} not found"))?;
    let chain = crate::render::terrain::textures::decode_xpr(&path)?;
    Ok(crate::render::terrain::textures::texture_2d(
        &chain,
        crate::render::terrain::textures::Texels::Linear,
        ImageSampler::Default,
    ))
}

pub struct OverlayLoad {
    drop: OverlayDrop,
    overlay: SurvivalOverlay,
    maps: [Image; 2],
    failed: Vec<String>,
}

pub fn load(big_root: &Path, drop: OverlayDrop, presets: &[String]) -> Result<OverlayLoad> {
    let (overlay, failed) = SurvivalOverlay::load(big_root, drop, presets)?;
    let maps = [overlay_image(big_root, overlay.maps[0])?, overlay_image(big_root, overlay.maps[1])?];
    Ok(OverlayLoad { drop, overlay, maps, failed })
}

pub fn insert(world: &mut World, load: OverlayLoad) -> Result<String> {
    let OverlayLoad { drop, overlay, maps: [a, b], failed } = load;
    let maps = {
        let mut images = world.get_resource_mut::<Assets<Image>>().context("no image assets")?;
        [images.add(a), images.add(b)]
    };
    let mut post = world.get_resource_mut::<PostOverlay>().context("the post chain is off")?;
    post.maps = maps;
    let presets: Vec<String> = overlay
        .presets
        .iter()
        .filter(|(_, (_, st))| st.intensity.iter().any(Option::is_some))
        .map(|(n, (p, _))| format!("{n} (layer priority {p:?})"))
        .collect();
    let line = format!(
        "screen effects: {drop:?}, maps {:?}, overlay presets {}; not loaded (their colour cubes): {failed:?}",
        overlay.maps,
        presets.join(", ")
    );
    world.insert_resource(ScreenEffects { overlay });
    Ok(line)
}
