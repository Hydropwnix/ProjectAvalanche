use super::*;

#[derive(Clone, Debug)]
pub struct AssemblyDefinition {
    pub parts: Vec<PartDefinition>,
    pub bit_table: usize,
    pub unk_part_words: Vec<u32>,
    pub unk_30: u32,
}

#[derive(Clone, Debug)]
pub struct PartDefinition {
    pub volume: usize,
    pub diagonal_terms: [f32; 3],
    pub mass_params: [f32; 5],
    pub transform: Matrix,
    pub material: [f32; 3],
    pub unk_words: [u32; 3],
}

#[derive(Clone, Debug)]
pub struct AssemblyBitTable {
    pub unk_0: u32,
    pub unk_1: u32,
    pub words: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct Assembly {
    pub bit_table: usize,
    pub definition: usize,
    pub parts: Vec<PartInstance>,
    pub unk_14: u32,
    pub unk_18: u32,
    pub unk_count_20: u32,
    pub unk_count_24: u32,
}

#[derive(Clone, Debug)]
pub struct PartInstance {
    pub transform: Matrix,
    pub volume: usize,
    pub part_transform: Option<usize>,
    pub part_mass: usize,
    pub part_vec3: usize,
    pub unk_words: [u32; 4],
}

pub(super) fn parse_bit_table(obj: &[u8]) -> Result<AssemblyBitTable> {
    let mut r = Reader::le(obj);
    let unk_0 = r.u32()?;
    let unk_1 = r.u32()?;
    let n = r.u32()? as usize;
    if n > r.remaining() / 4 {
        return Err(bad(format!("bit table of {n} words")));
    }
    let words = (0..n).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
    Ok(AssemblyBitTable { unk_0, unk_1, words })
}

pub(super) fn parse_definition(obj: &[u8]) -> Result<AssemblyDefinition> {
    let len = obj.len();
    let mut r = Reader::le(obj);
    let w: Vec<u32> = (0..13).map(|_| r.u32()).collect::<Result<_>>()?;
    let num_parts = w[1] as usize;
    if w[2] != 0 || w[6] != 0 {
        return Err(bad(format!(
            "definition with {} / {} joint-like entries (layout unread; none on the disc)",
            w[2], w[6]
        )));
    }
    let parts_at = local(w[0], len, "definition parts")?.ok_or_else(|| bad("null definition parts"))?;
    let words_at = local(w[11], len, "definition part words")?.ok_or_else(|| bad("null part words"))?;
    if num_parts > len / 0x90 {
        return Err(bad(format!("{num_parts} parts")));
    }
    let mut parts = Vec::with_capacity(num_parts);
    for k in 0..num_parts {
        let mut p = at(obj, parts_at + 0x90 * k)?;
        let volume = global(p.u32()?, "part volume")?;
        let diagonal_terms = p.vec3()?;
        let mass_params = [p.f32()?, p.f32()?, p.f32()?, p.f32()?, p.f32()?];
        p.skip(0x0C)?;
        let transform = parse_matrix(&mut p)?;
        let material = p.vec3()?;
        let unk_words = [p.u32()?, p.u32()?, p.u32()?];
        parts.push(PartDefinition { volume, diagonal_terms, mass_params, transform, material, unk_words });
    }
    let mut pw = at(obj, words_at)?;
    let unk_part_words = (0..num_parts).map(|_| pw.u32()).collect::<Result<Vec<_>>>()?;
    Ok(AssemblyDefinition { parts, bit_table: global(w[10], "definition bit table")?, unk_part_words, unk_30: w[12] })
}

pub(super) fn parse_assembly(obj: &[u8]) -> Result<Assembly> {
    let mut r = Reader::le(obj);
    let w: Vec<u32> = (0..10).map(|_| r.u32()).collect::<Result<_>>()?;
    if w[8] != 0 || w[9] != 0 {
        return Err(bad(format!("assembly with {} / {} extra entries (layout unread; none on the disc)", w[8], w[9])));
    }
    let parts_at = local(w[2], obj.len(), "assembly parts")?.ok_or_else(|| bad("null assembly parts"))?;
    let num_parts = w[7] as usize;
    if num_parts > obj.len() / 0x60 {
        return Err(bad(format!("{num_parts} assembly parts")));
    }
    let mut parts = Vec::with_capacity(num_parts);
    let mut p = at(obj, parts_at)?;
    for _ in 0..num_parts {
        let transform = parse_matrix(&mut p)?;
        let volume = global(p.u32()?, "part instance volume")?;
        let part_transform = match p.u32()? {
            NULL => None,
            v => Some(v as usize),
        };
        let part_mass = global(p.u32()?, "part instance mass")?;
        let unk_4c = p.u32()?;
        let part_vec3 = global(p.u32()?, "part instance vec3")?;
        let unk_words = [unk_4c, p.u32()?, p.u32()?, p.u32()?];
        parts.push(PartInstance { transform, volume, part_transform, part_mass, part_vec3, unk_words });
    }
    Ok(Assembly {
        bit_table: global(w[0], "assembly bit table")?,
        definition: global(w[1], "assembly definition")?,
        parts,
        unk_14: w[5],
        unk_18: w[6],
        unk_count_20: w[8],
        unk_count_24: w[9],
    })
}
