use std::fmt::Write as _;

pub fn ground(z: f32) -> f32 {
    if z <= 300.0 { -0.3 * z } else { -90.0 - 0.05 * (z - 300.0) }
}

pub const HALF_WIDTH: f32 = 30.0;
pub const LENGTH: f32 = 360.0;
const STEP: f32 = 10.0;

pub const HELI_INTRO: [f32; 2] = [400.0, -1500.0];
pub const HELI_PICKUP_Z: f32 = 355.0;
pub const FLARE: &str = "ssx:object/markerflare_sml_red_gameobject";
pub const FLARE_AT: [f32; 2] = [1.5, 150.0];

pub const MAP_TOML: &str = r#"# A map of this package (every key: Docs/architecture/content.md §22.3).
[map]
title = "Template slope"
# The scene: its nodes say what they are (extras ava_type: terrain, rail, spawn, checkpoint, finish, marker, death …).
scene = "maps/course.gltf"
# A layout on a disc drop names its drop instead of bringing terrain: base = "ssx:drop/<ui_drop name>".
modes = ["race", "freeride"]
# Where the ride starts: "spawn" (at the spawn node), or "heli" — in the helicopter, which flies the scene's two
# heli_path empties (order 0 the pilot's shot, 1 the pickup after the finish) as a disc drop's helicopter does.
intro = "spawn"
# A new track's distant scenery: a disc track's mid-ground, drawn far off without collision.
# midground = "ssx:track/alps0101"
"#;

fn f(x: f32) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
}

struct Part {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

fn slope() -> Part {
    let (nx, nz) = ((2.0 * HALF_WIDTH / STEP) as u32 + 1, (LENGTH / STEP) as u32 + 1);
    let mut positions = Vec::new();
    for j in 0..nz {
        for i in 0..nx {
            let (x, z) = (-HALF_WIDTH + i as f32 * STEP, j as f32 * STEP);
            positions.push([x, ground(z), z]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..nz - 1 {
        for i in 0..nx - 1 {
            let (a, b, c, d) = (j * nx + i, j * nx + i + 1, (j + 1) * nx + i, (j + 1) * nx + i + 1);
            indices.extend([a, c, b, b, c, d]);
        }
    }
    Part { positions, indices }
}

fn prism(ends: [(f32, [[f32; 2]; 4]); 2]) -> Part {
    let mut positions = Vec::new();
    for (z, corners) in ends {
        for [x, y] in corners {
            positions.push([x, y, z]);
        }
    }
    let indices = vec![
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5,
    ];
    Part { positions, indices }
}

fn rail() -> Part {
    let end = |z: f32| {
        let g = ground(z);
        (z, [[9.85, g + 0.2], [10.15, g + 0.2], [10.15, g + 0.5], [9.85, g + 0.5]])
    };
    prism([end(120.0), end(180.0)])
}

fn unit_cube() -> Part {
    let end = |z: f32| (z, [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]]);
    prism([end(-0.5), end(0.5)])
}

pub(crate) fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                out.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn gltf() -> String {
    let parts = [slope(), rail(), unit_cube()];
    let (mut bin, mut accessors, mut views) = (Vec::new(), String::new(), String::new());
    for (k, p) in parts.iter().enumerate() {
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        let start = bin.len();
        for q in &p.positions {
            for c in 0..3 {
                lo[c] = lo[c].min(q[c]);
                hi[c] = hi[c].max(q[c]);
            }
            q.iter().for_each(|x| bin.extend(x.to_le_bytes()));
        }
        let istart = bin.len();
        p.indices.iter().for_each(|i| bin.extend(i.to_le_bytes()));
        let sep = if k == 0 { "" } else { "," };
        let _ = write!(
            accessors,
            "{sep}{{\"bufferView\":{},\"componentType\":5126,\"count\":{},\"type\":\"VEC3\",\"min\":[{},{},{}],\"max\":[{},{},{}]}},\
             {{\"bufferView\":{},\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}",
            2 * k,
            p.positions.len(),
            f(lo[0]),
            f(lo[1]),
            f(lo[2]),
            f(hi[0]),
            f(hi[1]),
            f(hi[2]),
            2 * k + 1,
            p.indices.len()
        );
        let _ = write!(
            views,
            "{sep}{{\"buffer\":0,\"byteOffset\":{start},\"byteLength\":{}}},{{\"buffer\":0,\"byteOffset\":{istart},\"byteLength\":{}}}",
            istart - start,
            bin.len() - istart
        );
    }
    let at = |z: f32, up: f32| format!("[0.0,{},{}]", f(ground(z) + up), f(z));
    let nodes = [
        "{\"name\":\"course\",\"children\":[1,2,3,4,5,6,7,8,9,10,11]}".to_string(),
        "{\"name\":\"slope\",\"mesh\":0,\"extras\":{\"ava_type\":\"terrain\",\"ava_surface\":\"groomed\"}}".to_string(),
        "{\"name\":\"rail\",\"mesh\":1,\"extras\":{\"ava_type\":\"rail\",\"ava_surface\":\"metal\"}}".to_string(),
        format!("{{\"name\":\"spawn\",\"translation\":{},\"extras\":{{\"ava_type\":\"spawn\",\"ava_slot\":0}}}}", at(5.0, 0.0)),
        format!(
            "{{\"name\":\"checkpoint 1\",\"translation\":{},\"extras\":{{\"ava_type\":\"checkpoint\",\"ava_order\":0,\"ava_radius\":15.0}}}}",
            at(100.0, 2.0)
        ),
        format!(
            "{{\"name\":\"checkpoint 2\",\"translation\":{},\"extras\":{{\"ava_type\":\"checkpoint\",\"ava_order\":1,\"ava_radius\":15.0}}}}",
            at(220.0, 2.0)
        ),
        format!(
            "{{\"name\":\"finish\",\"mesh\":2,\"translation\":{},\"scale\":[60.0,20.0,10.0],\"extras\":{{\"ava_type\":\"finish\"}}}}",
            at(345.0, 5.0)
        ),
        format!(
            "{{\"name\":\"gate\",\"translation\":{},\"extras\":{{\"ava_type\":\"marker\",\"ava_marker\":\"ava:marker/gate\"}}}}",
            at(100.0, 0.0)
        ),
        "{\"name\":\"below\",\"mesh\":2,\"translation\":[0.0,-170.0,200.0],\"scale\":[600.0,100.0,600.0],\"extras\":{\"ava_type\":\"death\"}}".to_string(),
        format!(
            "{{\"name\":\"heli intro\",\"translation\":[0.0,{},{}],\"extras\":{{\"ava_type\":\"heli_path\",\"ava_order\":0}}}}",
            f(HELI_INTRO[0]),
            f(HELI_INTRO[1])
        ),
        format!(
            "{{\"name\":\"heli pickup\",\"translation\":{},\"extras\":{{\"ava_type\":\"heli_path\",\"ava_order\":1}}}}",
            at(HELI_PICKUP_Z, 0.0)
        ),
        format!(
            "{{\"name\":\"flare\",\"translation\":[{},{},{}],\"extras\":{{\"ava_type\":\"prop\",\"ava_object\":\"{FLARE}\"}}}}",
            f(FLARE_AT[0]),
            f(ground(FLARE_AT[1])),
            f(FLARE_AT[1])
        ),
    ];
    let meshes = ["slope", "rail", "box"]
        .iter()
        .enumerate()
        .map(|(k, n)| {
            format!(
                "{{\"name\":\"{n}\",\"primitives\":[{{\"attributes\":{{\"POSITION\":{}}},\"indices\":{}}}]}}",
                2 * k,
                2 * k + 1
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"Project Avalanche map template\"}},\"scene\":0,\
         \"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{}],\"meshes\":[{meshes}],\"accessors\":[{accessors}],\
         \"bufferViews\":[{views}],\"buffers\":[{{\"byteLength\":{},\"uri\":\"data:application/octet-stream;base64,{}\"}}]}}\n",
        nodes.join(","),
        bin.len(),
        base64(&bin)
    )
}
