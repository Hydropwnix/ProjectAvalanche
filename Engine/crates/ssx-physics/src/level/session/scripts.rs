use super::*;

impl Session {
    pub fn update_objectives(&mut self, s: &mut RiderState, w: &World, nis_active: bool, dt_s: f32) {
        self.load_objectives(w);
        self.update_objective_sets(s, w, nis_active, dt_s);
        self.update_geotags(s, dt_s);
    }

    pub(super) fn update_geotags(&mut self, s: &RiderState, dt_s: f32) {
        let Some(g) = self.geotags.as_mut() else { return };
        let intro = s.start.as_ref().and_then(|l| l.intro.as_ref());
        let hidden = intro.is_some_and(|i| i.signals().iter().any(|x| x == "BLN_DisableWorldRender"));
        g.update(dt_s, s.traj, hidden);
        if let Some(o) = self.objectives.as_mut() {
            for e in &g.events {
                if let geotags::GeotagEvent::Collected { .. } = e {
                    o.runner.events.post(crate::objectives::Name::new("OfflineGeoCacheCollected"), None);
                }
            }
        }
    }

    pub(super) fn update_objective_sets(&mut self, s: &mut RiderState, w: &World, nis_active: bool, dt_s: f32) {
        let Some(o) = self.objectives.as_mut() else { return };
        let fresh = std::mem::take(&mut self.effects_fresh);
        if o.runner.time > 0.0 && fresh {
            o.effects.clear();
        }
        self.effects_from = if o.runner.time > 0.0 { o.effects.len() } else { 0 };
        let t = self.tutorial.get_or_insert_default();
        let mut world = crate::objectives::SessionWorld {
            s,
            w,
            tracker: &self.tracker,
            event: &mut self.event,
            rewind: &mut self.manager.count,
            nis_active,
            heli_contact: self.heli_jump.as_ref().map(|h| h.contact),
            overlay: &mut o.overlay,
            effects: &mut o.effects,
            heli: &mut o.heli,
            nis_starts: &o.nis_starts,
            intercept: self.heli_jump.as_mut().map(|h| &mut h.intercept),
            tutorial: Some(t),
            unapplied: &mut o.unapplied,
            nis: &self.nis_events,
            postfx: &mut o.postfx,
            backwards: self.rewind_backwards,
            stats: &mut self.stats,
            ui_flags: &self.ui_flags,
        };
        o.runner.update(dt_s, &mut world);
        for (name, v) in &self.flag_overrides {
            o.runner.flags.set(&crate::objectives::Name::new(name), *v, true);
        }
        if let Some(c) = self.controllers.as_mut() {
            let mut ctx =
                crate::controllers::Ctx { sw: &mut world, flags: &mut o.runner.flags, events: &o.runner.events };
            c.update(dt_s, &mut ctx);
        }
        self.screen.update(&o.effects[self.effects_from..], dt_s);
    }

    pub(super) fn new_effects(&self) -> &[(String, crate::objectives::Effect)] {
        self.objectives.as_ref().map_or(&[][..], |o| o.effects.get(self.effects_from..).unwrap_or(&[]))
    }

    pub fn load_objectives(&mut self, w: &World) {
        if !self.objectives_loaded {
            self.nis_events = crate::nis_events::NisEvents::new(w.tuning.posted_events.clone());
            self.objectives_loaded = true;
            let event = self.event.setup.objectives.as_ref().map(|e| &e.scripts);
            let batch = match (&w.survival.objectives, event) {
                (Some(d), e) => d.level_batch(e),
                (None, Some(e)) => e.batch(),
                (None, None) => Vec::new(),
            };
            let root = w
                .survival
                .objectives
                .as_ref()
                .and_then(|d| d.root.clone())
                .or_else(|| event.and_then(|e| e.root.clone()));
            if !batch.is_empty() {
                let event_type = self.event.setup.event_type.map(|t| t as i32);
                let (slots, log) = (w.survival.slots, self.objectives_log);
                let drop = w.survival.objectives.as_deref();
                let game = drop.map_or_else(Vec::new, |d| d.named(&crate::objectives::GAME_SCRIPTS));
                self.stats.level_start(&slots, event_type);
                let seed: Vec<u32> = self.named_flag_seed.iter().copied().collect();
                let message = self.event.setup.message.clone();
                let load = crate::objectives::LoadWorld { slots, event_type, message, effects: Vec::new() };
                let o = crate::objectives::LevelObjectives::load(&game, &batch, load, log, root.as_deref(), &seed);
                self.objectives = Some(o);
                if let Some(d) = drop {
                    let scripts = crate::objectives::LevelScripts::load(d, event);
                    let mut c = crate::controllers::Controllers::load(
                        &scripts.state_sets,
                        &scripts.scene,
                        &scripts.schemas,
                        &w.triggers,
                        w.survival.head_lamp_scale,
                    );
                    c.log = self.controllers_log.then(Vec::new);
                    for e in &self.named_entities {
                        if let Some(x) = e.controller() {
                            c.add_entity(x);
                        }
                    }
                    self.controllers = Some(c);
                    self.scripts = Some(std::sync::Arc::new(scripts));
                }
            }
        }
    }

    pub fn drive_helicopter(&mut self, heli: &mut crate::helicopter::Helicopter) {
        let Some(o) = self.objectives.as_mut() else { return };
        if heli.orders_seen != o.heli.generation {
            (heli.move_to, heli.orders_seen) = (o.heli.move_to, o.heli.generation);
        }
        heli.ends_with_heli_jump = o.heli.ends_with_heli_jump;
        heli.reverse_cam = o.heli.reverse_cam;
        if let Some((p, q)) = o.heli.warp.take() {
            heli.warp(p, q);
        }
    }

    pub fn heli_interaction(
        &self,
        heli: &mut crate::helicopter::Helicopter,
        s: &mut RiderState,
        w: &World,
        tricks: &TrickInput,
    ) {
        use crate::helicopter::interaction::InteractionInputs;
        let stick = if self.pads_released { glam::Vec2::ZERO } else { tricks.right_stick };
        let angle = stick.y.atan2(stick.x).rem_euclid(std::f32::consts::TAU);
        let mag = stick.length().min(1.0);
        let rstick_down = (3.926_991..=5.497_787).contains(&angle) && (0.49..=1.0).contains(&mag);
        let intercept = self.heli_jump.as_ref().is_some_and(|h| h.intercept.valid);
        let x = InteractionInputs {
            dt: DT_TICKS * self.dilation,
            rewind_active: self.buffer.is_active(),
            rewind_terminating: self.buffer.is_terminating(),
            in_air: s.in_air(),
            can_interact: !(intercept || s.state.is_wingsuit()) && s.heli_interaction_allowed,
            aerial_trick_priority: s.tricks.is_tricking && (s.score.is_awesome || s.score.is_super_awesome),
            track_finished: self.flow.track_finished,
            rstick_down,
            goofy: s.goofy,
            ice_axe: w.survival.slots[crate::character::SLOT_ICE_AXE],
            boarder_position: s.traj,
            boarder_velocity: s.traj_velocity,
            drag: crate::intercept::DragModel {
                wind: s.grind.wind,
                kh: w.props.k_h_air,
                kv: w.props.k_v_air,
                g: w.props.g_air.y.abs(),
            },
            heli_position: glam::Vec3::ZERO,
            heli_orientation: glam::Quat::IDENTITY,
            heli_velocity: glam::Vec3::ZERO,
            floor_joint: glam::Vec3::ZERO,
        };
        if let Some(v) = heli.interaction(&mut s.heli, x) {
            s.traj_velocity = v;
        }
    }

    // TODO(guess): the controller reads the query one frame late (the caller queries after the execution, whose end already ran the next frame's controllers, 0x82D3DB98 / 0x823D312C); settle: the helicopter's step and queries inside the execution's simulation phase (scene op 26's place in `SOM_Game`).
    pub fn set_downwash(&mut self, d: &crate::helicopter::downwash::Downwash) {
        crate::guess_hit!();
        use crate::helicopter::downwash::{ID_DOWNWASH_GROUND_AO, ID_DOWNWASH_VISUAL_STRENGTH};
        use ssx_lua::game::objectives::GsValue;
        let Some(o) = self.objectives.as_mut() else { return };
        let heli = crate::objectives::Name::new("DropHeli").hash;
        o.overlay.insert((heli, ID_DOWNWASH_VISUAL_STRENGTH), GsValue::Float(d.visual_strength));
        o.overlay.insert((heli, ID_DOWNWASH_GROUND_AO), GsValue::Float(d.ground_ao));
    }

    pub fn restart_objectives(&mut self, s: &mut RiderState, w: &World, nis_active: bool) {
        let Some(o) = self.objectives.as_mut() else { return };
        let mut world = crate::objectives::SessionWorld {
            s,
            w,
            tracker: &self.tracker,
            event: &mut self.event,
            rewind: &mut self.manager.count,
            nis_active,
            heli_contact: self.heli_jump.as_ref().map(|h| h.contact),
            overlay: &mut o.overlay,
            effects: &mut o.effects,
            heli: &mut o.heli,
            nis_starts: &o.nis_starts,
            intercept: self.heli_jump.as_mut().map(|h| &mut h.intercept),
            tutorial: self.tutorial.as_mut(),
            unapplied: &mut o.unapplied,
            nis: &self.nis_events,
            postfx: &mut o.postfx,
            backwards: self.rewind_backwards,
            stats: &mut self.stats,
            ui_flags: &self.ui_flags,
        };
        o.runner.restart_set(crate::objectives::SetId::Level, &mut world);
    }
}
