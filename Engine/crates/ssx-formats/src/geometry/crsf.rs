use crate::reader::Reader;
use crate::{Error, Result};

pub use crate::geom::{MAGIC, RSF_HEADER};

pub type Mat4 = [[f32; 4]; 4];

pub const TAG_WEIT: [u8; 4] = *b"WEIT";
pub const TAG_SKTN: [u8; 4] = *b"SKTN";
pub const TAG_FPOF: [u8; 4] = *b"FPOF";
pub const TAG_SGRF: [u8; 4] = *b"SGRF";

const VERSION_WEIT: u16 = 1;
const VERSION_SKTN: u16 = 2;
const VERSION_FPOF: u16 = 1;
const VERSION_SGRF: u16 = 0;
const CHUNK_HEADER_SIZE: usize = 16;
const CHUNK_ALIGNMENT: usize = 16;

const VERSION_NODE: u32 = 4;
const VERSION_GROUP: u32 = 2;
const VERSION_TRANSFORM: u32 = 1;
const VERSION_JOINT: u32 = 2;
const VERSION_RENDERABLE: u32 = 6;
const VERSION_ATTRIBUTES: u32 = 2;
const ATTRIBUTE_V3: u32 = 0x7633;
const MIN_NODE_SIZE: usize = 28;

#[derive(Clone, Debug)]
pub struct CrsfFile {
    pub chunks: Vec<ChunkInfo>,
    pub weights: Option<SkinWeights>,
    pub palette: Option<BonePalette>,
    pub face_offsets: Option<Vec<FaceOffset>>,
    pub scene: Option<SceneGraph>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkInfo {
    pub tag: [u8; 4],
    pub version: u16,
    pub unk: u16,
    pub offset: usize,
    pub size: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinWeights {
    pub unk_a: u32,
    pub unk_b: u32,
    pub influences: usize,
    pub vertex_count: usize,
    pub weights: Vec<f32>,
    pub bones: Vec<u8>,
}

impl SkinWeights {
    pub fn vertex(&self, v: usize) -> (&[f32], &[u8]) {
        let range = v * self.influences..(v + 1) * self.influences;
        (&self.weights[range.clone()], &self.bones[range])
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BonePalette {
    pub bones: Vec<PaletteBone>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaletteBone {
    pub name: String,
    pub inverse_bind: Mat4,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FaceOffset {
    pub joint: String,
    pub translate: [f32; 3],
    pub unk_w: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneGraph {
    pub nodes: Vec<SceneNode>,
}

impl SceneGraph {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    pub fn joints(&self) -> impl Iterator<Item = usize> + '_ {
        self.nodes.iter().enumerate().filter(|(_, n)| matches!(n.kind, NodeKind::Joint(_))).map(|(i, _)| i)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneNode {
    pub parent: Option<usize>,
    pub child_count: usize,
    pub name: String,
    pub unk_a: u16,
    pub flags: u16,
    pub attributes: Option<Vec<Attribute>>,
    pub unk_group_flag: u8,
    pub kind: NodeKind,
}

impl SceneNode {
    pub fn attribute(&self, name: &str) -> Option<[f32; 3]> {
        self.attributes.as_ref()?.iter().find(|a| a.name == name).map(|a| a.value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    Group,
    Transform { local: Mat4 },
    Joint(Joint),
    Renderable(Renderable),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub local: Mat4,
    pub unk_matrix: Mat4,
    pub world_bind: Mat4,
    pub inverse_world_bind: Mat4,
    pub unk_tail: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Renderable {
    pub channels: Vec<Channel>,
    pub base_blend_shape: String,
    pub blend_envelope: f32,
    pub blend_geom_names: Vec<String>,
    pub blend_weights: Vec<f32>,
    pub is_instanced: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub kind: u32,
    pub influences: Vec<String>,
}

fn count(r: &mut Reader, min_size: usize, what: &str) -> Result<usize> {
    let n = r.u32()? as usize;
    if n.saturating_mul(min_size) > r.remaining() {
        return Err(Error::invalid(format!("{what} count {n} does not fit in {} bytes", r.remaining())));
    }
    Ok(n)
}

fn counted_str(r: &mut Reader) -> Result<String> {
    let len = count(r, 1, "string length")?;
    let raw = r.bytes(len)?;
    if raw.contains(&0) || r.u8()? != 0 {
        return Err(Error::invalid(format!("string of length {len} is not NUL-terminated exactly at its end")));
    }
    String::from_utf8(raw.to_vec()).map_err(|_| Error::invalid("string is not UTF-8"))
}

fn mat4(r: &mut Reader) -> Result<Mat4> {
    let mut m = [[0.0; 4]; 4];
    for row in &mut m {
        for v in row {
            *v = r.f32()?;
        }
    }
    Ok(m)
}

fn expect_u32(r: &mut Reader, expected: u32, what: &str) -> Result<()> {
    let at = r.pos();
    let v = r.u32()?;
    if v != expected {
        return Err(Error::invalid(format!("{what} {v} at {at:#x}; every file on the disc has {expected}")));
    }
    Ok(())
}

impl CrsfFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut le = Reader::le(data);
        le.expect(MAGIC)?;
        let rsf = [le.u32()?, le.u32()?, le.u32()?];
        if rsf != RSF_HEADER {
            return Err(Error::invalid(format!("unexpected RSF header {rsf:x?}")));
        }

        let mut file = CrsfFile { chunks: Vec::new(), weights: None, palette: None, face_offsets: None, scene: None };
        let mut r = Reader::be(data);
        r.seek(0x10)?;
        while r.remaining() > 0 {
            let offset = r.pos();
            let tag = r.array::<4>()?;
            let (version, unk) = (r.u16()?, r.u16()?);
            let size = r.u32()? as usize;
            let header_size = r.u32()? as usize;
            if header_size != CHUNK_HEADER_SIZE
                || size < CHUNK_HEADER_SIZE
                || !size.is_multiple_of(CHUNK_ALIGNMENT)
                || size > data.len() - offset
            {
                return Err(Error::invalid(format!(
                    "chunk {:?} at {offset:#x}: size {size:#x}, header size {header_size} ({:#x} bytes left)",
                    String::from_utf8_lossy(&tag),
                    data.len() - offset
                )));
            }
            if file.chunks.iter().any(|c| c.tag == tag) {
                return Err(Error::invalid(format!("second {:?} chunk at {offset:#x}", String::from_utf8_lossy(&tag))));
            }
            let mut c = Reader::be(&data[..offset + size]);
            c.seek(offset + CHUNK_HEADER_SIZE)?;
            let wanted = match tag {
                TAG_WEIT => VERSION_WEIT,
                TAG_SKTN => VERSION_SKTN,
                TAG_FPOF => VERSION_FPOF,
                TAG_SGRF => VERSION_SGRF,
                _ => return Err(Error::invalid(format!("unknown chunk {tag:x?} at {offset:#x}"))),
            };
            if version != wanted {
                return Err(Error::invalid(format!("chunk {tag:x?} version {version}, expected {wanted}")));
            }
            match tag {
                TAG_WEIT => file.weights = Some(SkinWeights::parse(&mut c)?),
                TAG_SKTN => file.palette = Some(BonePalette::parse(&mut c)?),
                TAG_FPOF => file.face_offsets = Some(parse_face_offsets(&mut c)?),
                _ => file.scene = Some(SceneGraph::parse(&mut c)?),
            }
            let padding = c.bytes(c.remaining())?;
            if padding.len() >= CHUNK_ALIGNMENT || padding.iter().any(|&b| b != 0) {
                return Err(Error::invalid(format!(
                    "chunk {tag:x?} at {offset:#x}: {} bytes after the payload are not zero padding",
                    padding.len()
                )));
            }
            file.chunks.push(ChunkInfo { tag, version, unk, offset, size });
            r.seek(offset + size)?;
        }
        Ok(file)
    }
}

impl SkinWeights {
    fn parse(r: &mut Reader) -> Result<Self> {
        let (unk_a, unk_b) = (r.u32()?, r.u32()?);
        let influences = r.u32()? as usize;
        let vertex_count = r.u32()? as usize;
        let n = influences
            .checked_mul(vertex_count)
            .filter(|n| n.saturating_mul(5) <= r.remaining())
            .ok_or_else(|| Error::invalid(format!("{vertex_count} vertices × {influences} influences do not fit")))?;
        let weights = (0..n).map(|_| r.f32()).collect::<Result<Vec<_>>>()?;
        let bones = r.bytes(n)?.to_vec();
        Ok(SkinWeights { unk_a, unk_b, influences, vertex_count, weights, bones })
    }
}

impl BonePalette {
    fn parse(r: &mut Reader) -> Result<Self> {
        let n = count(r, 69, "bone")?;
        let names = (0..n).map(|_| counted_str(r)).collect::<Result<Vec<_>>>()?;
        let mut bones = Vec::with_capacity(n);
        for name in names {
            bones.push(PaletteBone { name, inverse_bind: mat4(r)? });
        }
        Ok(BonePalette { bones })
    }
}

fn parse_face_offsets(r: &mut Reader) -> Result<Vec<FaceOffset>> {
    let n = count(r, 21, "face offset")?;
    (0..n).map(|_| Ok(FaceOffset { joint: counted_str(r)?, translate: r.vec3()?, unk_w: r.f32()? })).collect()
}

impl SceneGraph {
    fn parse(r: &mut Reader) -> Result<Self> {
        let mut nodes: Vec<SceneNode> = Vec::new();
        let mut open: Vec<(usize, usize)> = Vec::new();
        loop {
            let parent = open.last_mut().map(|(i, left)| {
                *left -= 1;
                *i
            });
            let mut node = SceneNode::parse(r, parent)?;
            node.child_count = count(r, MIN_NODE_SIZE, "child")?;
            if node.child_count > 0 {
                open.push((nodes.len(), node.child_count));
            }
            nodes.push(node);
            while open.last().is_some_and(|&(_, left)| left == 0) {
                open.pop();
            }
            if open.is_empty() {
                return Ok(SceneGraph { nodes });
            }
        }
    }
}

impl SceneNode {
    fn parse(r: &mut Reader, parent: Option<usize>) -> Result<Self> {
        let class = counted_str(r)?;
        expect_u32(r, VERSION_NODE, "SGNodeC version")?;
        let name = counted_str(r)?;
        let (unk_a, flags) = (r.u16()?, r.u16()?);
        let attributes = match r.u8()? {
            0 => None,
            1 => Some(parse_attributes(r)?),
            b => return Err(Error::invalid(format!("node {name:?}: attribute flag {b}"))),
        };
        expect_u32(r, VERSION_GROUP, "SGNodeGroupC version")?;
        let unk_group_flag = r.u8()?;
        let kind = match class.as_str() {
            "SGNodeGroup" => NodeKind::Group,
            "SGNodeTransform" => {
                expect_u32(r, VERSION_TRANSFORM, "SGNodeTransformC version")?;
                NodeKind::Transform { local: mat4(r)? }
            }
            "SGNodeJoint" => {
                expect_u32(r, VERSION_TRANSFORM, "SGNodeJointC transform version")?;
                let local = mat4(r)?;
                expect_u32(r, VERSION_JOINT, "SGNodeJointC version")?;
                NodeKind::Joint(Joint {
                    local,
                    unk_matrix: mat4(r)?,
                    world_bind: mat4(r)?,
                    inverse_world_bind: mat4(r)?,
                    unk_tail: r.u32()?,
                })
            }
            "SGNodeRenderable" => NodeKind::Renderable(Renderable::parse(r)?),
            _ => return Err(Error::invalid(format!("node {name:?}: class {class:?} does not occur on the disc"))),
        };
        Ok(SceneNode { parent, child_count: 0, name, unk_a, flags, attributes, unk_group_flag, kind })
    }
}

fn parse_attributes(r: &mut Reader) -> Result<Vec<Attribute>> {
    expect_u32(r, VERSION_ATTRIBUTES, "attribute array version")?;
    let n = count(r, 25, "attribute")?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let (ty, elements) = (r.u32()?, r.u32()?);
        if ty != ATTRIBUTE_V3 || elements != 1 {
            return Err(Error::invalid(format!("attribute type {ty:#x} × {elements}; the disc only has one 'v3'")));
        }
        out.push(Attribute { name: counted_str(r)?, value: r.vec3()? });
    }
    Ok(out)
}

impl Renderable {
    fn parse(r: &mut Reader) -> Result<Self> {
        expect_u32(r, VERSION_RENDERABLE, "SGNodeRenderableC version")?;
        let channel_count = count(r, 8, "channel")?;
        let mut channels = Vec::with_capacity(channel_count);
        for _ in 0..channel_count {
            let kind = r.u32()?;
            let n = count(r, 5, "influence")?;
            channels.push(Channel { kind, influences: (0..n).map(|_| counted_str(r)).collect::<Result<_>>()? });
        }
        let base_blend_shape = counted_str(r)?;
        let blend_envelope = r.f32()?;
        let n = count(r, 5, "blend geometry")?;
        let blend_geom_names = (0..n).map(|_| counted_str(r)).collect::<Result<_>>()?;
        let n = count(r, 4, "blend weight")?;
        let blend_weights = (0..n).map(|_| r.f32()).collect::<Result<_>>()?;
        Ok(Renderable {
            channels,
            base_blend_shape,
            blend_envelope,
            blend_geom_names,
            blend_weights,
            is_instanced: r.u8()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(out: &mut Vec<u8>, text: &str) {
        out.extend((text.len() as u32).to_be_bytes());
        out.extend(text.as_bytes());
        out.push(0);
    }

    fn u(out: &mut Vec<u8>, v: u32) {
        out.extend(v.to_be_bytes());
    }

    fn node(out: &mut Vec<u8>, class: &str, name: &str, flags: u16) {
        s(out, class);
        u(out, VERSION_NODE);
        s(out, name);
        out.extend([0, 0]);
        out.extend(flags.to_be_bytes());
        out.push(0);
        u(out, VERSION_GROUP);
        out.push(1);
    }

    fn file(chunks: &[(&[u8; 4], u16, Vec<u8>)]) -> Vec<u8> {
        let mut out = b"RSF\0".to_vec();
        for w in RSF_HEADER {
            out.extend(w.to_le_bytes());
        }
        for (tag, version, payload) in chunks {
            let size = (16 + payload.len()).next_multiple_of(16);
            out.extend(*tag);
            out.extend(version.to_be_bytes());
            out.extend([0, 0]);
            u(&mut out, size as u32);
            u(&mut out, 16);
            out.extend(payload);
            out.resize(out.len() + size - 16 - payload.len(), 0);
        }
        out
    }

    fn proxy_tree() -> Vec<u8> {
        let mut p = Vec::new();
        node(&mut p, "SGNodeGroup", "RootNode", 0x10);
        u(&mut p, 1);
        node(&mut p, "SGNodeTransform", "base transform", 0x10);
        u(&mut p, VERSION_TRANSFORM);
        for i in 0..16 {
            p.extend(if i % 5 == 0 { 1.0f32 } else { 0.0 }.to_be_bytes());
        }
        u(&mut p, 2);
        for name in ["a", "b"] {
            node(&mut p, "SGNodeRenderable", name, 0x14);
            u(&mut p, VERSION_RENDERABLE);
            p.extend([0; 4 + 5 + 4 + 4 + 4 + 1]);
            u(&mut p, 0);
        }
        p
    }

    #[test]
    fn scene_tree_parents_follow_the_child_counts() {
        let f = CrsfFile::parse(&file(&[(b"SGRF", 0, proxy_tree())])).unwrap();
        let g = f.scene.unwrap();
        let parents: Vec<_> = g.nodes.iter().map(|n| n.parent).collect();
        assert_eq!(parents, [None, Some(0), Some(1), Some(1)]);
        assert_eq!(g.nodes[1].child_count, 2);
        assert!(matches!(g.nodes[3].kind, NodeKind::Renderable(_)));
    }

    #[test]
    fn weights_and_palette() {
        let mut weit = Vec::new();
        for w in [1, 0, 2, 2] {
            u(&mut weit, w);
        }
        for w in [0.75f32, 0.25, 1.0, 0.0] {
            weit.extend(w.to_be_bytes());
        }
        weit.extend([1, 0, 0, 0]);
        let mut sktn = Vec::new();
        u(&mut sktn, 1);
        s(&mut sktn, "Hips");
        sktn.extend([0; 64]);
        let f = CrsfFile::parse(&file(&[(b"WEIT", 1, weit), (b"SKTN", 2, sktn)])).unwrap();
        let w = f.weights.unwrap();
        assert_eq!((w.vertex_count, w.influences), (2, 2));
        assert_eq!(w.vertex(0), (&[0.75, 0.25][..], &[1, 0][..]));
        assert_eq!(f.palette.unwrap().bones[0].name, "Hips");
    }

    #[test]
    fn rejects_unknown_chunks_padding_and_short_trees() {
        assert!(CrsfFile::parse(&file(&[(b"CLTH", 1, vec![0; 4])])).is_err());
        let mut bad_padding = file(&[(b"SGRF", 0, proxy_tree())]);
        *bad_padding.last_mut().unwrap() = 1;
        assert!(CrsfFile::parse(&bad_padding).is_err());
        let mut short = proxy_tree();
        short.truncate(short.len() - 4);
        u(&mut short, 1);
        assert!(CrsfFile::parse(&file(&[(b"SGRF", 0, short)])).is_err());
        let twice = file(&[(b"SGRF", 0, proxy_tree()), (b"SGRF", 0, proxy_tree())]);
        assert!(CrsfFile::parse(&twice).is_err());
    }
}
