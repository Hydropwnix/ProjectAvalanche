use std::collections::BTreeMap;

use bevy::prelude::*;
use ssx_lua::game::controllers::Track;
use ssx_lua::game::objectives::Effect;
use ssx_physics::controllers::EntityController;
use ssx_ui::game::fe::globe_look::LightState;

use super::GameLighting;
use crate::ride::Ride;

#[derive(Clone, Debug, Default)]
pub struct Driver {
    read_step: Option<u64>,
    read_time: f64,
    read_effects: usize,
    tracks: BTreeMap<(u32, usize), (u32, u32, bool)>,
    fog: (u32, u32),
    serial: u64,
    fade_step: u64,
    fading: bool,
    textures_noted: bool,
    off: bool,
}

impl Driver {
    pub fn new() -> Self {
        Driver { off: crate::render::post::env_is("SSX_LIGHTING_FADE", "off"), ..Default::default() }
    }
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LightingFadeSet;

pub fn faded(lighting: Option<Res<GameLighting>>) -> bool {
    lighting.is_some_and(|l| l.is_changed() && !l.is_added())
}

fn track_of(e: &EntityController, condition: u32) -> (f32, bool) {
    let found = e.controller.sets.kinds.iter().find_map(|t| match t {
        Track::GlobalLighting { condition: c, blend_out, append, .. } if c.hash == condition => {
            Some((blend_out.unwrap_or(0.0), *append))
        }
        _ => None,
    });
    found.unwrap_or((0.0, false))
}

fn name(presets: &mut super::service::Presets, key: u32, text: &str) {
    if let Some(p) = presets.get_mut(&key).filter(|p| p.name.starts_with("0x")) {
        p.name = text.to_string();
    }
}

pub fn read_ride(ride: Option<Res<Ride>>, lighting: Option<ResMut<GameLighting>>) {
    let (Some(ride), Some(mut lighting)) = (ride, lighting) else { return };
    let GameLighting { service, presets, driver: d, .. } = lighting.bypass_change_detection();
    if d.off || d.read_step == Some(ride.steps) {
        return;
    }
    d.read_step = Some(ride.steps);
    if let Some(o) = ride.session.objectives.as_ref() {
        let start = if d.read_time > 0.0 { 0 } else { d.read_effects.min(o.effects.len()) };
        for (_, effect) in &o.effects[start..] {
            match effect {
                Effect::LightingOverrideFromParam { target, source } => {
                    service.override_from_param(&target.text, &source.text, presets);
                }
                Effect::LightingOverrideRemove(target) => service.remove_override(&target.text),
                Effect::LightingFlush(blend) => service.flush(*blend, presets),
                Effect::SurviveFogBegin { source } => {
                    name(presets, source.hash, &source.text);
                    d.fog.0 = service.add_condition(0.0, source.hash, presets);
                }
                Effect::SurviveFogTarget { target, on: true, blend } => {
                    name(presets, target.hash, &target.text);
                    d.fog.1 = service.add_condition(*blend, target.hash, presets);
                }
                Effect::SurviveFogTarget { on: false, blend, .. } if d.fog.1 != 0 => {
                    service.remove_condition(*blend, std::mem::take(&mut d.fog.1), presets);
                }
                Effect::SurviveFogEnd => {
                    for handle in [std::mem::take(&mut d.fog.0), std::mem::take(&mut d.fog.1)] {
                        if handle != 0 {
                            service.remove_condition(0.0, handle, presets);
                        }
                    }
                }
                _ => {}
            }
        }
        d.read_effects = o.effects.len();
        d.read_time = o.runner.time;
    }
    let Some(controllers) = ride.session.controllers.as_ref() else { return };
    for e in &controllers.entities {
        let id = (e.instance, e.placement);
        let now = e.fx.lighting.as_ref().map(|(condition, blend_in)| (condition.hash, *blend_in));
        if let Some((condition, _)) = &e.fx.lighting {
            name(presets, condition.hash, &condition.text);
        }
        let before = d.tracks.get(&id).copied();
        if before.map(|b| b.0) == now.map(|n| n.0) {
            continue;
        }
        if let Some((condition, handle, append)) = before {
            let (blend_out, _) = track_of(e, condition);
            match append {
                true => service.remove_appended(blend_out, handle, presets),
                false => service.remove_condition(blend_out, handle, presets),
            }
            d.tracks.remove(&id);
        }
        if let Some((condition, blend_in)) = now {
            let (_, append) = track_of(e, condition);
            let handle = match append {
                true => service.add_appended(blend_in, condition, presets),
                false => service.add_condition(blend_in, condition, presets),
            };
            d.tracks.insert(id, (condition, handle, append));
        }
    }
}

pub fn frame(ride: Option<Res<Ride>>, lighting: Option<ResMut<GameLighting>>) {
    let Some(mut lighting) = lighting else { return };
    let l = lighting.bypass_change_detection();
    let steps = ride.as_ref().map_or(l.driver.fade_step, |r| r.steps);
    // TODO(guess): the fade's clock is the ride's simulation clock (a pause holds it); settle: the caller of the render objects' slot +4 (vtable 0x8219DC18, 0x82FBC440).
    let dt = steps.saturating_sub(l.driver.fade_step) as f32 / ssx_physics::SIM_HZ;
    l.driver.fade_step = steps;
    let mut write = false;
    if l.service.serial != l.driver.serial {
        l.driver.serial = l.service.serial;
        if let Some(target) = l.service.collection(&l.presets) {
            let differ: Vec<&String> =
                target.textures.iter().filter(|(k, v)| l.preset.textures.get(*k) != Some(*v)).map(|(k, _)| k).collect();
            if !differ.is_empty() && !std::mem::replace(&mut l.driver.textures_noted, true) {
                warn!(
                    "lighting: {} has other textures ({differ:?}): the game's texture entries take them at once \
                     (0x82FE63D4); the port keeps the level's",
                    target.name
                );
            }
            info!("lighting: {} over {} s at ride step {steps}", target.name, l.service.blend);
            l.fade.set(l.service.condition, &target, l.service.blend);
            l.fade.target = super::light_state(l.service.condition, &target);
            // TODO(guess): the frame of a change does not advance the new fade (0x82FD4C80 ends with a 0 s update); settle: the order of vtable 0x8219DC18's slots +4 and +0x38 in a frame.
            l.driver.fading = true;
            write = true;
        }
    } else if l.driver.fading && dt > 0.0 {
        l.fade.step(dt);
        write = true;
    }
    if write {
        let now = l.fade.current();
        l.driver.fading = l.fade.alpha() < 1.0;
        let fading = l.driver.fading;
        l.take(&now, fading);
        lighting.set_changed();
    }
}

impl GameLighting {
    fn take(&mut self, s: &LightState, fading: bool) {
        let unit = |v: [f32; 3]| if fading { Vec3::from(v).normalize_or_zero() } else { Vec3::from(v) };
        self.preset.name.clone_from(&s.name);
        self.preset.floats.clone_from(&s.floats);
        self.preset.vectors.clone_from(&s.vectors);
        self.sun = unit(s.sun);
        self.sun_offset = unit(s.sun_offset);
        self.bounce = Vec3::from(s.bounce);
        self.fill = unit(s.fill);
        self.rim_offset_dir = Vec3::from(s.rim);
    }
}
