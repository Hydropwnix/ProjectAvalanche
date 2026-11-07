use std::sync::Arc;

use ssx_content::ContentId;
use ssx_content::anim::{Clip, FloatTrack, Interpolation, JointTrack, Keys, Path};
use ssx_formats::ant_anim::{Dof, DofType};
use ssx_formats::crsf::{NodeKind, SceneGraph, SceneNode};

use super::{Replaced, bind};

fn node(name: &str, parent: Option<usize>) -> SceneNode {
    SceneNode {
        parent,
        child_count: 0,
        name: name.into(),
        unk_a: 0,
        flags: 0,
        attributes: None,
        unk_group_flag: 0,
        kind: NodeKind::Group,
    }
}

fn rig() -> (SceneGraph, Vec<Dof>) {
    let sg = SceneGraph { nodes: vec![node("AITrajectory", None), node("Hips", Some(0)), node("Spine", Some(1))] };
    let dof = |layout: &str, name: &str, ty| Dof { layout: layout.into(), name: name.into(), ty };
    let dofs = vec![
        dof("Torso", "Spine.q", DofType::Quaternion),
        dof("Hips", "Hips.q", DofType::Quaternion),
        dof("Hips", "Hips.t", DofType::Translation),
        dof("TrajectoryVelocityRig", "AITrajectory.height", DofType::Float),
    ];
    (sg, dofs)
}

fn track(joint: &str, path: Path, parent: Option<&str>, times: Vec<f32>, values: Vec<f32>) -> JointTrack {
    JointTrack {
        joint: joint.into(),
        path,
        keys: Keys { interpolation: Interpolation::Linear, times, values },
        parent: parent.map(str::to_string),
    }
}

fn clip(joints: Vec<JointTrack>, floats: Vec<FloatTrack>, additive: bool) -> Arc<Clip> {
    let duration = joints
        .iter()
        .map(|t| &t.keys)
        .chain(floats.iter().map(|f| &f.keys))
        .filter_map(|k| k.times.last().copied())
        .fold(0.0, f32::max);
    Arc::new(Clip { joints, floats, additive, duration })
}

fn id() -> ContentId {
    "ssx:anim/anim_all/test".parse().expect("an id")
}

#[test]
fn channels_are_ordered_as_the_codecs_order_them_and_sampled_in_seconds_of_the_controller() {
    let (sg, dofs) = rig();
    let height = FloatTrack {
        dof: "TrajectoryVelocityRig.AITrajectory.height".into(),
        keys: Keys { interpolation: Interpolation::Linear, times: vec![0.0, 1.0], values: vec![0.0, 2.0] },
    };
    let c = clip(
        vec![
            track("Hips", Path::Translation, Some("AITrajectory"), vec![0.0, 1.0], vec![0.0, 0.0, 0.0, 6.0, 0.0, 0.0]),
            track("Hips", Path::Rotation, Some("AITrajectory"), vec![0.0], vec![0.0, 0.0, 0.0, 1.0]),
            track("Spine", Path::Rotation, Some("Hips"), vec![0.0], vec![0.0, 1.0, 0.0, 0.0]),
        ],
        vec![height],
        false,
    );
    let id = id();
    let r = Replaced { id: &id, end_frame: 30, additive: false, frame_rate: 30.0 };
    let m = bind(c, "pkg", r, &dofs, &sg).expect("it binds");
    assert_eq!(m.channel_to_dof, vec![0, 1, 2, 3]);
    let p = m.sample(15.0);
    assert_eq!(p.quats, vec![[0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);
    assert_eq!(p.vec3s, vec![[3.0, 0.0, 0.0]]);
    assert_eq!(p.floats, vec![1.0]);
    assert_eq!(m.sample(300.0).vec3s, vec![[6.0, 0.0, 0.0]]);
    assert_eq!(super::ClipData::Mod(Arc::new(m)).end_frame(), 30);
}

#[test]
fn what_the_rig_cannot_play_is_refused_with_the_track() {
    let (sg, dofs) = rig();
    let id = id();
    let r = Replaced { id: &id, end_frame: 30, additive: false, frame_rate: 60.0 };
    let q = || vec![0.0, 0.0, 0.0, 1.0];
    let cases: Vec<(Arc<Clip>, &str)> = vec![
        (clip(vec![track("Spine", Path::Rotation, Some("Hips"), vec![0.0], q())], vec![], true), "additive = true"),
        (
            clip(vec![track("Head", Path::Rotation, Some("Hips"), vec![0.0], q())], vec![], false),
            "joint \"Head\" rotation: no DOF",
        ),
        (
            clip(vec![track("Spine", Path::Translation, Some("Hips"), vec![0.0], vec![0.0; 3])], vec![], false),
            "the rig drives Spine.q of this joint, not its translation",
        ),
        (
            clip(vec![track("Hips", Path::Scale, Some("AITrajectory"), vec![0.0], vec![1.0; 3])], vec![], false),
            "not its scale",
        ),
        (
            clip(vec![track("Spine", Path::Rotation, Some("AITrajectory"), vec![0.0], q())], vec![], false),
            "its parent node in the glTF is \"AITrajectory\", the skeleton's is \"Hips\"",
        ),
        (
            clip(vec![track("Spine", Path::Rotation, None, vec![0.0], q())], vec![], false),
            "is none, the skeleton's is \"Hips\"",
        ),
        (
            clip(
                vec![],
                vec![FloatTrack {
                    dof: "AITrajectory.height".into(),
                    keys: Keys { interpolation: Interpolation::Linear, times: vec![0.0], values: vec![0.0] },
                }],
                false,
            ),
            "[[anim.float]] 1: dof \"AITrajectory.height\" is no float DOF",
        ),
    ];
    for (c, want) in cases {
        let e = bind(c, "pkg", r, &dofs, &sg).expect_err("refused");
        assert!(e.contains(want), "{e:?} lacks {want:?}");
    }
    let bad_rate = Replaced { frame_rate: 0.0, ..r };
    let ok = clip(vec![track("Spine", Path::Rotation, Some("Hips"), vec![0.0], q())], vec![], false);
    assert!(bind(ok, "pkg", bad_rate, &dofs, &sg).is_err());
}
