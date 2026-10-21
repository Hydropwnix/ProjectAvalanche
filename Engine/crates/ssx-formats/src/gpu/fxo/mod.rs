mod shader;
pub mod state;

use crate::reader::Reader;
use crate::xenos_ucode::{ConstSymbol, Program, ShaderType, Symbols};
use crate::{Error, Result};

pub use shader::*;

pub const MAGIC: u32 = 0xA3D7_0141;
pub const IMAGE_BASE: usize = 0xC;
pub const SHADER_MAGIC_PIXEL: u32 = 0x102A_1100;
pub const SHADER_MAGIC_VERTEX: u32 = 0x102A_1101;

mod hdr {
    pub const PARAM_DESCS: usize = 0x108;
    pub const PARAM_COUNT: usize = 0x118;
    pub const TECHNIQUES: usize = 0x200;
    pub const TECHNIQUE_COUNT: usize = 0x20C;
    pub const PASSES: usize = 0x210;
    pub const PASS_COUNT: usize = 0x220;
    pub const VS_REGION: usize = 0x230;
    pub const PS_REGION: usize = 0x23C;
    pub const RS_REGION: usize = 0x248;
    pub const SS_REGION: usize = 0x254;
    pub const STRINGS: usize = 0x288;
    pub const STRINGS_SIZE: usize = 0x28C;
    pub const PARAM_NAMES: usize = 0x298;
    pub const CREATOR: usize = 0x2A4;
}

const PASS_STATE_VS: usize = 0x48;
const PASS_STATE_PS: usize = 0x4C;

#[derive(Clone, Debug)]
pub struct Effect {
    pub unk_header_08: u32,
    pub creator: String,
    pub parameters: Vec<Parameter>,
    pub techniques: Vec<Technique>,
    pub vertex_shaders: Vec<Option<Shader>>,
    pub pixel_shaders: Vec<Option<Shader>>,
}

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: Option<String>,
    pub desc: u32,
    pub size: u16,
    pub value_offset: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterKind {
    Sampler { dimension: u32 },
    Array { elements: u32 },
    Numeric { base: u32, class: u32, rows: u32, columns: u32 },
}

impl Parameter {
    pub fn kind(&self) -> ParameterKind {
        let d = self.desc;
        if d & 0x40_0000 != 0 {
            ParameterKind::Sampler { dimension: (d >> 4) & 0xF }
        } else if d & 0x2 != 0 {
            ParameterKind::Array { elements: (d >> 2) & 0x7FFFF }
        } else {
            ParameterKind::Numeric {
                base: (d >> 2) & 3,
                class: (d >> 4) & 3,
                rows: ((d >> 7) & 3) + 1,
                columns: ((d >> 9) & 3) + 1,
            }
        }
    }

    pub fn unk_flag_21(&self) -> bool {
        self.desc & 0x20_0000 != 0
    }
}

#[derive(Clone, Debug)]
pub struct Technique {
    pub name: String,
    pub unk_08: u32,
    pub sampler_mask_words: u32,
    pub passes: Vec<Pass>,
}

#[derive(Clone, Debug)]
pub struct Pass {
    pub name: String,
    pub unk_04: u32,
    pub vertex_shader: Option<usize>,
    pub pixel_shader: Option<usize>,
    pub state_offset: u32,
    pub render_state_offset: u32,
    pub sampler_state_offset: u32,
    pub render_states: StateBlock,
    pub sampler_states: StateBlock,
}

impl Pass {
    pub fn render_state(&self, state: u16) -> Option<u32> {
        if self.render_states.is_parameter_bound(0, state) {
            return None;
        }
        self.render_states.literal_value(0, state).or_else(|| state::render_state_default(state))
    }

    pub fn sampler_state(&self, sampler: u16, state: u16) -> Option<u32> {
        if self.sampler_states.is_parameter_bound(sampler, state) {
            return None;
        }
        self.sampler_states.literal_value(sampler, state).or_else(|| state::sampler_state_default(state))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateBlock {
    pub mask: Vec<u32>,
    pub parameter_int: Vec<StateEntry>,
    pub parameter_float: Vec<StateEntry>,
    pub literal: Vec<StateEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateEntry {
    pub sampler: u16,
    pub state: u16,
    pub value: u32,
}

impl StateBlock {
    pub fn literal_value(&self, sampler: u16, state: u16) -> Option<u32> {
        self.literal.iter().find(|e| e.sampler == sampler && e.state == state).map(|e| e.value)
    }

    fn is_parameter_bound(&self, sampler: u16, state: u16) -> bool {
        self.parameter_int.iter().chain(&self.parameter_float).any(|e| e.sampler == sampler && e.state == state)
    }

    pub fn entries(&self) -> impl Iterator<Item = &StateEntry> {
        self.parameter_int.iter().chain(&self.parameter_float).chain(&self.literal)
    }

    pub fn expected_mask(&self, sampler_block: bool) -> Result<Vec<u32>> {
        let mut mask = vec![0u32; if sampler_block { 32 } else { 4 }];
        for e in self.entries() {
            let bit = if sampler_block {
                if e.state >= 0x50 || !e.state.is_multiple_of(4) || e.sampler >= 32 {
                    return Err(Error::invalid(format!("sampler state {:#x} slot {}", e.state, e.sampler)));
                }
                e.sampler as usize * 32 + (e.state / 4) as usize
            } else {
                state::RENDER_STATES
                    .iter()
                    .position(|(s, _, _)| *s == e.state)
                    .ok_or_else(|| Error::invalid(format!("render state {:#x} not in the runtime table", e.state)))?
            };
            mask[bit / 32] |= 0x8000_0000 >> (bit % 32);
        }
        Ok(mask)
    }
}

struct Image<'a>(&'a [u8]);

impl Image<'_> {
    fn u32(&self, off: usize) -> Result<u32> {
        u32_at(self.0, off)
    }
    fn u16(&self, off: usize) -> Result<u16> {
        let mut r = Reader::be(self.0);
        r.seek(off)?;
        r.u16()
    }
    fn ptr(&self, off: usize) -> Result<Option<usize>> {
        let v = self.u32(off)?;
        Ok((v != u32::MAX).then_some(v as usize))
    }
    fn string(&self, off: usize) -> Result<String> {
        cstr_at(self.0, off)
    }
}

fn parse_shader_region(img: &Image, region: usize) -> Result<Vec<(usize, Option<Shader>)>> {
    let start = img.u32(region)? as usize;
    let count = img.u32(region + 4)? as usize;
    let size = img.u32(region + 8)? as usize;
    let mut out = Vec::with_capacity(count);
    let mut off = start;
    for i in 0..count {
        let slot = img.u32(off)?;
        let len = img.u32(off + 4)? as usize;
        if slot != 0 {
            return Err(Error::invalid(format!("shader entry {i}: runtime slot {slot:#x} not zero")));
        }
        let shader = if len == 0 {
            None
        } else {
            let blob = img.0.get(off + 8..off + 8 + len).ok_or_else(|| Error::invalid("shader blob out of range"))?;
            Some(Shader::parse(blob).map_err(|e| Error::invalid(format!("shader entry {i}: {e}")))?)
        };
        if i == 0 && shader.is_some() {
            return Err(Error::invalid("shader entry 0 is not empty"));
        }
        out.push((off, shader));
        off += 8 + len;
    }
    if off != start + size {
        return Err(Error::invalid(format!("shader region ends at {off:#x}, header says {:#x}", start + size)));
    }
    Ok(out)
}

fn parse_state_region(img: &Image, region: usize, sampler_block: bool) -> Result<Vec<(usize, StateBlock)>> {
    let what = if sampler_block { "sampler-state" } else { "render-state" };
    let start = img.u32(region)? as usize;
    let count = img.u32(region + 4)? as usize;
    let size = img.u32(region + 8)? as usize;
    let mask_words = if sampler_block { 32 } else { 4 };
    let mut out = Vec::with_capacity(count);
    let mut off = start;
    for i in 0..count {
        let mask: Vec<u32> = (0..mask_words).map(|k| img.u32(off + 4 * k)).collect::<Result<_>>()?;
        let counts_at = off + 4 * mask_words;
        let counts = [img.u32(counts_at)?, img.u32(counts_at + 4)?, img.u32(counts_at + 8)?];
        let mut at = counts_at + 12;
        let mut lists: [Vec<StateEntry>; 3] = Default::default();
        for (list, &n) in lists.iter_mut().zip(&counts) {
            for _ in 0..n {
                let (key, value) = (img.u32(at)?, img.u32(at + 4)?);
                let (sampler, state) = if sampler_block { ((key >> 16) as u16, key as u16) } else { (0, key as u16) };
                if !sampler_block && key > 0xFFFF {
                    return Err(Error::invalid(format!("{what} block {i}: state word {key:#x}")));
                }
                list.push(StateEntry { sampler, state, value });
                at += 8;
            }
        }
        let [parameter_int, parameter_float, literal] = lists;
        let block = StateBlock { mask, parameter_int, parameter_float, literal };
        if block.expected_mask(sampler_block)? != block.mask {
            return Err(Error::invalid(format!("{what} block at {off:#x}: mask does not match its entries")));
        }
        if i == 0 && (block.entries().next().is_some() || block.mask.iter().any(|&w| w != 0)) {
            return Err(Error::invalid(format!("{what} entry 0 is not empty")));
        }
        out.push((off, block));
        off = at.next_multiple_of(16);
    }
    if off != start + size {
        return Err(Error::invalid(format!("{what} region ends at {off:#x}, header says {:#x}", start + size)));
    }
    Ok(out)
}

impl Effect {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let magic = r.u32()?;
        if magic != MAGIC {
            return Err(Error::invalid(format!("effect magic {magic:#010x}")));
        }
        let size = r.u32()? as usize;
        if size + IMAGE_BASE != data.len() {
            return Err(Error::invalid(format!("effect size {size:#x} + 12 != file size {:#x}", data.len())));
        }
        let unk_header_08 = r.u32()?;
        let img = Image(&data[IMAGE_BASE..]);

        let strings = img.u32(hdr::STRINGS)? as usize;
        let strings_end = strings + img.u32(hdr::STRINGS_SIZE)? as usize;
        let in_strings = |p: usize| -> Result<usize> {
            if (strings..strings_end).contains(&p) {
                Ok(p)
            } else {
                Err(Error::invalid(format!("name pointer {p:#x} outside the string pool")))
            }
        };

        let param_count = img.u32(hdr::PARAM_COUNT)? as usize;
        let names = img.u32(hdr::PARAM_NAMES)? as usize;
        let descs = img.u32(hdr::PARAM_DESCS)? as usize;
        let mut parameters = Vec::with_capacity(param_count);
        for i in 0..param_count {
            let name = match img.ptr(names + 4 * i)? {
                Some(p) => Some(img.string(in_strings(p)?)?),
                None => None,
            };
            let d = descs + 8 * i;
            parameters.push(Parameter {
                name,
                desc: img.u32(d)?,
                size: img.u16(d + 4)?,
                value_offset: img.u16(d + 6)?,
            });
        }

        let vs = parse_shader_region(&img, hdr::VS_REGION)?;
        let ps = parse_shader_region(&img, hdr::PS_REGION)?;
        let find = |list: &[(usize, Option<Shader>)], off: usize, what: &str| -> Result<Option<usize>> {
            match list.iter().position(|(o, _)| *o == off) {
                Some(0) => Ok(None),
                Some(i) => Ok(Some(i)),
                None => Err(Error::invalid(format!("pass {what} pointer {off:#x} is not a shader entry"))),
            }
        };

        let render_blocks = parse_state_region(&img, hdr::RS_REGION, false)?;
        let sampler_blocks = parse_state_region(&img, hdr::SS_REGION, true)?;
        let block = |list: &[(usize, StateBlock)], off: u32, what: &str| -> Result<StateBlock> {
            list.iter()
                .find(|(o, _)| *o == off as usize)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| Error::invalid(format!("pass {what} pointer {off:#x} is not a block of its region")))
        };

        let tech_table = img.u32(hdr::TECHNIQUES)? as usize;
        let tech_count = img.u32(hdr::TECHNIQUE_COUNT)? as usize;
        let pass_table = img.u32(hdr::PASSES)? as usize;
        let pass_count = img.u32(hdr::PASS_COUNT)? as usize;
        let mut techniques = Vec::with_capacity(tech_count);
        let mut seen_passes = 0;
        for t in 0..tech_count {
            let to = img.u32(tech_table + 4 * t)? as usize;
            let name = img.string(in_strings(img.u32(to)? as usize)?)?;
            let n = img.u32(to + 4)? as usize;
            let mut passes = Vec::with_capacity(n);
            for p in 0..n {
                let po = img.u32(to + 0x10 + 4 * p)? as usize;
                if po < pass_table || po >= pass_table + 0x14 * pass_count || !(po - pass_table).is_multiple_of(0x14) {
                    return Err(Error::invalid(format!(
                        "technique {name}: pass pointer {po:#x} outside the pass table"
                    )));
                }
                let state = img.u32(po + 8)? as usize;
                let (rs, ss) = (img.u32(po + 12)?, img.u32(po + 16)?);
                passes.push(Pass {
                    name: img.string(in_strings(img.u32(po)? as usize)?)?,
                    unk_04: img.u32(po + 4)?,
                    vertex_shader: find(&vs, img.u32(state + PASS_STATE_VS)? as usize, "vertex shader")?,
                    pixel_shader: find(&ps, img.u32(state + PASS_STATE_PS)? as usize, "pixel shader")?,
                    state_offset: state as u32,
                    render_state_offset: rs,
                    sampler_state_offset: ss,
                    render_states: block(&render_blocks, rs, "render-state")?,
                    sampler_states: block(&sampler_blocks, ss, "sampler-state")?,
                });
                seen_passes += 1;
            }
            let sampler_mask_words = img.u32(to + 12)?;
            let used = passes
                .iter()
                .filter_map(|p| p.sampler_states.mask.chunks(2).rposition(|pair| pair.iter().any(|&w| w != 0)))
                .map(|pair| 2 * pair as u32 + 2)
                .max()
                .unwrap_or(0);
            if used != sampler_mask_words {
                return Err(Error::invalid(format!(
                    "technique {name}: sampler-mask word count {sampler_mask_words}, masks use {used}"
                )));
            }
            techniques.push(Technique { name, unk_08: img.u32(to + 8)?, sampler_mask_words, passes });
        }
        if seen_passes != pass_count {
            return Err(Error::invalid(format!("techniques reference {seen_passes} passes, table has {pass_count}")));
        }

        Ok(Self {
            unk_header_08,
            creator: img.string(img.u32(hdr::CREATOR)? as usize)?,
            parameters,
            techniques,
            vertex_shaders: vs.into_iter().map(|(_, s)| s).collect(),
            pixel_shaders: ps.into_iter().map(|(_, s)| s).collect(),
        })
    }

    pub fn technique(&self, name: &str) -> Option<&Technique> {
        self.techniques.iter().find(|t| t.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn padded(mut v: Vec<u8>) -> Vec<u8> {
        v.resize(v.len().next_multiple_of(4), 0);
        v
    }

    fn sample_shader() -> Vec<u8> {
        let mut ctab = be(&[0x1C, 0, 0xFFFF_0300, 1, 0x1C, 0, 0]);
        ctab.extend(be(&[0x40, 0x0002_0002, 0x0001_0000, 0x30, 0]));
        ctab.extend(be(&[0x0001_0003, 0x0001_0004, 0x0001_0000, 0]));
        ctab.extend(padded(b"tint\0".to_vec()));
        ctab.extend(padded(b"ps_3_0\0".to_vec()));
        let target = 0x48u32;
        ctab[4..8].copy_from_slice(&target.to_be_bytes());
        ctab[24..28].copy_from_slice(&target.to_be_bytes());

        let debug = [be(&[0x1234_5678, 4]), b"a.pd".to_vec()].concat();
        let ctab_off = 0x24 + debug.len();
        let def_off = ctab_off + 4 + ctab.len();
        let defs = be(&[0, 1, 0, 0, 20, 0x01FF_0004, 0, 0, 0, 0]);
        let desc_off = def_off + defs.len();
        let desc = be(&[0x10, 24, 0, 0]);
        let virtual_size = desc_off + desc.len();
        let cf = [1 | (1 << 12), 2 << 12, 0];
        let alu = [(1 << 15) | (0b1111 << 16) | (50 << 26), 0, (2 << 24) | (1 << 30) | (1 << 31)];
        let phys = [be(&[0x3F00_0000, 0x3F80_0000, 0x4000_0000, 0x4080_0000]), be(&cf), be(&alu)].concat();
        let header = be(&[
            SHADER_MAGIC_PIXEL,
            virtual_size as u32,
            phys.len() as u32,
            0x24,
            ctab_off as u32,
            def_off as u32,
            desc_off as u32,
            0,
            0,
        ]);
        let ctab_block = [be(&[ctab.len() as u32]), ctab].concat();
        [header, debug, ctab_block, defs, desc, phys].concat()
    }

    #[test]
    fn parses_shader_container() {
        let s = Shader::parse(&sample_shader()).unwrap();
        assert_eq!(s.shader_type, ShaderType::Pixel);
        assert_eq!(s.debug_path, "a.pd");
        assert_eq!(s.target, "ps_3_0");
        assert_eq!(s.constants.len(), 1);
        let c = &s.constants[0];
        assert_eq!((c.name.as_str(), c.register_set, c.register_index, c.columns), ("tint", RegisterSet::Float4, 2, 4));
        assert_eq!(s.float_literals.len(), 1);
        assert_eq!(
            (s.float_literals[0].register, s.float_literals[0].values.clone()),
            (0x1FF, vec![0.5, 1.0, 2.0, 4.0])
        );
        assert_eq!(s.microcode.len(), 6);
        let syms = s.symbols();
        assert_eq!(syms.floats.get(&255), Some(&ConstSymbol::Literal([0.5, 1.0, 2.0, 4.0])));
        let listing = s.program().unwrap().disassemble(&syms);
        assert!(listing.contains("oC0.xyzw = max(r0.xyzw, r0.xyzw)"), "{listing}");
    }

    #[test]
    fn rejects_size_mismatch() {
        let mut blob = sample_shader();
        blob.push(0);
        assert!(Shader::parse(&blob).is_err());
    }

    #[test]
    fn parameter_kinds() {
        let p = |desc| Parameter { name: None, desc, size: 0, value_offset: 0 };
        assert_eq!(p(0x0020_0610).kind(), ParameterKind::Numeric { base: 0, class: 1, rows: 1, columns: 4 });
        assert_eq!(p(0x0020_07A0).kind(), ParameterKind::Numeric { base: 0, class: 2, rows: 4, columns: 4 });
        assert_eq!(p(0x0020_0008).kind(), ParameterKind::Numeric { base: 2, class: 0, rows: 1, columns: 1 });
        assert_eq!(p(0x0040_004C).kind(), ParameterKind::Sampler { dimension: 4 });
        assert_eq!(p(0x0020_00C2).kind(), ParameterKind::Array { elements: 48 });
    }
}
