use crate::reader::Reader;
use crate::{Error, Result};

pub const MAGIC: &[u8; 4] = b"RSF\0";

pub const RSF_HEADER: [u32; 3] = [1, 0x30, 0];

const FILE_ALIGNMENT: usize = 16;
const GEOM_VERSION: u16 = 4;
const CHUNK_HEADER_SIZE: usize = 16;

#[derive(Clone, Debug)]
pub struct GeomFile {
    pub version: u16,
    pub unk_chunk: u16,
    pub header_size: u32,
    pub vertex_format: VertexFormat,
    pub streams: Vec<VertexStream>,
    pub index_buffers: Vec<IndexBuffer>,
    pub meshes: Vec<Mesh>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VertexFormat {
    pub text: String,
    pub attributes: Vec<VertexAttribute>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VertexAttribute {
    pub name: String,
    pub semantic: Semantic,
    pub set: u8,
    pub offset: usize,
    pub stream: usize,
    pub unk_field: u32,
    pub ty: AttributeType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Semantic {
    Position,
    Normal,
    Tangent,
    TexCoord,
    Colour,
}

impl Semantic {
    fn from_letter(c: char) -> Option<Self> {
        Some(match c {
            'p' => Semantic::Position,
            'n' => Semantic::Normal,
            'g' => Semantic::Tangent,
            't' => Semantic::TexCoord,
            'c' => Semantic::Colour,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttributeType {
    F32x4,
    F32x3,
    F32x2,
    F16x4,
    F16x2,
    S10x3Norm,
    U8x4Norm,
}

impl AttributeType {
    pub const ALL: [AttributeType; 7] = [
        AttributeType::F32x4,
        AttributeType::F32x3,
        AttributeType::F32x2,
        AttributeType::F16x4,
        AttributeType::F16x2,
        AttributeType::S10x3Norm,
        AttributeType::U8x4Norm,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            AttributeType::F32x4 => "4f32",
            AttributeType::F32x3 => "3f32",
            AttributeType::F32x2 => "2f32",
            AttributeType::F16x4 => "4f16",
            AttributeType::F16x2 => "2f16",
            AttributeType::S10x3Norm => "3s10n",
            AttributeType::U8x4Norm => "4u8n",
        }
    }

    pub fn size(self) -> usize {
        match self {
            AttributeType::F32x4 => 16,
            AttributeType::F32x3 => 12,
            AttributeType::F32x2 | AttributeType::F16x4 => 8,
            AttributeType::F16x2 | AttributeType::S10x3Norm | AttributeType::U8x4Norm => 4,
        }
    }

    pub fn components(self) -> usize {
        match self {
            AttributeType::F32x4 | AttributeType::F16x4 | AttributeType::U8x4Norm => 4,
            AttributeType::F32x3 | AttributeType::S10x3Norm => 3,
            AttributeType::F32x2 | AttributeType::F16x2 => 2,
        }
    }

    pub fn decode(self, bytes: &[u8]) -> Result<[f32; 4]> {
        let mut r = Reader::be(bytes);
        let mut out = [0.0f32; 4];
        match self {
            AttributeType::F32x4 | AttributeType::F32x3 | AttributeType::F32x2 => {
                for v in &mut out[..self.components()] {
                    *v = r.f32()?;
                }
            }
            AttributeType::F16x4 | AttributeType::F16x2 => {
                for v in &mut out[..self.components()] {
                    *v = f16_to_f32(r.u16()?);
                }
            }
            AttributeType::S10x3Norm => {
                let [x, y, z] = unpack_s10x3(r.u32()?);
                out[..3].copy_from_slice(&[snorm10(x), snorm10(y), snorm10(z)]);
            }
            AttributeType::U8x4Norm => {
                for v in &mut out {
                    *v = f32::from(r.u8()?) / 255.0;
                }
            }
        }
        Ok(out)
    }
}

pub fn unpack_s10x3(word: u32) -> [i16; 3] {
    std::array::from_fn(|i| {
        let field = (word >> (10 * i)) & 0x3ff;
        (field as i16 ^ 0x200) - 0x200
    })
}

pub fn snorm10(v: i16) -> f32 {
    (f32::from(v) / 511.0).max(-1.0)
}

pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from(bits >> 10) & 0x1f;
    let mantissa = u32::from(bits) & 0x3ff;
    match exponent {
        0 => {
            let magnitude = mantissa as f32 * (1.0 / 16_777_216.0);
            if sign != 0 { -magnitude } else { magnitude }
        }
        0x1f => f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13)),
        _ => f32::from_bits(sign | ((exponent + 127 - 15) << 23) | (mantissa << 13)),
    }
}

#[derive(Clone, Debug)]
pub struct VertexStream {
    pub unk_flag: u8,
    pub unk_kind: u32,
    pub vertex_count: usize,
    pub stride: usize,
    pub format_index: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct IndexBuffer {
    pub unk_flag: u8,
    pub unk_kind: u32,
    pub indices: Vec<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshVersion {
    Geo3,
    Geo4,
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub unk_header: u32,
    pub triangles: TriangleList,
    pub version: MeshVersion,
    pub name: String,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub damage_targets: Vec<DamageTarget>,
    pub translucent: bool,
    pub groups: Vec<MeshGroup>,
    pub unk_trailer: [u32; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamageTarget {
    pub name: String,
    pub slot: String,
}

#[derive(Clone, Debug)]
pub struct TriangleList {
    pub unk_a: u32,
    pub unk_start: u32,
    pub triangle_count: usize,
    pub format_index: u32,
    pub unk_b: u32,
    pub unk_c: u32,
    pub stream_ids: Vec<u32>,
    pub unk_d: u32,
    pub unk_e: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct MeshGroup {
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub first_triangle: usize,
    pub triangle_count: usize,
}

impl VertexFormat {
    pub fn parse(text: &str) -> Result<Self> {
        let mut attributes: Vec<VertexAttribute> = Vec::new();
        for entry in text.split(' ') {
            let bad = |why: &str| Error::invalid(format!("VFMT entry {entry:?}: {why}"));
            let fields: Vec<&str> = entry.split(':').collect();
            let [name, offset, stream, unk, ty] = fields[..] else {
                return Err(bad("expected 5 ':'-separated fields"));
            };
            let mut chars = name.chars();
            let semantic = chars.next().and_then(Semantic::from_letter).ok_or_else(|| bad("unknown semantic"))?;
            let set = chars.as_str().parse::<u8>().map_err(|_| bad("bad set index"))?;
            let hex = |s: &str| u32::from_str_radix(s, 16).map_err(|_| bad("bad hex field"));
            let attribute = VertexAttribute {
                name: name.to_owned(),
                semantic,
                set,
                offset: hex(offset)? as usize,
                stream: hex(stream)? as usize,
                unk_field: hex(unk)?,
                ty: AttributeType::from_name(ty).ok_or_else(|| bad("attribute type not seen on the disc"))?,
            };
            if attributes.iter().any(|a| a.name == attribute.name) {
                return Err(bad("duplicate attribute name"));
            }
            let packed_offset = attributes.last().map_or(0, |a| a.offset + a.ty.size());
            if attribute.offset != packed_offset {
                return Err(bad(&format!(
                    "offset {:#x}, attributes so far end at {packed_offset:#x}",
                    attribute.offset
                )));
            }
            attributes.push(attribute);
        }
        Ok(VertexFormat { text: text.to_owned(), attributes })
    }

    pub fn packed_size(&self) -> usize {
        self.attributes.iter().map(|a| a.ty.size()).sum()
    }

    pub fn attribute(&self, semantic: Semantic, set: u8) -> Option<&VertexAttribute> {
        self.attributes.iter().find(|a| a.semantic == semantic && a.set == set)
    }
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

fn exactly_one(n: usize, what: &str) -> Result<()> {
    if n != 1 {
        return Err(Error::invalid(format!("{n} {what}; every .geom on the disc has exactly one")));
    }
    Ok(())
}

impl GeomFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut le = Reader::le(data);
        le.expect(MAGIC)?;
        let rsf = [le.u32()?, le.u32()?, le.u32()?];
        if rsf != RSF_HEADER {
            return Err(Error::invalid(format!("unexpected RSF header {rsf:x?}")));
        }

        let mut r = Reader::be(data);
        r.seek(0x10)?;
        r.expect(b"GEOM")?;
        let (version, unk_chunk) = (r.u16()?, r.u16()?);
        if version != GEOM_VERSION {
            return Err(Error::invalid(format!("GEOM version {version}; every .geom on the disc has {GEOM_VERSION}")));
        }
        let size = r.u32()? as usize;
        if size != data.len() - 0x10 {
            return Err(Error::invalid(format!(
                "GEOM size {size:#x}, file has {:#x} bytes after the header",
                data.len() - 0x10
            )));
        }
        let header_size = r.u32()?;
        if header_size as usize != CHUNK_HEADER_SIZE {
            return Err(Error::invalid(format!("GEOM header size {header_size}, expected {CHUNK_HEADER_SIZE}")));
        }

        let format_count = count(&mut r, 9, "vertex format")?;
        exactly_one(format_count, "vertex formats")?;
        let mut formats = Vec::with_capacity(format_count);
        for _ in 0..format_count {
            r.expect(b"VFMT")?;
            formats.push(VertexFormat::parse(&counted_str(&mut r)?)?);
        }
        let vertex_format = formats.remove(0);

        let stream_count = count(&mut r, 21, "stream")?;
        exactly_one(stream_count, "vertex streams")?;
        let mut streams = Vec::with_capacity(stream_count);
        for _ in 0..stream_count {
            r.expect(b"STRM")?;
            let (unk_flag, unk_kind) = (r.u8()?, r.u32()?);
            let (vertex_count, stride) = (r.u32()? as usize, r.u32()? as usize);
            let format_index = r.u32()?;
            if format_index as usize >= format_count {
                return Err(Error::invalid(format!("stream names vertex format {format_index}")));
            }
            let len = vertex_count.checked_mul(stride).ok_or_else(|| Error::invalid("stream size overflows"))?;
            let data = r.bytes(len)?.to_vec();
            streams.push(VertexStream { unk_flag, unk_kind, vertex_count, stride, format_index, data });
        }
        Self::check_streams(&vertex_format, &streams)?;
        let vertex_count = streams[0].vertex_count;

        let buffer_count = count(&mut r, 17, "index buffer")?;
        exactly_one(buffer_count, "index buffers")?;
        let mut index_buffers = Vec::with_capacity(buffer_count);
        for _ in 0..buffer_count {
            r.expect(b"INDX")?;
            let (unk_flag, unk_kind) = (r.u8()?, r.u32()?);
            let index_count = r.u32()? as usize;
            r.expect(b"ID16")?;
            let raw = r.bytes(index_count.checked_mul(2).ok_or_else(|| Error::invalid("index count overflows"))?)?;
            let indices: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|&b| u16::from_be_bytes(b)).collect();
            if let Some(&bad) = indices.iter().find(|&&i| usize::from(i) >= vertex_count) {
                return Err(Error::invalid(format!("index {bad} out of range ({vertex_count} vertices)")));
            }
            index_buffers.push(IndexBuffer { unk_flag, unk_kind, indices });
        }

        let mesh_packets = r.u32()?;
        if mesh_packets != 0 {
            return Err(Error::invalid(format!("{mesh_packets} mesh packets; none occur on the disc")));
        }
        let mesh_count = count(&mut r, 8, "mesh")?;
        exactly_one(mesh_count, "meshes")?;
        let mut meshes = Vec::with_capacity(mesh_count);
        for _ in 0..mesh_count {
            meshes.push(Mesh::parse(&mut r, streams.len(), format_count, index_buffers[0].indices.len())?);
        }

        let end = r.pos();
        let padding = r.bytes(r.remaining())?;
        if padding.len() >= FILE_ALIGNMENT
            || !data.len().is_multiple_of(FILE_ALIGNMENT)
            || padding.iter().any(|&b| b != 0)
        {
            return Err(Error::invalid(format!(
                "unexpected data after the mesh: {} bytes from {end:#x} to the end ({:#x})",
                padding.len(),
                data.len()
            )));
        }

        Ok(GeomFile { version, unk_chunk, header_size, vertex_format, streams, index_buffers, meshes })
    }

    fn check_streams(format: &VertexFormat, streams: &[VertexStream]) -> Result<()> {
        for (i, stream) in streams.iter().enumerate() {
            if stream.stride != format.packed_size() {
                return Err(Error::invalid(format!(
                    "stream {i} stride {} but the VFMT attributes take {} bytes",
                    stream.stride,
                    format.packed_size()
                )));
            }
        }
        for attribute in &format.attributes {
            let stream = streams.get(attribute.stream).ok_or_else(|| {
                Error::invalid(format!("attribute {} names stream {}", attribute.name, attribute.stream))
            })?;
            if attribute.ty == AttributeType::S10x3Norm {
                let o = attribute.offset;
                for vertex in stream.data.chunks_exact(stream.stride) {
                    let word = u32::from_be_bytes([vertex[o], vertex[o + 1], vertex[o + 2], vertex[o + 3]]);
                    if word >> 30 != 0 {
                        return Err(Error::invalid(format!("{} word {word:#010x} has bits 30-31 set", attribute.name)));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn vertex_count(&self) -> usize {
        self.streams.first().map_or(0, |s| s.vertex_count)
    }

    pub fn attribute(&self, semantic: Semantic, set: u8) -> Option<&VertexAttribute> {
        self.vertex_format.attribute(semantic, set)
    }

    pub fn attribute_values(&self, attribute: &VertexAttribute) -> Result<Vec<[f32; 4]>> {
        let stream = self
            .streams
            .get(attribute.stream)
            .ok_or_else(|| Error::invalid(format!("attribute {} names stream {}", attribute.name, attribute.stream)))?;
        if stream.stride == 0 {
            return Ok(Vec::new());
        }
        stream
            .data
            .chunks_exact(stream.stride)
            .map(|vertex| attribute.ty.decode(vertex.get(attribute.offset..).unwrap_or_default()))
            .collect()
    }

    pub fn indices(&self) -> &[u16] {
        self.index_buffers.first().map_or(&[], |b| &b.indices)
    }
}

impl Mesh {
    fn parse(r: &mut Reader, stream_count: usize, format_count: usize, index_count: usize) -> Result<Self> {
        r.expect(b"MESH")?;
        let unk_header = r.u32()?;
        r.expect(b"TLST")?;
        let (unk_a, unk_start) = (r.u32()?, r.u32()?);
        let triangle_count = r.u32()? as usize;
        let (format_index, unk_b) = (r.u32()?, r.u32()?);
        if format_index as usize >= format_count {
            return Err(Error::invalid(format!("TLST names vertex format {format_index}")));
        }
        let n = count(r, 4, "TLST stream")?;
        if n != stream_count {
            return Err(Error::invalid(format!("TLST lists {n} streams, the file has {stream_count}")));
        }
        let unk_c = r.u32()?;
        let stream_ids = (0..n).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
        if let Some(id) = stream_ids.iter().find(|&&id| id as usize >= stream_count) {
            return Err(Error::invalid(format!("TLST names stream {id}")));
        }
        let (unk_d, unk_e) = (r.u32()?, r.u32()?);
        if triangle_count.checked_mul(3) != Some(index_count) {
            return Err(Error::invalid(format!("TLST has {triangle_count} triangles for {index_count} indices")));
        }

        r.expect(b"GEOM")?;
        let version = match r.array::<4>()? {
            [b'G', b'E', b'O', b'3'] => MeshVersion::Geo3,
            [b'G', b'E', b'O', b'4'] => MeshVersion::Geo4,
            other => return Err(Error::invalid(format!("unknown mesh version {other:?}"))),
        };
        let name = counted_str(r)?;
        let (bbox_min, bbox_max) = (r.vec3()?, r.vec3()?);
        let target_count = count(r, 10, "damage target")?;
        let damage_targets = (0..target_count)
            .map(|_| Ok(DamageTarget { name: counted_str(r)?, slot: counted_str(r)? }))
            .collect::<Result<Vec<_>>>()?;
        let translucent = match r.u8()? {
            0 => false,
            1 => true,
            other => return Err(Error::invalid(format!("translucent byte {other}; the disc has only 0 and 1"))),
        };

        let mut groups = Vec::new();
        if version == MeshVersion::Geo4 {
            let n = count(r, 32, "group")?;
            let boxes = (0..n).map(|_| Ok((r.vec3()?, r.vec3()?))).collect::<Result<Vec<_>>>()?;
            let mut next = 0;
            for (bbox_min, bbox_max) in boxes {
                let (first_triangle, triangle_count) = (r.u32()? as usize, r.u32()? as usize);
                if first_triangle != next {
                    return Err(Error::invalid(format!("group starts at triangle {first_triangle}, expected {next}")));
                }
                next = first_triangle + triangle_count;
                groups.push(MeshGroup { bbox_min, bbox_max, first_triangle, triangle_count });
            }
            if next != triangle_count {
                return Err(Error::invalid(format!("groups cover {next} of {triangle_count} triangles")));
            }
        }
        let unk_trailer = [r.u32()?, r.u32()?, r.u32()?];

        Ok(Mesh {
            unk_header,
            triangles: TriangleList {
                unk_a,
                unk_start,
                triangle_count,
                format_index,
                unk_b,
                unk_c,
                stream_ids,
                unk_d,
                unk_e,
            },
            version,
            name,
            bbox_min,
            bbox_max,
            damage_targets,
            translucent,
            groups,
            unk_trailer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s10x3_fields_and_scale() {
        let word = 0x1ff | (0x201 << 10) | (0x200 << 20);
        assert_eq!(unpack_s10x3(word), [511, -511, -512]);
        let v = AttributeType::S10x3Norm.decode(&word.to_be_bytes()).unwrap();
        assert_eq!(v, [1.0, -1.0, -1.0, 0.0]);
        assert_eq!(unpack_s10x3(0x3ff | (1 << 10)), [-1, 1, 0]);
        assert_eq!(snorm10(0), 0.0);
    }

    #[test]
    fn half_floats() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_to_f32(0x7bff), 65504.0);
        assert_eq!(f16_to_f32(0x0001), 2.0f32.powi(-24));
        assert_eq!(f16_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert!(f16_to_f32(0x7e00).is_nan());
        let uv = AttributeType::F16x2.decode(&[0x38, 0x00, 0xbc, 0x00]).unwrap();
        assert_eq!(uv, [0.5, -1.0, 0.0, 0.0]);
    }

    #[test]
    fn other_types() {
        let mut bytes = Vec::new();
        for v in [1.5f32, -2.0, 3.25, 0.0] {
            bytes.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(AttributeType::F32x4.decode(&bytes).unwrap(), [1.5, -2.0, 3.25, 0.0]);
        assert_eq!(AttributeType::F32x2.decode(&bytes).unwrap(), [1.5, -2.0, 0.0, 0.0]);
        assert_eq!(AttributeType::U8x4Norm.decode(&[255, 0, 51, 255]).unwrap(), [1.0, 0.0, 0.2, 1.0]);
        assert!(AttributeType::F32x3.decode(&bytes[..8]).is_err());
        for t in AttributeType::ALL {
            assert_eq!(AttributeType::from_name(t.name()), Some(t));
        }
    }

    #[test]
    fn vertex_format_text() {
        let f = VertexFormat::parse("p0:00:00:0001:4f32 n0:10:00:0001:3s10n t1:14:00:0001:2f16 c0:18:00:0001:4u8n")
            .unwrap();
        assert_eq!(f.attributes.len(), 4);
        assert_eq!(f.packed_size(), 0x1c);
        let t = f.attribute(Semantic::TexCoord, 1).unwrap();
        assert_eq!((t.offset, t.stream, t.unk_field, t.ty), (0x14, 0, 1, AttributeType::F16x2));
        assert_eq!(f.attributes[0].semantic, Semantic::Position);

        assert!(VertexFormat::parse("p0:00:00:0001:4f32 n0:14:00:0001:3s10n").is_err());
        assert!(VertexFormat::parse("p0:00:00:0001:4f32 n0:0C:00:0001:3s10n").is_err());
        assert!(VertexFormat::parse("p0:00:00:0001:3s11n").is_err());
        assert!(VertexFormat::parse("w0:00:00:0001:4u8").is_err());
        assert!(VertexFormat::parse("p0:00:00:4f32").is_err());
        assert!(VertexFormat::parse("p0:00:00:0001:4f32 p0:10:00:0001:4f32").is_err());
        assert!(VertexFormat::parse("").is_err());
    }

    fn synthetic(indices: [u16; 3], trailing: &[u8]) -> Vec<u8> {
        let be = |v: u32| v.to_be_bytes();
        let mut f = b"RSF\0".to_vec();
        for w in RSF_HEADER {
            f.extend(w.to_le_bytes());
        }
        f.extend(b"GEOM");
        f.extend([be(0x0004_0000), be(0), be(16), be(1)].concat());
        let vfmt = b"p0:00:00:0001:3f32";
        f.extend([&b"VFMT"[..], &be(vfmt.len() as u32), vfmt, &[0]].concat());
        f.extend([&be(1)[..], b"STRM", &[0], &be(2), &be(3), &be(12), &be(0)].concat());
        for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]].iter().flatten() {
            f.extend(v.to_be_bytes());
        }
        f.extend([&be(1)[..], b"INDX", &[0], &be(0), &be(3), b"ID16"].concat());
        for i in indices {
            f.extend(i.to_be_bytes());
        }
        f.extend([be(0), be(1)].concat());
        f.extend([&b"MESH"[..], &be(0), b"TLST", &[be(0), be(0), be(1), be(0), be(1), be(1), be(0)].concat()].concat());
        f.extend([be(0), be(0), be(1)].concat());
        f.extend([&b"GEOM"[..], b"GEO3", &be(3), b"tri", &[0]].concat());
        for v in [0.0f32, 0.0, 0.0, 1.0, 1.0, 0.0] {
            f.extend(v.to_be_bytes());
        }
        f.extend([&be(0)[..], &[0], &be(1), &be(0), &be(0)].concat());
        f.extend(trailing);
        f.resize(f.len().next_multiple_of(16), 0);
        let size = (f.len() - 0x10) as u32;
        f[0x18..0x1c].copy_from_slice(&be(size));
        f
    }

    #[test]
    fn synthetic_file() {
        let g = GeomFile::parse(&synthetic([0, 1, 2], &[])).unwrap();
        assert_eq!((g.vertex_count(), g.indices(), g.meshes[0].name.as_str()), (3, &[0, 1, 2][..], "tri"));
        assert_eq!(g.meshes[0].version, MeshVersion::Geo3);
        assert_eq!(g.meshes[0].unk_trailer, [1, 0, 0]);
        let p = g.attribute_values(g.attribute(Semantic::Position, 0).unwrap()).unwrap();
        assert_eq!(p[1], [1.0, 0.0, 0.0, 0.0]);

        assert!(GeomFile::parse(&synthetic([0, 1, 3], &[])).is_err());
        assert!(GeomFile::parse(&synthetic([0, 1, 2], &[7])).is_err());
        let good = synthetic([0, 1, 2], &[]);
        assert!(GeomFile::parse(&good[..good.len() - 16]).is_err());
        let mut bad_size = good.clone();
        bad_size[0x1b] ^= 0x10;
        assert!(GeomFile::parse(&bad_size).is_err());
    }
}
