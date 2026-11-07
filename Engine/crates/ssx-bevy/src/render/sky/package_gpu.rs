use std::sync::Arc;

use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::ecs::system::SystemParam;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_cube, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, Msaa, ViewTarget, ViewUniform, ViewUniforms};
use bevy::shader::Shader;

use super::SkySettings;
use super::package::{Background, CubeSkyUniform, DomeMesh, DomeUniform};

const CUBE_SHADER: &str = "embedded://ssx_bevy/render/sky/sky_cube.wesl";
const DOME_SHADER: &str = "embedded://ssx_bevy/render/sky/sky_dome.wesl";

#[derive(Resource)]
pub struct PackagePipelines {
    cube_layout: BindGroupLayoutDescriptor,
    dome_layout: BindGroupLayoutDescriptor,
    cube_shader: Handle<Shader>,
    dome_shader: Handle<Shader>,
    sampler: Sampler,
    depth: (CompareFunction, bool),
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PackageKey {
    format: TextureFormat,
    samples: u32,
    dome: bool,
    hook: Option<(Handle<Shader>, bool, bool)>,
}

impl SpecializedRenderPipeline for PackagePipelines {
    type Key = PackageKey;

    fn specialize(&self, key: PackageKey) -> RenderPipelineDescriptor {
        let (layout, mut shader, vertex_buffers) = if key.dome {
            let stream = |format: VertexFormat, location: u32| VertexBufferLayout {
                array_stride: format.size(),
                step_mode: VertexStepMode::Vertex,
                attributes: vec![VertexAttribute { format, offset: 0, shader_location: location }],
            };
            let buffers = vec![
                stream(VertexFormat::Float32x3, 0),
                stream(VertexFormat::Float32x3, 1),
                stream(VertexFormat::Float32x2, 2),
            ];
            (self.dome_layout.clone(), self.dome_shader.clone(), buffers)
        } else {
            (self.cube_layout.clone(), self.cube_shader.clone(), Vec::new())
        };
        let mut defs: Vec<bevy::shader::ShaderDefVal> = Vec::new();
        if let Some((variant, surface, vertex)) = &key.hook {
            shader = variant.clone();
            if *surface {
                defs.push("SSX_HOOK_SURFACE".into());
            }
            if *vertex {
                defs.push("SSX_HOOK_VERTEX".into());
            }
        }
        RenderPipelineDescriptor {
            label: Some(if key.dome { "ssx_sky_dome".into() } else { "ssx_sky_cube".into() }),
            layout: vec![layout],
            vertex: VertexState {
                shader: shader.clone(),
                entry_point: Some("vertex".into()),
                shader_defs: defs.clone(),
                buffers: vertex_buffers,
                ..default()
            },
            primitive: PrimitiveState { cull_mode: None, ..default() },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(self.depth.1),
                depth_compare: Some(self.depth.0),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState { count: key.samples, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(FragmentState {
                shader,
                entry_point: Some("fragment".into()),
                shader_defs: defs,
                targets: vec![Some(ColorTargetState { format: key.format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        }
    }
}

pub(super) fn init(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>) {
    let float = TextureSampleType::Float { filterable: true };
    let cube_layout = BindGroupLayoutDescriptor::new(
        "ssx_sky_cube",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer::<ViewUniform>(true).visibility(ShaderStages::VERTEX_FRAGMENT),
                uniform_buffer::<CubeSkyUniform>(false),
                texture_cube(float),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let dome_layout = BindGroupLayoutDescriptor::new(
        "ssx_sky_dome",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer::<DomeUniform>(false),
                texture_2d(float),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let depth = match crate::render::fx_states::depth_test("sky", "mainTechnique") {
        Ok(d) => d,
        Err(e) => {
            warn!("package skies off: {e:#}");
            return;
        }
    };
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("ssx_sky_package"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    commands.insert_resource(PackagePipelines {
        cube_layout,
        dome_layout,
        cube_shader: assets.load(CUBE_SHADER),
        dome_shader: assets.load(DOME_SHADER),
        sampler,
        depth,
    });
}

#[derive(Resource, Default)]
pub struct DomeBuffers {
    mesh: Option<(Arc<DomeMesh>, [Buffer; 3], Buffer, u32)>,
}

#[derive(Component)]
pub struct ViewPackageSky {
    pub(super) pipeline: CachedRenderPipelineId,
    pub(super) bind_group: BindGroup,
    pub(super) dome: bool,
}

fn bytes<T: Copy>(v: &[T], f: impl Fn(&T) -> Vec<u8>) -> Vec<u8> {
    v.iter().flat_map(f).collect()
}

#[derive(SystemParam)]
pub(super) struct Gpu<'w> {
    cache: Res<'w, PipelineCache>,
    images: Res<'w, RenderAssets<GpuImage>>,
    device: Res<'w, RenderDevice>,
    queue: Res<'w, RenderQueue>,
    view_uniforms: Res<'w, ViewUniforms>,
}

pub(super) fn prepare(
    mut commands: Commands,
    settings: Option<Res<SkySettings>>,
    pipeline: Option<Res<PackagePipelines>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PackagePipelines>>,
    mut buffers: ResMut<DomeBuffers>,
    gpu: Gpu,
    views: Query<(Entity, &ExtractedView, &Msaa), With<ViewTarget>>,
) {
    let Gpu { cache, images, device, queue, view_uniforms } = gpu;
    let package = settings.as_ref().and_then(|s| s.package.as_ref());
    let (Some(package), Some(pipeline), Some(view)) = (package, pipeline, view_uniforms.uniforms.binding()) else {
        buffers.mesh = None;
        for (entity, ..) in &views {
            commands.entity(entity).remove::<ViewPackageSky>();
        }
        return;
    };
    let (key_dome, hook, bind_group) = match &package.background {
        Background::Cube { cube, uniform } => {
            let Some(gpu) = images.get(cube) else { return };
            let mut u = UniformBuffer::from(*uniform);
            u.write_buffer(&device, &queue);
            let bg = device.create_bind_group(
                "ssx_sky_cube",
                &cache.get_bind_group_layout(&pipeline.cube_layout),
                &BindGroupEntries::sequential((
                    view.clone(),
                    u.binding().expect("written"),
                    &gpu.texture_view,
                    &pipeline.sampler,
                )),
            );
            (false, None, bg)
        }
        Background::Dome { mesh, texture, uniform, hook } => {
            let Some(gpu) = images.get(texture) else { return };
            if buffers.mesh.as_ref().is_none_or(|(m, ..)| !Arc::ptr_eq(m, mesh)) {
                let vertex = |label: &'static str, contents: Vec<u8>| {
                    device.create_buffer_with_data(&BufferInitDescriptor {
                        label: Some(label),
                        contents: &contents,
                        usage: BufferUsages::VERTEX,
                    })
                };
                let streams = [
                    vertex(
                        "ssx_sky_dome_positions",
                        bytes(&mesh.positions, |p| p.iter().flat_map(|f| f.to_le_bytes()).collect()),
                    ),
                    vertex(
                        "ssx_sky_dome_normals",
                        bytes(&mesh.normals, |p| p.iter().flat_map(|f| f.to_le_bytes()).collect()),
                    ),
                    vertex("ssx_sky_dome_uvs", bytes(&mesh.uvs, |p| p.iter().flat_map(|f| f.to_le_bytes()).collect())),
                ];
                let indices = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("ssx_sky_dome_indices"),
                    contents: &bytes(&mesh.indices, |i| i.to_le_bytes().to_vec()),
                    usage: BufferUsages::INDEX,
                });
                buffers.mesh = Some((mesh.clone(), streams, indices, mesh.indices.len() as u32));
            }
            let mut u = UniformBuffer::from(*uniform);
            u.write_buffer(&device, &queue);
            let bg = device.create_bind_group(
                "ssx_sky_dome",
                &cache.get_bind_group_layout(&pipeline.dome_layout),
                &BindGroupEntries::sequential((
                    view.clone(),
                    u.binding().expect("written"),
                    &gpu.texture_view,
                    &pipeline.sampler,
                )),
            );
            let variant =
                hook.as_ref().and_then(|(h, key)| crate::render::hooks::runtime::drawn(*h, key)).and_then(|h| {
                    use crate::render::hooks::runtime::{admit, hook_defs, variant_shader};
                    const DOME: &str = "render/sky/sky_dome.wesl";
                    let found = variant_shader(h, DOME)?;
                    admit(h, DOME, &hook_defs(h)?).then_some(found)
                });
            (true, variant, bg)
        }
    };
    for (entity, view, msaa) in &views {
        let key =
            PackageKey { format: view.target_format, samples: msaa.samples(), dome: key_dome, hook: hook.clone() };
        let id = pipelines.specialize(&cache, &pipeline, key);
        commands.entity(entity).insert(ViewPackageSky { pipeline: id, bind_group: bind_group.clone(), dome: key_dome });
    }
}

pub(super) fn draw<'a>(
    pass: &mut bevy::render::render_phase::TrackedRenderPass<'a>,
    view: &'a ViewPackageSky,
    buffers: &'a DomeBuffers,
    cache: &'a PipelineCache,
    offset: u32,
) -> bool {
    let Some(pipeline) = cache.get_render_pipeline(view.pipeline) else { return false };
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &view.bind_group, &[offset]);
    if view.dome {
        let Some((_, streams, indices, count)) = &buffers.mesh else { return false };
        for (slot, s) in streams.iter().enumerate() {
            pass.set_vertex_buffer(slot, s.slice(..));
        }
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..*count, 0, 0..1);
    } else {
        pass.draw(0..3, 0..1);
    }
    true
}
