use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::write::{WriteModel, glb};
use super::*;

mod probe;
mod skins;

fn triangle() -> (Value, Vec<u8>) {
    let mut bin = Vec::new();
    for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        bin.extend(p.iter().flat_map(|v| v.to_le_bytes()));
    }
    bin.extend([0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
    bin.extend([0, 0]);
    let doc = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "rock", "mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "accessors": [
            {"bufferView": 0, "count": 3, "type": "VEC3", "componentType": 5126, "min": [0, 0, 0], "max": [1, 1, 0]},
            {"bufferView": 1, "count": 3, "type": "SCALAR", "componentType": 5123}
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 6}
        ],
        "buffers": [{"byteLength": 44}]
    });
    (doc, bin)
}

fn pack(doc: &Value, bin: &[u8]) -> Vec<u8> {
    assemble_glb(&serde_json::to_vec(doc).unwrap(), Some(bin))
}

fn decode(doc: &Value, bin: &[u8]) -> Result<Model, ModelError> {
    decode_file("models/m.glb", &pack(doc, bin), &(), &Limits::default())
}

#[track_caller]
fn refused(r: Result<Model, ModelError>, want: &str) {
    match r {
        Ok(_) => panic!("accepted; expected a refusal naming {want:?}"),
        Err(e) => assert!(e.message.contains(want), "{:?} does not name {want:?}", e.message),
    }
}

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
        range(self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?, path, offset, len)
    }
}

fn range(b: &[u8], path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
    let end = offset.checked_add(len).filter(|&e| e <= b.len() as u64);
    let end = end.ok_or_else(|| format!("{path}: bytes {offset}+{len} lie outside its {} bytes", b.len()))?;
    Ok(b[offset as usize..end as usize].to_vec())
}

#[test]
fn a_triangle_reads_with_flat_normals_and_its_box() {
    let (doc, bin) = triangle();
    let m = decode(&doc, &bin).unwrap();
    assert_eq!(m.lods.len(), 1);
    let mesh = &m.lods[0].parts[0].mesh;
    assert_eq!(mesh.indices, [0, 1, 2]);
    assert_eq!(mesh.normals.as_deref(), Some(&[[0.0, 0.0, 1.0]; 3][..]), "flat normals: glTF 2.0 3.7.2.1");
    assert_eq!(mesh.bounds, Some(([0.0, 0.0, 0.0], [1.0, 1.0, 0.0])));
    assert!(mesh.tangents.is_none(), "no UV, no tangents");
    assert!(m.collision.is_none() && m.source.is_some());
}

#[test]
fn a_mirroring_node_keeps_the_winding_and_the_normals_outside() {
    let (mut doc, bin) = triangle();
    doc["nodes"][0]["scale"] = json!([-1.0, 1.0, 1.0]);
    let mesh = decode(&doc, &bin).unwrap().lods[0].parts[0].mesh.clone();
    assert!(mesh.positions.contains(&[-1.0, 0.0, 0.0]), "{:?}", mesh.positions);
    let p = |i: u32| mesh.positions[mesh.indices[i as usize] as usize];
    let (e1, e2) = ([p(1)[0] - p(0)[0], p(1)[1] - p(0)[1]], [p(2)[0] - p(0)[0], p(2)[1] - p(0)[1]]);
    assert!(e1[0] * e2[1] - e1[1] * e2[0] > 0.0, "counter-clockwise seen from +Z");
    assert_eq!(mesh.normals.as_ref().unwrap()[0], [0.0, 0.0, 1.0]);
}

#[test]
fn strips_and_fans_become_lists() {
    for (mode, want) in [(5, vec![0, 1, 2, 2, 1, 3]), (6, vec![1, 2, 0, 2, 3, 0])] {
        let (mut doc, mut bin) = triangle();
        bin.truncate(36);
        bin.extend([1.0f32, 1.0, 0.0].iter().flat_map(|v| v.to_le_bytes()));
        bin.extend([0u16, 1, 2, 3].iter().flat_map(|i| i.to_le_bytes()));
        doc["accessors"][0]["count"] = json!(4);
        doc["accessors"][1]["count"] = json!(4);
        doc["bufferViews"][0]["byteLength"] = json!(48);
        doc["bufferViews"][1] = json!({"buffer": 0, "byteOffset": 48, "byteLength": 8});
        doc["buffers"][0]["byteLength"] = json!(56);
        doc["meshes"][0]["primitives"][0]["mode"] = json!(mode);
        doc["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(0);
        let m = decode(&doc, &bin).unwrap();
        assert_eq!(m.lods[0].parts[0].mesh.indices, want, "mode {mode}");
    }
}

fn quad() -> Mesh {
    Mesh {
        name: "quad".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        normals: Some(vec![[0.0, 0.0, 1.0]; 4]),
        uv0: Some(vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]),
        colours: Some(vec![[255, 128, 64, 0], [1, 2, 3, 4], [9, 8, 7, 6], [0, 0, 0, 0]]),
        indices: vec![0, 1, 2, 0, 2, 3],
        ..Mesh::default()
    }
}

#[test]
fn the_writer_round_trips_lods_colours_and_collision() {
    let lod0 = quad();
    let mut lod1 = quad();
    lod1.positions.iter_mut().for_each(|p| p[2] = 2.0);
    let (rails, _) = collision::build(&[
        CollisionTriangle { corners: [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], surface_id: 41, group: 2 },
        CollisionTriangle { corners: [[0.0; 3], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], surface_id: 10, group: 0 },
    ]);
    let w = WriteModel {
        name: "boulder",
        lods: vec![vec![(&lod0, Some(0))], vec![(&lod1, Some(1))]],
        materials: vec!["rock".into(), "moss".into()],
        collision: Some(&rails),
    };
    let bytes = glb(&w).unwrap();
    assert_eq!(bytes, glb(&w).unwrap(), "the same model gives the same bytes");
    let m = decode_file("m.glb", &bytes, &(), &Limits::default()).unwrap();
    assert_eq!(m.lods.len(), 2);
    let got = &m.lods[0].parts[0];
    assert_eq!(got.mesh.positions, lod0.positions);
    assert_eq!(got.mesh.colours, lod0.colours, "c0's bytes come back through COLOR_0");
    assert_eq!(got.mesh.indices, lod0.indices);
    assert_eq!(got.material, Some(0));
    assert_eq!(m.lods[1].parts[0].mesh.positions[0], [0.0, 0.0, 2.0]);
    assert_eq!(m.materials.iter().map(|x| x.name.clone().unwrap()).collect::<Vec<_>>(), ["rock", "moss"]);
    let t = got.mesh.tangents.as_ref().unwrap();
    assert!(t.iter().all(|t| (t[0] - 1.0).abs() < 1e-6), "{t:?}");
    let c = m.collision.unwrap();
    let mut sets: Vec<(u32, u32, usize)> =
        c.clusters.iter().map(|c| (c.surface_id, c.group, c.triangles.len())).collect();
    sets.sort();
    assert_eq!(sets, [(10, 0, 1), (41, 2, 1)]);
}

#[test]
fn collision_edges_are_made_from_adjacency() {
    let (c, degenerate) = collision::build(&[
        CollisionTriangle { corners: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 1.0]], surface_id: 10, group: 0 },
        CollisionTriangle { corners: [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 1.0, -1.0]], surface_id: 10, group: 0 },
        CollisionTriangle { corners: [[0.0; 3], [0.0; 3], [1.0, 0.0, 0.0]], surface_id: 10, group: 0 },
    ]);
    assert_eq!(degenerate, 1);
    let e = &c.clusters[0].edges[0];
    assert_eq!(e.flags & 0x113, 0x113);
    assert_eq!(e.flags & 0xE0, 0xE0, "the ridge and both open edges are convex");
    assert!((e.edge_cos[0] - 0.0).abs() < 1e-6, "the faces meet at 90 degrees: {:?}", e.edge_cos);
    assert!((e.edge_cos[1] - (1.0 - f32::from_bits(0x411D_E9E7) / 8.0)).abs() < 1e-7);
}

#[test]
fn generated_tangents_point_up_the_image() {
    let q = quad();
    let w = WriteModel { name: "q", lods: vec![vec![(&q, Some(0))]], materials: vec!["m".into()], collision: None };
    let m = decode_file("q.glb", &glb(&w).unwrap(), &(), &Limits::default()).unwrap();
    let mesh = &m.lods[0].parts[0].mesh;
    assert!(mesh.tangent_signs.as_ref().unwrap().iter().all(|&s| s == 1.0));
}

#[test]
fn hostile_containers() {
    let (doc, bin) = triangle();
    let good = pack(&doc, &bin);
    let lim = Limits::default();
    let d = |rel: &str, bytes: &[u8]| decode_file(rel, bytes, &(), &lim);
    refused(d("m.glb", b"{\"asset\":{}}"), "not a binary glTF");
    refused(d("m.gltf", &good), "content is a binary glTF");
    refused(d("m.obj", &good), "not .obj");
    let mut short = good.clone();
    short.truncate(good.len() - 4);
    refused(d("m.glb", &short), "the header states");
    let mut v1 = good.clone();
    v1[4] = 1;
    refused(d("m.glb", &v1), "version 1");
    let mut odd = good.clone();
    odd[12] = odd[12].wrapping_add(1);
    refused(d("m.glb", &odd), "not a multiple of 4");
    let mut extra = good.clone();
    extra.extend([4, 0, 0, 0, b'X', b'T', b'R', b'A', 0, 0, 0, 0]);
    let total = extra.len() as u32;
    extra[8..12].copy_from_slice(&total.to_le_bytes());
    refused(d("m.glb", &extra), "only JSON and BIN");
    let mut cut = good.clone();
    cut[12..16].copy_from_slice(&(u32::MAX - 3).to_le_bytes());
    refused(d("m.glb", &cut), "bytes stated");
    let bin_first = assemble_glb(b"{}", None);
    let mut swapped = bin_first.clone();
    swapped[16..20].copy_from_slice(&0x004E_4942u32.to_le_bytes());
    refused(d("m.glb", &swapped), "before the JSON chunk");
    let small = Limits { max_json_bytes: 64, ..lim };
    refused(decode_file("m.glb", &good, &(), &small), "at most 64 are read");
    let tiny = Limits { max_file_bytes: 100, ..lim };
    refused(decode_file("m.glb", &good, &(), &tiny), "at most 100 are read");
}

#[test]
fn hostile_json() {
    let (doc, bin) = triangle();
    let deep = format!("{}1{}", "[".repeat(100), "]".repeat(100));
    let text = serde_json::to_vec(&doc).unwrap();
    let mut patched = text.clone();
    let at = patched.len() - 1;
    patched.splice(at..at, format!(",\"extras\":{deep}").bytes());
    refused(decode_file("m.glb", &assemble_glb(&patched, Some(&bin)), &(), &Limits::default()), "nested 101 deep");
    refused(decode_file("m.glb", &assemble_glb(b"{\"asset\":", Some(&bin)), &(), &Limits::default()), "JSON:");
    let mut v = doc.clone();
    v["asset"]["version"] = json!("1.0");
    refused(decode(&v, &bin), "only 2.x");
    let mut v = doc.clone();
    v["asset"]["minVersion"] = json!("2.1");
    refused(decode(&v, &bin), "this reader is 2.0");
    let mut v = doc.clone();
    v["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
    v["extensionsUsed"] = json!(["KHR_draco_mesh_compression"]);
    refused(decode(&v, &bin), "requires the extensions");
    let mut v = doc.clone();
    v["extensionsUsed"] = json!(["KHR_texture_transform"]);
    let m = decode(&v, &bin).expect("an optional extension is left out, not refused");
    assert!(m.notes.iter().any(|n| n.contains("KHR_texture_transform")), "{:?}", m.notes);
    let mut v = doc.clone();
    v["nodes"][0]["mesh"] = json!(7);
    refused(decode(&v, &bin), "IndexOutOfBounds");
    let mut v = doc.clone();
    v["meshes"][0]["primitives"][0]["mode"] = json!(99);
    refused(decode(&v, &bin), "invalid glTF");
    let few = Limits { max_items: 1, ..Limits::default() };
    refused(decode_file("m.glb", &pack(&doc, &bin), &(), &few), "2 accessors");
}

#[test]
fn hostile_scenes_and_nodes() {
    let (doc, bin) = triangle();
    let mut v = doc.clone();
    v["nodes"] = json!([{"name": "a", "mesh": 0, "children": [1]}, {"name": "b", "children": [0]}]);
    refused(decode(&v, &bin), "reached twice");
    let mut v = doc.clone();
    v["scenes"][0]["nodes"] = json!([0, 0]);
    refused(decode(&v, &bin), "scenes[0].nodes: 2 entries, more than there are nodes (1)");
    v["nodes"] = json!([v["nodes"][0].clone(), {"name": "spare"}]);
    refused(decode(&v, &bin), "reached twice");
    let mut v = doc.clone();
    let chain: Vec<Value> = (0..80).map(|i| json!({"children": [i + 1]})).chain([json!({"mesh": 0})]).collect();
    v["nodes"] = json!(chain);
    refused(decode(&v, &bin), "nested deeper than 64");
    let mut v = doc.clone();
    v.as_object_mut().unwrap().remove("scene");
    v["scenes"] = json!([{"nodes": [0]}, {"nodes": [0]}]);
    refused(decode(&v, &bin), "none chosen");
    let mut v = doc.clone();
    v["nodes"][0]["matrix"] = json!([1, 0, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    refused(decode(&v, &bin), "not affine");
    let mut v = doc.clone();
    v["nodes"][0]["rotation"] = json!([0.0, 0.0, 0.0, 2.0]);
    refused(decode(&v, &bin), "not a unit quaternion");
    let mut v = doc.clone();
    v["nodes"][0]["scale"] = json!([1.0, 0.0, 1.0]);
    refused(decode(&v, &bin), "flattens it");
    let mut v = doc.clone();
    v["nodes"][0]["scale"] = json!([3.0e37, 1.0, 1.0]);
    refused(decode(&v, &bin), "beyond");
    let mut v = doc.clone();
    v["nodes"][0]["name"] = json!("rock_lod1");
    refused(decode(&v, &bin), "no mesh in LOD 0");
    let mut v = doc.clone();
    v["scenes"][0]["nodes"] = json!([0, 1]);
    v["nodes"] = json!([{"name": "rock", "mesh": 0}, {"name": "rock_lod2", "mesh": 0}]);
    refused(decode(&v, &bin), "LOD 1 has no mesh");
    let mut v = doc.clone();
    v["nodes"][0]["name"] = json!("rock_lod9");
    v["scenes"][0]["nodes"] = json!([0, 1]);
    v["nodes"] = json!([{"name": "rock", "mesh": 0}, {"name": "rock_lod9", "mesh": 0}]);
    refused(decode(&v, &bin), "at most 8 LODs");
}

#[test]
fn hostile_accessors_and_buffers() {
    let (doc, bin) = triangle();
    let case = |f: &dyn Fn(&mut Value), want: &str| {
        let mut v = doc.clone();
        f(&mut v);
        refused(decode(&v, &bin), want);
    };
    case(&|v| v["accessors"][0]["count"] = json!(4), "do not fit its view");
    case(&|v| v["bufferViews"][0]["byteLength"] = json!(400), "lie outside buffers[0]");
    case(&|v| v["buffers"][0]["byteLength"] = json!(4000), "the BIN chunk has");
    case(&|v| v["bufferViews"][0]["byteStride"] = json!(8), "below its element of 12 bytes");
    case(&|v| v["bufferViews"][0]["byteStride"] = json!(14), "a multiple of 4");
    case(&|v| v["accessors"][0]["byteOffset"] = json!(2), "not aligned");
    case(&|v| v["accessors"][0]["count"] = json!(0), "elements (1 to");
    case(&|v| v["accessors"][0]["type"] = json!("VEC2"), "is not accepted here");
    case(&|v| v["accessors"][0]["componentType"] = json!(5121), "is not accepted here");
    case(&|v| v["accessors"][1]["count"] = json!(2), "not a multiple of 3");
    case(&|v| v["meshes"][0]["primitives"][0]["mode"] = json!(1), "only triangles are drawn");
    case(&|v| v["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] = json!(1), "is not accepted here");
    case(&|v| v["buffers"][0]["uri"] = json!("https://example.com/m.bin"), "only files of the package");
    case(&|v| v["buffers"][0]["uri"] = json!("/etc/passwd"), "absolute path");
    case(&|v| v["buffers"][0]["uri"] = json!("../../m.bin"), "buffers[0]");
    case(&|v| v["buffers"][0]["uri"] = json!("m.bin"), "no files are read beside");
    case(&|v| v["buffers"].as_array_mut().unwrap().push(json!({"byteLength": 4})), "buffers[1] has no data");
    case(&|v| v["buffers"][0]["uri"] = json!("data:application/octet-stream;base64,AAAA"), "its data has 3");
    let mut v = doc.clone();
    v["accessors"][1]["count"] = json!(3);
    let mut bad = bin.clone();
    bad[36..38].copy_from_slice(&7u16.to_le_bytes());
    refused(decode(&v, &bad), "index 7 of 3 vertices");
    let mut nan = bin.clone();
    nan[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    refused(decode(&doc, &nan), "not a finite number");
    let mut inf = bin.clone();
    inf[4..8].copy_from_slice(&f32::INFINITY.to_le_bytes());
    refused(decode(&doc, &inf), "not a finite number");
    let few = Limits { max_vertices: 2, ..Limits::default() };
    refused(decode_file("m.glb", &pack(&doc, &bin), &(), &few), "at most 2 and");
    let over = Limits { max_held_bytes: 40, ..Limits::default() };
    refused(decode_file("m.glb", &pack(&doc, &bin), &(), &over), "a model holds at most 40 bytes");
}

#[test]
fn hostile_sparse_accessors() {
    let (mut doc, mut bin) = triangle();
    bin.extend([2u16, 1].iter().flat_map(|i| i.to_le_bytes()));
    bin.extend([0u8; 4]);
    bin.extend([5.0f32, 5.0, 5.0, 6.0, 6.0, 6.0].iter().flat_map(|v| v.to_le_bytes()));
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": 44, "byteLength": 4}),
        json!({"buffer": 0, "byteOffset": 52, "byteLength": 24}),
    ]);
    doc["accessors"][0]["sparse"] = json!({
        "count": 2,
        "indices": {"bufferView": 2, "componentType": 5123},
        "values": {"bufferView": 3}
    });
    refused(decode(&doc, &bin), "out of order");
    let mut v = doc.clone();
    v["accessors"][0]["sparse"]["count"] = json!(4);
    refused(decode(&v, &bin), "4 sparse values for 3 elements");
    let mut ok = bin.clone();
    ok[44..48].copy_from_slice(&[1, 0, 2, 0]);
    let m = decode(&doc, &ok).unwrap();
    let p = &m.lods[0].parts[0].mesh;
    assert_eq!(p.positions[..], [[0.0, 0.0, 0.0], [5.0, 5.0, 5.0], [6.0, 6.0, 6.0]][..]);
    let mut high = bin.clone();
    high[44..48].copy_from_slice(&[1, 0, 9, 0]);
    refused(decode(&doc, &high), "not below 3");
}

#[test]
fn gltf_with_package_files_and_data_uris() {
    let (mut doc, bin) = triangle();
    doc["buffers"][0]["uri"] = json!("m%2Bpart.bin");
    let files = Files(BTreeMap::from([("models/m+part.bin".to_string(), bin.clone())]));
    let text = serde_json::to_vec(&doc).unwrap();
    let m = decode_file("models/m.gltf", &text, &files, &Limits::default()).unwrap();
    assert_eq!(m.lods[0].parts[0].mesh.positions.len(), 3);
    let mut b64 = String::new();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for c in bin.chunks(3) {
        let n = c.iter().enumerate().fold(0u32, |a, (i, &b)| a | u32::from(b) << (16 - 8 * i));
        for k in 0..=c.len() {
            b64.push(alphabet[(n >> (18 - 6 * k) & 63) as usize] as char);
        }
    }
    doc["buffers"][0]["uri"] = json!(format!("data:application/octet-stream;base64,{b64}"));
    let text = serde_json::to_vec(&doc).unwrap();
    assert!(decode_file("m.gltf", &text, &(), &Limits::default()).is_ok());
    let cap = Limits { max_data_uri_bytes: 8, ..Limits::default() };
    refused(decode_file("m.gltf", &text, &(), &cap), "at most 8 are read");
}

#[test]
fn hostile_collision_and_images() {
    let (doc, bin) = triangle();
    let mut v = doc.clone();
    v["nodes"] = json!([{"name": "rock", "mesh": 0}, {"name": "collision", "mesh": 0}]);
    v["scenes"][0]["nodes"] = json!([0, 1]);
    refused(decode(&v, &bin), "no ava_surface");
    v["nodes"][1]["extras"] = json!({"ava_surface": "snow"});
    refused(decode(&v, &bin), "a terrain surface");
    v["nodes"][1]["extras"] = json!({"ava_surface": "wood", "ava_group": 5});
    refused(decode(&v, &bin), "ava_group: 5");
    v["nodes"][1]["extras"] = json!({"ava_surface": 3});
    refused(decode(&v, &bin), "not a string");
    v["nodes"][1]["extras"] = json!({"ava_surface": "metal_snow", "ava_group": 2});
    let m = decode(&v, &bin).unwrap();
    let c = &m.collision.unwrap().clusters[0];
    assert_eq!((c.surface_id, c.group, c.triangles.len()), (41, 2, 1));
    let mut w = doc.clone();
    w["materials"] = json!([{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}]);
    w["textures"] = json!([{"source": 0}]);
    w["images"] = json!([{"bufferView": 0, "mimeType": "image/png"}]);
    w["meshes"][0]["primitives"][0]["material"] = json!(0);
    refused(decode(&w, &bin), "neither PNG nor JPEG");
    w["images"] = json!([{"uri": "file:///etc/shadow"}]);
    refused(decode(&w, &bin), "only files of the package");
    w["images"] = json!([{}]);
    refused(decode(&w, &bin), "exactly one is required");
}

#[test]
fn extras_are_read_by_their_ava_members_alone() {
    let (doc, bin) = triangle();
    let mut v = doc.clone();
    v["nodes"] = json!([{"name": "rock", "mesh": 0}, {"name": "collision", "mesh": 0}]);
    v["scenes"][0]["nodes"] = json!([0, 1]);
    let wide: Vec<u32> = (0..100_000).collect();
    v["nodes"][1]["extras"] = json!({"blender": {"deep": [wide]}, "ava_group": 2, "ava_surface": "wood"});
    let m = decode(&v, &bin).expect("the ava members read past the rest");
    assert_eq!(m.collision.as_ref().map(|c| (c.clusters[0].surface_id, c.clusters[0].group)), Some((30, 2)));
    v["nodes"][1]["extras"] = json!([1, {"ava_surface": "wood"}]);
    refused(decode(&v, &bin), "no ava_surface");
    v["nodes"][1]["extras"] = json!({"ava_surface": "wood", "ava_group": "x".repeat(10_000)});
    let e = decode(&v, &bin).expect_err("a group that is not one");
    assert!(e.message.contains("ava_group: \"xxx") && e.message.len() < 400, "{}", e.message);
    v["nodes"][1]["extras"] = json!({"ava_surface": "wood"});
    let text = String::from_utf8(serde_json::to_vec(&v).unwrap()).unwrap();
    let twice = text.replacen(r#""ava_surface":"wood""#, r#""ava_surface":"snow","ava_surface":"metal""#, 1);
    let m = decode_file("models/m.glb", &assemble_glb(twice.as_bytes(), Some(&bin)), &(), &Limits::default());
    assert_eq!(m.expect("the last of a member named twice").collision.map(|c| c.clusters[0].surface_id), Some(40));
}

#[test]
fn a_position_index_out_of_range_is_refused_not_a_panic() {
    let (mut doc, bin) = triangle();
    doc["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(9);
    refused(decode(&doc, &bin), "IndexOutOfBounds (2 accessors)");
    let crash = include_bytes!("tests/fuzz_crash_position_index.bin");
    refused(decode_file("models/m.gltf", crash, &(), &Limits::default()), "IndexOutOfBounds");
}

struct Counted {
    files: BTreeMap<String, Vec<u8>>,
    reads: std::cell::Cell<usize>,
    bytes: std::cell::Cell<u64>,
}

impl Counted {
    fn new(files: impl IntoIterator<Item = (&'static str, Vec<u8>)>) -> Self {
        let files = files.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        Counted { files, reads: std::cell::Cell::new(0), bytes: std::cell::Cell::new(0) }
    }
}

impl FileReader for Counted {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self.files.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        if b.len() as u64 > max {
            return Err(format!("{path}: {} bytes (at most {max})", b.len()));
        }
        self.reads.set(self.reads.get() + 1);
        self.bytes.set(self.bytes.get() + b.len() as u64);
        Ok(b.clone())
    }

    fn size(&self, path: &str) -> Result<u64, String> {
        Ok(self.files.get(path).ok_or_else(|| format!("{path}: not in the package"))?.len() as u64)
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let out = range(self.files.get(path).ok_or_else(|| format!("{path}: not in the package"))?, path, offset, len)?;
        self.bytes.set(self.bytes.get() + out.len() as u64);
        Ok(out)
    }
}

fn triangle_gltf() -> (Value, Vec<u8>) {
    let (mut doc, bin) = triangle();
    doc["buffers"][0]["uri"] = json!("m.bin");
    (doc, bin)
}

fn png_bytes() -> Vec<u8> {
    crate::test_dir::png_rgba(2, 2, [200, 100, 50, 255])
}

fn materials_with_images(doc: &mut Value, n: usize, image: impl Fn(usize) -> Value) {
    doc["materials"] = (0..n).map(|i| json!({"pbrMetallicRoughness": {"baseColorTexture": {"index": i}}})).collect();
    doc["textures"] = (0..n).map(|i| json!({"source": i})).collect();
    doc["images"] = (0..n).map(image).collect();
    doc["meshes"][0]["primitives"][0]["material"] = json!(0);
}

#[test]
fn buffers_naming_one_file_read_it_once() {
    let (mut doc, bin) = triangle_gltf();
    doc["buffers"] = (0..4096).map(|_| json!({"uri": "m.bin", "byteLength": 44})).collect();
    let files = Counted::new([("models/m.bin", bin.clone())]);
    let tight = Limits { max_held_bytes: 44, ..Limits::default() };
    let m = decode_file("models/m.gltf", &serde_json::to_vec(&doc).unwrap(), &files, &tight).unwrap();
    assert_eq!((files.reads.get(), m.lods[0].parts[0].mesh.positions.len()), (1, 3));
    doc["buffers"] = (0..8).map(|i| json!({"uri": format!("b{i}.bin"), "byteLength": 44})).collect();
    let names = ["models/b0.bin", "models/b1.bin", "models/b2.bin", "models/b3.bin"];
    let more = ["models/b4.bin", "models/b5.bin", "models/b6.bin", "models/b7.bin"];
    let files = Counted::new(names.iter().chain(&more).map(|n| (*n, bin.clone())));
    let total = Limits { max_held_bytes: 176, ..Limits::default() };
    let text = serde_json::to_vec(&doc).unwrap();
    refused(decode_file("models/m.gltf", &text, &files, &total), "a model holds at most 176 bytes");
    assert_eq!(files.reads.get(), 4, "the fifth file is not read: it would pass the total");
}

#[test]
fn images_naming_one_file_read_it_once() {
    let (mut doc, bin) = triangle_gltf();
    materials_with_images(&mut doc, 4096, |_| json!({"uri": "t.png"}));
    let png = png_bytes();
    let files = Counted::new([("models/m.bin", bin.clone()), ("models/t.png", png.clone())]);
    let tight = Limits { max_held_bytes: 44 + png.len() as u64, ..Limits::default() };
    let m = decode_file("models/m.gltf", &serde_json::to_vec(&doc).unwrap(), &files, &tight).unwrap();
    assert_eq!((files.reads.get(), m.images.len()), (2, 1));
    assert!(m.materials.iter().all(|x| x.base_colour_texture.is_some_and(|r| r.image == 0)));
    materials_with_images(&mut doc, 3, |i| json!({"uri": format!("t{i}.png")}));
    let files = Counted::new([
        ("models/m.bin", bin),
        ("models/t0.png", png.clone()),
        ("models/t1.png", png.clone()),
        ("models/t2.png", png.clone()),
    ]);
    let two = Limits { max_held_bytes: 44 + 2 * png.len() as u64, ..Limits::default() };
    refused(decode_file("models/m.gltf", &serde_json::to_vec(&doc).unwrap(), &files, &two), "images[2]");
}

#[test]
fn images_on_one_buffer_view_copy_it_once() {
    let (mut doc, mut bin) = triangle();
    let png = png_bytes();
    bin.extend(&png);
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    let views = doc["bufferViews"].as_array_mut().unwrap();
    views.push(json!({"buffer": 0, "byteOffset": 44, "byteLength": png.len()}));
    views.push(json!({"buffer": 0, "byteOffset": 44, "byteLength": png.len() - 4}));
    materials_with_images(&mut doc, 4096, |_| json!({"bufferView": 2, "mimeType": "image/png"}));
    let tight = Limits { max_held_bytes: (bin.len() + png.len()) as u64, ..Limits::default() };
    let m = decode_file("models/m.glb", &pack(&doc, &bin), &(), &tight).unwrap();
    assert_eq!(m.images.len(), 1);
    assert_eq!(&*m.images[0].bytes, &png[..]);
    materials_with_images(&mut doc, 2, |i| json!({"bufferView": 2 + i, "mimeType": "image/png"}));
    let more = format!("images[1]: {} bytes more", png.len() - 4);
    refused(decode_file("models/m.glb", &pack(&doc, &bin), &(), &tight), &more);
}

#[test]
fn images_drawn_are_measured_together_before_decoding() {
    let (mut doc, bin) = triangle_gltf();
    let big = crate::test_dir::png_header_only(4096, 4096);
    let names: Vec<String> = (0..64).map(|i| format!("models/t{i}.png")).collect();
    let mut files = Counted::new([("models/m.bin", bin)]);
    for n in &names {
        files.files.insert(n.clone(), big.clone());
    }
    materials_with_images(&mut doc, 64, |i| json!({"uri": format!("t{i}.png")}));
    let text = serde_json::to_vec(&doc).unwrap();
    refused(decode_file("models/m.gltf", &text, &files, &Limits::default()), "decode to 357913936 bytes or more");
    materials_with_images(&mut doc, 3, |i| json!({"uri": format!("t{i}.png")}));
    assert!(decode_file("models/m.gltf", &serde_json::to_vec(&doc).unwrap(), &files, &Limits::default()).is_ok());
    files.files.insert("models/wide.png".into(), crate::test_dir::png_header_only(8192, 16));
    materials_with_images(&mut doc, 1, |_| json!({"uri": "wide.png"}));
    let text = serde_json::to_vec(&doc).unwrap();
    refused(decode_file("models/m.gltf", &text, &files, &Limits::default()), "8192×16");
    doc["materials"] = json!([{"occlusionTexture": {"index": 0}}]);
    assert!(decode_file("models/m.gltf", &serde_json::to_vec(&doc).unwrap(), &files, &Limits::default()).is_ok());
}

#[test]
fn a_buffer_file_longer_than_its_byte_length_is_refused() {
    let (doc, bin) = triangle_gltf();
    let text = serde_json::to_vec(&doc).unwrap();
    let mut padded = bin.clone();
    padded.extend([0u8; 3]);
    let files = Counted::new([("models/m.bin", padded.clone())]);
    assert!(decode_file("models/m.gltf", &text, &files, &Limits::default()).is_ok(), "3 bytes of padding");
    padded.push(0);
    let files = Counted::new([("models/m.bin", padded)]);
    refused(decode_file("models/m.gltf", &text, &files, &Limits::default()), "buffers[0]: models/m.bin: 48 bytes");
    assert_eq!(files.reads.get(), 0);
    let short = Counted::new([("models/m.bin", bin[..40].to_vec())]);
    refused(decode_file("models/m.gltf", &text, &short, &Limits::default()), "its data has 40");
}
