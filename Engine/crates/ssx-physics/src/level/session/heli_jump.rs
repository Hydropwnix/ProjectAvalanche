use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use glam::{Quat, Vec3};

use crate::event::EventManager;
use crate::helicopter::Helicopter;
use crate::nis::IntroAssets;
use crate::rider::{RideState, RiderState, World};
use crate::triggers::TriggerSet;

pub const END_LOCATION: &str = "worlds_heli_end_location";
pub const GOTO_END: &str = "worlds_heli_goto_end";
pub const END_JUMP: &str = "worlds_heli_end_jump";
pub const CONTACT_DISTANCE: f32 = 2.5;
pub const RAIL_JOINT: &str = "RightRailGrab";
pub const CONTACT_NODES: [&str; 2] = ["state_airborne", "state_airborne_jump"];
#[allow(clippy::approx_constant)]
pub const INTERCEPT_ANGLE: f32 = 3.14159;
pub const INTERCEPT_LENGTH_SCALE: f32 = 8.0;

type Rt = (Quat, Vec3);

fn compose(a: Rt, b: Rt) -> Rt {
    ((a.0 * b.0).normalize(), a.1 + a.0 * b.1)
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeliJumpData {
    pub end_location: (Vec3, Quat),
    pub heli_start: (Vec3, Quat),
    pub rail: Rt,
}

impl HeliJumpData {
    pub fn load(big_root: &Path, triggers: &TriggerSet) -> Result<Self> {
        use ssx_assets::nis::{HELI_GRAB_OUTRO_BINDING, HELI_RIG, ROLE_HELICOPTER1, Rig, binding_item_start};
        let end_location =
            crate::outro::locator(triggers, END_LOCATION).with_context(|| format!("no {END_LOCATION}"))?;
        let (sp, sq) = binding_item_start(big_root, HELI_GRAB_OUTRO_BINDING, ROLE_HELICOPTER1)?
            .context("nis_end_heli_grab binds no helicopter")?;
        let (p, q) = end_location;
        let heli_start = (p + q * Vec3::from_array(sp), (q * Quat::from_array(sq)).normalize());
        let dir = big_root.join(ssx_assets::rig_anim::ANIM_DIR);
        let db = ssx_assets::anim_db::AssetDb::load(&dir, &[ssx_assets::rig_anim::RIG_BUNDLE])?;
        let all = &db.streams.first().context("no anim_all.cba")?.1;
        let rig = Rig::load(all, HELI_RIG)?;
        let chain = rig.chain(RAIL_JOINT, "AITrajectory").with_context(|| format!("{HELI_RIG}: no {RAIL_JOINT}"))?;
        let mut rail = (Quat::IDENTITY, Vec3::ZERO);
        for j in chain.iter().rev() {
            if let Some(d) = rig.defaults.get(j) {
                rail = compose(rail, (Quat::from_array(d.q).normalize(), Vec3::new(d.t[0], d.t[1], d.t[2])));
            }
        }
        Ok(Self { end_location, heli_start, rail })
    }

    pub fn rail_of(&self, traj: Rt) -> Vec3 {
        compose(traj, self.rail).1
    }

    pub fn rail_at_start(&self) -> Vec3 {
        self.rail_of((self.heli_start.1, self.heli_start.0))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intercept {
    pub valid: bool,
    pub position: Vec3,
    pub angle: f32,
    pub length_scale: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeliJumpUpdate {
    pub moved_heli: bool,
    pub intercept_changed: bool,
    pub outro: bool,
}

#[derive(Clone, Debug)]
pub struct HeliJump {
    pub data: Arc<HeliJumpData>,
    pub outro_script: bool,
    pub big_root: PathBuf,
    pub event: String,
    pub character: String,
    pub intercept: Intercept,
    pub contact: bool,
    pub outro: Option<Arc<IntroAssets>>,
    launch: Option<(Vec3, Vec3)>,
}

impl HeliJump {
    pub fn new(data: Arc<HeliJumpData>, outro_script: bool, big_root: &Path, event: &str, character: &str) -> Self {
        Self {
            data,
            outro_script,
            big_root: big_root.to_path_buf(),
            event: event.to_string(),
            character: character.to_string(),
            intercept: Intercept::default(),
            contact: false,
            outro: None,
            launch: None,
        }
    }

    pub fn restart(&mut self) {
        (self.intercept, self.contact, self.outro, self.launch) = (Intercept::default(), false, None, None);
    }

    pub fn prepare_takeoff(&mut self, s: &mut RiderState, w: &World, dt: f32) {
        let orig = match self.launch {
            Some((o, written)) if s.jump.launch == written => o,
            _ => s.jump.launch,
        };
        let i = self.intercept;
        let model = crate::intercept::DragModel {
            wind: s.grind.wind,
            kh: w.props.k_h_air,
            kv: w.props.k_v_air,
            g: w.props.g_air.y.abs(),
        };
        let assisted = if i.valid && s.start.is_none() {
            let v_air = s.traj_velocity + orig;
            crate::intercept::takeoff(v_air, orig, s.traj, i.position, i.angle, i.length_scale, &model, dt)
        } else {
            None
        };
        match assisted {
            Some((v, _)) => {
                let written = v - s.traj_velocity;
                s.jump.launch = written;
                self.launch = Some((orig, written));
            }
            None => {
                s.jump.launch = orig;
                self.launch = None;
            }
        }
    }

    pub fn contact_query(&mut self, s: &RiderState, heli: Option<&Helicopter>, nodes: Option<&[String]>) {
        let airborne = match nodes {
            Some(p) => p.iter().any(|n| CONTACT_NODES.contains(&n.as_str())),
            None => s.start.is_none() && matches!(s.state, RideState::Airborne | RideState::Jump),
        };
        if airborne && let Some(h) = heli {
            // TODO(guess): `RightRailGrab` from the rig's defaults (no gameplay tilt), the `Distance Filter`'s `Comparison` 2 as "closer than", a failing test's write as false: `QueryTask_Player_NearHeliGrabHandle` (on `state_airborne` / `state_airborne_jump`) takes the rider's `AITrajectory` within 2.5 m (`Axes` 7) of the helicopter's `RightRailGrab`
            // and sets `BLN_PlayerHeliContact` by "Filter Succeed"; the processor's write on a failing query was not located; settle: the filter's and the processor's code, the helicopter's gameplay pose.
            crate::guess_hit!();
            self.contact = s.traj.distance(self.data.rail_of(h.traj)) < CONTACT_DISTANCE;
        }
    }

    pub fn apply_objectives(
        &mut self,
        s: &mut RiderState,
        w: &World,
        ev: &EventManager,
        effects: &[(String, crate::objectives::Effect)],
        intercept_before: Intercept,
        next_dt: f32,
    ) -> Result<HeliJumpUpdate> {
        use crate::objectives::Effect;
        let mut out = HeliJumpUpdate {
            moved_heli: effects
                .iter()
                .any(|(_, e)| matches!(e, Effect::MoveToNisStartLocation { entity, .. } if entity.is("DropHeli"))),
            intercept_changed: intercept_before != self.intercept,
            outro: false,
        };
        let grab = effects
            .iter()
            .any(|(_, e)| matches!(e, Effect::PlayNis(r) if r.name == ssx_assets::nis::HELI_GRAB_OUTRO_BINDING));
        if grab && self.outro.is_none() {
            use crate::outro::{game_mode, track_end_reaction};
            let reaction = ev.result.as_ref().and_then(|r| track_end_reaction(game_mode(&self.event), r));
            let assets = crate::outro::prepare_heli_grab(&self.big_root, &self.event, &self.character, reaction)?;
            let mut r = crate::outro::start_at(w, assets.clone(), END_LOCATION, s.level_start)?;
            crate::outro::carry_bound_rider(&mut r, s);
            *s = r;
            self.outro = Some(assets);
            out.outro = true;
        }
        self.prepare_takeoff(s, w, next_dt);
        Ok(out)
    }
}
