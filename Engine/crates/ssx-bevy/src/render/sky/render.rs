use bevy::core_pipeline::Core3d;
use bevy::core_pipeline::core_3d::{CORE_3D_DEPTH_FORMAT, main_opaque_pass_3d, main_transparent_pass_3d};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{
    ExtractedView, Msaa, ViewDepthTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{GpuResourceAppExt, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;

use super::midground::gpu as midground;
use super::{SkySettings, SkyUniform};

const SHADER: &str = "embedded://ssx_bevy/render/sky/sky.wgsl";

pub(super) fn build(app: &mut App) {
    app.add_plugins(ExtractResourcePlugin::<SkySettings>::default());
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    render_app
        .init_gpu_resource::<SpecializedRenderPipelines<SkyPipeline>>()
        .init_resource::<SkyBuffer>()
        .init_gpu_resource::<SpecializedRenderPipelines<midground::MidgroundPipeline>>()
        .init_resource::<midground::MidgroundBuffers>()
        .add_systems(RenderStartup, (init_pipeline, midground::init_pipeline))
        .add_systems(
            Render,
            (
                prepare_pipelines.in_set(RenderSystems::Prepare),
                prepare_bind_group.in_set(RenderSystems::PrepareBindGroups),
                midground::prepare.in_set(RenderSystems::PrepareBindGroups),
            ),
        )
        .add_systems(
            Core3d,
            (
                draw_sky.after(main_opaque_pass_3d).before(main_transparent_pass_3d),
                midground::draw.after(draw_sky).before(main_transparent_pass_3d),
            ),
        );
}

#[derive(Resource)]
struct SkyPipeline {
    layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    repeat: Sampler,
    clamp: Sampler,
    depth: (CompareFunction, bool),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SkyKey {
    format: TextureFormat,
    samples: u32,
}

impl SpecializedRenderPipeline for SkyPipeline {
    type Key = SkyKey;

    fn specialize(&self, key: SkyKey) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("ssx_sky".into()),
            layout: vec![self.layout.clone()],
            vertex: VertexState { shader: self.shader.clone(), entry_point: Some("vertex".into()), ..default() },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(self.depth.1),
                depth_compare: Some(self.depth.0),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState { count: key.samples, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState { format: key.format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

fn init_pipeline(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>) {
    let float = TextureSampleType::Float { filterable: true };
    let layout = BindGroupLayoutDescriptor::new(
        "ssx_sky",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<ViewUniform>(true).visibility(ShaderStages::VERTEX_FRAGMENT),
                uniform_buffer::<SkyUniform>(false),
                texture_2d(float),
                texture_2d(float),
                texture_2d(float),
                sampler(SamplerBindingType::Filtering),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let sampler = |name: &str, label: &'static str| -> Option<Sampler> {
        let states = crate::render::fx_states::sampler("sky", "mainTechnique", name).and_then(|s| s.descriptor());
        match states {
            Ok(d) => Some(device.create_sampler(&SamplerDescriptor { label: Some(label), ..d.as_wgpu() })),
            Err(e) => {
                warn!("game sky off: {e:#}");
                None
            }
        }
    };
    let (Some(cloud), Some(light)) =
        (sampler("cloudDensityTextureSampler", "ssx_sky_cloud"), sampler("dirLightTextureSampler", "ssx_sky_light"))
    else {
        return;
    };
    let depth = match crate::render::fx_states::depth_test("sky", "mainTechnique") {
        Ok(d) => d,
        Err(e) => {
            warn!("game sky off: {e:#}");
            return;
        }
    };
    commands.insert_resource(SkyPipeline { layout, shader: assets.load(SHADER), repeat: cloud, clamp: light, depth });
}

#[derive(Component)]
struct SkyPipelineId(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<SkyPipeline>>,
    pipeline: Option<Res<SkyPipeline>>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<ViewTarget>>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    for (entity, view, msaa) in &views {
        let key = SkyKey { format: view.target_format, samples: msaa.samples() };
        let id = pipelines.specialize(&cache, &pipeline, key);
        commands.entity(entity).insert(SkyPipelineId(id));
    }
}

#[derive(Resource, Default)]
struct SkyBuffer {
    uniform: UniformBuffer<SkyUniform>,
    bind_group: Option<BindGroup>,
}

#[allow(clippy::too_many_arguments)]
fn prepare_bind_group(
    settings: Option<Res<SkySettings>>,
    pipeline: Option<Res<SkyPipeline>>,
    view_uniforms: Res<ViewUniforms>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    mut buffer: ResMut<SkyBuffer>,
) {
    buffer.bind_group = None;
    let (Some(settings), Some(pipeline), Some(view)) = (settings, pipeline, view_uniforms.uniforms.binding()) else {
        return;
    };
    let (Some(density), Some(normal), Some(light)) =
        (images.get(&settings.cloud_density), images.get(&settings.cloud_normal), images.get(&settings.light_texture))
    else {
        return;
    };
    buffer.uniform.set(settings.uniform);
    buffer.uniform.write_buffer(&device, &queue);
    let bind_group = device.create_bind_group(
        "ssx_sky",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            view,
            &buffer.uniform,
            &density.texture_view,
            &normal.texture_view,
            &light.texture_view,
            &pipeline.repeat,
            &pipeline.clamp,
        )),
    );
    buffer.bind_group = Some(bind_group);
}

fn draw_sky(
    view: ViewQuery<(&ViewTarget, &ViewDepthTexture, &ViewUniformOffset, &SkyPipelineId)>,
    buffer: Res<SkyBuffer>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, depth, offset, pipeline_id) = view.into_inner();
    let (Some(bind_group), Some(pipeline)) = (&buffer.bind_group, cache.get_render_pipeline(pipeline_id.0)) else {
        return;
    };
    let color = [Some(target.get_color_attachment())];
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("ssx_sky"),
        color_attachments: &color,
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[offset.offset]);
    pass.draw(0..3, 0..1);
}
