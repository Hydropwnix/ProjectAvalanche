use crate::eaac::SoundHeader;
use crate::reader::Reader;
use crate::{Error, Result};

pub const HEADER_SIZE: usize = 0x5C;
pub const MODULE_RECORD_SIZE: usize = 0x3C;

#[derive(Clone, Debug)]
pub struct AbkFile {
    pub unk_04: [u8; 6],
    pub file_size: u32,
    pub s10a_size: u32,
    pub ast_word: u32,
    pub s10a_offset: u32,
    pub section_a_offset: u32,
    pub section_b_offset: u32,
    pub section_c_offset: u32,
    pub modules: Vec<Module>,
    pub sample_tables: Vec<SampleTable>,
    pub s10a: S10a,
    pub section_a: Vec<u32>,
    pub section_b: Vec<u32>,
    pub names: Vec<NamedRef>,
    pub names_end: usize,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub offset: u32,
    pub id: u32,
    pub max_instances: u16,
    pub num_globals: u16,
    pub num_events: u16,
    pub num_players: u8,
    pub has_stop: u8,
    pub has_params: u8,
    pub num_extra_slots: u8,
    pub script_offset: u32,
    pub data_offset: u32,
    pub data_size: u32,
    pub end_state: u32,
    pub slots: Vec<u32>,
    pub program: Vec<crate::aems::Node>,
}

#[derive(Clone, Debug)]
pub struct SampleTable {
    pub offset: u32,
    pub entries: Vec<SampleEntry>,
}

#[derive(Clone, Copy, Debug)]
pub struct SampleEntry {
    pub sound_index: u16,
    pub unk_2: u8,
    pub azimuths: [u8; 5],
    pub stream_offset: u32,
}

impl SampleEntry {
    pub fn is_unused(&self) -> bool {
        self.sound_index == 0xFFFF
    }
    pub fn is_streamed(&self) -> bool {
        self.stream_offset != 0xFFFF_FFFF
    }
}

#[derive(Clone, Debug)]
pub struct S10a {
    pub unk_04: u32,
    pub sounds: Vec<Option<S10aSound>>,
}

#[derive(Clone, Debug)]
pub struct S10aSound {
    pub offset: u32,
    pub header: SoundHeader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Global,
    Function,
    Event,
}

#[derive(Clone, Debug)]
pub struct NamedRef {
    pub target: u32,
    pub name_offset: u32,
    pub kind: RefKind,
    pub unk_8: u32,
    pub id: u32,
    pub name: String,
}

fn zero(bytes: &[u8], what: &str) -> Result<()> {
    if bytes.iter().any(|&b| b != 0) {
        return Err(Error::invalid(format!("ABKC: {what} not zero")));
    }
    Ok(())
}

impl AbkFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(b"ABKC")?;
        let unk_04: [u8; 6] = r.array()?;
        let num_modules = r.u16()? as usize;
        zero(r.bytes(8)?, "0x0C..0x14")?;
        let file_size = r.u32()?;
        let s10a_dup = r.u32()?;
        let module_table = r.u32()?;
        let s10a_offset = r.u32()?;
        let s10a_size = r.u32()?;
        zero(r.bytes(4)?, "0x28")?;
        let ast_word = r.u32()?;
        let section_a_offset = r.u32()?;
        let section_b_offset = r.u32()?;
        let section_c_offset = r.u32()?;
        zero(r.bytes(4)?, "0x3C")?;
        if r.u32()? != u32::MAX || r.u32()? != u32::MAX {
            return Err(Error::invalid("ABKC: 0x40..0x48 not 0xFF"));
        }
        zero(r.bytes(HEADER_SIZE - 0x48)?, "0x48..0x5C")?;
        if file_size as usize != data.len() || s10a_dup != s10a_offset || module_table as usize != HEADER_SIZE {
            return Err(Error::invalid(format!(
                "ABKC header: size {file_size:#x}/{:#x}, S10A {s10a_dup:#x}/{s10a_offset:#x}, modules {module_table:#x}",
                data.len()
            )));
        }
        if s10a_offset + s10a_size != section_a_offset
            || section_a_offset + 4 != section_b_offset
            || section_b_offset >= section_c_offset
        {
            return Err(Error::invalid("ABKC: sections not in S10A, A, B, C order"));
        }

        let mut modules = Vec::with_capacity(num_modules);
        for _ in 0..num_modules {
            let offset = r.pos() as u32;
            let id = r.u32()?;
            zero(r.bytes(24)?, "module 0x04..0x1C")?;
            if r.u16()? != 0 {
                return Err(Error::invalid("ABKC: module live-instance count not zero"));
            }
            let max_instances = r.u16()?;
            let num_globals = r.u16()?;
            let num_events = r.u16()?;
            let (num_players, has_stop, has_params, num_extra_slots) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            let script_offset = r.u32()?;
            let data_offset = r.u32()?;
            let data_size = r.u32()?;
            let end_state = r.u32()?;
            zero(r.bytes(4)?, "module 0x38")?;
            let n = num_players as usize + num_extra_slots as usize;
            let slots = (0..n).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
            let (program, _) = crate::aems::parse_program(data, script_offset as usize)?;
            let state: u32 = program.iter().map(|n| n.state_size).sum();
            if state + crate::aems::INSTANCE_HEADER != data_size
                || end_state + 0x10 != data_size
                || program.last().is_none_or(|n| n.op != crate::aems::Op::End)
            {
                return Err(Error::invalid(format!(
                    "ABKC module {id:#x}: program state {state:#x} vs size {data_size:#x}"
                )));
            }
            modules.push(Module {
                offset,
                id,
                max_instances,
                num_globals,
                num_events,
                num_players,
                has_stop,
                has_params,
                num_extra_slots,
                script_offset,
                data_offset,
                data_size,
                end_state,
                slots,
                program,
            });
        }

        let s10a = S10a::parse(data, s10a_offset as usize)?;

        let mut sample_tables: Vec<SampleTable> = Vec::new();
        for m in &modules {
            for &slot in &m.slots[..m.num_players as usize] {
                let player = (m.data_offset + slot) as usize;
                let mut pr = Reader::be(data);
                pr.seek(player + 4)?;
                let table = pr.u32()?;
                if sample_tables.iter().any(|t| t.offset == table) {
                    continue;
                }
                sample_tables.push(SampleTable::parse(data, table, s10a.sounds.len())?);
            }
        }

        let mut r = Reader::be(data);
        r.seek(section_a_offset as usize)?;
        let na = r.u32()? as usize;
        let section_a = (0..na).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
        if r.pos() != section_b_offset as usize {
            return Err(Error::invalid("ABKC: section A does not end at section B"));
        }
        let nb = r.u32()? as usize;
        let section_b = (0..nb).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
        if r.pos() != section_c_offset as usize {
            return Err(Error::invalid(format!("ABKC: section B ends at {:#x}, C at {section_c_offset:#x}", r.pos())));
        }
        let nc = r.u32()? as usize;
        let mut raw = Vec::with_capacity(nc);
        for _ in 0..nc {
            raw.push((r.u32()?, r.u32()?, r.u32()?));
        }
        let mut names = Vec::with_capacity(nc);
        let mut names_end = r.pos();
        for (target, name_offset, unk_8) in raw {
            let mut nr = Reader::be(data);
            nr.seek(name_offset as usize)?;
            let id = nr.u32()?;
            let name = nr.cstr()?;
            nr.align(4)?;
            names_end = names_end.max(nr.pos());
            let kind = match unk_8 >> 24 {
                0 => RefKind::Global,
                1 => RefKind::Function,
                2 => RefKind::Event,
                k => return Err(Error::invalid(format!("ABKC named reference kind {k}"))),
            };
            names.push(NamedRef { target, kind, unk_8: unk_8 & 0xFF_FFFF, name_offset, id, name });
        }

        Ok(Self {
            unk_04,
            file_size,
            s10a_size,
            ast_word,
            s10a_offset,
            section_a_offset,
            section_b_offset,
            section_c_offset,
            modules,
            sample_tables,
            s10a,
            section_a,
            section_b,
            names,
            names_end,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = &SampleEntry> {
        self.sample_tables.iter().flat_map(|t| t.entries.iter()).filter(|e| !e.is_unused())
    }
}

impl SampleTable {
    fn parse(data: &[u8], offset: u32, num_sounds: usize) -> Result<Self> {
        let mut r = Reader::be(data);
        r.seek(offset as usize)?;
        let n = r.u32()? as usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let e =
                SampleEntry { sound_index: r.u16()?, unk_2: r.u8()?, azimuths: r.array()?, stream_offset: r.u32()? };
            if !e.is_unused() && e.sound_index as usize >= num_sounds {
                return Err(Error::invalid(format!("ABKC sample table {offset:#x}: index {}", e.sound_index)));
            }
            entries.push(e);
        }
        Ok(Self { offset, entries })
    }

    pub fn end(&self) -> u32 {
        self.offset + 4 + 12 * self.entries.len() as u32
    }
}

impl S10a {
    pub fn parse(data: &[u8], offset: usize) -> Result<Self> {
        let mut r = Reader::be(data);
        r.seek(offset)?;
        r.expect(b"S10A")?;
        let unk_04 = r.u32()?;
        let n = r.u32()? as usize;
        let mut sounds = Vec::with_capacity(n);
        for _ in 0..n {
            let rel = r.u32()?;
            if rel == u32::MAX {
                sounds.push(None);
                continue;
            }
            let at = offset + rel as usize;
            let header = SoundHeader::parse(data.get(at..).ok_or_else(|| Error::invalid("S10A offset past end"))?)?;
            sounds.push(Some(S10aSound { offset: at as u32, header }));
        }
        Ok(Self { unk_04, sounds })
    }
}

pub const AST_HEADER_SIZE: usize = 0x40;

pub fn parse_ast_header(data: &[u8]) -> Result<u32> {
    let mut r = Reader::be(data);
    r.expect(b"S10S")?;
    let word = r.u32()?;
    zero(r.bytes(AST_HEADER_SIZE - 8)?, "S10S 0x08..0x40")?;
    Ok(word)
}
