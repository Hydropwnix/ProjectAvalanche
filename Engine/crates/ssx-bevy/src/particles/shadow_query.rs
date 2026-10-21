use bevy::asset::embedded_asset;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{
    AsBindGroup, BufferUsages, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

use crate::render::post::GamePostCamera;
use crate::ride::Ride;

const SHADER_PATH: &str = "embedded://ssx_bevy/particles/shadow_query.wgsl";
const QUAD_DISTANCE: f32 = 0.6;
const QUAD_HALF: f32 = 0.01;

#[derive(Resource, Clone, Copy, Debug)]
pub struct PlayerShadowQuery {
    pub value: f32,
    pub readbacks: u64,
    pub cascade: f32,
    pub margin: f32,
}

impl Default for PlayerShadowQuery {
    fn default() -> Self {
        Self { value: 1.0, readbacks: 0, cascade: -1.0, margin: 0.0 }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct ShadowQueryMaterial {
    #[uniform(0)]
    pub point: Vec4,
    #[storage(1, visibility(fragment))]
    pub result: Handle<ShaderBuffer>,
}

impl Material for ShadowQueryMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::Always);
        }
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub fn plugin(app: &mut App) {
    embedded_asset!(app, "shadow_query.wgsl");
    app.add_plugins(MaterialPlugin::<ShadowQueryMaterial>::default())
        .init_resource::<PlayerShadowQuery>()
        .add_systems(Update, (spawn_query, set_point));
    crate::shell::level::scoped::<QueryMaterial>(app);
}

#[derive(Resource)]
struct QueryMaterial(Handle<ShadowQueryMaterial>);

fn spawn_query(
    mut commands: Commands,
    existing: Option<Res<QueryMaterial>>,
    cameras: Query<Entity, With<GamePostCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ShadowQueryMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    if existing.is_some() {
        return;
    }
    let Some(camera) = cameras.iter().next() else { return };
    let mut buffer = ShaderBuffer::from(vec![Vec4::new(1.0, 0.0, 0.0, 0.0)]);
    buffer.buffer_description.usage |= BufferUsages::COPY_SRC;
    let result = buffers.add(buffer);
    let material = materials.add(ShadowQueryMaterial { point: Vec4::ZERO, result: result.clone() });
    let quad = meshes.add(Rectangle::new(2.0 * QUAD_HALF, 2.0 * QUAD_HALF));
    commands.spawn((
        Name::new("player shadow query"),
        Mesh3d(quad),
        MeshMaterial3d(material.clone()),
        Transform::from_xyz(0.0, 0.0, -QUAD_DISTANCE),
        NotShadowCaster,
        NotShadowReceiver,
        ChildOf(camera),
    ));
    commands
        .spawn((Name::new("player shadow read-back"), Readback::buffer(result), crate::shell::level::LevelEntity))
        .observe(|event: On<ReadbackComplete>, mut q: ResMut<PlayerShadowQuery>| {
            let v: Vec<Vec4> = event.to_shader_type();
            if let Some(v) = v.first() {
                (q.value, q.readbacks, q.cascade, q.margin) = (v.x, q.readbacks + 1, v.z, v.w);
                if q.readbacks % 30 == 1 && crate::render::post::env_is("SSX_SHADOW_QUERY_LOG", "on") {
                    info!("player shadow query: {:.3} (point valid {}, read-back {})", v.x, v.y, q.readbacks);
                }
            }
        });
    commands.insert_resource(QueryMaterial(material));
}

fn set_point(
    ride: Option<Res<Ride>>,
    handle: Option<Res<QueryMaterial>>,
    mut materials: ResMut<Assets<ShadowQueryMaterial>>,
) {
    let Some(handle) = handle else { return };
    let point = ride.map_or(Vec4::ZERO, |r| r.render.traj.extend(1.0));
    if let Some(m) = materials.get(&handle.0)
        && m.point != point
        && let Some(mut m) = materials.get_mut(&handle.0)
    {
        m.point = point;
    }
}
