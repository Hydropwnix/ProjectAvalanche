use std::path::Path;

use anyhow::{Context, Result};

pub const SCRIPTS_DIR: &str = "memresident/data/common/scripts";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneOrigin {
    Absolute,
    LevelStart,
    LevelEnd,
    Locator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntroVariant {
    Heli,
    Snow,
    Outro,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IntroChoice {
    pub script: String,
    pub binding: String,
    pub origin: SceneOrigin,
    pub locator_alternate: Option<String>,
    pub variant: IntroVariant,
}

pub fn intro_of_probe(p: &crate::objectives::Probe) -> Option<IntroChoice> {
    use crate::objectives::{Effect, GsValue, game_state_name};
    let (objective, r) = p.first_nis()?;
    let snow = p.before_first_nis().iter().any(|(_, e)| {
        matches!(e, Effect::SetGameState { entity, id, value: GsValue::Bool(true) }
            if entity.is("Player1") && game_state_name(*id) == "BLN_NisSnowStart")
    });
    let script = p.scripts.get(objective).map_or("", String::as_str);
    let origin = match r.origin {
        0 => SceneOrigin::Absolute,
        1 => SceneOrigin::LevelStart,
        2 => SceneOrigin::LevelEnd,
        _ => SceneOrigin::Locator,
    };
    Some(IntroChoice {
        script: script.rsplit(['/', '\\']).next().unwrap_or(script).to_string(),
        binding: r.name.clone(),
        origin,
        locator_alternate: r.locator_alternate.as_ref().map(|n| n.text.to_string()),
        variant: if snow { IntroVariant::Snow } else { IntroVariant::Heli },
    })
}

pub fn intro_of_script(name: &str, text: &str) -> Option<IntroChoice> {
    let slots = crate::character::GearStats::default_gear().slots;
    intro_of_probe(&crate::objectives::Probe::run(&[(name.to_string(), text.as_bytes().to_vec())], None, slots))
}

pub fn intro_of_event(big_root: &Path, event: &crate::event::EventDef) -> Result<Option<IntroChoice>> {
    match &event.objectives {
        Some(o) => Ok(intro_of_probe(&o.probe)),
        None => intro_of_scripts(big_root, &event.scripts),
    }
}

pub fn intro_of_scripts(big_root: &Path, scripts: &[String]) -> Result<Option<IntroChoice>> {
    let d = crate::objectives::DropScripts::of(big_root, scripts);
    let slots = crate::character::GearStats::default_gear().slots;
    Ok(intro_of_probe(&crate::objectives::Probe::run(&d.batch(), None, slots)))
}

use std::sync::Arc;

use glam::{Quat, Vec3};
pub use ssx_assets::nis::IntroPath;
use ssx_assets::nis::{IntroNis, NisActor, NisScene, ROLE_CAMERA1, ROLE_CHARACTER1, ROLE_HELICOPTER1};

use crate::tags::{TagEvent, TagTracker, TagWindow};

#[derive(Debug)]
pub struct IntroAssets {
    pub choice: IntroChoice,
    pub nis: IntroNis,
    pub jump_clip: usize,
    pub event: String,
    pub heli: Result<ssx_assets::helicopter::HeliChoice, String>,
}

impl IntroAssets {
    pub fn scene(&self, s: Scene) -> &NisScene {
        match s {
            Scene::Ready { .. } => &self.nis.ready,
            Scene::Jump { .. } => &self.nis.jump,
        }
    }

    pub fn cabin_view(&self, index: usize) -> Option<IntroNis> {
        let c = self.nis.cabin.get(index)?;
        let mut ready = c.scene.clone();
        ready.loop_end = None;
        Some(IntroNis { ready, cabin: Vec::new(), ..self.nis.clone() })
    }

    pub fn sequence_time(&self, s: Scene) -> f32 {
        let ticks = match s {
            Scene::Ready { ticks } => ticks,
            Scene::Jump { ticks } => self.nis.jump_start + ticks,
        };
        match self.scene(s).loop_end {
            Some(end) if end > 0.0 => ticks.rem_euclid(end),
            _ => ticks,
        }
    }
}

pub fn prepare(
    world: &mut crate::rider::World,
    big_root: &Path,
    event: &crate::event::EventDef,
    character: &str,
) -> Result<Option<Arc<IntroAssets>>> {
    prepare_path(world, big_root, event, character, IntroPath::Short)
}

pub fn prepare_path(
    world: &mut crate::rider::World,
    big_root: &Path,
    event: &crate::event::EventDef,
    character: &str,
    path: IntroPath,
) -> Result<Option<Arc<IntroAssets>>> {
    let Some(choice) = intro_of_event(big_root, event)? else { return Ok(None) };
    let snow = choice.variant == IntroVariant::Snow;
    let heli = ssx_assets::helicopter::drop_helicopter(big_root, &event.name).map_err(|e| format!("{e:#}"));
    let reverse = world.survival.avalanche.reverse_cam;
    let h = heli.as_ref().ok().map(|h| h.name.as_str());
    let mut nis = ssx_assets::nis::load_drop_intro(big_root, &choice.binding, character, h, snow, path, reverse)?;
    snow_picks(&mut nis)?;
    let rider = nis.jump.actor(ROLE_CHARACTER1).context("the go scene has no Character1 actor")?;
    let jump_clip = match rider.clip.delta_trajectory {
        true => world.tuning.root_motion.push_go(crate::rootmotion::RootClip::from_nis(&rider.clip)?),
        false => usize::MAX,
    };
    Ok(Some(Arc::new(IntroAssets { choice, nis, jump_clip, event: event.name.clone(), heli })))
}

pub fn profile_path(
    big_root: &Path,
    event: &crate::event::EventDef,
    visits: u32,
    wingsuit_enabled: bool,
) -> Result<IntroPath> {
    let Some(choice) = intro_of_event(big_root, event)? else { return Ok(IntroPath::Short) };
    let requires = ssx_assets::nis::requires_wingsuit(big_root, &choice.binding)?;
    Ok(IntroPath::decide(false, false, requires, wingsuit_enabled, visits))
}

fn random_pick(state: &mut u32, weights: &[f32]) -> usize {
    *state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    let total: f32 = weights.iter().sum();
    let mut r = total * f32::from((*state >> 16) as u16) * 1.525_902_2e-5;
    weights
        .iter()
        .position(|w| {
            r -= w;
            r <= 0.0
        })
        .unwrap_or(weights.len().saturating_sub(1))
}

// TODO(guess): the random value is fresh (seeded) at the NIS's start, as `levelstart::go_pick` takes it; settle: the actor's draws from its creation to the NIS (the gameloop's choosers before `state_gameloop_playnis`).
pub fn snow_picks(nis: &mut IntroNis) -> Result<()> {
    let Some(seed) = nis.ready.actor(ROLE_CHARACTER1).filter(|a| !a.sequences.is_empty()).and_then(|a| a.seed) else {
        return Ok(());
    };
    crate::guess_hit!();
    let mut state = seed;
    let pick = |state: &mut u32, a: &NisActor| random_pick(state, &a.choices.iter().map(|c| c.1).collect::<Vec<_>>());
    let ready = nis.ready.actors.iter_mut().find(|a| a.role == ROLE_CHARACTER1).context("no Character1")?;
    let k = pick(&mut state, ready);
    ready.clip = ready.choices[k].0.clone();
    ready.length = ready.sequences[k].1;
    nis.ready.sequence = ready.sequences[k].0.clone();
    // TODO(guess): the ready loop repeats its first pick (the game's `chooser_intro_snow_scene5` draws again at each end and blends into it: a `RandomChooserPolicy` — `Looping`, `ChooseOnLoop`, `BezierBlend` 20 ticks — over `proxy_nis_intro_snow_sc05a_1` / `_2` / `_3`, weight 0.15 each, and `proxy_nis_intro_snow_sc05`, 1.0;
    // `graph::IntroGraph` plays one sequence: the rider's graph cannot resolve the NIS banks' proxies); settle: the chooser in `IntroGraph` with the NIS banks.
    crate::guess_hit!();
    nis.ready.loop_end = Some(ready.length);
    if let Some(go) = nis.jump.actors.iter_mut().find(|a| a.role == ROLE_CHARACTER1 && a.choices.len() > 1) {
        let k = pick(&mut state, go);
        go.clip = go.choices[k].0.clone();
        nis.jump.sequence = go.clip.name.clone();
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scene {
    Ready { ticks: f32 },
    Jump { ticks: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CabinClock {
    pub index: usize,
    pub ticks: f32,
}

pub type Rt = (Quat, Vec3);

fn compose(a: Rt, b: Rt) -> Rt {
    ((a.0 * b.0).normalize(), a.1 + a.0 * b.1)
}

fn inverse(a: Rt) -> Rt {
    let q = a.0.conjugate();
    (q, -(q * a.1))
}

fn rt(q: [f32; 4], t: [f32; 3]) -> Rt {
    (Quat::from_array(q).normalize(), Vec3::from_array(t))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NisView {
    pub position: Vec3,
    pub rotation: Quat,
    pub hfov_deg: f32,
    pub trajectory: Rt,
    pub eye: Rt,
}

#[derive(Clone, Debug)]
pub struct IntroState {
    pub assets: Arc<IntroAssets>,
    pub position: Vec3,
    pub orientation: Quat,
    pub scene: Scene,
    pub cabin: Option<CabinClock>,
    pub tuning: Vec3,
    pub alternate: Rt,
    pub dt: f32,
    pub attached: bool,
    drivers: TagTracker,
    stop: TagTracker,
    game: TagTracker,
    game_events: Vec<(usize, TagEvent)>,
}

impl PartialEq for IntroState {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.assets, &o.assets)
            && (self.position, self.orientation, self.scene, self.dt, self.attached)
                == (o.position, o.orientation, o.scene, o.dt, o.attached)
    }
}

fn windows(ws: impl Iterator<Item = ssx_assets::nis::Window>) -> Vec<TagWindow> {
    ws.map(|w| TagWindow::new(w.time, w.duration, w.always_active)).collect()
}

impl IntroState {
    pub fn new(
        assets: Arc<IntroAssets>,
        world: &crate::rider::World,
        position: Vec3,
        orientation: Quat,
    ) -> Option<Self> {
        let at = crate::levelstart::offset_level_start(world, position).unwrap_or(position);
        let orientation = orientation.normalize();
        let alternate = assets
            .choice
            .locator_alternate
            .as_deref()
            .and_then(|n| crate::outro::locator(&world.triggers, n))
            .map_or((orientation, position), |(p, q)| (q.normalize(), p));
        let cabin = (!assets.nis.cabin.is_empty()).then_some(CabinClock { index: 0, ticks: 0.0 });
        let direct = cabin.is_none() && !assets.nis.ready_loop;
        let first = match cabin {
            Some(_) => &assets.nis.cabin[0].scene,
            None if direct => &assets.nis.jump,
            None => &assets.nis.ready,
        };
        let rider = first.actor(ROLE_CHARACTER1)?;
        let drivers = TagTracker::new(&windows(rider.tags.drivers.iter().map(|d| d.window)));
        let attached = !rider.tags.drivers.is_empty();
        let tuning = match cabin {
            Some(_) => assets.nis.cabin[0].tuning_offset,
            None => first.node_tags.tuning_offset,
        };
        let stop = TagTracker::new(&windows(rider.tags.stop.into_iter().filter(|_| direct)));
        let game = Self::game_tracker(rider);
        Some(Self {
            scene: if direct { Scene::Jump { ticks: 0.0 } } else { Scene::Ready { ticks: 0.0 } },
            assets,
            position: at,
            orientation,
            cabin,
            tuning: tuning.map_or(Vec3::ZERO, Vec3::from_array),
            alternate,
            dt: 0.0,
            attached,
            drivers,
            stop,
            game,
            game_events: Vec::new(),
        })
    }

    fn game_tracker(a: &NisActor) -> TagTracker {
        TagTracker::new(&windows(a.tags.game_state.iter().map(|t| t.window)))
    }

    pub fn game_state_events(&self) -> Vec<(&ssx_assets::nis::GameStateTag, TagEvent)> {
        let Some(a) = self.actor(ROLE_CHARACTER1) else { return Vec::new() };
        self.game_events.iter().filter_map(|&(i, e)| Some((a.tags.game_state.get(i)?, e))).collect()
    }

    pub fn in_ready_loop(&self) -> bool {
        self.cabin.is_none() && matches!(self.scene, Scene::Ready { .. })
    }

    pub fn signals(&self) -> &[String] {
        &self.current().active_signals
    }

    pub fn current(&self) -> &NisScene {
        match self.cabin {
            Some(c) => &self.assets.nis.cabin[c.index].scene,
            None => self.assets.scene(self.scene),
        }
    }

    pub fn ticks(&self) -> f32 {
        if let Some(c) = self.cabin {
            return c.ticks;
        }
        match self.scene {
            Scene::Ready { ticks } | Scene::Jump { ticks } => ticks,
        }
    }

    pub fn sequence_time(&self, ticks: f32) -> f32 {
        if let Some(c) = self.cabin {
            return self.assets.nis.cabin[c.index].start + ticks;
        }
        let s = match self.scene {
            Scene::Ready { .. } => Scene::Ready { ticks },
            Scene::Jump { .. } => Scene::Jump { ticks },
        };
        self.assets.sequence_time(s)
    }

    fn actor(&self, role: i64) -> Option<&NisActor> {
        self.current().actor(role)
    }

    pub fn branch_due(&self) -> bool {
        self.cabin.is_none()
            && matches!(self.scene, Scene::Ready { ticks } if self.assets.nis.branch_out.is_some_and(|t| ticks >= t))
    }

    pub fn root_motion(&self) -> bool {
        matches!(self.scene, Scene::Jump { .. })
            && !self.attached
            && self.actor(ROLE_CHARACTER1).is_some_and(|a| a.clip.delta_trajectory)
    }

    pub fn transition(&mut self) {
        if self.cabin.is_none()
            && let Scene::Ready { .. } = self.scene
        {
            self.enter_jump(0.0);
        }
    }

    fn enter_jump(&mut self, ticks: f32) {
        self.scene = Scene::Jump { ticks };
        let jump = &self.assets.nis.jump;
        if let Some(t) = jump.node_tags.tuning_offset {
            self.tuning = Vec3::from_array(t);
        }
        let rider = jump.actor(ROLE_CHARACTER1);
        self.drivers =
            TagTracker::new(&windows(rider.into_iter().flat_map(|a| a.tags.drivers.iter().map(|d| d.window))));
        self.stop = TagTracker::new(&windows(rider.and_then(|a| a.tags.stop).into_iter()));
        self.game = rider.map_or_else(|| TagTracker::new(&[]), Self::game_tracker);
    }

    pub fn advance(&mut self, dt: f32) {
        if self.cabin.is_some() {
            self.advance_cabin(dt);
        } else {
            match &mut self.scene {
                Scene::Ready { ticks } | Scene::Jump { ticks } => *ticks += dt,
            }
        }
        self.dt = dt;
        let t = self.sequence_time(self.ticks());
        self.drivers.update(t);
        self.attached = self.drivers.process().iter().any(|(_, e)| matches!(e, TagEvent::Apply(_)));
        self.game_events = if self.game.update(t) { self.game.process() } else { Vec::new() };
    }

    // TODO(guess): the chain's transitions are cuts (the Red Bull scene 4 blends 5 ticks into the ready loop); settle: the cabin chain as one state flow in the rider's graph.
    fn advance_cabin(&mut self, dt: f32) {
        let Some(mut c) = self.cabin else { return };
        crate::guess_hit!();
        let mut rest = dt;
        let assets = self.assets.clone();
        let cabin = &assets.nis.cabin;
        loop {
            let sc = &cabin[c.index];
            let left = (sc.end - (sc.start + c.ticks)).max(0.0);
            if rest <= left {
                c.ticks += rest;
                self.cabin = Some(c);
                return;
            }
            rest -= left;
            if c.index + 1 < cabin.len() {
                self.enter_cabin(c.index + 1);
                c = CabinClock { index: c.index + 1, ticks: 0.0 };
            } else {
                self.cabin = None;
                if !self.assets.nis.ready_loop {
                    self.enter_jump(rest);
                } else {
                    self.enter_ready(rest);
                }
                return;
            }
        }
    }

    pub fn stop_request(&mut self) -> bool {
        match self.scene {
            Scene::Jump { ticks } => {
                self.stop.update(self.assets.sequence_time(Scene::Jump { ticks }));
                self.stop.process().iter().any(|(_, e)| matches!(e, TagEvent::Apply(_)))
            }
            Scene::Ready { .. } => false,
        }
    }

    pub fn outro_active(&self) -> bool {
        if self.current().active_signals.iter().any(|s| s == "BLN_NisOutroActive") {
            return true;
        }
        let t = self.sequence_time(self.ticks());
        self.actor(ROLE_CHARACTER1)
            .and_then(|a| a.tags.outro_active)
            .is_some_and(|w| w.always_active || (t >= w.time && t <= w.time + w.duration))
    }

    pub fn takes_go(&self) -> bool {
        let tags = |a: Option<&NisActor>| a.is_some_and(|a| a.tags.runs_nis_script(0));
        tags(self.actor(ROLE_CHARACTER1)) || self.current().node_tags.runs_nis_script(0)
    }

    pub fn takes_skip(&self) -> bool {
        let t = self.sequence_time(self.ticks());
        let active = |tags: &ssx_assets::nis::Tags, t: f32| {
            tags.scripts.iter().any(|c| {
                c.script == "script_nis"
                    && c.update == 1
                    && (c.window.always_active || (t >= c.window.time && t <= c.window.time + c.window.duration))
            })
        };
        self.actor(ROLE_CHARACTER1).is_some_and(|a| active(&a.tags, t))
            || active(&self.current().node_tags, (self.ticks() - self.dt).max(0.0))
    }

    pub fn skip(&mut self) -> bool {
        let Some(c) = self.cabin else { return false };
        let Some((to, _blend)) = self.assets.nis.cabin[c.index].skip else { return false };
        crate::guess_hit!();
        // TODO(guess): the skip's blend is a cut (all but the Red Bull scene 4's have `BlendTime` 0); settle: the chain as one state flow in the rider's graph.
        match to {
            ssx_assets::nis::SkipTo::Cabin(i) => self.enter_cabin(i),
            ssx_assets::nis::SkipTo::ReadyLoop => self.enter_ready(0.0),
            ssx_assets::nis::SkipTo::Jump => {
                self.cabin = None;
                self.enter_jump(0.0);
            }
        }
        true
    }

    fn enter_cabin(&mut self, index: usize) {
        self.cabin = Some(CabinClock { index, ticks: 0.0 });
        let next = &self.assets.nis.cabin[index];
        if let Some(t) = next.tuning_offset {
            self.tuning = Vec3::from_array(t);
        }
        let rider = next.scene.actor(ROLE_CHARACTER1);
        self.drivers =
            TagTracker::new(&windows(rider.into_iter().flat_map(|a| a.tags.drivers.iter().map(|d| d.window))));
        self.game = rider.map_or_else(|| TagTracker::new(&[]), Self::game_tracker);
    }

    fn enter_ready(&mut self, ticks: f32) {
        self.cabin = None;
        self.scene = Scene::Ready { ticks };
        let ready = &self.assets.nis.ready;
        if let Some(t) = ready.node_tags.tuning_offset {
            self.tuning = Vec3::from_array(t);
        }
        let rider = ready.actor(ROLE_CHARACTER1);
        self.drivers =
            TagTracker::new(&windows(rider.into_iter().flat_map(|a| a.tags.drivers.iter().map(|d| d.window))));
        self.game = rider.map_or_else(|| TagTracker::new(&[]), Self::game_tracker);
    }
}

impl IntroState {
    fn pose(&self, role: i64, ticks: f32) -> Option<(&NisActor, ssx_formats::ant_anim::Pose)> {
        let a = self.actor(role)?;
        Some((a, a.clip.sample(self.sequence_time(ticks)).ok()?))
    }

    fn joint(&self, a: &NisActor, p: &ssx_formats::ant_anim::Pose, joint: &str) -> Rt {
        let (q, t) = a.clip.joint(p, self.assets.nis.rig(a.role), joint);
        rt(q, t)
    }

    pub fn heli(&self, ticks: f32) -> Option<Rt> {
        let (a, p) = self.pose(ROLE_HELICOPTER1, ticks)?;
        let traj = self.joint(a, &p, "AITrajectory");
        Some(compose(self.toc_frame(a), traj))
    }

    pub fn toc_frame(&self, a: &NisActor) -> Rt {
        let (t, r) = a.offset.as_ref().map_or(("", ""), |(t, r)| (t.as_str(), r.as_str()));
        let p = if t == "VEC_NisPositionAlternate" { self.alternate.1 } else { self.position + self.tuning };
        let q = if r == "QUAT_NisOrientationAlternate" { self.alternate.0 } else { self.orientation };
        (q, p)
    }

    pub fn heli_joint(&self, ticks: f32, joint: &str) -> Option<Rt> {
        let (a, p) = self.pose(ROLE_HELICOPTER1, ticks)?;
        let chain = self.assets.nis.heli.chain(joint, "AITrajectory")?;
        let mut m = self.heli(ticks)?;
        for j in chain.iter().rev() {
            m = compose(m, self.joint(a, &p, j));
        }
        Some(m)
    }

    pub fn attached_root(&self, ticks: f32) -> Option<Rt> {
        let (a, p) = self.pose(ROLE_CHARACTER1, ticks)?;
        let d = a.tags.drivers.first()?;
        let target = compose(self.heli_joint(ticks, &d.target_joint)?, rt(d.offset_q, d.offset_t));
        let chain = self.assets.nis.rider.chain(&d.source_joint, "AITrajectory")?;
        let mut src = (Quat::IDENTITY, Vec3::ZERO);
        for j in chain.iter().rev() {
            src = compose(src, self.joint(a, &p, j));
        }
        Some(compose(target, inverse(src)))
    }

    pub fn placed_root(&self) -> Option<Rt> {
        let (a, p) = self.pose(ROLE_CHARACTER1, self.ticks())?;
        let driven = !a.tags.drivers.is_empty() && (self.attached || a.offset.is_none() || a.clip.delta_trajectory);
        if !driven && a.offset.is_some() && !a.clip.delta_trajectory {
            let traj = self.joint(a, &p, "AITrajectory");
            return Some(compose(self.toc_frame(a), traj));
        }
        if !driven && !a.clip.delta_trajectory {
            // TODO(guess): the clip's trajectory is taken as the world's (no reader of the params' interface 0x307A58D6 places an actor: of its 13 uses the loader 0x83522130 fills X, Y, Z, `LaunchAngle`, `Scale`, `Mirror`, `StartTick`, the actor inits 0x83535B78 / 0x835002D0 read `StartTick` only, the tracks `Mirror`, 0x8350C840 `Scale`,
            // the camera controller keeps them at +0x98 (its update is the actor's); a read of the embedded `DefaultInitParam` is not ruled out — if X, Y, Z, `LaunchAngle` placed the actor, the scene would sit 3 m off and turned a quarter); settle: the game's picture of this NIS or the sequence container's creation of its actors.
            crate::guess_hit!();
            return Some(self.joint(a, &p, "AITrajectory"));
        }
        self.attached_root(self.ticks())
    }

    pub fn camera(&self, rider: Rt) -> Option<NisView> {
        let t = self.ticks();
        let (a, p) = self.pose(ROLE_CAMERA1, t)?;
        crate::guess_hit!();
        // TODO(guess): a random camera chooser (the survive intro's go, the heli-pickup outro's, cabin scenes 1 / 2a / 2) shows its first item; settle: the camera animatable's draws before the NIS (`Seed` 1876900708).
        let traj = match a.tags.attachments.first() {
            Some(att) if att.target == "ID_NisTargetHelicopter" => {
                let target = self.heli_joint(t, &att.target_joint).or_else(|| self.heli(t))?;
                let arm = self.joint(a, &p, &att.source_joint);
                compose(compose(target, rt(att.offset_q, att.offset_t)), inverse(arm))
            }
            Some(att) => {
                let mut target = rider;
                if let Some((ra, rp)) = self.pose(ROLE_CHARACTER1, t)
                    && let Some(chain) = self.assets.nis.rider.chain(&att.target_joint, "AITrajectory")
                {
                    for j in chain.iter().rev() {
                        target = compose(target, self.joint(ra, &rp, j));
                    }
                }
                let arm = self.joint(a, &p, &att.source_joint);
                compose(compose(target, rt(att.offset_q, att.offset_t)), inverse(arm))
            }
            None if a.offset.is_some() => compose((self.orientation, self.position), self.joint(a, &p, "Trajectory")),
            None => return None,
        };
        let local = self.joint(a, &p, "CameraEye");
        let eye = compose(traj, local);
        let hfov = a.clip.float(&p, "FieldOfView")?;
        Some(NisView { position: eye.1, rotation: eye.0, hfov_deg: hfov, trajectory: traj, eye: local })
    }
}

pub fn level_start(
    world: &crate::rider::World,
    position: Vec3,
    orientation: Quat,
    assets: Arc<IntroAssets>,
) -> Option<crate::rider::RiderState> {
    level_start_on(world, position, position, orientation, assets)
}

pub fn level_start_on(
    world: &crate::rider::World,
    ground: Vec3,
    position: Vec3,
    orientation: Quat,
    assets: Arc<IntroAssets>,
) -> Option<crate::rider::RiderState> {
    let mut s = crate::rider::RiderState::level_start(world, ground, orientation)?;
    s.level_start = position;
    let ls = crate::levelstart::LevelStart::new(world, position, orientation)?.with_intro(world, assets)?;
    let (traj, t) = ls.idle_pose(&world.tuning.root_motion)?;
    (s.traj, s.traj_cached, s.traj_cached_prev) = (traj, traj, traj);
    (s.alignment, s.traj_orientation, s.heading) = (t, t, t * Vec3::X);
    s.janked = crate::janked::JankedState::new(traj);
    s.grind.tail_pos = traj;
    s.start = Some(ls);
    s.position = s.board_point();
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intro_of_a_script() {
        let race = r#"BeginObjective("Race Intro NIS")
	DO_PlayNis(
	{
		name = "nis_binding_intro_race",
		sceneOrigin = NIS_SCENE_ORIGIN_LEVEL_START,
		locatorAlternate = "nis_intro_locator",
		actors = { {role = "Character",  entity = "Player1"} }
	})
EndObjective()"#;
        let i = intro_of_script("eventtype_race.lua", race).expect("intro");
        assert_eq!(i.binding, "nis_binding_intro_race");
        assert_eq!(i.origin, SceneOrigin::LevelStart);
        assert_eq!(i.locator_alternate.as_deref(), Some("nis_intro_locator"));
        assert_eq!(i.variant, IntroVariant::Heli);
        let snow = format!(
            "BeginObjective(\"x\")\n\tDO_SetGameStateValue(\"Player1\", \tBLN_NisSnowStart, true)\n{}",
            &race[race.find("\tDO_PlayNis").unwrap_or(0)..]
        );
        assert_eq!(intro_of_script("eventtype_headtohead.lua", &snow).map(|i| i.variant), Some(IntroVariant::Snow));
        assert!(intro_of_script("x.lua", "BeginObjective(\"none\")\nEndObjective()").is_none());
    }

    #[test]
    fn attachment_places_the_source_on_the_target() {
        let target: Rt = (Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.2), Vec3::new(3.0, 12.0, -1.0));
        let src: Rt = (Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2), Vec3::new(0.01, -0.86, 1.92));
        let root = compose(target, inverse(src));
        let back = compose(root, src);
        assert!(back.1.distance(target.1) < 1e-5 && back.0.dot(target.0).abs() > 1.0 - 1e-6);
    }
}
