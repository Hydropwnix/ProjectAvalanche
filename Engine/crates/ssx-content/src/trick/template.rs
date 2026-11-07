use crate::anim::{Clip, Interpolation, JointTrack, Keys, Path};
use crate::model::write_skinned::WriteJoint;

pub const NAME: &str = "air_walk";
pub const ANIMATION: &str = "air_walk";
pub const GLTF_PATH: &str = "anims/air_walk.glb";
pub const ANIM_PATH: &str = "anims/air_walk.toml";
pub const TRICK_PATH: &str = "tricks/air_walk.toml";

const KEYS: u32 = 20;
const KEY_RATE: f32 = 20.0;

pub const ANIM_TOML: &str = "\
# The trick's clip: the joint tracks of the animation `air_walk` of anims/air_walk.glb (bound by the joints' names).
# Not additive: it stands in the Indy's clips, which are not.
[anim]
gltf = \"anims/air_walk.glb\"
animation = \"air_walk\"
";

pub fn trick_toml(ns: &str) -> String {
    let mut clips = String::new();
    for (role, _) in super::ROLES {
        clips.push_str(&format!("{role} = \"{ns}:anim/{NAME}\"\n"));
    }
    format!(
        "# An added trick: it plays in the places of the disc grab `base` (its sequences, timing, blends and landing \
         windows),\n# with your clips in the base's 12 clip places (the same clip may fill several). `input` is one the \
         disc leaves\n# free: up-up, left-left or right-right (Y Y, X X, B B or the right stick twice; the goofy stance's \
         is the mirror).\n# `points` are its TRICK_POINTS (1 to 2400; the Indy's are 1000); the names are string ids \
         (strings/en.toml).\n[trick]\nbase = \"indy\"\ninput = \"up-up\"\nhand = \"both\"\npoints = 1300\n\
         name = \"{ns}:string/{NAME}\"\nname_uber = \"{ns}:string/{NAME}_uber\"\n\
         name_super_uber = \"{ns}:string/{NAME}_super_uber\"\n\n[trick.clips]\n{clips}"
    )
}

pub const STRINGS_EN: &str = "\
# The package's own strings, looked up by their ids \"<package>:string/<key>\".
air_walk = \"Air Walk\"
air_walk_uber = \"Über Air Walk\"
air_walk_super_uber = \"Super Über Air Walk\"
";

const POSE: [(&str, &str, [f32; 3], f32, f32); 6] = [
    ("Spine2", "Spine1", [1.0, 0.0, 0.0], 0.10, 0.05),
    ("Head", "Neck1", [0.0, 1.0, 0.0], 0.20, 0.10),
    ("LeftArm", "LeftShoulder", [0.0, 0.0, 1.0], 0.60, 0.25),
    ("RightArm", "RightShoulder", [0.0, 0.0, 1.0], -0.60, -0.25),
    ("LeftForeArm", "LeftArm", [0.0, 1.0, 0.0], 0.30, 0.15),
    ("RightForeArm", "RightArm", [0.0, 1.0, 0.0], -0.30, -0.15),
];

fn wave(u: f32) -> f32 {
    let p = 2.0 * u - 1.0;
    4.0 * p * (1.0 - p.abs())
}

pub fn clip() -> Clip {
    let times: Vec<f32> = (0..=KEYS).map(|k| k as f32 / KEY_RATE).collect();
    let joints = POSE
        .iter()
        .map(|&(joint, parent, axis, centre, amp)| {
            let mut values = Vec::with_capacity(times.len() * 4);
            for k in 0..=KEYS {
                let s = centre + amp * wave(k as f32 / KEYS as f32);
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
    Clip { joints, floats: Vec::new(), additive: false, duration: KEYS as f32 / KEY_RATE }
}

pub fn joints() -> Vec<WriteJoint> {
    let mut out: Vec<WriteJoint> = Vec::new();
    let node = |name: &str| WriteJoint {
        name: name.into(),
        parent: None,
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
    for &(joint, parent, ..) in &POSE {
        let p = match out.iter().position(|j| j.name == parent) {
            Some(p) => p,
            None => {
                out.push(node(parent));
                out.len() - 1
            }
        };
        match out.iter().position(|j| j.name == joint) {
            Some(i) => out[i].parent = Some(p),
            None => out.push(WriteJoint { parent: Some(p), ..node(joint) }),
        }
    }
    out
}

pub fn glb() -> Result<Vec<u8>, String> {
    crate::anim::write::glb(&joints(), ANIMATION, &clip())
}
