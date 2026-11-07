use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleEdges {
    pub flags: u32,
    pub edge_cos: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CollisionCluster {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u8; 3]>,
    pub surface_id: u32,
    pub group: u32,
    pub edges: Vec<TriangleEdges>,
    pub material: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collision {
    pub clusters: Vec<CollisionCluster>,
    pub cylinders: usize,
}

impl Collision {
    pub fn triangle_count(&self) -> usize {
        self.clusters.iter().map(|c| c.triangles.len()).sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionTriangle {
    pub corners: [[f32; 3]; 3],
    pub surface_id: u32,
    pub group: u32,
}

pub const MESH_TRIANGLE_FLAGS: u32 = 0x113;
pub const DISC_PART_MATERIAL: [f32; 3] = [0.2, 0.2, 0.1];
pub const MAX_CLUSTER_VERTICES: usize = 256;
pub const MAX_CLUSTER_TRIANGLES: usize = 1024;

pub const SURFACES: [(&str, u32); 13] = [
    ("powder", 0),
    ("rock", 10),
    ("rock_snow", 11),
    ("ice", 20),
    ("ice_snow", 21),
    ("wood", 30),
    ("wood_snow", 31),
    ("metal", 40),
    ("metal_snow", 41),
    ("concrete", 50),
    ("concrete_snow", 51),
    ("metal_hollow", 60),
    ("metal_hollow_snow", 61),
];

pub const GROUPS: [u32; 4] = [0, 1, 2, 11];

pub fn surface_id(name: &str) -> Result<u32, String> {
    if let Some(&(_, id)) = SURFACES.iter().find(|(n, _)| *n == name) {
        return Ok(id);
    }
    let known: Vec<&str> = SURFACES.iter().map(|(n, _)| *n).collect();
    match name {
        "snow" | "packed" => {
            Err(format!("{} is a terrain surface; an object's are {}", ssx_core::text::quoted(name), known.join(", ")))
        }
        _ => Err(format!("ava_surface {}: not one of {}", super::uri::quoted(name), known.join(", "))),
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - b[1] * a[2], a[2] * b[0] - b[2] * a[0], a[0] * b[1] - b[0] * a[1]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

type TriangleSet = ((u32, u32), Vec<[[f32; 3]; 3]>);

pub fn build(triangles: &[CollisionTriangle]) -> (Collision, usize) {
    let mut sets: Vec<TriangleSet> = Vec::new();
    let mut degenerate = 0;
    for t in triangles {
        let n = cross(sub(t.corners[1], t.corners[0]), sub(t.corners[2], t.corners[0]));
        let len2 = dot(n, n);
        if len2 == 0.0 || !len2.is_finite() {
            degenerate += 1;
            continue;
        }
        let key = (t.surface_id, t.group);
        match sets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(t.corners),
            None => sets.push((key, vec![t.corners])),
        }
    }
    let mut out = Collision::default();
    for ((surface_id, group), list) in sets {
        let edges = edges_of(&list);
        let mut cluster = new_cluster(surface_id, group);
        let mut index: HashMap<[u32; 3], u8> = HashMap::new();
        for (t, e) in list.iter().zip(edges) {
            let fresh = t.iter().filter(|p| !index.contains_key(&p.map(f32::to_bits))).count();
            if cluster.vertices.len() + fresh > MAX_CLUSTER_VERTICES || cluster.triangles.len() == MAX_CLUSTER_TRIANGLES
            {
                out.clusters.push(std::mem::replace(&mut cluster, new_cluster(surface_id, group)));
                index.clear();
            }
            let tri = t.map(|p| {
                *index.entry(p.map(f32::to_bits)).or_insert_with(|| {
                    cluster.vertices.push(p);
                    (cluster.vertices.len() - 1) as u8
                })
            });
            cluster.triangles.push(tri);
            cluster.edges.push(e);
        }
        if !cluster.triangles.is_empty() {
            out.clusters.push(cluster);
        }
    }
    (out, degenerate)
}

fn new_cluster(surface_id: u32, group: u32) -> CollisionCluster {
    CollisionCluster {
        vertices: Vec::new(),
        triangles: Vec::new(),
        surface_id,
        group,
        edges: Vec::new(),
        material: DISC_PART_MATERIAL,
    }
}

fn open_edge_cosine() -> f32 {
    1.0 - f32::from_bits(0x411D_E9E7) / 8.0
}

fn edges_of(list: &[[[f32; 3]; 3]]) -> Vec<TriangleEdges> {
    let key = |p: [f32; 3]| p.map(f32::to_bits);
    let mut by_edge: HashMap<([u32; 3], [u32; 3]), Vec<usize>> = HashMap::new();
    for (i, t) in list.iter().enumerate() {
        for k in 0..3 {
            by_edge.entry((key(t[k]), key(t[(k + 1) % 3]))).or_default().push(i);
        }
    }
    list.iter()
        .enumerate()
        .map(|(i, t)| {
            let nb = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let mut flags = MESH_TRIANGLE_FLAGS;
            let mut cos = [1.0f32; 3];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let forward = by_edge.get(&(key(a), key(b))).map_or(0, Vec::len);
                let back = by_edge.get(&(key(b), key(a))).map(Vec::as_slice).unwrap_or_default();
                // TODO(guess): a mod collision mesh's edge codes are made from its own adjacency: an edge shared with
                // exactly one triangle the other way round is convex when (b − a)·(n × n') > 0 (the test the game's
                // reader applies to a quad's diagonal, 0x82F2A100) with the exact cosine of the two face normals; a
                // concave or flat one is not convex (cosine 1.0, as FUN_82E358D0 leaves non-convex edges); an open or
                // non-manifold edge is convex with the cosine of edge code 0 (1 − π²/8, the widest a code carries).
                // The disc's codes come from RenderWare's offline mesh builder (not in the executable), whose metric
                // runs 1–3 codes flatter than n·n' of the quantised faces; no vertex is disabled (0x200 << k unset).
                // Only the rigid bodies (the flares) read these (rigid/fixup.rs); settle: the builder's rule, or the
                // codes of a disc .pob rebuilt from its own triangles compared edge by edge.
                match (forward, back) {
                    (1, [j]) if *j != i => {
                        let u = list[*j];
                        let na = cross(sub(u[1], u[0]), sub(u[2], u[0]));
                        let convex = dot(sub(b, a), cross(nb, na)) > 0.0;
                        let (lb, la) = (dot(nb, nb), dot(na, na));
                        if convex {
                            flags |= 0x20 << k;
                            cos[k] = if lb > f32::MIN_POSITIVE && la > f32::MIN_POSITIVE {
                                (dot(nb, na) / (lb.sqrt() * la.sqrt())).clamp(-1.0, 1.0)
                            } else {
                                1.0
                            };
                        }
                    }
                    _ => {
                        flags |= 0x20 << k;
                        cos[k] = open_edge_cosine();
                    }
                }
            }
            TriangleEdges { flags, edge_cos: cos }
        })
        .collect()
}
