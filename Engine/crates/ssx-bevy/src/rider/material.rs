use bevy::asset::{RenderAssetUsages, embedded_asset, uuid_handle};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

use crate::render::shadows::{ShadowPassKey, specialize_shadow_pass};

use crate::render::lighting::GameLighting;
use crate::render::lighting::game::GameLightUniform;

const SHADER_PATH: &str = "embedded://ssx_bevy/rider/character.wesl";

pub const CHARACTER_STATUS: &str = "\
rider (listing port): player_body/hair/eye/apparel gbuffer + material passes (normal z:=1, d^2, rim + fill, \
dynamicAOMult ramp tint, hair AO map, eye AO, apparel glow)\n\
board (listing port): player_board passes (rim * rimMult, glow = d^2 * d.a, hologlitch off), texture common_board_diffuse \
(the game binds no board texture pack)\n\
rider: playerShadowTestResult = shadowtest.fxo query at the AITrajectory joint (8-bit read-back), \
dynamicGlowMult = GlowOff (SSX_RIDER_GLOW=on: GlowOn)\n\
bots (listing port): player_secondary LOD characters = body passes without the shadow test, alpha kill 0.25, \
no hologlitch (racer kind 1)";

pub type CharacterMaterial = ExtendedMaterial<StandardMaterial, CharacterExtension>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterShading {
    Body = 0,
    Hair = 1,
    Eye = 2,
    Apparel = 3,
    Board = 4,
    Wingsuit = 5,
    Secondary = 6,
}

impl CharacterShading {
    pub fn of(effect: &str) -> Option<CharacterShading> {
        match effect {
            "player_body.fx" => Some(CharacterShading::Body),
            "player_hair.fx" => Some(CharacterShading::Hair),
            "player_eye.fx" => Some(CharacterShading::Eye),
            "player_apparel.fx" => Some(CharacterShading::Apparel),
            "player_board.fx" => Some(CharacterShading::Board),
            "player_wingsuit.fx" => Some(CharacterShading::Wingsuit),
            "player_secondary.fx" => Some(CharacterShading::Secondary),
            _ => None,
        }
    }
}

pub struct CharacterMaterialPlugin;

impl Plugin for CharacterMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "character.wesl");
        app.add_plugins(MaterialPlugin::<CharacterMaterial>::default());
        let frame = RiderFrame::default();
        let _ = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .insert(RIDER_FRAME.id(), ShaderBuffer::new(frame.bytes(), RenderAssetUsages::default()));
        app.add_systems(PostUpdate, upload_rider_frame);
        use crate::render::lighting::fade::{LightingFadeSet, faded};
        app.add_systems(Update, follow_lighting.after(LightingFadeSet).run_if(faded));
    }
}

fn follow_lighting(lighting: Res<GameLighting>, mut materials: ResMut<Assets<CharacterMaterial>>) {
    let preset = CharacterLightUniform::from_preset(&lighting);
    for (_, m) in materials.iter_mut() {
        let r = &mut m.extension.rider;
        (r.rim_color, r.rim, r.rim_dir) = (preset.rim_color, preset.rim, preset.rim_dir);
        (r.fill_color, r.fill_dir) = (preset.fill_color, preset.fill_dir);
    }
}

pub const RIDER_FRAME: Handle<ShaderBuffer> = uuid_handle!("3f6b2a1c-8d4e-4b7a-9c2e-5a1d7e3f9b04");

#[derive(Clone, Copy, Debug, PartialEq)]
struct RiderFrame {
    dynamic: Vec4,
    shadow_point: Vec4,
}

impl Default for RiderFrame {
    fn default() -> Self {
        RiderFrame { dynamic: Vec4::new(AO_IN_AIR, 0.0, 0.0, 0.0), shadow_point: Vec4::ZERO }
    }
}

impl RiderFrame {
    fn bytes(&self) -> Vec<u8> {
        self.dynamic.to_array().iter().chain(&self.shadow_point.to_array()).flat_map(|f| f.to_le_bytes()).collect()
    }
}

fn upload_rider_frame(
    shading: Option<Res<RiderShading>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut last: Local<Option<RiderFrame>>,
) {
    let Some(shading) = shading else { return };
    let Some(frame) = shading.frame else { return };
    if *last == Some(frame) {
        return;
    }
    if let Some(mut buffer) = buffers.get_mut(&RIDER_FRAME) {
        buffer.clear();
        buffer.extend_from_slice(&frame.bytes());
        *last = Some(frame);
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(ShadowPassKey)]
pub struct CharacterExtension {
    #[uniform(100)]
    pub light: GameLightUniform,
    #[uniform(101)]
    pub rider: CharacterLightUniform,
    #[uniform(102)]
    pub params: CharacterParams,
    #[texture(103)]
    #[sampler(104)]
    pub normal: Handle<Image>,
    #[texture(105)]
    pub specular: Handle<Image>,
    #[texture(106)]
    pub ambient: Handle<Image>,
    #[uniform(120)]
    pub shadow_alpha: Vec4,
    #[storage(140, read_only)]
    pub local_lights: Handle<bevy::render::storage::ShaderBuffer>,
    #[texture(141, dimension = "2d_array")]
    #[sampler(142)]
    pub local_light_textures: Handle<Image>,
    #[storage(121, read_only)]
    pub frame: Handle<ShaderBuffer>,
    pub shadow_pass: ShadowPassKey,
}

impl From<&CharacterExtension> for ShadowPassKey {
    fn from(e: &CharacterExtension) -> Self {
        e.shadow_pass
    }
}

impl MaterialExtension for CharacterExtension {
    fn vertex_shader() -> ShaderRef {
        crate::render::lighting::game::GAME_VERTEX_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        crate::render::objects::material::SHADOW_ALPHA_PATH.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_shadow_pass(descriptor, key.mesh_key, key.bind_group_data);
        crate::render::lighting::game::specialize_game_vertex(descriptor, 100);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct CharacterParams {
    pub shading: Vec4,
    pub glow: Vec4,
    pub lod_bias: Vec4,
    pub board: Vec4,
    pub holo: Vec4,
    pub holo_bloom: Vec4,
}

impl CharacterParams {
    pub fn new(shading: CharacterShading, spec_intensity: f32, eccentricity: f32, glow: Vec3) -> Self {
        CharacterParams {
            shading: Vec4::new(shading as u8 as f32, spec_intensity, eccentricity, 0.0),
            glow: glow.extend(0.0),
            lod_bias: Vec4::ZERO,
            board: Vec4::ZERO,
            holo: Vec4::ZERO,
            holo_bloom: Vec4::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct CharacterLightUniform {
    pub rim_color: Vec4,
    pub rim: Vec4,
    pub rim_dir: Vec4,
    pub fill_color: Vec4,
    pub fill_dir: Vec4,
    pub dynamic: Vec4,
    pub shadow_point: Vec4,
    pub frame: Vec4,
}

impl CharacterLightUniform {
    pub fn from_preset(l: &GameLighting) -> Self {
        let (f, v) = (|n: &str| l.f(n), |n: &str| l.v(n));
        CharacterLightUniform {
            rim_color: v("rimColor").truncate().extend(f("rimIntensity")),
            rim: Vec4::new(f("rimFalloff"), f("rimSpread"), f("rimOffsetFalloff"), f("rimOffsetAmount")),
            rim_dir: l.rim_offset_dir.extend(f("rimMaxShadowAttenuation")),
            fill_color: v("fillColor").truncate().extend(f("fillIntensity")),
            fill_dir: l.fill.extend(f("fillMaxShadowAttenuation")),
            dynamic: Vec4::new(AO_IN_AIR, 0.0, 0.0, 0.0),
            shadow_point: Vec4::ZERO,
            frame: Vec4::ZERO,
        }
    }

    pub fn viewer_default() -> Self {
        CharacterLightUniform {
            rim_dir: Vec4::new(0.0, 1.0, 0.0, 0.0),
            fill_dir: Vec4::new(0.0, 1.0, 0.0, 0.0),
            dynamic: Vec4::new(AO_IN_AIR, 0.0, 0.0, 0.0),
            ..default()
        }
    }
}

const AO_IN_AIR: f32 = 0.6;
const AO_CONTACT_SCALE: f32 = 0.8;
const AO_CONTACT_WEIGHT: f32 = 2.0;
const AO_SAMPLES: usize = 45;

#[derive(Resource, Default)]
pub struct RiderShading {
    pub materials: Vec<Handle<CharacterMaterial>>,
    samples: std::collections::VecDeque<(f32, f32)>,
    last_step: Option<u64>,
    pub glow: f32,
    pub ao: f32,
    pub wingsuit_deployed: bool,
    pub wingsuit: Vec<Handle<CharacterMaterial>>,
    pub ao_override: Option<f32>,
    frame: Option<RiderFrame>,
    bound: bool,
    wingsuit_alpha: Option<f32>,
}

impl RiderShading {
    pub fn new(materials: Vec<Handle<CharacterMaterial>>, wingsuit: Vec<Handle<CharacterMaterial>>) -> Self {
        let glow = std::env::var("SSX_RIDER_GLOW").is_ok_and(|v| v == "on" || v == "1");
        RiderShading { materials, wingsuit, glow: if glow { 1.0 } else { 0.0 }, ao: AO_IN_AIR, ..default() }
    }

    pub fn sample(&mut self, step: u64, contact_ao: Option<f32>) {
        if self.last_step == Some(step) {
            return;
        }
        self.last_step = Some(step);
        let sample = match contact_ao {
            Some(a) => (1.0 - AO_CONTACT_SCALE * (1.0 - a), AO_CONTACT_WEIGHT),
            None => (AO_IN_AIR, 1.0),
        };
        let sample = match self.ao_override {
            Some(v) if (v - 1.0).abs() > 1.525_878_9e-5 => (v, 1.0),
            _ => sample,
        };
        if self.samples.len() == AO_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
        let (sum, weight) = self.samples.iter().fold((0.0, 0.0), |(s, w), (v, k)| (s + v * k, w + k));
        self.ao = if weight > 0.0 { (sum / weight).clamp(0.0, 1.0) } else { AO_IN_AIR };
    }

    pub fn replace(
        &mut self,
        old: &[Handle<CharacterMaterial>],
        new: Vec<Handle<CharacterMaterial>>,
        wingsuit: Vec<Handle<CharacterMaterial>>,
    ) {
        self.materials.retain(|m| !old.contains(m));
        self.materials.extend(new);
        self.wingsuit = wingsuit;
        (self.bound, self.wingsuit_alpha) = (false, None);
    }

    pub fn apply(&mut self, shadow_point: Vec3, materials: &mut Assets<CharacterMaterial>) {
        let deployed = if self.wingsuit_deployed { 1.0 } else { 0.0 };
        if !self.bound {
            for handle in &self.materials {
                if let Some(mut m) = materials.get_mut(handle) {
                    m.extension.rider.frame.x = 1.0;
                }
            }
            self.bound = true;
        }
        self.frame = Some(RiderFrame {
            dynamic: Vec4::new(self.ao, self.glow, deployed, 0.0),
            shadow_point: shadow_point.extend(1.0),
        });
        if self.wingsuit_alpha != Some(deployed) {
            for handle in &self.wingsuit {
                if let Some(mut m) = materials.get_mut(handle) {
                    m.extension.shadow_alpha.y = deployed;
                }
            }
            self.wingsuit_alpha = Some(deployed);
        }
    }
}
