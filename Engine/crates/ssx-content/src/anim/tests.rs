use std::collections::BTreeMap;

use super::*;
use crate::model::write_skinned::WriteJoint;

struct Files(BTreeMap<String, Vec<u8>>);

impl FileReader for Files {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        if b.len() as u64 > max { Err(format!("{path}: over {max} bytes")) } else { Ok(b.clone()) }
    }

    fn size(&self, path: &str) -> Result<u64, String> {
        Ok(self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?.len() as u64)
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        let (o, l) =
            (usize::try_from(offset).map_err(|e| e.to_string())?, usize::try_from(len).map_err(|e| e.to_string())?);
        b.get(o..o.saturating_add(l)).map(<[u8]>::to_vec).ok_or_else(|| format!("{path}: short"))
    }
}

fn read(toml: &str, glb: Option<Vec<u8>>) -> Result<Clip, String> {
    let mut files = BTreeMap::new();
    if let Some(g) = glb {
        files.insert("a.glb".to_string(), g);
    }
    read_anim(toml.as_bytes(), "anim.toml", &Files(files))
        .map_err(|e| e.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n"))
}

const GLTF: &str = "[anim]\ngltf = \"a.glb\"\n";

fn joint(name: &str, parent: Option<usize>) -> WriteJoint {
    WriteJoint { name: name.into(), parent, translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
}

fn track(joint: &str, path: Path, times: Vec<f32>, values: Vec<f32>) -> JointTrack {
    JointTrack {
        joint: joint.into(),
        path,
        keys: Keys { interpolation: Interpolation::Linear, times, values },
        parent: None,
    }
}

fn glb_of(tracks: Vec<JointTrack>) -> Vec<u8> {
    let clip = Clip { joints: tracks, floats: Vec::new(), additive: false, duration: 0.0 };
    write::glb(&[joint("Hips", None), joint("Spine", Some(0))], "x", &clip).expect("written")
}

#[test]
fn the_template_reads_back_as_made_and_its_committed_copy_is_the_generated_one() {
    let made = template::clip();
    let glb = template::glb().expect("the template is written");
    let mut files = BTreeMap::new();
    files.insert(template::GLTF_PATH.to_string(), glb.clone());
    let back = read_anim(template::ANIM_TOML.as_bytes(), ANIM_FILE, &Files(files)).expect("the template reads");
    assert_eq!(back.joints.len(), made.joints.len());
    for (a, b) in back.joints.iter().zip(&made.joints) {
        assert_eq!((&a.joint, a.path, &a.keys), (&b.joint, b.path, &b.keys));
        assert_eq!(a.parent, b.parent, "the glTF parents are the skeleton's");
    }
    assert!(back.additive && (back.duration - 3.0).abs() < 1e-6 && back.floats.is_empty());
    assert_eq!(sim_hash(&back), sim_hash(&made), "the hash leaves the files' order and parents out");
    let committed = include_bytes!("../../../../assets/templates/anim/anims/carve_sway.glb");
    assert!(committed[..] == glb[..], "rewrite it: cargo run -p ssx-content --example anim_template");
}

#[test]
fn every_key_of_the_template_falls_on_a_tick() {
    for t in &template::clip().joints[0].keys.times {
        let tick = (t * TICKS_PER_SECOND).round();
        assert_eq!(tick / TICKS_PER_SECOND, *t, "{t} s");
    }
}

#[test]
fn sampling_clamps_and_interpolates_as_declared() {
    let mut t = track("Hips", Path::Translation, vec![1.0, 2.0], vec![0.0, 0.0, 0.0, 2.0, 4.0, 6.0]);
    let clip = |t: &JointTrack| Clip { joints: vec![t.clone()], floats: Vec::new(), additive: false, duration: 2.0 };
    assert_eq!(clip(&t).joint_at(0, 0.0)[..3], [0.0, 0.0, 0.0], "before the first key: its value");
    assert_eq!(clip(&t).joint_at(0, 1.5)[..3], [1.0, 2.0, 3.0]);
    assert_eq!(clip(&t).joint_at(0, 9.0)[..3], [2.0, 4.0, 6.0], "after the last: its value");
    assert_eq!(clip(&t).joint_at(0, f32::NAN)[..3], [0.0, 0.0, 0.0], "a NaN time takes the first key");
    t.keys.interpolation = Interpolation::Step;
    assert_eq!(clip(&t).joint_at(0, 1.99)[..3], [0.0, 0.0, 0.0]);
    t.keys.interpolation = Interpolation::CubicSpline;
    t.keys.values = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 4.0, 6.0, 0.0, 0.0, 0.0];
    assert_eq!(clip(&t).joint_at(0, 1.5)[..3], [1.0, 2.0, 3.0]);
}

#[test]
fn a_cubic_rotation_whose_tangents_cancel_it_takes_the_nearer_key() {
    let q = [0.0, 0.0, 0.0, 1.0];
    let m = [0.0, 0.0, 0.0, -16.0];
    let mut values = Vec::new();
    for part in [[0.0; 4], q, m, [0.0, 0.0, 0.0, 16.0], q, [0.0; 4]] {
        values.extend(part);
    }
    let t = JointTrack {
        joint: "Hips".into(),
        path: Path::Rotation,
        keys: Keys { interpolation: Interpolation::CubicSpline, times: vec![0.0, 1.0], values },
        parent: None,
    };
    let c = Clip { joints: vec![t], floats: Vec::new(), additive: false, duration: 1.0 };
    for k in 0..=100 {
        let v = c.joint_at(0, k as f32 / 100.0);
        assert!(v.iter().all(|x| x.is_finite()), "{v:?}");
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] + v[3] * v[3]).sqrt();
        assert!((len - 1.0).abs() < 1e-5, "{len}");
    }
}

#[test]
fn hostile_clips_are_refused_with_the_channel_and_field() {
    let q = |n: usize| -> Vec<f32> { (0..n).flat_map(|_| [0.0, 0.0, 0.0, 1.0]).collect() };
    let cases: Vec<(Vec<JointTrack>, &str)> = vec![
        (vec![track("Hips", Path::Rotation, vec![0.0, 0.0], q(2))], "not after the key before it"),
        (vec![track("Hips", Path::Rotation, vec![1.0, 0.5], q(2))], "not after the key before it"),
        (vec![track("Hips", Path::Rotation, vec![0.0, 25_000.0], q(2))], "key 1 at 25000 s"),
        (vec![track("Hips", Path::Rotation, vec![-1.0], q(1))], "key 0 at -1 s"),
        (vec![track("Hips", Path::Rotation, vec![0.0], vec![0.0; 4])], "a rotation of length 0"),
        (vec![track("Hips", Path::Scale, vec![0.0], vec![0.0, 1.0, 1.0])], "a scale of 0"),
        (vec![track("Hips", Path::Translation, vec![0.0], vec![5000.0, 0.0, 0.0])], "beyond ±4096"),
        (
            vec![track("Hips", Path::Rotation, vec![0.0], q(1)), track("Hips", Path::Rotation, vec![0.0], q(1))],
            "has a channel already",
        ),
        (vec![track("Spine", Path::Rotation, vec![0.0, f32::NAN], q(2))], "not a finite number"),
        (
            vec![track("Hips", Path::Rotation, vec![0.0, 1.0], vec![0.0, 0.0, 0.0, f32::INFINITY, 0.0, 0.0, 0.0, 1.0])],
            "not a finite number",
        ),
    ];
    for (tracks, want) in cases {
        let err = read(GLTF, Some(glb_of(tracks))).expect_err(want);
        assert!(err.contains(want) && err.contains("animations[0].channels["), "{want}: {err}");
    }
    let many: Vec<f32> = (0..=MAX_KEYS).map(|k| k as f32 / 100.0).collect();
    let err = read(GLTF, Some(glb_of(vec![track("Hips", Path::Rotation, many.clone(), q(many.len()))]))).unwrap_err();
    assert!(err.contains("1 to 3601 are read"), "{err}");
}

#[test]
fn hostile_records_are_refused_at_their_line() {
    let cases = [
        ("[anim]\n", "no track"),
        ("[anim]\nanimation = \"x\"\n[[anim.float]]\ndof = \"d\"\ntimes = [0.0]\nvalues = [1.0]\n", "names none"),
        ("[anim]\ngltf = \"../a.glb\"\n", "gltf:"),
        ("[anim]\ngltf = \"a.png\"\n", ".glb or .gltf"),
        ("[anim]\nspeed = 2\n", "unknown field"),
        ("[anim]\n[[anim.float]]\ndof = \"d\"\ntimes = [0.0, 70.0]\nvalues = [1.0, 2.0]\n", "key 1 at 70 s"),
        ("[anim]\n[[anim.float]]\ndof = \"d\"\ntimes = [0.0, 1.0]\nvalues = [1.0]\n", "1 values for 2 keys"),
        ("[anim]\n[[anim.float]]\ndof = \"d\"\ntimes = [0.0]\nvalues = [nan]\n", "not a finite 32-bit number"),
        ("[anim]\n[[anim.float]]\ndof = \"d\"\ntimes = [0.0]\nvalues = [1e39]\n", "not a finite 32-bit number"),
        (
            "[anim]\n[[anim.float]]\ndof = \"d\"\ntimes = [0.0]\nvalues = [1.0]\n[[anim.float]]\ndof = \"d\"\ntimes = [0.0]\nvalues = [1.0]\n",
            "has a track already",
        ),
        (
            "[anim]\n[[anim.float]]\ndof = \"d\"\ninterpolation = \"smooth\"\ntimes = [0.0]\nvalues = [1.0]\n",
            "step, linear",
        ),
    ];
    for (text, want) in cases {
        let err = read(text, None).expect_err(want);
        assert!(err.contains(want), "{want}: {err}");
        assert!(err.starts_with("anim.toml:"), "{err}");
    }
    let tracks: String = (0..=MAX_FLOAT_TRACKS)
        .map(|i| format!("[[anim.float]]\ndof = \"d{i}\"\ntimes = [0.0]\nvalues = [1.0]\n"))
        .collect();
    assert!(read(&format!("[anim]\n{tracks}"), None).unwrap_err().contains("at most 128"));
}

#[test]
fn a_key_the_parser_quotes_is_shown_escaped_and_bounded() {
    let cases = [
        ("[anim]\n\"\\u001b]0;x\\u0007\\t\" = 1\n".to_string(), "\\u{1b}]0;x\\u{7}\\t"),
        (format!("\"{}\\u001b[2J\" = 1\n", "k".repeat(100_000)), "… (100037 bytes)"),
        ("[anim]\n\"\\u001b[2J\" = 1\n\"\\u001b[2J\" = 2\n".to_string(), "duplicate key"),
    ];
    for (text, want) in cases {
        let problems = parse_toml(text.as_bytes()).expect_err("refused");
        for (_, _, m) in &problems {
            assert!(!m.chars().any(char::is_control), "a control character in {m:?}");
            assert!(m.chars().count() <= 512 + 32, "{} characters", m.chars().count());
        }
        let shown: String = problems.iter().map(|p| p.2.clone()).collect();
        assert!(shown.contains(want), "{want}: {shown}");
    }
}

#[test]
fn the_glb_is_chosen_by_name_and_morph_weights_are_refused() {
    let ok = glb_of(vec![track("Hips", Path::Rotation, vec![0.0], vec![0.0, 0.0, 0.0, 1.0])]);
    assert!(read(GLTF, Some(ok.clone())).is_ok());
    let at = ok.windows(17).position(|w| w == b"\"path\":\"rotation\"").expect("the channel's path");
    let mut weights = ok.clone();
    weights[at..at + 17].copy_from_slice(b"\"path\":\"weights\" ");
    assert!(read(GLTF, Some(weights)).unwrap_err().contains("morph target weights"));
    assert!(
        read("[anim]\ngltf = \"a.glb\"\nanimation = \"y\"\n", Some(ok)).unwrap_err().contains("no animation is named")
    );
    assert!(read(GLTF, None).unwrap_err().contains("not in the package"));
}

#[test]
fn the_record_writer_reads_back_exactly() {
    let f = FloatTrack {
        dof: "TrajectoryVelocityRig.AITrajectory.height".into(),
        keys: Keys {
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0 / 60.0, 0.1],
            values: vec![1e-7, -0.3, 261.43417],
        },
    };
    let clip = Clip { joints: Vec::new(), floats: vec![f.clone()], additive: true, duration: 0.1 };
    let text = write::anim_toml(&clip, None, "x", "# a header\n");
    let back = read(&text, None).expect("the written record reads");
    assert_eq!(back.floats, vec![f]);
    assert!(back.additive);
}

#[test]
fn the_hash_follows_every_value_and_no_order() {
    let a = track("Hips", Path::Rotation, vec![0.0], vec![0.0, 0.0, 0.0, 1.0]);
    let b = track("Spine", Path::Translation, vec![0.0], vec![0.0, 1.0, 0.0]);
    let c = |j: Vec<JointTrack>, additive| Clip { joints: j, floats: Vec::new(), additive, duration: 0.0 };
    assert_eq!(sim_hash(&c(vec![a.clone(), b.clone()], false)), sim_hash(&c(vec![b.clone(), a.clone()], false)));
    assert_ne!(sim_hash(&c(vec![a.clone(), b.clone()], false)), sim_hash(&c(vec![a.clone(), b.clone()], true)));
    let mut moved = b.clone();
    moved.keys.values[1] = f32::from_bits(1.0f32.to_bits() + 1);
    assert_ne!(sim_hash(&c(vec![a.clone(), b], false)), sim_hash(&c(vec![a, moved], false)));
}
