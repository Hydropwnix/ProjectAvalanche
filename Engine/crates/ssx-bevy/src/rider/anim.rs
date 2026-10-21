use bevy::math::{Quat, Vec3, Vec4};
use ssx_assets::anim_eval::Animator;
use ssx_assets::anim_rigops::{Compression, LegRigOps};
use ssx_assets::character::Srt;
use ssx_assets::rig_anim::RigPose;
use ssx_assets::rigamate::RigamateRig;
use ssx_formats::crsf::SceneGraph;
use ssx_physics::curve::Curve;
use ssx_physics::graph::RiderGraph;
use ssx_physics::rider::RiderState;

pub use ssx_physics::graph::CarveAnimTuning;

#[derive(Clone, Copy, Debug)]
pub struct AnimInput<'a> {
    pub state: &'a RiderState,
    pub frac: f32,
    pub roots: (Vec3, Vec3),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointRecord {
    pub s: Vec3,
    pub q: Quat,
    pub p: Vec3,
}

impl JointRecord {
    fn compose(parent: &JointRecord, local: &Srt) -> JointRecord {
        let q = Quat::from_array(local.rotation).normalize();
        let (s, t) = (Vec3::from_array(local.scale), Vec3::from_array(local.translation));
        JointRecord { s: parent.s * s, q: (parent.q * q).normalize(), p: parent.p + parent.q * (parent.s * t) }
    }
}

pub fn world_records(rig: &SceneGraph, nodes: &[Srt], root: Vec3) -> Vec<JointRecord> {
    let origin = JointRecord { s: Vec3::ONE, q: Quat::IDENTITY, p: root };
    let mut out: Vec<JointRecord> = Vec::with_capacity(nodes.len());
    for (node, srt) in rig.nodes.iter().zip(nodes) {
        let parent = node.parent.map_or(origin, |p| out[p]);
        out.push(JointRecord::compose(&parent, srt));
    }
    out
}

pub fn interpolate_records(a: &[JointRecord], b: &[JointRecord], frac: f32, out: &mut Vec<JointRecord>) {
    out.clear();
    if a.len() != b.len() {
        out.extend_from_slice(if frac < 0.5 { a } else { b });
        return;
    }
    out.extend(a.iter().zip(b).map(|(a, b)| {
        let qb = if a.q.dot(b.q) > 0.0 { b.q } else { -b.q };
        JointRecord {
            s: a.s.lerp(b.s, frac),
            q: Quat::from_vec4(Vec4::from(a.q).lerp(Vec4::from(qb), frac)).normalize(),
            p: a.p.lerp(b.p, frac),
        }
    }));
}

pub fn player_speed(s: &RiderState, in_air: bool) -> f32 {
    let _ = in_air;
    s.player_speed
}

fn wrap_pi(x: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let y = (x + std::f32::consts::PI).rem_euclid(t) - std::f32::consts::PI;
    if y <= -std::f32::consts::PI { y + t } else { y }
}

pub struct RiderAnimation {
    graph: RiderGraph,
    rigamate: Option<(RigamateRig, Vec<Option<usize>>)>,
    legs: Option<LegRigOps>,
    rig: SceneGraph,
    lean_curve: Curve,
    prev: RigPose,
    curr: RigPose,
    render: Vec<JointRecord>,
    bind_records: Vec<JointRecord>,
    player_speed: f32,
    posed: u64,
}

pub struct LoadedAnimation {
    pub anim: RiderAnimation,
    pub rig: SceneGraph,
    pub bind: Vec<Srt>,
    pub problems: Vec<String>,
}

impl RiderAnimation {
    pub fn load(
        big_root: &std::path::Path,
        character: &ssx_assets::character::Character,
        name: &str,
    ) -> anyhow::Result<LoadedAnimation> {
        use anyhow::Context;
        use ssx_assets::rig_anim::{ANIM_DIR, RIG_BUNDLE, float_dofs};
        let mut problems = Vec::new();
        let (rig, bind, _) =
            ssx_assets::anim_rigops::extend_character_rig(big_root, &character.rig, &character.bind_pose())?;
        rig.find("AITrajectory").context("the rig has no AITrajectory node")?;
        let floats = float_dofs(big_root)?;
        let rigamate = match RigamateRig::load(big_root, &rig, &bind, name) {
            Ok((r, p)) => {
                problems.extend(p);
                Some(r)
            }
            Err(e) => {
                problems.push(format!("rigamate not loaded: {e:#}"));
                None
            }
        };
        let own = format!("{name}.cba");
        let anim_dir = big_root.join(ANIM_DIR);
        let bundles: Vec<&str> =
            if anim_dir.join(&own).exists() { vec![RIG_BUNDLE, own.as_str()] } else { vec![RIG_BUNDLE] };
        let db = ssx_assets::anim_db::AssetDb::load(&anim_dir, &bundles)?;
        let animator = Animator::new(&db, ssx_physics::graph::RIDER_GRAPH_ROOT, &rig, bind.clone(), &floats)?;
        let legs = LegRigOps::new(&rig, &floats);
        if legs.is_none() {
            problems.push("leg rig ops: the rig lacks a leg or effector joint".into());
        }
        let curve = db
            .by_name("curve_traj_height_lean_modifier_blend_curve")
            .map(|c| {
                c.array("RuntimeControlPoints")
                    .iter()
                    .filter_map(|v| match v {
                        ssx_formats::ant::Value::Vector2(p) => Some(*p),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let carve = CarveAnimTuning::load(&db)?;
        let level_start = match ssx_physics::graph::LevelStartGraph::load(&db, &rig, bind.clone(), &floats) {
            Ok(s) => Some(s),
            Err(e) => {
                problems.push(format!("level start graph not loaded: {e:#}"));
                None
            }
        };
        let scripts = ssx_physics::anim_lua::AnimScripts::from_db(&db)?;
        let mut anim =
            RiderAnimation::new(animator, rigamate, legs, rig.clone(), Curve::new(curve), carve, &floats, scripts);
        if let Some(level_start) = level_start {
            anim.set_level_start(level_start);
        }
        match ssx_physics::graph::RewindAssets::load(&db, &rig, bind.clone(), &floats) {
            Ok(r) => anim.graph_mut().set_rewind(r),
            Err(e) => problems.push(format!("rewind graph not loaded: {e:#}")),
        }
        Ok(LoadedAnimation { anim, rig, bind, problems })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        animator: Animator,
        rigamate: Option<RigamateRig>,
        legs: Option<LegRigOps>,
        rig: SceneGraph,
        lean_curve: Curve,
        carve: CarveAnimTuning,
        floats: &[String],
        scripts: ssx_physics::anim_lua::AnimScripts,
    ) -> Self {
        let rigamate = rigamate.map(|r| {
            let slots = r.float_names.iter().map(|n| floats.iter().position(|f| f == n)).collect();
            (r, slots)
        });
        let graph = RiderGraph::new(animator, rig.clone(), floats, carve, scripts);
        let bind = graph.pose().clone();
        let bind_records = world_records(&rig, &bind.nodes, Vec3::ZERO);
        Self {
            graph,
            rigamate,
            legs,
            rig,
            lean_curve,
            prev: bind.clone(),
            render: Vec::new(),
            bind_records,
            curr: bind,
            player_speed: 0.0,
            posed: 0,
        }
    }

    pub fn set_level_start(&mut self, start: ssx_physics::graph::LevelStartGraph) {
        self.graph.set_level_start(start);
    }

    pub fn graph_mut(&mut self) -> &mut RiderGraph {
        &mut self.graph
    }

    pub fn reset(&mut self) {
        self.graph.reset();
        if let Some((rm, _)) = &mut self.rigamate {
            rm.reset_history();
        }
        self.posed = 0;
    }

    pub fn clip_lines(&self) -> Vec<String> {
        let a = self.graph.animator();
        let mut v = vec![format!(
            "graph SF_Ride: {} controllers, {} signals, {} clips bound",
            a.graph.ctrls.len(),
            a.graph.signals.len(),
            a.clip_count()
        )];
        v.extend(a.problems.iter().map(|p| format!("clip problem: {p}")));
        if let Some((r, _)) = &self.rigamate {
            v.push(format!("rigamate: {}", r.summary()));
        }
        v
    }
}

impl RiderAnimation {
    pub fn after_step(&mut self, s: &RiderState) {
        self.player_speed = s.player_speed;
        let mut pose = self.graph.pose().clone();
        if let Some(legs) = &mut self.legs {
            let riding = self.graph.path().first().is_some_and(|n| n == "state_riding");
            let pitch = wrap_pi(s.landing.pitch).abs();
            let reach = if s.landing.bail_on_landing {
                0.0
            } else if pitch <= std::f32::consts::FRAC_PI_2 {
                1.0
            } else {
                pitch.sin()
            };
            let c = s.kin.lean.abs().cos().max(0.0);
            let compression = riding.then_some(Compression {
                riding_height: s.carving.riding_height,
                lean_modifier: c + (1.0 - c) * self.lean_curve.eval(s.landing.stable_ticks),
                chatter: s.carving.chatter_height,
                reach_value: 1.0,
                reach,
                ground_up: s.ground_normal.to_array(),
            });
            let placement = s.board_particle.placement;
            legs.apply_with(&self.rig, &mut pose.nodes, &pose.floats, compression, placement.is_some());
            if let Some((p, q)) = placement {
                let t = p - (s.traj - s.correction_shift);
                legs.place_board(&self.rig, &mut pose.nodes, q.to_array(), [t.x, t.y, t.z, 0.0]);
            }
        }
        if let Some((rm, slots)) = &mut self.rigamate {
            let floats: Vec<f32> = slots.iter().map(|s| s.map_or(0.0, |i| pose.floats[i])).collect();
            rm.apply(&mut pose.nodes, &floats);
        }
        self.prev = if self.posed == 0 { pose.clone() } else { std::mem::replace(&mut self.curr, pose.clone()) };
        self.curr = pose;
        self.posed += 1;
    }

    pub fn update(&mut self, a: &AnimInput) -> &[JointRecord] {
        if self.posed == 0 {
            self.after_step(a.state);
        }
        let a_records = world_records(&self.rig, &self.prev.nodes, a.roots.0);
        let b_records = world_records(&self.rig, &self.curr.nodes, a.roots.1);
        interpolate_records(&a_records, &b_records, a.frac.clamp(0.0, 1.0), &mut self.render);
        &self.render
    }

    #[allow(dead_code)]
    pub fn posed(&self) -> (&RigPose, &SceneGraph, Option<[ssx_assets::anim_rigops::Effector; 2]>) {
        (&self.curr, &self.rig, self.legs.as_ref().map(|l| l.effectors))
    }

    pub fn drawn_joint(&self, name: &str) -> Option<JointRecord> {
        self.rig.find(name).and_then(|i| self.render.get(i).copied())
    }

    pub fn drawn_axis(&self, name: &str, axis: Vec3) -> Option<Vec3> {
        let i = self.rig.find(name)?;
        Some(self.render.get(i)?.q * (self.bind_records.get(i)?.q.inverse() * axis))
    }

    pub fn graph(&self) -> &RiderGraph {
        &self.graph
    }

    pub fn player_speed(&self) -> f32 {
        self.player_speed
    }

    pub fn trajectory_joint(&self) -> Option<usize> {
        self.graph.trajectory_joint()
    }

    pub fn label(&self) -> String {
        let a = self.graph.animator();
        let clips: Vec<String> = a.playing().iter().take(4).map(|(n, w)| format!("{n} {w:.2}")).collect();
        let (angle, board_move) = self.graph.carving_outputs();
        format!(
            "anim: {} | {} | angle to ground {:+.2}, board move {:+.2} (fwd {:+.0})",
            a.path().join(" > "),
            clips.join(", "),
            angle,
            board_move,
            if self.graph.forward() { 1.0 } else { -1.0 }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(q: Quat, p: Vec3) -> JointRecord {
        JointRecord { s: Vec3::ONE, q, p }
    }

    #[test]
    fn records_interpolate_like_the_pose_block() {
        let qa = Quat::from_rotation_y(0.2);
        let qb = -Quat::from_rotation_y(0.6);
        let a = [rec(qa, Vec3::new(0.0, 1.0, 0.0))];
        let b = [rec(qb, Vec3::new(2.0, 1.0, 0.0))];
        let mut out = Vec::new();
        interpolate_records(&a, &b, 0.5, &mut out);
        assert!(out[0].q.dot(Quat::from_rotation_y(0.4)) > 1.0 - 1e-6);
        assert!((out[0].p - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-6);
        let b2 = [b[0], b[0]];
        interpolate_records(&a, &b2, 0.49, &mut out);
        assert_eq!(out, a);
        interpolate_records(&a, &b2, 0.5, &mut out);
        assert_eq!(out, b2);
    }

    #[test]
    fn world_records_compose_from_the_root() {
        let parent =
            JointRecord { s: Vec3::splat(2.0), q: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), p: Vec3::X };
        let local = Srt { scale: [1.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], translation: [1.0, 0.0, 0.0] };
        let w = JointRecord::compose(&parent, &local);
        assert!((w.p - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-6);
        assert_eq!(w.s, Vec3::splat(2.0));
    }
}
