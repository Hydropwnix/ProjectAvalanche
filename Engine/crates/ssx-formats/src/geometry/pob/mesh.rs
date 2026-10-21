use super::*;

#[derive(Clone, Debug)]
pub struct ClusteredMesh {
    pub header: AggregateHeader,
    pub granularity: f32,
    pub flags: u16,
    pub group_id_bytes: u8,
    pub surface_id_bytes: u8,
    pub unk_max_clusters: u32,
    pub num_units: u32,
    pub unk_max_units: u32,
    pub size: u32,
    pub unk_54: u16,
    pub unk_56: u16,
    pub unk_58: u8,
    pub cluster_bits: u32,
    pub kd_tree: KdTree,
    pub clusters: Vec<Cluster>,
}

#[derive(Clone, Debug)]
pub struct KdTree {
    pub num_entries: u32,
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub num_sub_trees: u32,
    pub nodes: Vec<KdNode>,
}

#[derive(Clone, Copy, Debug)]
pub struct KdNode {
    pub parent: u32,
    pub axis: u32,
    pub children: [KdChild; 2],
    pub extents: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KdChild {
    Branch(u32),
    Leaf { count: u32, cluster: u16, offset: u16 },
}

#[derive(Clone, Debug)]
pub struct Cluster {
    pub offset: usize,
    pub unit_data_size: u16,
    pub unit_data_start: u16,
    pub normal_start: u16,
    pub total_size: u16,
    pub compression: u8,
    pub base: Option<[i32; 3]>,
    pub vertices: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub units: Vec<Unit>,
}

#[derive(Clone, Debug)]
pub struct Unit {
    pub offset: u16,
    pub flags: u8,
    pub indices: Vec<u8>,
    pub edge_codes: Vec<u8>,
    pub group_id: Option<u16>,
    pub surface_id: Option<u16>,
}

impl Unit {
    pub fn kind(&self) -> u8 {
        self.flags & 0x0F
    }

    pub fn triangles(&self) -> Option<Vec<[u8; 3]>> {
        let i = &self.indices;
        match self.kind() {
            1 => Some(vec![[i[0], i[1], i[2]]]),
            2 => Some(vec![[i[0], i[1], i[2]], [i[3], i[2], i[1]]]),
            _ => None,
        }
    }

    pub fn triangle_edge_codes(&self) -> Option<Vec<[Option<u8>; 3]>> {
        if self.edge_codes.is_empty() {
            return None;
        }
        let e = &self.edge_codes;
        match self.kind() {
            1 => Some(vec![[Some(e[0]), Some(e[1]), Some(e[2])]]),
            2 => Some(vec![[Some(e[0]), None, Some(e[2])], [Some(e[3]), None, Some(e[1])]]),
            _ => None,
        }
    }
}

pub fn edge_cosine(code: u8) -> f32 {
    let pi2 = f32::from_bits(0x411D_E9E7);
    1.0 - pi2 / ((8i64 << (code & 0x1F)) as f32)
}

pub(super) fn parse_clustered_mesh(obj: &[u8]) -> Result<ClusteredMesh> {
    let len = obj.len();
    let mut r = Reader::le(obj);
    let header = parse_aggregate_header(&mut r)?;
    let kd_at = local(r.u32()?, len, "KD-tree")?.ok_or_else(|| bad("null KD-tree"))?;
    let table_at = local(r.u32()?, len, "cluster table")?.ok_or_else(|| bad("null cluster table"))?;
    let granularity = r.f32()?;
    let flags = r.u16()?;
    let group_id_bytes = r.u8()?;
    let surface_id_bytes = r.u8()?;
    let num_clusters = r.u32()? as usize;
    let unk_max_clusters = r.u32()?;
    let num_units = r.u32()?;
    let unk_max_units = r.u32()?;
    let size = r.u32()?;
    let unk_54 = r.u16()?;
    let unk_56 = r.u16()?;
    let unk_58 = r.u8()?;
    r.skip(3)?;
    let cluster_bits = r.u32()?;
    if size as usize != len {
        return Err(bad(format!("mesh size field {size} != object size {len}")));
    }
    if !(granularity.is_finite() && granularity > 0.0) {
        return Err(bad(format!("granularity {granularity}")));
    }
    let kd_tree = parse_kd_tree(obj, kd_at)?;
    let mut t = at(obj, table_at)?;
    let mut clusters = Vec::with_capacity(num_clusters.min(len / 4));
    for i in 0..num_clusters {
        let rel = t.u32()? as usize;
        let c = parse_cluster(obj, table_at + rel, granularity, group_id_bytes, surface_id_bytes)
            .map_err(|e| bad(format!("cluster {i}: {e}")))?;
        clusters.push(c);
    }
    let units: usize = clusters.iter().map(|c| c.units.len()).sum();
    if units != num_units as usize || units != header.num_volumes as usize || units != kd_tree.num_entries as usize {
        return Err(bad(format!(
            "unit count {units} vs header {num_units}, aggregate {}, KD-tree {}",
            header.num_volumes, kd_tree.num_entries
        )));
    }
    let mesh = ClusteredMesh {
        header,
        granularity,
        flags,
        group_id_bytes,
        surface_id_bytes,
        unk_max_clusters,
        num_units,
        unk_max_units,
        size,
        unk_54,
        unk_56,
        unk_58,
        cluster_bits,
        kd_tree,
        clusters,
    };
    mesh.check_leaves()?;
    Ok(mesh)
}

impl ClusteredMesh {
    fn check_leaves(&self) -> Result<()> {
        for leaf in self.kd_tree.leaves() {
            let KdChild::Leaf { count, cluster, offset } = leaf else { continue };
            let c = self.clusters.get(cluster as usize).ok_or_else(|| bad(format!("leaf cluster {cluster}")))?;
            if count == 0 {
                continue;
            }
            let first = c.units.iter().position(|u| u.offset == offset);
            match first {
                Some(i) if i + count as usize <= c.units.len() => {}
                _ => {
                    return Err(bad(format!("leaf ({count} units at {cluster}:{offset:#x}) does not fit its cluster")));
                }
            }
        }
        Ok(())
    }

    pub fn unit(&self, cluster: u16, offset: u16) -> Option<&Unit> {
        self.clusters.get(cluster as usize)?.units.iter().find(|u| u.offset == offset)
    }
}

impl KdTree {
    pub fn leaves(&self) -> Vec<KdChild> {
        if self.nodes.is_empty() {
            return vec![KdChild::Leaf { count: self.num_entries, cluster: 0, offset: 0 }];
        }
        self.nodes.iter().flat_map(|n| n.children).filter(|c| matches!(c, KdChild::Leaf { .. })).collect()
    }
}

fn parse_kd_tree(obj: &[u8], pos: usize) -> Result<KdTree> {
    let mut r = at(obj, pos)?;
    let nodes_at = local(r.u32()?, obj.len(), "KD nodes")?;
    let num_nodes = r.u32()? as usize;
    let num_entries = r.u32()?;
    r.skip(4)?;
    let aabb_min = r.vec3()?;
    r.skip(4)?;
    let aabb_max = r.vec3()?;
    r.skip(4)?;
    let num_sub_trees = r.u32()?;
    let sub_trees = r.u32()?;
    if num_sub_trees != 0 || sub_trees != NULL {
        return Err(bad(format!("KD sub-trees ({num_sub_trees} at {sub_trees:#x}) not supported")));
    }
    let mut nodes = Vec::with_capacity(num_nodes.min(obj.len() / 32));
    if num_nodes > 0 {
        let mut n = at(obj, nodes_at.ok_or_else(|| bad("null KD nodes"))?)?;
        for i in 0..num_nodes {
            let parent = n.u32()?;
            let axis = n.u32()?;
            let mut children = [KdChild::Branch(0); 2];
            for child in &mut children {
                let (content, index) = (n.u32()?, n.u32()?);
                *child = if content == NULL {
                    if index as usize <= i || index as usize >= num_nodes {
                        return Err(bad(format!("KD node {i} child index {index} of {num_nodes}")));
                    }
                    KdChild::Branch(index)
                } else {
                    KdChild::Leaf { count: content, cluster: (index >> 16) as u16, offset: index as u16 }
                };
            }
            let extents = [n.f32()?, n.f32()?];
            if axis > 2 || (i > 0 && parent as usize >= i) {
                return Err(bad(format!("KD node {i}: axis {axis}, parent {parent}")));
            }
            nodes.push(KdNode { parent, axis, children, extents });
        }
    }
    Ok(KdTree { num_entries, aabb_min, aabb_max, num_sub_trees, nodes })
}

fn parse_cluster(obj: &[u8], pos: usize, granularity: f32, group_bytes: u8, surface_bytes: u8) -> Result<Cluster> {
    let mut r = at(obj, pos)?;
    let unit_count = r.u16()?;
    let unit_data_size = r.u16()?;
    let unit_data_start = r.u16()?;
    let normal_start = r.u16()?;
    let total_size = r.u16()?;
    let vertex_count = r.u8()? as usize;
    let normal_count = r.u8()? as usize;
    let compression = r.u8()?;
    let cl = obj.get(pos..pos + total_size as usize).ok_or_else(|| bad("cluster past end of mesh"))?;
    let units_at = (unit_data_start as usize + 1) * 16;
    let normals_at = (normal_start as usize + 1) * 16;
    if units_at + unit_data_size as usize != cl.len() || normals_at + 16 * normal_count > units_at {
        return Err(bad(format!(
            "spans: normals {normals_at:#x}+{normal_count}, units {units_at:#x}+{unit_data_size}, size {total_size}"
        )));
    }
    let mut v = at(cl, 16)?;
    let mut base = None;
    let vertices = match compression {
        0 => (0..vertex_count)
            .map(|_| {
                let p = v.vec3()?;
                v.skip(4)?;
                Ok(p)
            })
            .collect::<Result<Vec<_>>>()?,
        1 => {
            let b = [v.i32()?, v.i32()?, v.i32()?];
            base = Some(b);
            (0..vertex_count)
                .map(|_| {
                    let d = [v.u16()?, v.u16()?, v.u16()?];
                    Ok(std::array::from_fn(|a| b[a].saturating_add(d[a] as i32) as f32 * granularity))
                })
                .collect::<Result<Vec<_>>>()?
        }
        2 => (0..vertex_count)
            .map(|_| Ok([v.i32()? as f32 * granularity, v.i32()? as f32 * granularity, v.i32()? as f32 * granularity]))
            .collect::<Result<Vec<_>>>()?,
        other => return Err(bad(format!("compression mode {other}"))),
    };
    if v.pos() > normals_at {
        return Err(bad(format!("vertices end at {:#x}, past normal start {normals_at:#x}", v.pos())));
    }
    let mut n = at(cl, normals_at)?;
    let normals = (0..normal_count)
        .map(|_| {
            let p = n.vec3()?;
            n.skip(4)?;
            Ok(p)
        })
        .collect::<Result<Vec<_>>>()?;
    let stream = &cl[units_at..];
    let mut u = Reader::le(stream);
    let mut units = Vec::with_capacity(unit_count as usize);
    for _ in 0..unit_count {
        units.push(parse_unit(&mut u, vertex_count, group_bytes, surface_bytes)?);
    }
    if u.remaining() != 0 {
        return Err(bad(format!("{} bytes left after {unit_count} units", u.remaining())));
    }
    Ok(Cluster {
        offset: pos,
        unit_data_size,
        unit_data_start,
        normal_start,
        total_size,
        compression,
        base,
        vertices,
        normals,
        units,
    })
}

pub(super) fn parse_unit(r: &mut Reader, vertex_count: usize, group_bytes: u8, surface_bytes: u8) -> Result<Unit> {
    let offset = r.pos() as u16;
    let flags = r.u8()?;
    let n = match flags & 0x0F {
        1 => 3,
        2 => 4,
        3 => r.u8()? as usize + 2,
        k => return Err(bad(format!("unit kind {k} at {offset:#x}"))),
    };
    let indices = r.bytes(n)?.to_vec();
    if let Some(&i) = indices.iter().find(|&&i| i as usize >= vertex_count) {
        return Err(bad(format!("unit at {offset:#x} uses vertex {i} of {vertex_count}")));
    }
    let edge_codes = if flags & 0x20 != 0 { r.bytes(n)?.to_vec() } else { Vec::new() };
    let mut id = |width: u8| -> Result<u16> {
        let lo = r.u8()? as u16;
        Ok(if width == 2 { lo | (r.u8()? as u16) << 8 } else { lo })
    };
    let group_id = if flags & 0x40 != 0 { Some(id(group_bytes)?) } else { None };
    let surface_id = if flags & 0x80 != 0 { Some(id(surface_bytes)?) } else { None };
    Ok(Unit { offset, flags, indices, edge_codes, group_id, surface_id })
}
