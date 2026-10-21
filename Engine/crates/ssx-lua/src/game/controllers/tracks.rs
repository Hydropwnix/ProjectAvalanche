use super::{Arg, TrackDef};
use crate::game::objectives::{GsType, GsValue, Name, game_state_type};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    None,
    Activate,
    Deactivate,
    DeactivateImmediately,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Node(String),
    Effect(String),
    Group(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightUpdate {
    None,
    Blink { strength: f32, frequency: f32, gap: f32 },
    Flicker { strength: f32 },
    HeadLamp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PhysicsMode {
    None,
    Freeze,
    UnFreeze,
    PushAwayFromPlayer(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modification {
    pub field: String,
    pub stat: i32,
    pub x: f32,
    pub y: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    Ao {
        ao: f32,
        blend: f32,
    },
    AoGroundContact {
        blend: f32,
    },
    AoHelicopter {
        blend: f32,
    },
    Audio,
    Camera,
    GameState {
        id: u16,
        value: GsValue,
    },
    GeoCache,
    GlobalLighting {
        condition: Name,
        blend_in: f32,
        blend_out: Option<f32>,
        append: bool,
    },
    Light {
        name: String,
        group: bool,
        on_start: Switch,
        update: LightUpdate,
        on_end: Switch,
        head_lamp: bool,
    },
    MeshParticles,
    MoveToEntity {
        entity: Name,
        time: f32,
    },
    NisDataToGameState {
        entity: Name,
        position: u16,
        orientation: u16,
        fov_id: u16,
        fov: f32,
    },
    OutOfBounds {
        entity: Name,
        timer: f32,
        strength: f32,
    },
    Oxygen,
    Particle {
        target: Target,
        on_start: Switch,
        on_end: Switch,
    },
    ParticleFromGear {
        item: i32,
        instant: bool,
    },
    Physics(PhysicsMode),
    PostFxModulated {
        preset: Name,
        id: u16,
        inverted: bool,
    },
    PostFx {
        preset: Name,
        blend_in: f32,
        blend_out: f32,
        weight: f32,
    },
    Precipitation(f32),
    PulseGogglesGear {
        blend_in: f32,
        blend_out: f32,
        pulse_rate: f32,
        outer_range: f32,
        static_threshold: f32,
        modifications: Vec<Modification>,
    },
    RenderAttribute {
        name: Name,
        blend: f32,
        value: [f32; 4],
        reset_on_end: bool,
    },
    RibbonTrail,
    Ripple,
    SetNamedFlag {
        name: Name,
        value: bool,
        temporary: bool,
    },
    SubState {
        state: Name,
    },
    Temperature,
    TransformAnimation {
        rotate: Option<f32>,
        bounce: Option<(f32, f32)>,
    },
    WindDirection,
    WindSpeed(f32),
    NotPorted(&'static str),
}

fn num(a: &[Arg], i: usize) -> Option<f32> {
    a.get(i).and_then(Arg::num).map(|x| x as f32)
}

fn text(a: &Arg) -> String {
    a.str().unwrap_or_default().to_string()
}

fn int(a: &Arg) -> i32 {
    crate::number::to_int(a.num().unwrap_or(0.0))
}

impl Track {
    pub fn of(def: &TrackDef) -> Track {
        let opt = |i: usize, default: f32| def.arg(i).num().map_or(default, |x| x as f32);
        let name = |i: usize| Name::new(&text(def.arg(i)));
        match def.kind {
            "AoTrack" => {
                let (mut ao, mut blend) = (opt(1, 1.0), opt(2, 0.0));
                for (m, a) in &def.methods {
                    match *m {
                        "SetAmbientOcclusion" => ao = num(a, 0).unwrap_or(ao),
                        "SetBlendTime" => blend = num(a, 0).unwrap_or(blend),
                        _ => {}
                    }
                }
                Track::Ao { ao, blend }
            }
            "AoGroundContactTrack" | "AoHelicopterTrack" => {
                let blend = def.method("SetBlendTime").and_then(|a| num(a, 0)).unwrap_or(opt(1, 0.0));
                match def.kind {
                    "AoGroundContactTrack" => Track::AoGroundContact { blend },
                    _ => Track::AoHelicopter { blend },
                }
            }
            "AudioTrack" => Track::Audio,
            "CameraTrack" => Track::Camera,
            "GameStateTrack" => {
                let id = int(def.arg(1));
                let value = match game_state_type(id) {
                    Some(GsType::Bool) => GsValue::Bool(def.arg(2).truthy()),
                    Some(GsType::Float) => GsValue::Float(opt(2, 0.0)),
                    Some(GsType::Int) => GsValue::Int(int(def.arg(2))),
                    _ => return Track::NotPorted("GameStateTrack of a vector / quaternion"),
                };
                Track::GameState { id: id as u16, value }
            }
            "GeoCacheTrack" => Track::GeoCache,
            "GlobalLightingTrack" => {
                let (mut blend_out, mut append) = (None, false);
                for (m, a) in &def.methods {
                    match *m {
                        "BlendOutTime" => blend_out = num(a, 0),
                        "UseAppendMode" => append = true,
                        _ => {}
                    }
                }
                Track::GlobalLighting { condition: name(1), blend_in: opt(2, 0.0), blend_out, append }
            }
            "LightTrack" | "HeadLampGearTrack" => {
                let head_lamp = def.kind == "HeadLampGearTrack";
                let (mut group, mut on_start, mut on_end) = (false, Switch::None, Switch::None);
                let mut update = if head_lamp { LightUpdate::HeadLamp } else { LightUpdate::None };
                for (m, a) in &def.methods {
                    match *m {
                        "ActivateOnStart" => on_start = Switch::Activate,
                        "DeactivateOnStart" => on_start = Switch::Deactivate,
                        "DeactivateOnEnd" => on_end = Switch::Deactivate,
                        "SetIsGroupName" => group = true,
                        "BlinkOnUpdate" => {
                            let (s, f, g) = (num(a, 0), num(a, 1), num(a, 2));
                            update = LightUpdate::Blink {
                                strength: s.unwrap_or(0.0),
                                frequency: f.unwrap_or(0.0),
                                gap: g.unwrap_or(0.0),
                            }
                        }
                        "FlickerOnUpdate" => update = LightUpdate::Flicker { strength: num(a, 0).unwrap_or(0.0) },
                        _ => {}
                    }
                }
                let name = if head_lamp { String::new() } else { text(def.arg(1)) };
                Track::Light { name, group, on_start, update, on_end, head_lamp }
            }
            "MeshParticlesTrack" => Track::MeshParticles,
            "MoveToEntityTrack" => Track::MoveToEntity { entity: name(1), time: opt(2, 0.0) },
            "NISDataToGameStateTrack" => Track::NisDataToGameState {
                entity: name(1),
                position: int(def.arg(2)) as u16,
                orientation: int(def.arg(3)) as u16,
                fov_id: int(def.arg(4)) as u16,
                fov: opt(5, 0.0),
            },
            "OutOfBoundsTrack" => Track::OutOfBounds { entity: name(1), timer: opt(2, 0.5), strength: opt(3, 2.0) },
            "OxygenTrack" => Track::Oxygen,
            "ParticleTrack" => {
                let (mut on_start, mut on_end, mut kind) = (Switch::None, Switch::None, 0);
                for (m, _) in &def.methods {
                    match *m {
                        "ActivateOnStart" => on_start = Switch::Activate,
                        "DeactivateOnStart" => on_start = Switch::Deactivate,
                        "DeactivateImmediatelyOnStart" => on_start = Switch::DeactivateImmediately,
                        "DeactivateOnEnd" => on_end = Switch::Deactivate,
                        "DeactivateImmediatelyOnEnd" => on_end = Switch::DeactivateImmediately,
                        "SetIsEffectName" => kind = 1,
                        "SetIsGroupName" => kind = 2,
                        _ => {}
                    }
                }
                let n = text(def.arg(1));
                let target = match kind {
                    0 => Target::Node(n),
                    1 => Target::Effect(n),
                    _ => Target::Group(n),
                };
                Track::Particle { target, on_start, on_end }
            }
            "ParticleFromGearTrack" => Track::ParticleFromGear { item: int(def.arg(1)), instant: def.arg(2).truthy() },
            "PhysicsTrack" => {
                let mut mode = PhysicsMode::None;
                for (m, a) in &def.methods {
                    mode = match *m {
                        "Freeze" => PhysicsMode::Freeze,
                        "UnFreeze" => PhysicsMode::UnFreeze,
                        "PushAwayFromPlayer" => PhysicsMode::PushAwayFromPlayer(num(a, 0).unwrap_or(1.0)),
                        _ => mode,
                    };
                }
                Track::Physics(mode)
            }
            "PostFxModulatedTrack" => Track::PostFxModulated {
                preset: name(1),
                id: int(def.arg(2)) as u16,
                inverted: def.has("SetGameStateInverted"),
            },
            "PostFxTrack" => {
                Track::PostFx { preset: name(1), blend_in: opt(2, 0.0), blend_out: opt(3, 0.0), weight: opt(4, 0.0) }
            }
            "PrecipitationTrack" => Track::Precipitation(opt(1, 0.0)),
            "PulseGogglesGearTrack" => Track::PulseGogglesGear {
                blend_in: opt(1, 0.0),
                blend_out: opt(2, 0.0),
                pulse_rate: opt(3, 0.0),
                outer_range: opt(4, 0.0),
                static_threshold: opt(5, 0.0),
                modifications: def
                    .methods
                    .iter()
                    .filter(|(m, _)| matches!(*m, "AddModification" | "AddModificationXY"))
                    .map(|(m, a)| Modification {
                        field: a.first().map(text).unwrap_or_default(),
                        stat: a.get(1).map_or(0, int),
                        x: num(a, 2).unwrap_or(0.0),
                        y: (*m == "AddModificationXY").then(|| num(a, 3).unwrap_or(0.0)),
                    })
                    .collect(),
            },
            "RenderAttributeTrack" => {
                let x = opt(3, 0.0);
                let y = opt(4, x);
                let z = opt(5, y);
                let w = opt(6, z);
                Track::RenderAttribute {
                    name: name(1),
                    blend: opt(2, 0.0),
                    value: [x, y, z, w],
                    reset_on_end: def.has("ResetOnEnd"),
                }
            }
            "RibbonTrailTrack" => Track::RibbonTrail,
            "RippleTrack" => Track::Ripple,
            "SetNamedFlagTrack" => {
                Track::SetNamedFlag { name: name(1), value: def.arg(2).truthy(), temporary: int(def.arg(3)) != 0 }
            }
            "SubStateTrack" => Track::SubState { state: name(1) },
            "TemperatureTrack" => Track::Temperature,
            "TransformAnimationTrack" => {
                let (mut rotate, mut bounce) = (None, None);
                for (m, a) in &def.methods {
                    match *m {
                        "Rotate" => rotate = num(a, 0).map(|d| d * 0.017_453_29),
                        "Bounce" => bounce = Some((num(a, 0).unwrap_or(0.0), num(a, 1).unwrap_or(0.0))),
                        _ => {}
                    }
                }
                Track::TransformAnimation { rotate, bounce }
            }
            "WindDirectionTrack" => Track::WindDirection,
            "WindSpeedTrack" => Track::WindSpeed(opt(1, 0.0)),
            other => Track::NotPorted(other),
        }
    }
}
