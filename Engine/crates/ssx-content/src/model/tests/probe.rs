use super::*;

fn triangle_with_uv() -> (Value, Vec<u8>) {
    let (mut doc, mut bin) = triangle();
    for n in [[0.0f32, 0.0, 1.0]; 3] {
        bin.extend(n.iter().flat_map(|v| v.to_le_bytes()));
    }
    for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
        bin.extend(uv.iter().flat_map(|v| v.to_le_bytes()));
    }
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["bufferViews"].as_array_mut().unwrap().extend([
        json!({"buffer": 0, "byteOffset": 44, "byteLength": 36}),
        json!({"buffer": 0, "byteOffset": 80, "byteLength": 24}),
    ]);
    doc["accessors"].as_array_mut().unwrap().extend([
        json!({"bufferView": 2, "count": 3, "type": "VEC3", "componentType": 5126}),
        json!({"bufferView": 3, "count": 3, "type": "VEC2", "componentType": 5126}),
    ]);
    doc["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION": 0, "NORMAL": 2, "TEXCOORD_0": 3});
    (doc, bin)
}

fn glb_package(doc: &Value, bin: &[u8], extra: Vec<(&'static str, Vec<u8>)>) -> Counted {
    Counted::new([("models/m.glb", pack(doc, bin))].into_iter().chain(extra))
}

#[test]
fn the_probe_bounds_what_the_decoder_holds() {
    for (doc, bin) in [triangle(), triangle_with_uv()] {
        let files = glb_package(&doc, &bin, vec![]);
        let cost = probe_file("models/m.glb", &files, &Limits::default()).unwrap();
        let m = decode_file("models/m.glb", &pack(&doc, &bin), &(), &Limits::default()).unwrap();
        let vertices: usize = m.lods.iter().flat_map(|l| &l.parts).map(|p| p.mesh.positions.len()).sum();
        let triangles: usize = m.lods.iter().flat_map(|l| &l.parts).map(|p| p.mesh.triangle_count()).sum();
        assert!(cost.vertices >= vertices as u64, "{cost:?} for {vertices} vertices");
        assert_eq!(cost.triangles, triangles as u64);
        assert_eq!(cost.held_bytes, bin.len().next_multiple_of(4) as u64, "the BIN chunk");
    }
    let (doc, bin) = triangle();
    let cost = probe_file("models/m.glb", &glb_package(&doc, &bin, vec![]), &Limits::default()).unwrap();
    assert_eq!((cost.vertices, cost.triangles, cost.collision_triangles), (3, 1, 0));
    let mut c = doc.clone();
    c["nodes"] =
        json!([{"name": "rock", "mesh": 0}, {"name": "collision", "mesh": 0, "extras": {"ava_surface": "wood"}}]);
    c["scenes"][0]["nodes"] = json!([0, 1]);
    let cost = probe_file("models/m.glb", &glb_package(&c, &bin, vec![]), &Limits::default()).unwrap();
    assert_eq!((cost.triangles, cost.collision_triangles), (1, 1));
}

#[test]
fn the_probe_reads_no_buffer() {
    let (mut doc, mut bin) = triangle();
    bin.resize(1 << 20, 0);
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    let files = glb_package(&doc, &bin, vec![]);
    let cost = probe_file("models/m.glb", &files, &Limits::default()).unwrap();
    assert_eq!(cost.held_bytes, 1 << 20);
    let json = serde_json::to_vec(&doc).unwrap().len() as u64;
    assert!(files.bytes.get() <= json + 64, "{} bytes read for a JSON of {json}", files.bytes.get());
}

#[test]
fn the_probe_counts_a_file_or_range_once() {
    let (mut doc, bin) = triangle_gltf();
    doc["buffers"] = (0..4096).map(|_| json!({"uri": "m.bin", "byteLength": 44})).collect();
    let files = Counted::new([("models/m.bin", bin.clone()), ("models/m.gltf", serde_json::to_vec(&doc).unwrap())]);
    assert_eq!(probe_file("models/m.gltf", &files, &Limits::default()).unwrap().held_bytes, 44);
    let (mut doc, bin) = triangle_gltf();
    let png = png_bytes();
    materials_with_images(&mut doc, 4096, |_| json!({"uri": "t.png"}));
    let text = serde_json::to_vec(&doc).unwrap();
    let files = Counted::new([("models/m.bin", bin.clone()), ("models/t.png", png.clone()), ("models/m.gltf", text)]);
    let cost = probe_file("models/m.gltf", &files, &Limits::default()).unwrap();
    assert_eq!(cost.held_bytes, 44 + png.len() as u64);
    assert_eq!(cost.drawn_image_bytes, crate::texture::chain_bytes(2, 2));
    let (mut doc, mut bin) = triangle();
    bin.extend(&png);
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    doc["bufferViews"].as_array_mut().unwrap().push(json!({"buffer": 0, "byteOffset": 44, "byteLength": png.len()}));
    materials_with_images(&mut doc, 4096, |_| json!({"bufferView": 2, "mimeType": "image/png"}));
    let cost = probe_file("models/m.glb", &glb_package(&doc, &bin, vec![]), &Limits::default()).unwrap();
    assert_eq!(cost.held_bytes, (bin.len().next_multiple_of(4) + png.len()) as u64);
    let (mut doc, bin) = triangle_gltf();
    doc["buffers"] = (0..8).map(|i| json!({"uri": format!("b{i}.bin"), "byteLength": 44})).collect();
    let names = ["models/b0.bin", "models/b1.bin", "models/b2.bin", "models/b3.bin", "models/b4.bin"];
    let text = serde_json::to_vec(&doc).unwrap();
    let more = ["models/b5.bin", "models/b6.bin", "models/b7.bin"];
    let files = Counted::new(names.iter().chain(&more).map(|n| (*n, bin.clone())).chain([("models/m.gltf", text)]));
    let total = Limits { max_held_bytes: 176, ..Limits::default() };
    let e = probe_file("models/m.gltf", &files, &total).unwrap_err();
    assert!(e.message.contains("a model holds at most 176 bytes"), "{e}");
}

#[test]
fn the_probe_measures_drawn_images_by_header() {
    let (mut doc, bin) = triangle_gltf();
    let big = crate::test_dir::png_header_only(4096, 4096);
    let mut files = Counted::new([("models/m.bin", bin)]);
    for i in 0..4 {
        files.files.insert(format!("models/t{i}.png"), big.clone());
    }
    materials_with_images(&mut doc, 3, |i| json!({"uri": format!("t{i}.png")}));
    files.files.insert("models/m.gltf".into(), serde_json::to_vec(&doc).unwrap());
    let cost = probe_file("models/m.gltf", &files, &Limits::default()).unwrap();
    assert_eq!(cost.drawn_image_bytes, 3 * 89_478_484);
    materials_with_images(&mut doc, 4, |i| json!({"uri": format!("t{i}.png")}));
    files.files.insert("models/m.gltf".into(), serde_json::to_vec(&doc).unwrap());
    let e = probe_file("models/m.gltf", &files, &Limits::default()).unwrap_err();
    assert!(e.message.contains("decode to 357913936 bytes or more"), "{e}");
    doc["materials"] = json!([{"occlusionTexture": {"index": 0}}]);
    files.files.insert("models/m.gltf".into(), serde_json::to_vec(&doc).unwrap());
    let cost = probe_file("models/m.gltf", &files, &Limits::default()).unwrap();
    assert_eq!((cost.drawn_image_bytes, cost.held_bytes), (0, 44 + big.len() as u64));
}

#[test]
fn unused_vertices_leave_before_the_tangents_and_the_bound_holds() {
    let n = 1000usize;
    let mut bin = Vec::new();
    for i in 0..n {
        let p = match i {
            0 => [0.0f32, 0.0, 0.0],
            1 => [1.0, 0.0, 0.0],
            2 => [0.0, 1.0, 0.0],
            _ => [5.0 + i as f32, 0.0, 0.0],
        };
        bin.extend(p.iter().flat_map(|v| v.to_le_bytes()));
    }
    for _ in 0..n {
        bin.extend([0.0f32, 0.0, 1.0].iter().flat_map(|v| v.to_le_bytes()));
    }
    for i in 0..n {
        bin.extend([i as f32 / n as f32, 0.5].iter().flat_map(|v| v.to_le_bytes()));
    }
    let at_index = bin.len();
    bin.extend([0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
    bin.extend([0, 0]);
    let doc = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "rock", "mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3}]}],
        "accessors": [
            {"bufferView": 0, "count": n, "type": "VEC3", "componentType": 5126,
             "min": [0, 0, 0], "max": [5.0 + n as f32, 1, 0]},
            {"bufferView": 1, "count": n, "type": "VEC3", "componentType": 5126},
            {"bufferView": 2, "count": n, "type": "VEC2", "componentType": 5126},
            {"bufferView": 3, "count": 3, "type": "SCALAR", "componentType": 5123}
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": n * 12},
            {"buffer": 0, "byteOffset": n * 12, "byteLength": n * 12},
            {"buffer": 0, "byteOffset": n * 24, "byteLength": n * 8},
            {"buffer": 0, "byteOffset": at_index, "byteLength": 6}
        ],
        "buffers": [{"byteLength": bin.len()}]
    });
    let cost = probe_file("models/m.glb", &glb_package(&doc, &bin, vec![]), &Limits::default()).unwrap();
    let m = decode_file("models/m.glb", &pack(&doc, &bin), &(), &Limits::default()).unwrap();
    let mesh = &m.lods[0].parts[0].mesh;
    assert_eq!(cost.vertices, 3, "three corners");
    assert!(mesh.positions.len() <= 3, "{} vertices drawn", mesh.positions.len());
    assert!(mesh.tangents.as_ref().is_some_and(|t| t.len() == mesh.positions.len()));
    assert!(m.notes.iter().any(|n| n.contains("997")), "{:?}", m.notes);
}
