use super::*;
use ssx_assets::anim_rigops::{Compression, LegRigOps};
use ssx_assets::rigamate::RigamateRig;

pub struct RiderRigOps {
    legs: Option<LegRigOps>,
    lean_curve: crate::curve::Curve,
    rigamate: Option<(RigamateRig, Vec<Option<usize>>)>,
}

impl RiderRigOps {
    pub fn new(
        legs: Option<LegRigOps>,
        lean_curve: crate::curve::Curve,
        rigamate: Option<RigamateRig>,
        floats: &[String],
    ) -> Self {
        let rigamate = rigamate.map(|r| {
            let slots = r.float_names.iter().map(|n| floats.iter().position(|f| f == n)).collect();
            (r, slots)
        });
        Self { legs, lean_curve, rigamate }
    }

    pub fn load(big_root: &Path, track: &str, name: &str, graph: &RiderGraph) -> Result<(Self, Vec<String>)> {
        let character = ssx_assets::character::load_character(big_root, track, name)?;
        Self::of_character(big_root, name, &character, graph)
    }

    pub fn load_untracked(big_root: &Path, name: &str, graph: &RiderGraph) -> Result<(Self, Vec<String>)> {
        let loadout = ssx_assets::character::Loadout::default();
        let character = ssx_assets::character::load_character_untracked(big_root, name, &loadout)?;
        Self::of_character(big_root, name, &character, graph)
    }

    fn of_character(
        big_root: &Path,
        name: &str,
        character: &ssx_assets::character::Character,
        graph: &RiderGraph,
    ) -> Result<(Self, Vec<String>)> {
        use ssx_assets::rig_anim::{ANIM_DIR, RIG_BUNDLE, float_dofs};
        let (rig, bind, _) =
            ssx_assets::anim_rigops::extend_character_rig(big_root, &character.rig, &character.bind_pose())?;
        anyhow::ensure!(rig.nodes.len() == graph.rig.nodes.len(), "the rig ops' rig is not the graph's");
        let floats = float_dofs(big_root)?;
        let legs = LegRigOps::new(&rig, &floats).context("the rig lacks a leg or effector joint")?;
        let db = ssx_assets::anim_db::AssetDb::load(&big_root.join(ANIM_DIR), &[RIG_BUNDLE])?;
        let points = db
            .by_name("curve_traj_height_lean_modifier_blend_curve")
            .context("no curve_traj_height_lean_modifier_blend_curve")?
            .array("RuntimeControlPoints")
            .iter()
            .filter_map(|v| match v {
                ssx_formats::ant::Value::Vector2(p) => Some(*p),
                _ => None,
            })
            .collect();
        let (rm, problems) = RigamateRig::load(big_root, &rig, &bind, name)?;
        Ok((Self::new(Some(legs), crate::curve::Curve::new(points), Some(rm), &floats), problems))
    }

    pub fn reset(&mut self) {
        if let Some((rm, _)) = &mut self.rigamate {
            rm.reset_history();
        }
    }

    pub fn effectors(&self) -> Option<[ssx_assets::anim_rigops::Effector; 2]> {
        self.legs.as_ref().map(|l| l.effectors)
    }

    pub fn rigamate_summary(&self) -> Option<String> {
        self.rigamate.as_ref().map(|(r, _)| r.summary())
    }

    pub fn apply(&mut self, s: &RiderState, g: &RiderGraph) -> RigPose {
        let mut pose = g.pose().clone();
        if let Some(legs) = &mut self.legs {
            let riding = g.path().first().is_some_and(|n| n == "state_riding");
            let pitch = wrap_pi(s.landing.pitch).abs();
            let reach = if s.landing.bail_on_landing {
                0.0
            } else if pitch <= FRAC_PI_2 {
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
            legs.apply_with(&g.rig, &mut pose.nodes, &pose.floats, compression, placement.is_some());
            if let Some((p, q)) = placement {
                let t = p - (s.traj - s.correction_shift);
                legs.place_board(&g.rig, &mut pose.nodes, q.to_array(), [t.x, t.y, t.z, 0.0]);
            }
        }
        if let Some((rm, slots)) = &mut self.rigamate {
            let floats: Vec<f32> = slots.iter().map(|s| s.map_or(0.0, |i| pose.floats[i])).collect();
            rm.apply(&mut pose.nodes, &floats);
        }
        pose
    }
}
