use std::collections::BTreeMap;

use glam::{Quat, Vec3};
use ssx_lua::game::controllers::tracks::PhysicsMode;
use ssx_lua::game::controllers::{ControllerWorld, Phase, Track, TrackCall};
use ssx_lua::game::events::PostedEvents;
use ssx_lua::game::objectives::{GsValue, Name, NamedFlags};

use super::{Ctx, EntityFx, PLAYER, ParticleTarget, Switch, apply_light, push_op};
use crate::objectives::ObjectiveWorld;
use crate::weather::{Channel, WeatherStacks};

pub trait EntityHost {
    fn flags(&mut self) -> &mut NamedFlags;
    fn events(&self) -> &PostedEvents;
    fn touching(&self, volume: usize) -> bool;
    fn volume_name(&self, volume: usize) -> Option<&str>;
    fn player_position(&self) -> Option<Vec3>;
    fn game_state(&mut self, entity: &Name, id: u16) -> Option<GsValue>;
    fn write(&mut self, entity: &Name, id: u16, value: GsValue);
    fn boundary_force(&mut self, timer: f32, strength: f32, direction: Vec3);
    fn player_motion(&self) -> Option<(f32, Vec3)>;
    fn set_pushed_object(&mut self);
}

pub(super) struct SessionHost<'a, 'b, 'c> {
    pub ctx: &'a mut Ctx<'b, 'c>,
}

impl EntityHost for SessionHost<'_, '_, '_> {
    fn flags(&mut self) -> &mut NamedFlags {
        self.ctx.flags
    }
    fn events(&self) -> &PostedEvents {
        self.ctx.events
    }
    fn touching(&self, volume: usize) -> bool {
        self.ctx.sw.tracker.current.iter().any(|&(v, _)| v == volume)
    }
    fn volume_name(&self, volume: usize) -> Option<&str> {
        self.ctx.sw.w.triggers.volumes.get(volume).map(|v| v.name.as_str())
    }
    fn player_position(&self) -> Option<Vec3> {
        Some(self.ctx.sw.s.traj)
    }
    fn game_state(&mut self, entity: &Name, id: u16) -> Option<GsValue> {
        self.ctx.sw.game_state(entity, id)
    }
    fn write(&mut self, entity: &Name, id: u16, value: GsValue) {
        self.ctx.sw.write(entity, id, value)
    }
    fn boundary_force(&mut self, timer: f32, strength: f32, direction: Vec3) {
        let b = &mut self.ctx.sw.s.boundary;
        (b.force_timer, b.force_strength, b.force_direction) = (timer, strength, direction);
    }
    fn player_motion(&self) -> Option<(f32, Vec3)> {
        Some((self.ctx.sw.s.player_speed, self.ctx.sw.s.velocity))
    }
    fn set_pushed_object(&mut self) {
        self.ctx.sw.s.score.pushed_object = true;
    }
}

#[derive(Clone, Debug, Default)]
pub struct OfflineHost {
    pub flags: NamedFlags,
    pub events: PostedEvents,
    pub writes: BTreeMap<(u32, u16), GsValue>,
}

impl EntityHost for OfflineHost {
    fn flags(&mut self) -> &mut NamedFlags {
        &mut self.flags
    }
    fn events(&self) -> &PostedEvents {
        &self.events
    }
    fn touching(&self, _volume: usize) -> bool {
        false
    }
    fn volume_name(&self, _volume: usize) -> Option<&str> {
        None
    }
    fn player_position(&self) -> Option<Vec3> {
        None
    }
    fn game_state(&mut self, entity: &Name, id: u16) -> Option<GsValue> {
        self.writes.get(&(entity.hash, id)).copied()
    }
    fn write(&mut self, entity: &Name, id: u16, value: GsValue) {
        self.writes.insert((entity.hash, id), value);
    }
    fn boundary_force(&mut self, _timer: f32, _strength: f32, _direction: Vec3) {}
    fn player_motion(&self) -> Option<(f32, Vec3)> {
        None
    }
    fn set_pushed_object(&mut self) {}
}

pub(super) struct EntityWorld<'a> {
    pub host: &'a mut dyn EntityHost,
    pub body: Option<&'a mut crate::rigid::Body>,
    pub fx: &'a mut EntityFx,
    pub owner: Option<&'a Name>,
    pub position: Vec3,
    pub rotation: Quat,
    pub volumes: &'a [usize],
    pub weather: &'a mut WeatherStacks,
    pub pushed: &'a mut BTreeMap<u64, (Channel, u32)>,
    pub log: Option<&'a mut Vec<String>>,
    pub script: &'a str,
    pub time: f32,
}

impl ControllerWorld for EntityWorld<'_> {
    fn game_state(&mut self, id: u16) -> Option<GsValue> {
        let owner = self.owner?.clone();
        self.host.game_state(&owner, id)
    }
    fn named_flag(&mut self, n: &Name) -> bool {
        self.host.flags().check(n)
    }
    fn posted_event(&mut self, e: &Name) -> bool {
        match self.owner {
            Some(o) => self.host.events().posted(e, Some(o)),
            None => self.host.events().current().iter().any(|(ev, who)| ev.hash == e.hash && who.is_none()),
        }
    }
    fn global_posted_event(&mut self, i: i32) -> bool {
        usize::try_from(i).is_ok_and(|i| self.host.events().global(i))
    }
    fn trigger_volume(&mut self, volume: &str, entity: &str, client: &str) -> Option<bool> {
        if entity != PLAYER || client != "Touch" {
            return None;
        }
        let host = &*self.host;
        Some(
            self.volumes
                .iter()
                .any(|&i| (volume.is_empty() || host.volume_name(i) == Some(volume)) && host.touching(i)),
        )
    }
    fn player_distance(&mut self) -> Option<f32> {
        self.host.player_position().map(|p| (self.position - p).length())
    }
    fn track(&mut self, c: TrackCall<'_>, phase: Phase) {
        if let Some(log) = self.log.as_deref_mut()
            && !matches!(phase, Phase::Update(_))
        {
            log.push(format!("{} {} {phase:?}", self.script, c.def.kind));
        }
        let z = self.rotation * Vec3::Z;
        let fx = &mut *self.fx;
        match (c.kind, phase) {
            (Track::Particle { target, on_start, .. }, Phase::Begin) if *on_start != Switch::None => {
                push_op(&mut fx.particles, &mut fx.next_seq, ParticleTarget::Track(target.clone()), *on_start)
            }
            (Track::Particle { target, on_end, .. }, Phase::End) if *on_end != Switch::None => {
                push_op(&mut fx.particles, &mut fx.next_seq, ParticleTarget::Track(target.clone()), *on_end)
            }
            (Track::Light { .. }, _) => apply_light(&mut fx.lights, c, phase, 0.0, 1.0),
            (Track::Precipitation(x), Phase::Begin) => {
                let id = self.weather.push_precipitation(*x);
                self.pushed.insert(c.inst, (Channel::Precipitation, id));
            }
            (Track::WindSpeed(x), Phase::Begin) => {
                let id = self.weather.push_speed(*x);
                self.pushed.insert(c.inst, (Channel::Speed, id));
            }
            (Track::WindDirection, Phase::Begin) => {
                let id = self.weather.push_direction(z);
                self.pushed.insert(c.inst, (Channel::Direction, id));
            }
            (Track::Precipitation(_) | Track::WindSpeed(_) | Track::WindDirection, Phase::End) => {
                if let Some((ch, id)) = self.pushed.remove(&c.inst) {
                    self.weather.pop(ch, id);
                }
            }
            (Track::SetNamedFlag { name, value, temporary }, Phase::Begin) => {
                self.host.flags().set(name, *value, *temporary)
            }
            (Track::OutOfBounds { entity, timer, strength }, Phase::Update(_)) if entity.is(PLAYER) => {
                self.host.boundary_force(*timer, *strength, z)
            }
            (Track::NisDataToGameState { entity, position, orientation, fov_id, fov }, Phase::Begin) => {
                self.host.write(entity, *position, GsValue::Vector(self.position.to_array()));
                self.host.write(entity, *orientation, GsValue::Quat(self.rotation.to_array()));
                self.host.write(entity, *fov_id, GsValue::Float(*fov));
            }
            (Track::Physics(PhysicsMode::Freeze), Phase::Begin) => {
                fx.physics.frozen = true;
                if let Some(b) = self.body.as_deref_mut() {
                    b.frozen = true;
                }
            }
            (Track::Physics(PhysicsMode::UnFreeze), Phase::Begin) => {
                fx.physics.frozen = false;
                if let Some(b) = self.body.as_deref_mut() {
                    b.frozen = false;
                }
            }
            (Track::Physics(PhysicsMode::PushAwayFromPlayer(strength)), Phase::Begin) => {
                let (Some(player), Some((speed, velocity))) = (self.host.player_position(), self.host.player_motion())
                else {
                    return;
                };
                if let Some(push) =
                    super::physics::push_away(self.position, player, velocity, speed, *strength, self.time)
                {
                    fx.physics.frozen = false;
                    if let Some(b) = self.body.as_deref_mut() {
                        b.push(push.linear, push.angular);
                    }
                    fx.physics.pushes.push(push);
                    self.host.set_pushed_object();
                }
            }
            (Track::Camera, Phase::Begin) => fx.camera = Some(z),
            (Track::Camera, Phase::End) => fx.camera = None,
            (Track::GlobalLighting { condition, blend_in, .. }, Phase::Begin) => {
                fx.lighting = Some((condition.clone(), *blend_in))
            }
            (Track::GlobalLighting { blend_out: Some(_), .. }, Phase::End) => fx.lighting = None,
            (Track::AoHelicopter { .. }, Phase::Begin) => {
                let ao = fx.world_ao;
                let i = fx.heli_ao.entry(c.inst).or_default();
                (i.1, i.2) = (0.0, ao);
            }
            (Track::AoHelicopter { blend }, Phase::Update(dt)) => {
                use crate::helicopter::downwash::{ID_DOWNWASH_GROUND_AO, ao_blend};
                let sample = match self.owner.cloned().and_then(|o| self.host.game_state(&o, ID_DOWNWASH_GROUND_AO)) {
                    Some(GsValue::Float(v)) => v,
                    _ => 0.0,
                };
                if let Some((window, elapsed, start)) = fx.heli_ao.get_mut(&c.inst) {
                    *elapsed += dt;
                    window.push(sample);
                    fx.world_ao = ao_blend(*start, window.mean(), *elapsed, *blend);
                }
            }
            _ => {}
        }
    }
}
