use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use crate::particles::ParticleFrame;
use crate::render::post::GamePostCamera;

mod passes;
mod pipelines;

pub use passes::draw;
use pipelines::{Tech, init_pipelines};

const COLOUR_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
const DEPTH_COPY_FORMAT: TextureFormat = TextureFormat::R32Float;

pub fn build(app: &mut App) {
    embedded_asset!(app, "particles.wesl");
    embedded_asset!(app, "particles_pass.wesl");
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else { return };
    render_app
        .init_resource::<ParticleGpu>()
        .add_systems(RenderStartup, init_pipelines)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources));
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct Globals {
    view_from_world: Mat4,
    clip_from_view: Mat4,
    view_from_clip: Mat4,
    world_from_view: Mat4,
    view_prev: Mat4,
    camera_pos: Vec4,
    ramp0: Vec4,
    ramp1: Vec4,
    atten: Vec4,
    fog_data: Vec4,
    fog_color: Vec4,
    sizes: Vec4,
    game_proj: Vec4,
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct DrawUniform {
    world: Mat4,
    time: Vec4,
    stretch: Vec4,
    color_mult: Vec4,
    color_spread: Vec4,
    ramp: [Vec4; 4],
    ramp_times: Vec4,
    size_ramp: Vec4,
    size_ramp_times: Vec4,
    flags: Vec4,
    size_mult: Vec4,
    pivot: Vec4,
    fade: Vec4,
    feather: Vec4,
    light_pos: Vec4,
    light_col: Vec4,
    light_misc: Vec4,
    anim: Vec4,
    anim2: Vec4,
    modulo: Vec4,
    modulo2: Vec4,
    glare: [Vec4; 4],
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct PassGlobals {
    view_from_clip: Mat4,
    sizes: Vec4,
}

struct Targets {
    pub(super) size: UVec2,
    half: UVec2,
    colour_a: TextureView,
    colour_b: TextureView,
    pub(super) depth: TextureView,
    depth_copy: TextureView,
    depth_full: TextureView,
    dummy: TextureView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bin {
    Tonemapped,
    HalfRes,
    FullRes,
}

struct DrawCmd {
    tech: Tech,
    bin: Bin,
    texture: usize,
    offset: u32,
    vertices: u32,
}

#[derive(Resource, Default)]
struct ParticleGpu {
    textures: Option<(usize, Vec<TextureView>)>,
    targets: Option<Targets>,
    globals: UniformBuffer<Globals>,
    pass_globals: UniformBuffer<PassGlobals>,
    draws: DynamicUniformBuffer<DrawUniform>,
    records: Option<Buffer>,
    cmds: Vec<DrawCmd>,
    serial: u64,
    view_prev: Option<Mat4>,
    unknown: std::collections::HashSet<(String, String)>,
}

fn texture(device: &RenderDevice, label: &'static str, size: UVec2, format: TextureFormat, mips: u32) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn view(t: &Texture) -> TextureView {
    t.create_view(&TextureViewDescriptor::default())
}

fn extra(d: &crate::particles::sim::Draw, name: &str) -> Vec4 {
    let registry = d.registry.as_deref().map_or(&[][..], Vec::as_slice);
    registry.iter().find(|(n, _)| n == name).map_or(Vec4::ZERO, |(_, v)| *v)
}

fn draw_uniform(
    d: &crate::particles::sim::Draw,
    first: u32,
    light: &crate::particles::ParticleLight,
    full_res: bool,
) -> DrawUniform {
    let e = |n: &str| extra(d, n);
    let (light_pos, light_col, radius, falloff) = if d.use_global_lighting {
        (light.position, light.colour, light.radius, light.falloff)
    } else {
        (d.light.0, d.light.1, d.light.2, d.light.3)
    };
    DrawUniform {
        world: d.world,
        time: Vec4::new(d.time_now as f32, d.age_max as f32, e("fTimeSpeed").x, d.age_warp_inv),
        stretch: d.stretch.extend(d.stretch_per_particle as u8 as f32),
        color_mult: d.color_mult,
        color_spread: e("vColorMultSpread"),
        ramp: d.color_ramp,
        ramp_times: e("vColorRampTimes"),
        size_ramp: e("vSizeRamp"),
        size_ramp_times: e("vSizeRampTimes"),
        flags: Vec4::new(e("bUseColorRamp").x, e("bUseColorMult").x, e("bUseSizeRamp").x, e("bUseSizeMult").x),
        size_mult: Vec4::new(e("fSizeMultX").x, e("fSizeMultY").x, e("iAlign").x, e("iScreenSpace").x),
        pivot: e("vPivotShift"),
        fade: Vec4::new(e("fNearFadeStart").x, e("fNearFadeEnd").x, e("fZBias").x, first as f32),
        feather: Vec4::new(d.z_feather[2], d.z_feather[3], e("fContactShadowFalloff").x, e("fContactShadowOffset").x),
        light_pos: light_pos.extend(radius),
        light_col,
        light_misc: Vec4::new(falloff, 0.0, 0.0, 0.0),
        anim: Vec4::new(
            e("bUseAnimTexture").x,
            e("iAnimTextureMode").x,
            e("iAnimTextureCount").x,
            e("iAnimTextureRows").x,
        ),
        anim2: Vec4::new(
            e("fAnimTextureSpeed").x,
            e("iAnimTextureRandomStart").x,
            e("iTile").x,
            d.vertices.len() as f32,
        ),
        modulo: d.dimensions.extend(d.modulo_position as u8 as f32),
        modulo2: Vec4::new(
            e("fCameraPush").x,
            e("iNoFadeFar").x,
            d.stretch_per_camera as u8 as f32,
            full_res as u8 as f32,
        ),
        glare: [
            Vec4::new(
                e("fGlareBloomScale").x,
                e("fGlareBloomSpread").x,
                e("fGlareSensitivityCenter").x,
                e("fGlareSensitivityEdge").x,
            ),
            Vec4::new(
                e("fGlareSensitivityPower").x,
                e("fGlareRotationRate").x,
                e("fGlareDirCenterPower").x,
                e("fGlareDirEdgePower").x,
            ),
            Vec4::new(
                e("fGlareDirOrientation").x,
                e("fGlareSizeMultSpread").x,
                e("fFlareOffsetScale").x,
                e("fFlareEndScale").x,
            ),
            Vec4::new(e("bUseGlareSeed").x, e("fFlareMovementRate").x, 0.0, 0.0),
        ],
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    mut gpu: ResMut<ParticleGpu>,
    frame: Res<ParticleFrame>,
    views: Query<&ExtractedView, With<GamePostCamera>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Ok(v) = views.single() else { return };
    let gpu = &mut *gpu;
    let size = v.viewport.zw();
    if gpu.targets.as_ref().is_none_or(|t| t.size != size) {
        let half = (size + UVec2::ONE) / 2;
        gpu.targets = Some(Targets {
            size,
            half,
            colour_a: view(&texture(&device, "ssx_particles_colour_a", half, COLOUR_FORMAT, 1)),
            colour_b: view(&texture(&device, "ssx_particles_colour_b", half, COLOUR_FORMAT, 1)),
            depth: view(&texture(&device, "ssx_particles_depth", half, DEPTH_FORMAT, 1)),
            depth_copy: view(&texture(&device, "ssx_particles_depth_copy", half, DEPTH_COPY_FORMAT, 1)),
            depth_full: view(&texture(&device, "ssx_particles_depth_full", size, DEPTH_COPY_FORMAT, 1)),
            dummy: view(&texture(&device, "ssx_particles_dummy", UVec2::ONE, DEPTH_COPY_FORMAT, 1)),
        });
    }
    let id = std::sync::Arc::as_ptr(&frame.textures) as usize;
    if gpu.textures.as_ref().is_none_or(|(k, _)| *k != id) {
        let views = frame
            .textures
            .iter()
            .map(|t| {
                let tex = texture(&device, "ssx_particle_texture", t.size, COLOUR_FORMAT, t.levels.len() as u32);
                for (level, data) in t.levels.iter().enumerate() {
                    let (w, h) = ((t.size.x >> level).max(1), (t.size.y >> level).max(1));
                    queue.write_texture(
                        TexelCopyTextureInfo {
                            texture: &tex,
                            mip_level: level as u32,
                            origin: Origin3d::ZERO,
                            aspect: TextureAspect::All,
                        },
                        data,
                        TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * w), rows_per_image: Some(h) },
                        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    );
                }
                view(&tex)
            })
            .collect();
        gpu.textures = Some((id, views));
    }
    let t = gpu.targets.as_ref().unwrap();
    let sizes = Vec4::new(t.half.x as f32, t.half.y as f32, size.x as f32, size.y as f32);
    let world_from_view = v.world_from_view.to_matrix();
    let view_from_world = world_from_view.inverse();
    let view_prev = gpu.view_prev.replace(view_from_world).unwrap_or(view_from_world);
    let (a, b) = (v.clip_from_view.z_axis.z, v.clip_from_view.w_axis.z);
    let far = if a != 0.0 { b / a } else { 8000.0 };
    let near = a * far / (1.0 + a);
    let game_proj = Vec4::new(far / (near - far), near * far / (near - far), 0.0, 0.0);
    let l = &frame.light;
    gpu.globals.set(Globals {
        view_from_world,
        clip_from_view: v.clip_from_view,
        view_from_clip: v.clip_from_view.inverse(),
        world_from_view,
        view_prev,
        camera_pos: v.world_from_view.translation().extend(1.0),
        ramp0: l.ramp_lit,
        ramp1: l.ramp_shade.truncate().extend(0.0),
        atten: Vec4::new(l.rim_shadow.z, l.specular.w, l.misc.z, l.misc.y),
        fog_data: l.fog_data,
        fog_color: l.fog_color,
        sizes,
        game_proj,
    });
    gpu.globals.write_buffer(&device, &queue);
    gpu.pass_globals.set(PassGlobals { view_from_clip: v.clip_from_view.inverse(), sizes });
    gpu.pass_globals.write_buffer(&device, &queue);
    if gpu.serial == frame.serial {
        return;
    }
    gpu.serial = frame.serial;
    gpu.cmds.clear();
    gpu.draws.clear();
    let mut records: Vec<Vec4> = Vec::new();
    for d in frame.draws.iter() {
        let Some(texture) = frame.textures.iter().position(|t| t.name == d.texture) else { continue };
        let Some(tech) = Tech::of(&d.shader, &d.technique) else {
            if gpu.unknown.insert((d.shader.clone(), d.technique.clone())) {
                warn!("particles: no port of {} / {}", d.shader, d.technique);
            }
            continue;
        };
        let bin = if d.tonemap {
            Bin::Tonemapped
        } else if d.half {
            Bin::HalfRes
        } else {
            Bin::FullRes
        };
        if tech.is_ribbon() != (d.mesh_type == 2) {
            if gpu.unknown.insert((d.shader.clone(), format!("iType {}", d.mesh_type))) {
                warn!("particles: {} with mesh builder {} not ported", d.shader, d.mesh_type);
            }
            continue;
        }
        let vertices = match tech {
            t if t.is_ribbon() && d.vertices.len() < 2 => continue,
            t if t.is_ribbon() => d.vertices.len() as u32 * 2,
            _ => d.vertices.len() as u32 * 6,
        };
        let first = records.len() as u32;
        records.extend(d.vertices.iter().flatten());
        let offset = gpu.draws.push(&draw_uniform(d, first, &frame.global_light, bin == Bin::FullRes));
        gpu.cmds.push(DrawCmd { tech, bin, texture, offset, vertices });
    }
    gpu.draws.write_buffer(&device, &queue);
    if records.is_empty() {
        records.push(Vec4::ZERO);
    }
    let bytes: Vec<u8> = records.iter().flat_map(|v| v.to_array()).flat_map(f32::to_le_bytes).collect();
    gpu.records = Some(device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("ssx_particle_records"),
        contents: &bytes,
        usage: BufferUsages::STORAGE,
    }));
}
