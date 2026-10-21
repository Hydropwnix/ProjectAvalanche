use std::collections::HashMap;

use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d, uniform_buffer_sized,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::shader::Shader;

use super::super::UiBlend;
use super::{SAMPLES, STENCIL_FORMAT, UI_FORMAT, UiShaders};

#[derive(Resource)]
pub(super) struct UiPipelines {
    pub(super) fill_layout: BindGroupLayoutDescriptor,
    pub(super) paint_layout: BindGroupLayoutDescriptor,
    pub(super) full_layout: BindGroupLayoutDescriptor,
    pub(super) filter_layout: BindGroupLayoutDescriptor,
    pub(super) content: HashMap<UiBlend, CachedRenderPipelineId>,
    pub(super) mask_incr: CachedRenderPipelineId,
    pub(super) mask_decr: CachedRenderPipelineId,
    pub(super) layer: CachedRenderPipelineId,
    pub(super) scene_in: CachedRenderPipelineId,
    pub(super) blur: CachedRenderPipelineId,
    pub(super) compose: CachedRenderPipelineId,
    pub(super) result_out: HashMap<TextureFormat, CachedRenderPipelineId>,
    pub(super) linear: Sampler,
    pub(super) nearest: Sampler,
    pub(super) post_shader: Handle<Shader>,
}

fn component(src: BlendFactor, dst: BlendFactor, operation: BlendOperation) -> BlendComponent {
    BlendComponent { src_factor: src, dst_factor: dst, operation }
}

fn blend_state(b: UiBlend) -> BlendState {
    use BlendFactor::*;
    use BlendOperation::*;
    let keep_alpha = component(Zero, One, Add);
    match b {
        UiBlend::Normal => BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        UiBlend::Add => BlendState { color: component(One, One, Add), alpha: keep_alpha },
        UiBlend::Multiply => BlendState { color: component(Dst, Zero, Add), alpha: keep_alpha },
        UiBlend::Subtract => BlendState { color: component(One, One, ReverseSubtract), alpha: keep_alpha },
        UiBlend::Lighten => BlendState { color: component(One, One, Max), alpha: keep_alpha },
        UiBlend::Darken => BlendState { color: component(One, One, Min), alpha: keep_alpha },
    }
}

fn stencil(compare: CompareFunction, pass_op: StencilOperation, write: bool) -> DepthStencilState {
    let face =
        StencilFaceState { compare, fail_op: StencilOperation::Keep, depth_fail_op: StencilOperation::Keep, pass_op };
    DepthStencilState {
        format: STENCIL_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(CompareFunction::Always),
        stencil: StencilState { front: face, back: face, read_mask: 0xFF, write_mask: if write { 0xFF } else { 0 } },
        bias: DepthBiasState::default(),
    }
}

pub(super) fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    shaders: Res<UiShaders>,
    cache: Res<PipelineCache>,
) {
    let frag = ShaderStages::FRAGMENT;
    let both = ShaderStages::VERTEX_FRAGMENT;
    let float = TextureSampleType::Float { filterable: true };
    let fill_layout = BindGroupLayoutDescriptor::new(
        "ssx_ui_draws",
        &BindGroupLayoutEntries::single(both, storage_buffer_read_only_sized(false, None)),
    );
    let paint_layout = BindGroupLayoutDescriptor::new(
        "ssx_ui_paint",
        &BindGroupLayoutEntries::sequential(frag, (texture_2d(float), sampler(SamplerBindingType::Filtering))),
    );
    let full_layout = paint_layout.clone();
    let filter_layout = BindGroupLayoutDescriptor::new(
        "ssx_ui_filter",
        &BindGroupLayoutEntries::sequential(frag, (uniform_buffer_sized(false, None), texture_2d(float))),
    );
    let vertex = VertexState {
        shader: shaders.fill.clone(),
        shader_defs: Vec::new(),
        entry_point: Some("vs".into()),
        buffers: [(0, VertexFormat::Float32x2, 8), (1, VertexFormat::Float32x2, 8), (2, VertexFormat::Uint32, 4)]
            .map(|(location, format, array_stride)| VertexBufferLayout {
                array_stride,
                step_mode: VertexStepMode::Vertex,
                attributes: vec![VertexAttribute { format, offset: 0, shader_location: location }],
            })
            .to_vec(),
    };
    let multisample = MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false };
    let fill = |label: String,
                entry: &'static str,
                blend: Option<BlendState>,
                writes: ColorWrites,
                depth: Option<DepthStencilState>| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![fill_layout.clone(), paint_layout.clone()],
            vertex: vertex.clone(),
            fragment: Some(FragmentState {
                shader: shaders.fill.clone(),
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format: UI_FORMAT, blend, write_mask: writes })],
                ..default()
            }),
            primitive: PrimitiveState { cull_mode: None, ..default() },
            depth_stencil: depth,
            multisample,
            ..default()
        })
    };
    let content = UiBlend::ALL
        .iter()
        .map(|&b| {
            let depth = Some(stencil(CompareFunction::Equal, StencilOperation::Keep, false));
            (b, fill(format!("ssx_ui_fill_{b:?}"), "fs", Some(blend_state(b)), ColorWrites::ALL, depth))
        })
        .collect();
    let mask = |label: &str, op| {
        let depth = Some(stencil(CompareFunction::Equal, op, true));
        fill(label.into(), "fs_mask", None, ColorWrites::empty(), depth)
    };
    let mask_incr = mask("ssx_ui_mask_incr", StencilOperation::IncrementClamp);
    let mask_decr = mask("ssx_ui_mask_decr", StencilOperation::DecrementClamp);
    let layer =
        fill("ssx_ui_layer".into(), "fs", Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING), ColorWrites::ALL, None);
    let full =
        |label: &'static str, entry: &'static str, layouts: Vec<BindGroupLayoutDescriptor>, samples: u32, format| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some(label.into()),
                layout: layouts,
                vertex: VertexState {
                    shader: shaders.post.clone(),
                    shader_defs: Vec::new(),
                    entry_point: Some("vs_full".into()),
                    buffers: Vec::new(),
                },
                fragment: Some(FragmentState {
                    shader: shaders.post.clone(),
                    entry_point: Some(entry.into()),
                    targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                    ..default()
                }),
                multisample: MultisampleState { count: samples, mask: !0, alpha_to_coverage_enabled: false },
                ..default()
            })
        };
    let scene_in = full("ssx_ui_scene_in", "fs_scene_in", vec![full_layout.clone()], SAMPLES, UI_FORMAT);
    let blur = full("ssx_ui_blur", "fs_blur", vec![full_layout.clone(), filter_layout.clone()], 1, UI_FORMAT);
    let compose = full("ssx_ui_compose", "fs_compose", vec![full_layout.clone(), filter_layout.clone()], 1, UI_FORMAT);
    let sampler_desc = |filter: FilterMode| SamplerDescriptor {
        label: Some("ssx_ui_sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: filter,
        min_filter: filter,
        ..default()
    };
    let result_out = [TextureFormat::Rgba8UnormSrgb, TextureFormat::Rgba16Float]
        .into_iter()
        .map(|f| (f, result_out_pipeline(&cache, &full_layout, &shaders.post, f)))
        .collect();
    commands.insert_resource(UiPipelines {
        fill_layout,
        paint_layout,
        full_layout,
        filter_layout,
        content,
        mask_incr,
        mask_decr,
        layer,
        scene_in,
        blur,
        compose,
        result_out,
        linear: device.create_sampler(&sampler_desc(FilterMode::Linear)),
        nearest: device.create_sampler(&sampler_desc(FilterMode::Nearest)),
        post_shader: shaders.post.clone(),
    });
}

pub(super) fn result_out_pipeline(
    cache: &PipelineCache,
    layout: &BindGroupLayoutDescriptor,
    shader: &Handle<Shader>,
    format: TextureFormat,
) -> CachedRenderPipelineId {
    cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("ssx_ui_result_out".into()),
        layout: vec![layout.clone()],
        vertex: VertexState {
            shader: shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some("vs_full".into()),
            buffers: Vec::new(),
        },
        fragment: Some(FragmentState {
            shader: shader.clone(),
            entry_point: Some("fs_result_out".into()),
            targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    })
}
