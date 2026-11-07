use super::*;
use crate::skeleton::{BIND_TRANSLATION_TOLERANCE, RowMatrix, Skeleton, SkeletonJoint};

fn translate(y: f32) -> RowMatrix {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, y, 0.0, 1.0]]
}

fn skinned(joints: [[u16; 4]; 3], weights: [[f32; 4]; 3]) -> (Value, Vec<u8>) {
    let (mut doc, mut bin) = triangle();
    let at_joints = bin.len();
    bin.extend(joints.iter().flatten().flat_map(|j| j.to_le_bytes()));
    let at_weights = bin.len();
    bin.extend(weights.iter().flatten().flat_map(|w| w.to_le_bytes()));
    let at_ibm = bin.len();
    for m in [translate(-1.0), translate(-1.5)] {
        bin.extend(m.iter().flatten().flat_map(|v| v.to_le_bytes()));
    }
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": at_joints, "byteLength": 24}),
        json!({"buffer": 0, "byteOffset": at_weights, "byteLength": 48}),
        json!({"buffer": 0, "byteOffset": at_ibm, "byteLength": 128}),
    ]);
    doc["accessors"].as_array_mut().unwrap().extend([
        json!({"bufferView": 2, "count": 3, "type": "VEC4", "componentType": 5123}),
        json!({"bufferView": 3, "count": 3, "type": "VEC4", "componentType": 5126}),
        json!({"bufferView": 4, "count": 2, "type": "MAT4", "componentType": 5126}),
    ]);
    doc["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION": 0, "JOINTS_0": 2, "WEIGHTS_0": 3});
    doc["nodes"] = json!([
        {"name": "Hips", "translation": [0.0, 1.0, 0.0], "children": [1]},
        {"name": "Spine", "translation": [0.0, 0.5, 0.0]},
        {"name": "jackets", "mesh": 0, "skin": 0}
    ]);
    doc["scenes"] = json!([{"nodes": [0, 2]}]);
    doc["skins"] = json!([{"joints": [0, 1], "inverseBindMatrices": 4}]);
    (doc, bin)
}

const HALF: [[f32; 4]; 3] = [[0.5, 0.5, 0.0, 0.0]; 3];
const PAIR: [[u16; 4]; 3] = [[0, 1, 0, 0]; 3];

fn decode_skinned(doc: &Value, bin: &[u8]) -> Result<Model, ModelError> {
    decode_file("models/m.glb", &pack(doc, bin), &(), &Limits::skinned())
}

fn skeleton(spine_y: f32, spine: &str) -> Skeleton {
    let joints = vec![
        SkeletonJoint { name: "Hips".into(), parent: None, inverse_bind: translate(-1.0) },
        SkeletonJoint { name: spine.into(), parent: Some(0), inverse_bind: translate(-spine_y) },
    ];
    Skeleton::new(joints, None).unwrap()
}

#[test]
fn a_skin_reads_with_its_joints_and_binds_by_name() {
    let (doc, bin) = skinned(PAIR, HALF);
    let m = decode_skinned(&doc, &bin).unwrap();
    assert_eq!(m.skins.len(), 1);
    assert_eq!(m.skins[0].joints, ["Hips", "Spine"]);
    assert_eq!(m.skins[0].inverse_binds, [translate(-1.0), translate(-1.5)]);
    let part = &m.lods[0].parts[0];
    assert_eq!((part.skin, part.slot.as_deref()), (Some(0), None));
    let s = part.mesh.skin.as_ref().unwrap();
    assert_eq!((s.joints.as_slice(), s.weights.as_slice()), (&PAIR[..], &HALF[..]));
    assert_eq!(skin::bind(&m, &skeleton(1.5, "Spine")).unwrap(), [vec![0, 1]]);
    let st = decode_file("models/m.glb", &pack(&doc, &bin), &(), &Limits::default()).unwrap();
    assert!(st.skins.is_empty() && st.lods[0].parts[0].mesh.skin.is_none());
}

#[test]
fn a_joint_the_skeleton_lacks_or_a_moved_bind_pose_is_refused() {
    let (doc, bin) = skinned(PAIR, HALF);
    let m = decode_skinned(&doc, &bin).unwrap();
    let why = skin::bind(&m, &skeleton(1.5, "Chest")).unwrap_err().join("; ");
    assert!(why.contains("\"Spine\" is not a joint of the riders' skeleton"), "{why}");
    assert!(skin::bind(&m, &skeleton(1.5 + BIND_TRANSLATION_TOLERANCE * 0.5, "Spine")).is_ok());
    let why = skin::bind(&m, &skeleton(1.51, "Spine")).unwrap_err().join("; ");
    assert!(why.contains("a moved or changed skeleton is simulation content"), "{why}");
}

#[test]
fn more_than_four_influences_are_refused() {
    let (mut doc, bin) = skinned(PAIR, HALF);
    let attrs = &mut doc["meshes"][0]["primitives"][0]["attributes"];
    (attrs["JOINTS_1"], attrs["WEIGHTS_1"]) = (json!(2), json!(3));
    refused(decode_skinned(&doc, &bin), "more than 4 influences");
    let (mut doc, mut bin) = skinned(PAIR, HALF);
    let at = bin.len();
    bin.extend([0u8; 48]);
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["bufferViews"].as_array_mut().unwrap().push(json!({"buffer": 0, "byteOffset": at, "byteLength": 48}));
    doc["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView": 5, "count": 3, "type": "VEC4", "componentType": 5126}));
    let attrs = &mut doc["meshes"][0]["primitives"][0]["attributes"];
    (attrs["JOINTS_1"], attrs["WEIGHTS_1"]) = (json!(2), json!(5));
    assert!(decode_skinned(&doc, &bin).is_ok());
}

#[test]
fn weights_are_finite_non_negative_once_per_joint_and_sum_to_one() {
    let w = |a: f32, b: f32| [[a, b, 0.0, 0.0]; 3];
    let (doc, bin) = skinned(PAIR, w(f32::NAN, 0.5));
    refused(decode_skinned(&doc, &bin), "not a finite number");
    let (doc, bin) = skinned(PAIR, w(1.5, -0.5));
    refused(decode_skinned(&doc, &bin), "negative weight");
    let (doc, bin) = skinned([[1, 1, 0, 0]; 3], HALF);
    refused(decode_skinned(&doc, &bin), "names joint 1 twice");
    let (doc, bin) = skinned(PAIR, w(0.0, 0.0));
    refused(decode_skinned(&doc, &bin), "has no influence");
    let (doc, bin) = skinned(PAIR, w(0.6, 0.41));
    refused(decode_skinned(&doc, &bin), "sum to 1.010000");
    let (doc, bin) = skinned(PAIR, w(0.6, 0.4005));
    let m = decode_skinned(&doc, &bin).unwrap();
    let s = m.lods[0].parts[0].mesh.skin.as_ref().unwrap();
    assert!(s.weights.iter().all(|w| (w.iter().sum::<f32>() - 1.0).abs() < 1e-6), "{:?}", s.weights);
    let (doc, bin) = skinned([[0, 1, 2, 0]; 3], HALF);
    refused(decode_skinned(&doc, &bin), "names joint 2 of the skin, which has 2");
}

#[test]
fn normalised_integer_weights_sum_exactly() {
    let (mut doc, mut bin) = skinned(PAIR, HALF);
    let at = bin.len();
    for pair in [[128u8, 127], [128, 128]] {
        bin.truncate(at);
        bin.extend([pair[0], pair[1], 0, 0].repeat(3));
        bin.extend([0u8; 4]);
        doc["buffers"][0]["byteLength"] = json!(bin.len());
        let views = doc["bufferViews"].as_array_mut().unwrap();
        views.truncate(5);
        views.push(json!({"buffer": 0, "byteOffset": at, "byteLength": 12}));
        let acc = doc["accessors"].as_array_mut().unwrap();
        acc.truncate(5);
        acc.push(json!({"bufferView": 5, "count": 3, "type": "VEC4", "componentType": 5121, "normalized": true}));
        doc["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(5);
        let r = decode_skinned(&doc, &bin);
        if pair == [128, 127] {
            assert!(r.is_ok(), "{:?}", r.err());
        } else {
            refused(r, "sum to 256/255");
        }
    }
}

#[test]
fn inverse_binds_are_affine_and_invertible() {
    let (doc, mut bin) = skinned(PAIR, HALF);
    let at = bin.len() - 128;
    let zero: RowMatrix = [[0.0; 4], [0.0; 4], [0.0; 4], [0.0, 0.0, 0.0, 1.0]];
    bin.splice(at + 64.., zero.iter().flatten().flat_map(|v| v.to_le_bytes()));
    refused(decode_skinned(&doc, &bin), "cannot be inverted");
    let (doc, mut bin) = skinned(PAIR, HALF);
    let len = bin.len();
    bin[len - 4..].copy_from_slice(&2.0f32.to_le_bytes());
    refused(decode_skinned(&doc, &bin), "is not affine");
}

#[test]
fn joint_and_skin_counts_are_capped_from_the_json() {
    let (mut doc, bin) = skinned(PAIR, HALF);
    let limits = Limits::skinned();
    let many: Vec<usize> = (0..=limits.max_skin_joints).map(|_| 0).collect();
    doc["skins"][0]["joints"] = json!(many);
    refused(decode_skinned(&doc, &bin), &format!("{} joints (1 to {} are read)", many.len(), limits.max_skin_joints));
    let package = Counted::new([("models/m.glb", pack(&doc, &bin))]);
    assert!(probe_file("models/m.glb", &package, &limits).is_err());
    let (mut doc, bin) = skinned(PAIR, HALF);
    doc["skins"] = json!(vec![json!({"joints": [0, 1], "inverseBindMatrices": 4}); limits.max_skins + 1]);
    refused(decode_skinned(&doc, &bin), "skins (at most");
    let (mut doc, bin) = skinned(PAIR, HALF);
    doc["skins"][0]["joints"] = json!([0, 0]);
    refused(decode_skinned(&doc, &bin), "twice");
}
