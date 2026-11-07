pub mod downwash;
pub mod interaction;

mod flight;
mod movement;
mod pose;
mod tuning;

pub use movement::*;
pub use tuning::*;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use glam::{Quat, Vec3};

use crate::curve::Curve;
use crate::nis::{IntroAssets, IntroState, Scene};
use crate::tuning::{AssetView, Bundle};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeliNode {
    IdleStart,
    ChaseTarget,
    MoveToPosition,
    IdleEnd { ticks: f32 },
    PlayerInteractTarget,
    Trick { seq: usize, ticks: f32, mirror: bool },
}

#[derive(Clone, Debug)]
pub struct HeliFlightData {
    pub update: HeliUpdateTuning,
    pub follow: HeliMoveTuning,
    pub to_position: HeliMoveTuning,
    pub grid: Option<HeightGrid>,
    pub end_level: Option<ssx_assets::nis::NisClip>,
    pub idle_end_blend: (f32, Vec<f32>),
    pub posing: Option<HeliPosingTuning>,
    pub interaction: Option<interaction::InteractionTuning>,
    pub tricks: Vec<ssx_assets::helicopter::HeliTrickSeq>,
    pub hover: Option<ssx_assets::helicopter::HoverNoise>,
    pub wash: Option<(String, String)>,
}

const DEFAULT_BLEND_TICKS: f32 = 8.0;

impl HeliFlightData {
    pub fn load(
        big_root: &Path,
        track: &str,
        _scripts: &[String],
        _outro_locator: Option<(Vec3, Quat)>,
    ) -> Result<Self> {
        let b = crate::tuning::load(big_root)?;
        let bezier = b.view("Transition node_heli_movetoposition to node_heli_idle_end", "StateFlowTransitionAsset")?;
        let blend = bezier.asset("OverrideBlend")?;
        let samples = match blend.value("Samples")? {
            ssx_formats::ant::Value::Array(xs) => xs.iter().filter_map(|x| x.as_f32()).collect(),
            _ => Vec::new(),
        };
        let track_dir = big_root.join("data0/data/xenon/gameexplorer").join(track);
        Ok(Self {
            update: HeliUpdateTuning::load(&b)?,
            follow: HeliMoveTuning::load(&b, "heli_move_to_follow")?,
            to_position: HeliMoveTuning::load(&b, "heli_move_to_position")?,
            grid: HeightGrid::load(&track_dir, track).ok(),
            end_level: ssx_assets::helicopter::end_level_clip(big_root).ok(),
            idle_end_blend: (blend.f32("BlendTime").unwrap_or(0.0), samples),
            posing: HeliPosingTuning::load(&b).ok(),
            interaction: interaction::InteractionTuning::load(&b).ok(),
            tricks: ssx_assets::helicopter::heli_trick_sequences(big_root).unwrap_or_default(),
            hover: ssx_assets::helicopter::HoverNoise::load_disc(big_root).ok(),
            wash: downwash::wash_joints(big_root).ok(),
        })
    }
}

pub fn move_to_locator(p: Vec3, q: Quat, offset: Vec3, yaw_deg: f32) -> (Vec3, Quat) {
    (p + q * offset, (q * Quat::from_rotation_y(yaw_deg.to_radians())).normalize())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeliPhase {
    Nis(Scene),
    Gameplay { ticks: f32, from: Scene },
}

#[derive(Clone, Copy, Debug)]
pub struct HeliStep<'a> {
    pub intro: Option<&'a IntroState>,
    pub dt: f32,
    pub time: f32,
    pub race_proper_started: bool,
    pub track_finished: bool,
    pub remaining: Option<f32>,
    pub target: (Vec3, Vec3),
}

#[derive(Clone, Debug)]
pub struct Helicopter {
    pub assets: Arc<IntroAssets>,
    pub frame: (Quat, Vec3),
    pub phase: HeliPhase,
    pub cabin: Option<crate::nis::CabinClock>,
    pub flight: Option<Arc<HeliFlightData>>,
    pub traj: (Quat, Vec3),
    pub velocity: Vec3,
    pub angular_velocity: Quat,
    pub node: HeliNode,
    pub board: HeliBoard,
    pub move_to: Option<(Vec3, Quat)>,
    pub ends_with_heli_jump: bool,
    pub orders_seen: u32,
    pub at_move_to: bool,
    pub outputs: Option<MoveOutputs>,
    pub interaction: interaction::HeliInteractionBoard,
    pub interaction_node: interaction::InteractionNode,
    pub initial_interact_time: f32,
    pub reverse_cam: bool,
    ctrl: Option<MoveController>,
    blend: Option<(Option<MoveController>, HeliNode, f32)>,
    nis_last: Option<pose::NisLast>,
    nis_chain: Vec<(crate::nis::SceneRef, f32, f32, bool)>,
}

impl Helicopter {
    pub fn new(intro: &IntroState) -> Self {
        Self::with_flight(intro, None)
    }

    pub fn with_flight(intro: &IntroState, flight: Option<Arc<HeliFlightData>>) -> Self {
        let traj = intro.heli(intro.ticks()).unwrap_or((intro.orientation, intro.position));
        Self {
            assets: intro.assets.clone(),
            frame: nis_frame(intro),
            phase: HeliPhase::Nis(intro.scene),
            cabin: intro.cabin,
            flight,
            traj,
            velocity: Vec3::ZERO,
            angular_velocity: Quat::IDENTITY,
            node: HeliNode::IdleStart,
            board: HeliBoard::default(),
            move_to: None,
            ends_with_heli_jump: false,
            orders_seen: 0,
            at_move_to: false,
            outputs: None,
            interaction: interaction::HeliInteractionBoard::default(),
            interaction_node: interaction::InteractionNode::default(),
            initial_interact_time: 0.0,
            reverse_cam: false,
            ctrl: None,
            blend: None,
            nis_last: None,
            nis_chain: intro.chain(),
        }
    }

    pub fn warp(&mut self, p: Vec3, q: Quat) {
        // TODO(guess): the movement controller's own state (the PID's integral and last error, the facing spring) is kept — the warp writes the pose and game state, not the controller (0x82CFEF60 reaches no controller instance); settle: a read of `BLN_WarpedThisFrame` in `HeliMovementController` 0x82CC20F8 or its subject.
        crate::guess_hit!();
        self.traj = (q, p);
        (self.velocity, self.angular_velocity) = (Vec3::ZERO, Quat::IDENTITY);
    }

    pub fn nis_active(&self) -> bool {
        matches!(self.phase, HeliPhase::Nis(_))
    }

    pub fn clip(&self) -> Option<(&ssx_assets::nis::NisActor, f32)> {
        let HeliPhase::Nis(scene) = self.phase else { return None };
        if let Some(c) = self.cabin {
            let sc = self.assets.nis.cabin.get(c.index)?;
            return Some((sc.scene.actor(ssx_assets::nis::ROLE_HELICOPTER1)?, sc.start + c.ticks));
        }
        let actor = self.assets.scene(scene).actor(ssx_assets::nis::ROLE_HELICOPTER1)?;
        Some((actor, self.assets.sequence_time(scene)))
    }

    pub fn posing_sides(&self) -> Vec<(f32, Option<Quat>, Option<f32>)> {
        let side = |c: Option<&MoveController>| (c.and_then(|c| c.posing.root), c.map(|c| c.ticks));
        let (root, noise) = side(self.ctrl.as_ref());
        match &self.blend {
            Some((old, _, elapsed)) => {
                let w = self.blend_weight(*elapsed);
                let (r0, n0) = side(old.as_ref());
                vec![(1.0 - w, r0, n0), (w, root, noise)]
            }
            None => vec![(1.0, root, noise)],
        }
    }

    pub fn posing_applied(&self) -> Option<f32> {
        self.ctrl.as_ref().map(|c| c.posing.applied)
    }
    pub fn posing_wanted(&self) -> Option<f32> {
        self.ctrl.as_ref().map(|c| c.posing.wanted_angle)
    }
    pub fn posing_axis(&self) -> Option<Vec3> {
        self.ctrl.as_ref().map(|c| c.posing.axis)
    }

    pub fn blending(&self) -> Option<(HeliNode, f32)> {
        let (_, node, elapsed) = self.blend.as_ref()?;
        Some((*node, self.blend_weight(*elapsed)))
    }

    fn blend_weight(&self, elapsed: f32) -> f32 {
        match (self.node, self.flight.as_deref()) {
            (HeliNode::IdleEnd { .. }, Some(f)) => {
                let (time, s) = &f.idle_end_blend;
                let t = if *time <= 0.0 { 1.0 } else { (elapsed / time).clamp(0.0, 1.0) };
                let x = t * s.len() as f32;
                let i = x as usize;
                if i >= s.len() {
                    1.0
                } else {
                    let next = s.get(i + 1).copied().unwrap_or(1.0);
                    let f = (x - i as f32).clamp(0.0, 1.0);
                    (1.0 - f) * s[i] + f * next
                }
            }
            _ => match (self.blend.as_ref().map(|b| b.1), self.flight.as_deref()) {
                (Some(HeliNode::Trick { seq, .. }), Some(f))
                    if f.tricks.get(seq).is_some_and(|t| t.end_blend.0 > 0.0) =>
                {
                    let (time, curve) = f.tricks[seq].end_blend;
                    1.0 - ssx_assets::anim_eval::curve_weight(elapsed / time, curve)
                }
                _ => 1.0 - ssx_assets::anim_eval::curve_weight(elapsed / DEFAULT_BLEND_TICKS, 0),
            },
        }
    }
}

fn nis_frame(i: &IntroState) -> (Quat, Vec3) {
    match i.current().actor(ssx_assets::nis::ROLE_HELICOPTER1) {
        Some(a) => i.toc_frame(a),
        None => (i.orientation, i.position + i.tuning),
    }
}

fn accumulate_delta(first: Option<(Vec3, Quat)>, last: Option<(Vec3, Quat)>) -> Option<(Vec3, Quat)> {
    match (first, last) {
        (None, x) | (x, None) => x,
        (Some((t1, q1)), Some((t2, q2))) => Some((t1 + q1 * t2, q2 * q1)),
    }
}

fn blend_delta(old: Option<(Vec3, Quat)>, new: Option<(Vec3, Quat)>, w: f32) -> Option<(Vec3, Quat)> {
    match (old, new) {
        (None, x) | (x, None) => x,
        (Some((da, qa)), Some((db, qb))) => {
            let qb = if qa.dot(qb) < 0.0 { -qb } else { qb };
            Some((da.lerp(db, w), qa.lerp(qb, w).normalize()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cells: Vec<f32>, w: usize, h: usize) -> HeightGrid {
        HeightGrid {
            min: Vec3::new(0.0, -100.0, 0.0),
            max: Vec3::new(10.0 * w as f32, 1000.0, 10.0 * h as f32),
            inv_cell: [0.1, 0.1],
            width: w,
            height: h,
            cells,
        }
    }

    #[test]
    fn height_grid_lookup_and_ray() {
        let mut cells = vec![0.0; 16];
        cells[2 * 4 + 1] = 50.0;
        let g = grid(cells, 4, 4);
        assert_eq!(g.height_at(Vec3::new(10.0, 0.0, 20.0)), 50.0);
        assert!((g.height_at(Vec3::new(15.0, 0.0, 20.0)) - 25.0).abs() < 1e-4);
        assert!((g.height_at(Vec3::new(10.0, 0.0, 25.0)) - 25.0).abs() < 1e-4);
        let hit = g.ray(Vec3::new(5.0, 10.0, 5.0), Vec3::new(18.0, 10.0, 28.0));
        assert_eq!(hit.map(|h| (h.x, h.y, h.z)), Some((15.0, 50.0, 25.0)));
        assert!(g.ray(Vec3::new(5.0, 60.0, 5.0), Vec3::new(18.0, 60.0, 28.0)).is_none());
    }

    #[test]
    fn spring_is_critically_damped() {
        let (mut x, mut v) = (0.0f32, 0.0f32);
        let mut max = 0.0f32;
        for _ in 0..400 {
            (x, v) = spring(0.05, x, v, 1.0, 2.0);
            max = max.max(x);
        }
        assert!(max <= 1.0 + 1e-5 && (x - 1.0).abs() < 1e-3);
        let (a, _) = spring(0.05, 0.2, 0.01, 1.0, 1.0);
        let (b, _) = spring(0.05, 0.2, 0.01, 1.0, 1.001);
        let (_, r) = spring(0.05, 0.2, 0.01, 1.0, 1.0);
        assert!(((b - a) / 0.001 - r).abs() < 1e-3);
    }

    #[test]
    fn pid_clamps_and_unwinds() {
        let mut p = Pid::default();
        let out = p.update([20.0, 0.1, 10.0], 2.0, Vec3::new(5.0, 0.0, 0.0), Vec3::ZERO);
        assert!((out.length() - 1.0).abs() < 1e-6);
        assert_eq!(p.integral, Vec3::ZERO);
        assert_eq!(p.update([20.0, 0.1, 10.0], 2.0, Vec3::ZERO, Vec3::ZERO), -Vec3::X);
    }
}
