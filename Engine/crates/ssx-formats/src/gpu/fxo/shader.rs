use super::*;

#[derive(Clone, Debug)]
pub struct Shader {
    pub shader_type: ShaderType,
    pub virtual_size: u32,
    pub physical_size: u32,
    pub unk_debug_hash: u32,
    pub debug_path: String,
    pub target: String,
    pub compiler: String,
    pub unk_ctab_flags: u32,
    pub constants: Vec<ShaderConstant>,
    pub float_literals: Vec<FloatLiteral>,
    pub int_literals: Vec<IntLiteral>,
    pub bool_literals: Vec<IntLiteral>,
    pub unk_def_header: [u32; 4],
    pub descriptor: Vec<u32>,
    pub microcode: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterSet {
    Bool,
    Int4,
    Float4,
    Sampler,
}

#[derive(Clone, Debug)]
pub struct ShaderConstant {
    pub name: String,
    pub register_set: RegisterSet,
    pub register_index: u16,
    pub register_count: u16,
    pub class: u16,
    pub ty: u16,
    pub rows: u16,
    pub columns: u16,
    pub elements: u16,
}

#[derive(Clone, Debug)]
pub struct FloatLiteral {
    pub register: u16,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct IntLiteral {
    pub unk_register: u16,
    pub values: Vec<u32>,
}

pub(super) fn u32_at(data: &[u8], off: usize) -> Result<u32> {
    let mut r = Reader::be(data);
    r.seek(off)?;
    r.u32()
}

pub(super) fn cstr_at(data: &[u8], off: usize) -> Result<String> {
    let mut r = Reader::be(data);
    r.seek(off)?;
    r.cstr()
}

impl Shader {
    pub fn parse(blob: &[u8]) -> Result<Self> {
        let magic = u32_at(blob, 0)?;
        let shader_type = match magic {
            SHADER_MAGIC_PIXEL => ShaderType::Pixel,
            SHADER_MAGIC_VERTEX => ShaderType::Vertex,
            _ => return Err(Error::invalid(format!("shader container magic {magic:#010x}"))),
        };
        let h: Vec<u32> = (0..9).map(|k| u32_at(blob, 4 * k)).collect::<Result<_>>()?;
        let (virtual_size, physical_size) = (h[1], h[2]);
        if virtual_size as usize + physical_size as usize != blob.len() {
            return Err(Error::invalid(format!(
                "shader sizes {virtual_size:#x}+{physical_size:#x} != blob {:#x}",
                blob.len()
            )));
        }
        if h[7] != 0 || h[8] != 0 {
            return Err(Error::invalid("shader container words 7/8 not zero"));
        }
        let unk_debug_hash = u32_at(blob, h[3] as usize)?;
        let path_len = u32_at(blob, h[3] as usize + 4)? as usize;
        let mut r = Reader::be(blob);
        r.seek(h[3] as usize + 8)?;
        let debug_path = String::from_utf8_lossy(r.bytes(path_len)?).into_owned();

        let ct = h[4] as usize + 4;
        let ct_size = u32_at(blob, h[4] as usize)? as usize;
        let ctab = blob.get(ct..ct + ct_size).ok_or_else(|| Error::invalid("CTAB out of range"))?;
        let (target, compiler, unk_ctab_flags, constants) = parse_ctab(ctab)?;

        let phys = &blob[virtual_size as usize..];
        let defs = if h[5] != 0 {
            parse_definitions(blob, h[5] as usize, h[6] as usize, phys)?
        } else {
            Definitions { head: [0; 4], floats: Vec::new(), ints: Vec::new(), bools: Vec::new() }
        };

        let desc_words = virtual_size
            .checked_sub(h[6])
            .ok_or_else(|| Error::invalid("shader descriptor starts beyond the virtual part"))?
            as usize
            / 4;
        let descriptor: Vec<u32> =
            (0..desc_words).map(|k| u32_at(blob, h[6] as usize + 4 * k)).collect::<Result<_>>()?;
        if descriptor.len() < 2
            || descriptor[0] as u64 + descriptor[1] as u64 != physical_size as u64
            || !descriptor[1].is_multiple_of(12)
        {
            return Err(Error::invalid("shader descriptor does not cover the physical part"));
        }
        let mut r = Reader::be(phys);
        r.seek(descriptor[0] as usize)?;
        let microcode = (0..descriptor[1] / 4).map(|_| r.u32()).collect::<Result<_>>()?;
        Ok(Self {
            shader_type,
            virtual_size,
            physical_size,
            unk_debug_hash,
            debug_path,
            target,
            compiler,
            unk_ctab_flags,
            constants,
            float_literals: defs.floats,
            int_literals: defs.ints,
            bool_literals: defs.bools,
            unk_def_header: defs.head,
            descriptor,
            microcode,
        })
    }

    pub fn program(&self) -> Result<Program> {
        Program::parse(&self.microcode, self.shader_type)
    }

    pub fn symbols(&self) -> Symbols {
        let mut s = Symbols::default();
        for c in &self.constants {
            for k in 0..c.register_count as u32 {
                let reg = c.register_index as u32 + k;
                match c.register_set {
                    RegisterSet::Float4 => {
                        let sym =
                            ConstSymbol::Named { name: c.name.clone(), element: k, count: c.register_count as u32 };
                        s.floats.insert(reg, sym);
                    }
                    RegisterSet::Sampler => {
                        s.textures.insert(reg, c.name.clone());
                    }
                    RegisterSet::Bool => {
                        s.bools.insert(reg, c.name.clone());
                    }
                    RegisterSet::Int4 => {}
                }
            }
        }
        for lit in &self.float_literals {
            for (k, v) in lit.values.chunks(4).enumerate() {
                let reg = (lit.register as u32 & 0xFF) + k as u32;
                let mut val = [0.0; 4];
                val[..v.len()].copy_from_slice(v);
                s.floats.insert(reg, ConstSymbol::Literal(val));
            }
        }
        s
    }
}

fn parse_ctab(ctab: &[u8]) -> Result<(String, String, u32, Vec<ShaderConstant>)> {
    let mut r = Reader::be(ctab);
    let header_size = r.u32()?;
    if header_size != 0x1C {
        return Err(Error::invalid(format!("CTAB header size {header_size:#x}")));
    }
    let creator = r.u32()? as usize;
    let _version = r.u32()?;
    let count = r.u32()? as usize;
    let info = r.u32()? as usize;
    let flags = r.u32()?;
    let target = r.u32()? as usize;
    let mut constants = Vec::with_capacity(count);
    for i in 0..count {
        let mut e = Reader::be(ctab);
        e.seek(info + 20 * i)?;
        let name = cstr_at(ctab, e.u32()? as usize)?;
        let register_set = match e.u16()? {
            0 => RegisterSet::Bool,
            1 => RegisterSet::Int4,
            2 => RegisterSet::Float4,
            3 => RegisterSet::Sampler,
            other => return Err(Error::invalid(format!("CTAB register set {other}"))),
        };
        let register_index = e.u16()?;
        let register_count = e.u16()?;
        let _reserved = e.u16()?;
        let mut t = Reader::be(ctab);
        t.seek(e.u32()? as usize)?;
        let (class, ty, rows, columns, elements) = (t.u16()?, t.u16()?, t.u16()?, t.u16()?, t.u16()?);
        constants.push(ShaderConstant {
            name,
            register_set,
            register_index,
            register_count,
            class,
            ty,
            rows,
            columns,
            elements,
        });
    }
    Ok((cstr_at(ctab, target)?, cstr_at(ctab, creator)?, flags, constants))
}

struct Definitions {
    head: [u32; 4],
    floats: Vec<FloatLiteral>,
    ints: Vec<IntLiteral>,
    bools: Vec<IntLiteral>,
}

fn parse_definitions(blob: &[u8], off: usize, end: usize, phys: &[u8]) -> Result<Definitions> {
    let mut r = Reader::be(blob);
    r.seek(off)?;
    let head = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
    let size = r.u32()? as usize;
    let body_end = r.pos() + size;
    if body_end != end {
        return Err(Error::invalid(format!("definition table ends at {body_end:#x}, descriptor at {end:#x}")));
    }
    let mut floats = Vec::new();
    loop {
        let w = r.u32()?;
        if w == 0 {
            break;
        }
        let (register, count) = ((w >> 16) as u16, (w & 0xFFFF) as usize);
        let mut p = Reader::be(phys);
        p.seek(r.u32()? as usize)?;
        floats.push(FloatLiteral { register, values: (0..count).map(|_| p.f32()).collect::<Result<_>>()? });
    }
    let mut lists = [Vec::new(), Vec::new()];
    for list in &mut lists {
        loop {
            let w = r.u32()?;
            if w == 0 {
                break;
            }
            let values = (0..(w & 0xFFFF)).map(|_| r.u32()).collect::<Result<_>>()?;
            list.push(IntLiteral { unk_register: (w >> 16) as u16, values });
        }
    }
    if r.pos() != body_end {
        return Err(Error::invalid("definition table lists do not fill its size"));
    }
    let [ints, bools] = lists;
    Ok(Definitions { head, floats, ints, bools })
}
