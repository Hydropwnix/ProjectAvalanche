mod assembly;
mod mesh;

use crate::reader::Reader;
use crate::{Error, Result};

pub use assembly::*;
pub use mesh::*;

pub const MAGIC: u32 = 0xB8EF_44FE;
pub const VERSION: u32 = 1;
pub const NULL: u32 = 0xFFFF_FFFF;

pub mod type_id {
    pub const SIMPLE_MAPPED_ARRAY: u32 = 1;
    pub const VOLUME: u32 = 2;
    pub const CLUSTERED_MESH: u32 = 3;
    pub const ASSEMBLY_BIT_TABLE: u32 = 4;
    pub const ASSEMBLY_DEFINITION: u32 = 5;
    pub const ASSEMBLY: u32 = 6;
}

#[derive(Clone, Debug)]
pub struct PobFile {
    pub unk_header_word: u32,
    pub dictionary_offset: usize,
    pub unk_dictionary_word: u32,
    pub objects: Vec<PobObject>,
}

#[derive(Clone, Debug)]
pub struct PobObject {
    pub type_id: u32,
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub data: ObjectData,
}

#[derive(Clone, Debug)]
pub enum ObjectData {
    Volume(Volume),
    SimpleMappedArray(SimpleMappedArray),
    ClusteredMesh(Box<ClusteredMesh>),
    AssemblyBitTable(AssemblyBitTable),
    AssemblyDefinition(AssemblyDefinition),
    Assembly(Assembly),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VolumeKind {
    Sphere = 1,
    Capsule = 2,
    Triangle = 3,
    Box = 4,
    Cylinder = 5,
    Aggregate = 6,
}

#[derive(Clone, Debug)]
pub struct Volume {
    pub transform: Matrix,
    pub kind: VolumeKind,
    pub params: [u32; 3],
    pub radius: f32,
    pub group_id: u32,
    pub surface_id: u32,
    pub flags: u32,
    pub aggregate: Option<usize>,
}

impl Volume {
    pub fn cylinder(&self) -> Option<(f32, f32)> {
        (self.kind == VolumeKind::Cylinder).then(|| (f32::from_bits(self.params[0]), f32::from_bits(self.params[1])))
    }
}

pub type Matrix = [[f32; 3]; 4];

#[derive(Clone, Debug)]
pub struct AggregateHeader {
    pub aabb_min: [f32; 3],
    pub aabb_max: [f32; 3],
    pub num_tag_bits: u32,
    pub num_volumes: u32,
    pub unk_flags: u32,
}

#[derive(Clone, Debug)]
pub struct SimpleMappedArray {
    pub header: AggregateHeader,
    pub volumes: Vec<Volume>,
}

type DictionaryEntry = (u32, usize, String);

fn bad(msg: impl Into<String>) -> Error {
    Error::invalid(format!(".pob: {}", msg.into()))
}

fn at(data: &[u8], pos: usize) -> Result<Reader<'_>> {
    let mut r = Reader::le(data);
    r.seek(pos)?;
    Ok(r)
}

fn local(v: u32, len: usize, what: &str) -> Result<Option<usize>> {
    match v {
        NULL => Ok(None),
        v if (v as usize) <= len => Ok(Some(v as usize)),
        v => Err(bad(format!("{what} pointer {v:#x} outside its object ({len:#x} bytes)"))),
    }
}

fn global(v: u32, what: &str) -> Result<usize> {
    if v == NULL { Err(bad(format!("{what} pointer is null"))) } else { Ok(v as usize) }
}

impl PobFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::le(data);
        let magic = r.u32()?;
        if magic != MAGIC {
            return Err(bad(format!("magic {magic:#010x}, expected {MAGIC:#010x}")));
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(bad(format!("version {version}, expected {VERSION}")));
        }
        let size = r.u32()? as usize;
        if size != data.len() {
            return Err(bad(format!("header size {size} != file size {}", data.len())));
        }
        let unk_header_word = r.u32()?;
        let count = r.u32()? as usize;
        if count < 2 || count > data.len() / 4 {
            return Err(bad(format!("object count {count}")));
        }
        let offsets = (0..count).map(|_| r.u32().map(|v| v as usize)).collect::<Result<Vec<_>>>()?;
        let table_end = r.pos();
        if offsets[0] < table_end || offsets.windows(2).any(|w| w[0] >= w[1]) || offsets[count - 1] >= data.len() {
            return Err(bad(format!("object offsets not ascending inside the file: {offsets:x?}")));
        }
        let ends: Vec<usize> = offsets[1..].iter().copied().chain([data.len()]).collect();
        let dict = &data[offsets[0]..ends[0]];
        let (unk_dictionary_word, entries) = parse_dictionary(dict, count - 1)?;

        let mut objects = Vec::with_capacity(count - 1);
        for (k, (type_id, offset, name)) in entries.into_iter().enumerate() {
            if offset != offsets[k + 1] {
                return Err(bad(format!(
                    "dictionary entry {k} at {offset:#x}, object table says {:#x}",
                    offsets[k + 1]
                )));
            }
            let obj = &data[offset..ends[k + 1]];
            let data = match type_id {
                type_id::VOLUME => ObjectData::Volume(parse_volume(&mut Reader::le(obj))?),
                type_id::SIMPLE_MAPPED_ARRAY => ObjectData::SimpleMappedArray(parse_simple_mapped_array(obj)?),
                type_id::CLUSTERED_MESH => ObjectData::ClusteredMesh(Box::new(parse_clustered_mesh(obj)?)),
                type_id::ASSEMBLY_BIT_TABLE => ObjectData::AssemblyBitTable(parse_bit_table(obj)?),
                type_id::ASSEMBLY_DEFINITION => ObjectData::AssemblyDefinition(parse_definition(obj)?),
                type_id::ASSEMBLY => ObjectData::Assembly(parse_assembly(obj)?),
                other => return Err(bad(format!("unknown object type id {other} ({name})"))),
            };
            objects.push(PobObject { type_id, name, offset, size: obj.len(), data });
        }
        let file = Self { unk_header_word, dictionary_offset: offsets[0], unk_dictionary_word, objects };
        file.check_references()?;
        Ok(file)
    }

    pub fn object_at(&self, offset: usize) -> Option<&PobObject> {
        self.objects.iter().find(|o| o.offset == offset)
    }

    fn check_references(&self) -> Result<()> {
        let want = |offset: usize, types: &[u32], what: &str| -> Result<()> {
            match self.object_at(offset) {
                Some(o) if types.contains(&o.type_id) => Ok(()),
                Some(o) => Err(bad(format!("{what} at {offset:#x} is a type-{} object", o.type_id))),
                None => Err(bad(format!("{what} pointer {offset:#x} names no object"))),
            }
        };
        let aggregates = [type_id::SIMPLE_MAPPED_ARRAY, type_id::CLUSTERED_MESH];
        let check_volume = |v: &Volume| v.aggregate.map_or(Ok(()), |a| want(a, &aggregates, "aggregate volume target"));
        for o in &self.objects {
            match &o.data {
                ObjectData::Volume(v) => check_volume(v)?,
                ObjectData::SimpleMappedArray(s) => s.volumes.iter().try_for_each(check_volume)?,
                ObjectData::AssemblyDefinition(d) => {
                    want(d.bit_table, &[type_id::ASSEMBLY_BIT_TABLE], "definition bit table")?;
                    d.parts.iter().try_for_each(|p| want(p.volume, &[type_id::VOLUME], "part volume"))?;
                }
                ObjectData::Assembly(a) => {
                    want(a.bit_table, &[type_id::ASSEMBLY_BIT_TABLE], "assembly bit table")?;
                    want(a.definition, &[type_id::ASSEMBLY_DEFINITION], "assembly definition")?;
                    a.parts.iter().try_for_each(|p| want(p.volume, &[type_id::VOLUME], "part instance volume"))?;
                }
                ObjectData::ClusteredMesh(_) | ObjectData::AssemblyBitTable(_) => {}
            }
        }
        Ok(())
    }
}

fn parse_dictionary(dict: &[u8], expected: usize) -> Result<(u32, Vec<DictionaryEntry>)> {
    let mut r = Reader::le(dict);
    let unk = r.u32()?;
    let num_types = r.u32()? as usize;
    let num_objects = r.u32()? as usize;
    let string_bytes = r.u32()? as usize;
    let len = dict.len();
    let types_at = local(r.u32()?, len, "dictionary types")?.ok_or_else(|| bad("null dictionary types"))?;
    let objects_at = local(r.u32()?, len, "dictionary objects")?.ok_or_else(|| bad("null dictionary objects"))?;
    let strings_at = local(r.u32()?, len, "dictionary strings")?.ok_or_else(|| bad("null dictionary strings"))?;
    if num_objects != expected {
        return Err(bad(format!("dictionary lists {num_objects} objects, table has {expected}")));
    }
    let strings = dict.get(strings_at..strings_at + string_bytes).ok_or_else(|| bad("dictionary strings past end"))?;
    let mut entries = Vec::with_capacity(num_objects);
    let mut t = at(dict, types_at)?;
    for _ in 0..num_types {
        let type_id = t.u32()?;
        let n = t.u32()? as usize;
        let list = local(t.u32()?, len, "dictionary type list")?.ok_or_else(|| bad("null type list"))?;
        if list != objects_at + 8 * entries.len() {
            return Err(bad(format!("type {type_id} list at {list:#x} is not contiguous")));
        }
        let mut o = at(dict, list)?;
        for _ in 0..n {
            let offset = o.u32()? as usize;
            let name_at = (o.u32()? as usize).checked_sub(strings_at).ok_or_else(|| bad("name before strings"))?;
            let tail = strings.get(name_at..).ok_or_else(|| bad("name past strings"))?;
            let end = tail.iter().position(|&b| b == 0).ok_or_else(|| bad("unterminated name"))?;
            entries.push((type_id, offset, String::from_utf8_lossy(&tail[..end]).into_owned()));
        }
    }
    if entries.len() != num_objects {
        return Err(bad(format!("dictionary types cover {} objects, header says {num_objects}", entries.len())));
    }
    Ok((unk, entries))
}

fn parse_matrix(r: &mut Reader) -> Result<Matrix> {
    let mut m = [[0.0; 3]; 4];
    for row in &mut m {
        *row = r.vec3()?;
        r.skip(4)?;
    }
    Ok(m)
}

fn parse_volume(r: &mut Reader) -> Result<Volume> {
    let transform = parse_matrix(r)?;
    let kind = match r.u32()? {
        1 => VolumeKind::Sphere,
        2 => VolumeKind::Capsule,
        3 => VolumeKind::Triangle,
        4 => VolumeKind::Box,
        5 => VolumeKind::Cylinder,
        6 => VolumeKind::Aggregate,
        other => return Err(bad(format!("volume kind {other} (the game knows 1..=6)"))),
    };
    let params = [r.u32()?, r.u32()?, r.u32()?];
    let radius = r.f32()?;
    let group_id = r.u32()?;
    let surface_id = r.u32()?;
    let flags = r.u32()?;
    let aggregate = match kind {
        VolumeKind::Aggregate => Some(global(params[0], "aggregate volume")?),
        _ => None,
    };
    Ok(Volume { transform, kind, params, radius, group_id, surface_id, flags, aggregate })
}

fn parse_aggregate_header(r: &mut Reader) -> Result<AggregateHeader> {
    let aabb_min = r.vec3()?;
    r.skip(4)?;
    let aabb_max = r.vec3()?;
    r.skip(8)?;
    let num_tag_bits = r.u32()?;
    let num_volumes = r.u32()?;
    let unk_flags = r.u32()?;
    Ok(AggregateHeader { aabb_min, aabb_max, num_tag_bits, num_volumes, unk_flags })
}

fn parse_simple_mapped_array(obj: &[u8]) -> Result<SimpleMappedArray> {
    let mut r = Reader::le(obj);
    let header = parse_aggregate_header(&mut r)?;
    let at_volumes = local(r.u32()?, obj.len(), "volume array")?.ok_or_else(|| bad("null volume array"))?;
    let mut v = at(obj, at_volumes)?;
    let volumes = (0..header.num_volumes).map(|_| parse_volume(&mut v)).collect::<Result<Vec<_>>>()?;
    Ok(SimpleMappedArray { header, volumes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_cosine_matches_formula() {
        let pi2 = std::f32::consts::PI * std::f32::consts::PI;
        assert!((edge_cosine(0) - (1.0 - pi2 / 8.0)).abs() < 1e-6);
        assert!((edge_cosine(2) - (1.0 - pi2 / 32.0)).abs() < 1e-6);
        assert_eq!(edge_cosine(0xA0), edge_cosine(0));
        assert_eq!(edge_cosine(0x3A), edge_cosine(26));
    }

    #[test]
    fn unit_quad_with_ids_and_split() {
        let bytes = [0xE2, 0, 1, 2, 3, 10, 11, 12, 13, 7, 0x34, 0x12];
        let mut r = Reader::le(&bytes);
        let u = parse_unit(&mut r, 4, 1, 2).unwrap();
        assert_eq!(r.remaining(), 0);
        assert_eq!(u.group_id, Some(7));
        assert_eq!(u.surface_id, Some(0x1234));
        assert_eq!(u.triangles().unwrap(), vec![[0, 1, 2], [3, 2, 1]]);
        assert_eq!(u.triangle_edge_codes().unwrap(), vec![[Some(10), None, Some(12)], [Some(13), None, Some(11)]]);
    }

    #[test]
    fn unit_rejects_bad_index_and_kind() {
        let mut r = Reader::le(&[0x01, 0, 1, 5]);
        assert!(parse_unit(&mut r, 5, 0, 0).is_err());
        let mut r = Reader::le(&[0x04, 0, 1, 2]);
        assert!(parse_unit(&mut r, 8, 0, 0).is_err());
    }

    #[test]
    fn header_checks() {
        let mut f = vec![0u8; 32];
        f[..4].copy_from_slice(&MAGIC.to_le_bytes());
        f[4..8].copy_from_slice(&1u32.to_le_bytes());
        f[8..12].copy_from_slice(&31u32.to_le_bytes());
        assert!(PobFile::parse(&f).is_err());
        f[..4].copy_from_slice(&MAGIC.to_be_bytes());
        assert!(PobFile::parse(&f).is_err());
    }
}
