use bevy::core_pipeline::core_3d::{Transparent3d, main_transparent_pass_3d};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::image::ImageSamplerDescriptor;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{SortedRenderPhase, ViewSortedRenderPhases};
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, texture_3d, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::storage::GpuShaderBuffer;
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, RetainedViewEntity, ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

use super::adapt::{AdaptMode, HdrAdaptation};
use super::overlay::{OverlayUniform, PostOverlay};
use super::{GamePostCamera, PostSettings, PostUniform};
use crate::render::objects::material::{LDR_PIPELINE_LABEL, LDR_TONEMAP_BUFFER, LDR_TONEMAP_EXPOSURE_OFFSET};

const SHADER_DIR: &str = "embedded://ssx_bevy/render/post/";
const QUARTER_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const BLOOM_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const FRAME_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;
const EXPOSURE_BYTES: u64 = 16;

pub(super) fn build(app: &mut App) {
    app.add_plugins((
        ExtractResourcePlugin::<PostSettings>::default(),
        ExtractComponentPlugin::<GamePostCamera>::default(),
    ));
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_resource::<PostTargets>()
        .init_resource::<HdrAdaptation>()
        .init_resource::<LdrTranslucents>()
        .add_systems(RenderStartup, init_pipelines)
        .add_systems(Render, prepare_targets.in_set(RenderSystems::PrepareResources))
        .add_systems(
            Core3d,
            (
                take_ldr_translucents.in_set(Core3dSystems::MainPass).before(main_transparent_pass_3d),
                post_chain.in_set(Core3dSystems::PostProcess).before(tonemapping),
                restore_ldr_translucents.in_set(Core3dSystems::PostProcess).after(post_chain),
            ),
        );
}

#[derive(Resource)]
struct PostPipelines {
    _common: Handle<Shader>,
    sampler: Sampler,
    overlay_sampler: Sampler,
    fxaa_sampler: Sampler,
    downsample_layout: BindGroupLayoutDescriptor,
    fxaa_layout: BindGroupLayoutDescriptor,
    adapt_layout: BindGroupLayoutDescriptor,
    bloom_layout: BindGroupLayoutDescriptor,
    final_layout: BindGroupLayoutDescriptor,
    downsample: CachedRenderPipelineId,
    threshold: CachedRenderPipelineId,
    blur_h: CachedRenderPipelineId,
    blur_v: CachedRenderPipelineId,
    tonemap: CachedRenderPipelineId,
    fxaa: CachedRenderPipelineId,
    composite: CachedRenderPipelineId,
    adapt: CachedComputePipelineId,
    reduce: CachedComputePipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let shader = |name: &str| assets.load::<Shader>(format!("{SHADER_DIR}{name}.wesl"));
    let float = TextureSampleType::Float { filterable: true };
    let frag = ShaderStages::FRAGMENT;
    let downsample_layout = BindGroupLayoutDescriptor::new(
        "ssx_post_downsample",
        &BindGroupLayoutEntries::sequential(frag, (uniform_buffer::<PostUniform>(false), texture_2d(float))),
    );
    let adapt_layout = BindGroupLayoutDescriptor::new(
        "ssx_post_adapt",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<PostUniform>(false),
                texture_2d(float),
                storage_buffer_sized(false, None),
                storage_buffer_sized(false, None),
            ),
        ),
    );
    let bloom_layout = BindGroupLayoutDescriptor::new(
        "ssx_post_bloom",
        &BindGroupLayoutEntries::sequential(
            frag,
            (
                uniform_buffer::<PostUniform>(false),
                texture_2d(float),
                sampler(SamplerBindingType::Filtering),
                storage_buffer_read_only_sized(false, None),
            ),
        ),
    );
    let final_layout = BindGroupLayoutDescriptor::new(
        "ssx_post_final",
        &BindGroupLayoutEntries::sequential(
            frag,
            (
                uniform_buffer::<PostUniform>(false),
                texture_2d(float),
                texture_2d(float),
                sampler(SamplerBindingType::Filtering),
                texture_3d(float),
                storage_buffer_read_only_sized(false, None),
                uniform_buffer::<OverlayUniform>(false),
                texture_2d(float),
                texture_2d(float),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let fxaa_layout = BindGroupLayoutDescriptor::new(
        "ssx_post_fxaa",
        &BindGroupLayoutEntries::sequential(
            frag,
            (uniform_buffer::<PostUniform>(false), texture_2d(float), sampler(SamplerBindingType::Filtering)),
        ),
    );
    let render = |label: &'static str, layout: &BindGroupLayoutDescriptor, file: &str, entry: &'static str, format| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader(file),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        })
    };
    let compute = |label: &'static str, entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(label.into()),
            layout: vec![adapt_layout.clone()],
            shader: shader("post_adapt"),
            entry_point: Some(entry.into()),
            zero_initialize_workgroup_memory: true,
            ..default()
        })
    };
    let pipelines = PostPipelines {
        _common: shader("post_common"),
        sampler: device.create_sampler(&post_sampler()),
        overlay_sampler: device.create_sampler(&overlay_sampler()),
        fxaa_sampler: device.create_sampler(&fxaa_sampler()),
        downsample: render("ssx_post_downsample", &downsample_layout, "post_downsample", "downsample", QUARTER_FORMAT),
        threshold: render("ssx_post_threshold", &bloom_layout, "post_bloom", "threshold", BLOOM_FORMAT),
        blur_h: render("ssx_post_blur_h", &bloom_layout, "post_bloom", "blur_h", BLOOM_FORMAT),
        blur_v: render("ssx_post_blur_v", &bloom_layout, "post_bloom", "blur_v", BLOOM_FORMAT),
        tonemap: render("ssx_post_tonemap", &final_layout, "post_final", "tonemap_pass", FRAME_FORMAT),
        fxaa: render("ssx_post_fxaa", &fxaa_layout, "post_fxaa", "fxaa_pass", FRAME_FORMAT),
        composite: render("ssx_post_composite", &final_layout, "post_final", "composite_pass", HDR_FORMAT),
        adapt: compute("ssx_post_adapt", "adapt"),
        reduce: compute("ssx_post_reduce", "reduce"),
        downsample_layout,
        adapt_layout,
        bloom_layout,
        final_layout,
        fxaa_layout,
    };
    commands.insert_resource(pipelines);
}

struct ViewTargets {
    size: UVec2,
    quarter: TextureView,
    bloom_a: TextureView,
    bloom_b: TextureView,
    frame: TextureView,
    frame_aa: TextureView,
    adapted: Buffer,
    exposure: Buffer,
    uniform: UniformBuffer<PostUniform>,
    overlay: UniformBuffer<OverlayUniform>,
    fresh: bool,
    complete: bool,
}

#[derive(Resource, Default)]
struct PostTargets(HashMap<Entity, ViewTargets>);

fn quarter_size(size: UVec2) -> UVec2 {
    (size + UVec2::splat(3)) / 4
}

fn create_targets(device: &RenderDevice, size: UVec2) -> ViewTargets {
    let q = quarter_size(size);
    let sized = |label: &'static str, format: TextureFormat, q: UVec2| {
        device
            .create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d { width: q.x, height: q.y, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&TextureViewDescriptor::default())
    };
    let texture = |label: &'static str, format: TextureFormat| sized(label, format, q);
    let buffer = |label: &'static str, size: u64| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };
    ViewTargets {
        size,
        quarter: texture("ssx_post_quarter", QUARTER_FORMAT),
        bloom_a: texture("ssx_post_bloom_a", BLOOM_FORMAT),
        bloom_b: texture("ssx_post_bloom_b", BLOOM_FORMAT),
        frame: sized("ssx_post_frame", FRAME_FORMAT, size),
        frame_aa: sized("ssx_post_frame_aa", FRAME_FORMAT, size),
        adapted: buffer("ssx_post_adapted", (u64::from(q.x * q.y) * 2 + 64) * 4),
        exposure: buffer("ssx_post_exposure", EXPOSURE_BYTES),
        uniform: UniformBuffer::default(),
        overlay: UniformBuffer::default(),
        fresh: true,
        complete: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_targets(
    views: Query<(Entity, &ViewTarget), With<GamePostCamera>>,
    settings: Option<Res<PostSettings>>,
    overlay: Option<Res<PostOverlay>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    shader_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mut targets: ResMut<PostTargets>,
    mut adapt: ResMut<HdrAdaptation>,
) {
    let Some(settings) = settings else {
        return;
    };
    if let Some(ldr) = shader_buffers.get(&LDR_TONEMAP_BUFFER) {
        let u = &settings.uniform;
        let words: Vec<f32> =
            [u.hable_a.to_array(), u.hable_b.to_array()].concat().into_iter().chain([u.exposure.x]).collect();
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        queue.write_buffer(&ldr.buffer, 0, &bytes);
    }
    let loading = cache.waiting_pipelines().next().is_some() || !settings.scene_ready;
    targets.0.retain(|e, _| views.contains(*e));
    for (entity, view) in &views {
        let s = view.main_texture().size();
        let size = UVec2::new(s.width, s.height);
        if targets.0.get(&entity).is_none_or(|t| t.size != size) {
            targets.0.insert(entity, create_targets(&device, size));
        }
        let Some(t) = targets.0.get_mut(&entity) else { continue };
        let q = quarter_size(size);
        let mut u = settings.uniform;
        u.sizes = UVec4::new(size.x, size.y, q.x, q.y);
        let base = adapt.begin(&device, &queue, q, &t.adapted, t.fresh);
        // TODO(guess): the port's view counts as "sky not ready" until it first draws a complete scene (no pipeline compiling, meshes spawned): the adaptation runs toward black (L = 1) as main.scrape's `ifnot skyrenderobj ready` draws black, later it adapts every frame (until 2026-10-21 the port cleared it on every frame with a pipeline compiling: a bright second mid-ride); settle: the sky render object's ready test and how long a level's loading frames run the scrape.
        let starting = loading && !t.complete;
        t.complete |= !loading;
        let mode = if settings.converge && (t.fresh || starting || u.exposure.w == 0.0) {
            AdaptMode::Current
        } else if starting && base == AdaptMode::Adapt {
            AdaptMode::Black
        } else {
            base
        };
        adapt.ran();
        u.bloom.w = mode.code();
        t.fresh = false;
        t.uniform.set(u);
        t.uniform.write_buffer(&device, &queue);
        t.overlay.set(overlay.as_ref().map_or_else(OverlayUniform::default, |o| o.uniform));
        t.overlay.write_buffer(&device, &queue);
    }
}

fn fullscreen_pass(
    ctx: &mut RenderContext,
    label: &str,
    target: &TextureView,
    pipeline: &RenderPipeline,
    bind: &BindGroup,
) {
    let mut pass = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Clear(default()), store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..3, 0..1);
}

#[allow(clippy::too_many_arguments)]
fn post_chain(
    world: &World,
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture), With<GamePostCamera>>,
    pipelines: Option<Res<PostPipelines>>,
    targets: Res<PostTargets>,
    settings: Option<Res<PostSettings>>,
    overlay: Option<Res<PostOverlay>>,
    images: Res<RenderAssets<GpuImage>>,
    shader_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    ldr: Res<LdrTranslucents>,
    cache: Res<PipelineCache>,
    session: Res<HdrAdaptation>,
    mut ctx: RenderContext,
) {
    let (Some(p), Some(settings)) = (pipelines, settings) else {
        return;
    };
    let view_entity = view.entity();
    let Some(t) = targets.0.get(&view_entity) else {
        return;
    };
    let (target, depth) = view.into_inner();
    if target.main_texture_format() != HDR_FORMAT {
        return;
    }
    let render = |id| cache.get_render_pipeline(id);
    let (Some(downsample), Some(threshold), Some(blur_h), Some(blur_v)) =
        (render(p.downsample), render(p.threshold), render(p.blur_h), render(p.blur_v))
    else {
        return;
    };
    let (Some(tonemap), Some(fxaa), Some(composite)) = (render(p.tonemap), render(p.fxaa), render(p.composite)) else {
        return;
    };
    let (Some(adapt), Some(reduce)) = (cache.get_compute_pipeline(p.adapt), cache.get_compute_pipeline(p.reduce))
    else {
        return;
    };
    let Some(lut) = images.get(&settings.lut) else {
        return;
    };
    let Some(maps) = overlay.and_then(|o| Some([images.get(&o.maps[0])?, images.get(&o.maps[1])?])) else {
        return;
    };
    let device = ctx.render_device().clone();
    let layout = |l: &BindGroupLayoutDescriptor| cache.get_bind_group_layout(l);
    let post = target.post_process_write();
    let bind = |label: &'static str, l: &BindGroupLayoutDescriptor, src: &TextureView| {
        device.create_bind_group(
            label,
            &layout(l),
            &BindGroupEntries::sequential((&t.uniform, src, &p.sampler, t.exposure.as_entire_binding())),
        )
    };

    let down_bg = device.create_bind_group(
        "ssx_post_downsample",
        &layout(&p.downsample_layout),
        &BindGroupEntries::sequential((&t.uniform, post.source)),
    );
    fullscreen_pass(&mut ctx, "ssx_post_downsample", &t.quarter, downsample, &down_bg);

    let adapt_bg = device.create_bind_group(
        "ssx_post_adapt",
        &layout(&p.adapt_layout),
        &BindGroupEntries::sequential((
            &t.uniform,
            &t.quarter,
            t.adapted.as_entire_binding(),
            t.exposure.as_entire_binding(),
        )),
    );
    {
        let q = quarter_size(t.size);
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor { label: Some("ssx_post_exposure"), timestamp_writes: None });
        pass.set_bind_group(0, &adapt_bg, &[]);
        pass.set_pipeline(adapt);
        pass.dispatch_workgroups(q.x.div_ceil(8), q.y.div_ceil(8), 1);
        pass.set_pipeline(reduce);
        pass.dispatch_workgroups(1, 1, 1);
    }
    session.store(ctx.command_encoder(), quarter_size(t.size), &t.adapted);

    if settings.uniform.bloom.z != 0.0 {
        fullscreen_pass(
            &mut ctx,
            "ssx_post_threshold",
            &t.bloom_a,
            threshold,
            &bind("ssx_post_threshold", &p.bloom_layout, &t.quarter),
        );
        fullscreen_pass(
            &mut ctx,
            "ssx_post_blur_h",
            &t.bloom_b,
            blur_h,
            &bind("ssx_post_blur_h", &p.bloom_layout, &t.bloom_a),
        );
        fullscreen_pass(
            &mut ctx,
            "ssx_post_blur_v",
            &t.bloom_a,
            blur_v,
            &bind("ssx_post_blur_v", &p.bloom_layout, &t.bloom_b),
        );
    }

    let final_bg = |label: &'static str, src: &TextureView| {
        device.create_bind_group(
            label,
            &layout(&p.final_layout),
            &BindGroupEntries::sequential((
                &t.uniform,
                src,
                &t.bloom_a,
                &p.sampler,
                &lut.texture_view,
                t.exposure.as_entire_binding(),
                &t.overlay,
                &maps[0].texture_view,
                &maps[1].texture_view,
                &p.overlay_sampler,
            )),
        )
    };
    fullscreen_pass(&mut ctx, "ssx_post_tonemap", &t.frame, tonemap, &final_bg("ssx_post_tonemap", post.source));
    if let Some(tonemap_buffer) = shader_buffers.get(&LDR_TONEMAP_BUFFER) {
        draw_ldr_translucents(world, &mut ctx, &ldr, view_entity, depth, &t.frame, &t.exposure, &tonemap_buffer.buffer);
    }
    if let Some(post_uniform) = t.uniform.binding() {
        let bloom = (settings.uniform.bloom.z != 0.0).then_some(&t.bloom_a);
        crate::particles::render::draw(world, &mut ctx, depth, &t.frame, bloom, post_uniform, &t.exposure);
    }
    let fxaa_bg = device.create_bind_group(
        "ssx_post_fxaa",
        &layout(&p.fxaa_layout),
        &BindGroupEntries::sequential((&t.uniform, &t.frame, &p.fxaa_sampler)),
    );
    fullscreen_pass(&mut ctx, "ssx_post_fxaa", &t.frame_aa, fxaa, &fxaa_bg);
    let composite_bg = final_bg("ssx_post_composite", &t.frame_aa);
    fullscreen_pass(&mut ctx, "ssx_post_composite", post.destination, composite, &composite_bg);
}

fn fxaa_sampler() -> SamplerDescriptor<'static> {
    match crate::render::fx_states::sampler("fxaa", "Default", "FrameBufferSampler").and_then(|s| s.descriptor()) {
        Ok(d) => SamplerDescriptor { label: Some("ssx_post_fxaa"), ..d.as_wgpu() },
        Err(e) => {
            warn!("fxaa.fxo sampler states unavailable ({e:#}); clamp + linear");
            SamplerDescriptor {
                label: Some("ssx_post_fxaa"),
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..default()
            }
        }
    }
}

fn overlay_sampler() -> SamplerDescriptor<'static> {
    let technique = "BASE_BLOOM_COLOURLOOKUP_OVERLAY";
    match crate::render::fx_states::sampler("postfx", technique, "overlay_map_sampler0").and_then(|s| s.descriptor()) {
        Ok(d) => SamplerDescriptor { label: Some("ssx_post_overlay"), ..d.as_wgpu() },
        Err(e) => {
            warn!("postfx.fxo overlay sampler states unavailable ({e:#}); clamp + linear");
            SamplerDescriptor {
                label: Some("ssx_post_overlay"),
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                lod_max_clamp: 0.0,
                ..default()
            }
        }
    }
}

fn post_sampler() -> SamplerDescriptor<'static> {
    let technique = "BASE_BLOOM_COLOURLOOKUP";
    let states = ["bloom_glowmap_sampler", "colourLookupSampler1"]
        .map(|name| crate::render::fx_states::sampler("postfx", technique, name).and_then(|s| s.descriptor()));
    let fallback = SamplerDescriptor {
        label: Some("ssx_post_linear_clamp"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        lod_max_clamp: 0.0,
        ..default()
    };
    match states {
        [Ok(a), Ok(b)] if ImageSamplerDescriptor { address_mode_w: b.address_mode_w, ..a.clone() } == b => {
            SamplerDescriptor { label: Some("ssx_post_linear_clamp"), ..b.as_wgpu() }
        }
        [a, b] => {
            warn!("postfx.fxo sampler states unavailable or different ({:?} / {:?}); clamp + linear", a.err(), b.err());
            fallback
        }
    }
}

#[derive(Resource, Default)]
struct LdrTranslucents {
    view: Option<RetainedViewEntity>,
    phase: SortedRenderPhase<Transparent3d>,
}

fn is_ldr_pipeline(cache: &PipelineCache, id: CachedRenderPipelineId) -> bool {
    if matches!(cache.get_render_pipeline_state(id), CachedPipelineState::Queued) {
        return false;
    }
    cache.get_render_pipeline_descriptor(id).label.as_deref() == Some(LDR_PIPELINE_LABEL)
}

fn take_ldr_translucents(
    view: ViewQuery<&ExtractedView, With<GamePostCamera>>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    cache: Res<PipelineCache>,
    mut ldr: ResMut<LdrTranslucents>,
) {
    let retained = view.into_inner().retained_view_entity;
    let Some(phase) = phases.get_mut(&retained) else { return };
    let keys: Vec<_> =
        phase.items.iter().filter(|(_, item)| is_ldr_pipeline(&cache, item.pipeline)).map(|(k, _)| *k).collect();
    for key in keys {
        if let Some(item) = phase.items.shift_remove(&key) {
            ldr.phase.items.insert(key, item);
        }
    }
    ldr.view = Some(retained);
}

fn restore_ldr_translucents(
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    mut ldr: ResMut<LdrTranslucents>,
) {
    let Some(retained) = ldr.view.take() else { return };
    let items = std::mem::take(&mut ldr.phase.items);
    if let Some(phase) = phases.get_mut(&retained) {
        phase.items.extend(items);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ldr_translucents(
    world: &World,
    ctx: &mut RenderContext,
    ldr: &LdrTranslucents,
    view_entity: Entity,
    depth: &ViewDepthStencilTexture,
    frame: &TextureView,
    exposure: &Buffer,
    tonemap: &Buffer,
) {
    if ldr.phase.items.is_empty() {
        return;
    }
    ctx.command_encoder().copy_buffer_to_buffer(exposure, 8, tonemap, LDR_TONEMAP_EXPOSURE_OFFSET, 4);
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("ssx_translucent_low_lod_ldr"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: frame,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Err(e) = ldr.phase.render(&mut pass, world, view_entity) {
        error!("translucent_low_lod LDR pass: {e:?}");
    }
}
