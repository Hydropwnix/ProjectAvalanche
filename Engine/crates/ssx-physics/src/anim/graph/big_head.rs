use anyhow::{Context, Result, ensure};
use ssx_assets::anim_db::AssetDb;
use ssx_assets::character::Srt;
use ssx_assets::rig_anim::RIG_BUNDLE;
use ssx_formats::crsf::SceneGraph;

const OP: &str = "PlayerBigHeadSceneOp";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BigHeadScales {
    pub head: f32,
    pub hands: f32,
}

impl Default for BigHeadScales {
    fn default() -> Self {
        Self { head: 1.0, hands: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BigHeadOp {
    head: Option<usize>,
    left_hand: Option<usize>,
    right_hand: Option<usize>,
}

impl BigHeadOp {
    pub fn load(db: &AssetDb, rig: &SceneGraph) -> Result<Self> {
        let all = &db.streams.iter().find(|(n, _)| n == RIG_BUNDLE).with_context(|| format!("no {RIG_BUNDLE}"))?.1;
        let rider = ssx_assets::nis::Rig::load(all, ssx_assets::nis::RIDER_RIG)?;
        let op = db.by_name(OP).with_context(|| format!("no {OP}"))?;
        for (field, name) in [("BigHeadScaleAsset", "FLT_BigHeadScale"), ("BigHandsScaleAsset", "FLT_BigHandsScale")] {
            let asset = op.guid(field).and_then(|g| db.get(g)).with_context(|| format!("{OP}.{field}: no asset"))?;
            ensure!(asset.name() == name, "{OP}.{field} is {:?}, not {name}", asset.name());
        }
        let joint = |field: &str| {
            let id = db.get(op.guid(field)?)?;
            rig.find(&ssx_assets::nis::template_joint(db, &rider, &id)?)
        };
        Ok(Self {
            head: joint("BigHeadJointAsset"),
            left_hand: joint("BigLeftHandJointAsset"),
            right_hand: joint("BigRightHandJointAsset"),
        })
    }

    pub fn joints(&self) -> [Option<usize>; 3] {
        [self.head, self.left_hand, self.right_hand]
    }

    pub fn apply(&self, nodes: &mut [Srt], scales: BigHeadScales) {
        let joints = [(self.head, scales.head), (self.left_hand, scales.hands), (self.right_hand, scales.hands)];
        for (j, v) in joints {
            if let Some(n) = j.and_then(|j| nodes.get_mut(j)) {
                n.scale = [v; 3];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(s: f32, x: f32) -> Srt {
        Srt { scale: [s; 3], rotation: [0.0, 0.0, 0.6, 0.8], translation: [x, 1.0, 2.0] }
    }

    #[test]
    fn the_joints_scale_is_replaced() {
        let op = BigHeadOp { head: Some(1), left_hand: Some(2), right_hand: Some(3) };
        let mut nodes = vec![node(1.0, 0.0), node(0.5, 1.0), node(1.0, 2.0), node(1.0, 3.0), node(0.7, 4.0)];
        op.apply(&mut nodes, BigHeadScales { head: 2.0, hands: 3.0 });
        let scales: Vec<f32> = nodes.iter().map(|n| n.scale[0]).collect();
        assert_eq!(scales, [1.0, 2.0, 3.0, 3.0, 0.7]);
        assert!(nodes.iter().all(|n| n.scale[1] == n.scale[0] && n.scale[2] == n.scale[0]));
        assert!(nodes.iter().enumerate().all(|(i, n)| n.translation[0] == i as f32 && n.rotation[3] == 0.8));
    }

    #[test]
    fn the_default_stores_one() {
        let op = BigHeadOp { head: Some(0), left_hand: None, right_hand: Some(1) };
        let mut nodes = vec![node(0.5, 0.0), node(1.5, 1.0), node(0.5, 2.0)];
        op.apply(&mut nodes, BigHeadScales::default());
        let scales: Vec<f32> = nodes.iter().map(|n| n.scale[0]).collect();
        assert_eq!(scales, [1.0, 1.0, 0.5]);
    }

    #[test]
    fn missing_joints_are_skipped() {
        let op = BigHeadOp { head: None, left_hand: Some(7), right_hand: Some(0) };
        let mut nodes = vec![node(1.0, 0.0)];
        op.apply(&mut nodes, BigHeadScales { head: 2.0, hands: 2.0 });
        assert_eq!(nodes[0].scale, [2.0; 3]);
    }
}
