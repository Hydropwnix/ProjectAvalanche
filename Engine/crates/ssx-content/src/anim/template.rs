use super::{Clip, Interpolation, JointTrack, Keys, Path};
use crate::model::write_skinned::WriteJoint;

pub const TARGET: &str = "ssx:anim/anim_all/carv_regular_noise";
pub const ANIMATION: &str = "carve_sway";
pub const GLTF_PATH: &str = "anims/carve_sway.glb";

const KEYS: u32 = 60;
const KEY_RATE: f32 = 20.0;
const PERIOD: u32 = 30;

pub const ANIM_TOML: &str = "\
# A clip of the riders' skeleton (`ssx:anim/…`): it replaces the disc clip its id in resource.toml names, in every
# graph that plays it. `gltf` names the file whose animation gives the joints' tracks (bound by the joints' names);
# `additive` says its keys are offsets from the pose it is layered on (it must match the clip it replaces: the carve
# noise is additive). Float DOFs (fingers, face, the trajectory's height) go in [[anim.float]] tables:
#   [[anim.float]]
#   dof = \"TrajectoryVelocityRig.AITrajectory.height\"
#   interpolation = \"linear\"    # step, linear or cubic
#   times = [0.0, 3.0]           # seconds, strictly increasing
#   values = [0.0, 0.0]
[anim]
gltf = \"anims/carve_sway.glb\"
animation = \"carve_sway\"
additive = true
";

const SWAY: [(&str, &str, [f32; 3], f32, u32); 5] = [
    ("Spine2", "Spine1", [0.0, 1.0, 0.0], 0.04, 0),
    ("Neck", "Spine3", [1.0, 0.0, 0.0], 0.06, 8),
    ("Head", "Neck1", [0.0, 0.0, 1.0], 0.05, 15),
    ("LeftArm", "LeftShoulder", [0.0, 0.0, 1.0], 0.10, 0),
    ("RightArm", "RightShoulder", [0.0, 0.0, 1.0], -0.10, 0),
];

fn wave(u: f32) -> f32 {
    let p = 2.0 * u - 1.0;
    4.0 * p * (1.0 - p.abs())
}

pub fn clip() -> Clip {
    let times: Vec<f32> = (0..=KEYS).map(|k| k as f32 / KEY_RATE).collect();
    let joints = SWAY
        .iter()
        .map(|&(joint, parent, axis, amp, phase)| {
            let mut values = Vec::with_capacity(times.len() * 4);
            for k in 0..=KEYS {
                let u = ((k + phase) % PERIOD) as f32 / PERIOD as f32;
                let s = amp * wave(u);
                let q = [axis[0] * s, axis[1] * s, axis[2] * s, 1.0];
                let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
                values.extend(q.map(|x| x / len));
            }
            JointTrack {
                joint: joint.into(),
                path: Path::Rotation,
                keys: Keys { interpolation: Interpolation::Linear, times: times.clone(), values },
                parent: Some(parent.into()),
            }
        })
        .collect();
    Clip { joints, floats: Vec::new(), additive: true, duration: KEYS as f32 / KEY_RATE }
}

pub fn joints() -> Vec<WriteJoint> {
    let mut out = Vec::new();
    for &(joint, parent, ..) in &SWAY {
        let at = out.len();
        for (name, p) in [(parent, None), (joint, Some(at))] {
            out.push(WriteJoint {
                name: name.into(),
                parent: p,
                translation: [0.0; 3],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            });
        }
    }
    out
}

pub fn glb() -> Result<Vec<u8>, String> {
    super::write::glb(&joints(), ANIMATION, &clip())
}
