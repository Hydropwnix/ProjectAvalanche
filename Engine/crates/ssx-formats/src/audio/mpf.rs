use crate::reader::Reader;
use crate::{Error, Result};

pub const HEADER_SIZE: usize = 0x48;
pub const EVENT_HEADER_SIZE: usize = 0x14;
pub const ACTION_SIZE: usize = 12;
pub const VARIABLE_SIZE: usize = 20;

#[derive(Clone, Debug)]
pub struct MpfFile {
    pub version: u8,
    pub sub_version: u8,
    pub unk_06: [u8; 2],
    pub project: u8,
    pub unk_0e: u8,
    pub unk_10: u8,
    pub sections: [u32; 10],
    pub node_offsets: Vec<u32>,
    pub nodes: Vec<Node>,
    pub event_offsets: Vec<u32>,
    pub events: Vec<Event>,
    pub variables: Vec<Variable>,
    pub remap_table: Vec<u32>,
    pub tracks: Vec<Track>,
    pub samples: Vec<MpfSample>,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub offset: u32,
    pub first_sample: u32,
    pub num_subbanks: u16,
    pub unk_06: u16,
    pub checksum: u32,
    pub mus_header_size: u32,
    pub unk_10: u32,
    pub subbanks: Vec<[u32; 4]>,
}

#[derive(Clone, Copy, Debug)]
pub struct MpfSample {
    pub location: u32,
    pub duration_ms: u32,
}

impl MpfFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(b"PFDx")?;
        let version = r.u8()?;
        let sub_version = r.u8()?;
        if version != 5 || sub_version != 2 {
            return Err(Error::invalid(format!("PFDx version {version}.{sub_version}")));
        }
        let unk_06 = r.array()?;
        if r.u32()? != 0 {
            return Err(Error::invalid("PFDx 0x08 not zero"));
        }
        let project = r.u8()?;
        let num_tracks = r.u8()? as usize;
        let unk_0e = r.u8()?;
        let num_events = r.u8()? as usize;
        let unk_10 = r.u8()?;
        let num_variables = r.u8()? as usize;
        let num_nodes = r.u16()? as usize;
        let mut sections = [0u32; 10];
        for s in &mut sections {
            *s = r.u32()?;
        }
        if r.bytes(HEADER_SIZE - 0x3C)?.iter().any(|&b| b != 0) {
            return Err(Error::invalid("PFDx 0x3C..0x48 not zero"));
        }
        let [
            nodes_table,
            node_data,
            event_table,
            event_data,
            vars,
            remap,
            tracks_table,
            track_data,
            samples_table,
            eof,
        ] = sections;
        if nodes_table as usize != HEADER_SIZE || eof as usize != data.len() || !sections.is_sorted() {
            return Err(Error::invalid(format!("PFDx sections {sections:x?} (size {:#x})", data.len())));
        }

        let node_offsets = (0..num_nodes).map(|_| Ok(r.u16()? as u32 * 4)).collect::<Result<Vec<_>>>()?;
        if r.pos().next_multiple_of(4) != node_data as usize {
            return Err(Error::invalid(format!("PFDx node table ends at {:#x}", r.pos())));
        }
        let nodes = parse_nodes(data, &node_offsets, event_table)?;

        r.seek(event_table as usize)?;
        let event_offsets = (0..num_events).map(|_| Ok(r.u16()? as u32 * 4)).collect::<Result<Vec<_>>>()?;
        if r.pos().next_multiple_of(4) != event_data as usize {
            return Err(Error::invalid(format!("PFDx event table ends at {:#x} (data {event_data:#x})", r.pos())));
        }
        let events = parse_events(data, &event_offsets, event_data, vars)?;

        if (remap - vars) as usize != num_variables * VARIABLE_SIZE {
            return Err(Error::invalid(format!("PFDx {num_variables} variables in {:#x} bytes", remap - vars)));
        }
        r.seek(vars as usize)?;
        let variables = (0..num_variables).map(|_| Variable::read(&mut r)).collect::<Result<Vec<_>>>()?;
        let remap_table = (0..(tracks_table - remap) / 4).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;

        r.seek(tracks_table as usize)?;
        let track_offsets = (0..num_tracks).map(|_| Ok(r.u32()? * 4)).collect::<Result<Vec<_>>>()?;
        if r.pos() != track_data as usize {
            return Err(Error::invalid(format!("PFDx tracks table ends at {:#x}", r.pos())));
        }
        let mut tracks = Vec::with_capacity(num_tracks);
        for &offset in &track_offsets {
            tracks.push(Track::read(&mut r, offset)?);
        }
        if r.pos() != samples_table as usize {
            return Err(Error::invalid(format!("PFDx track records end at {:#x}", r.pos())));
        }
        let n = ((eof - samples_table) / 8) as usize;
        let samples =
            (0..n).map(|_| Ok(MpfSample { location: r.u32()?, duration_ms: r.u32()? })).collect::<Result<Vec<_>>>()?;
        if r.remaining() != 0 {
            return Err(Error::invalid("PFDx samples table does not end at EOF"));
        }
        Ok(Self {
            version,
            sub_version,
            unk_06,
            project,
            unk_0e,
            unk_10,
            sections,
            node_offsets,
            nodes,
            event_offsets,
            events,
            variables,
            remap_table,
            tracks,
            samples,
        })
    }
}

impl MpfFile {
    pub fn track_samples(&self, t: usize) -> std::ops::Range<usize> {
        let start = self.tracks[t].first_sample as usize;
        let end = self.tracks[t + 1..]
            .iter()
            .map(|tr| tr.first_sample as usize)
            .find(|&s| s > start)
            .unwrap_or(self.samples.len());
        start..end
    }

    pub fn event_by_id(&self, id: u32) -> Option<usize> {
        self.events.iter().rposition(|e| e.id == id & 0xFF_FFFF)
    }

    pub fn node_sample(&self, node: &Node) -> Option<&MpfSample> {
        (node.sample > 0).then(|| self.samples.get(node.sample as usize - 1)).flatten()
    }
}

impl Track {
    fn read(r: &mut Reader, offset: u32) -> Result<Self> {
        r.seek(offset as usize)?;
        let first_sample = r.u32()?;
        let num_subbanks = r.u16()?;
        let unk_06 = r.u16()?;
        let checksum = r.u32()?;
        let mus_header_size = r.u32()?;
        let unk_10 = r.u32()?;
        let subbanks =
            (0..num_subbanks).map(|_| Ok([r.u32()?, r.u32()?, r.u32()?, r.u32()?])).collect::<Result<Vec<_>>>()?;
        Ok(Self { offset, first_sample, num_subbanks, unk_06, checksum, mus_header_size, unk_10, subbanks })
    }
}

#[derive(Clone, Debug)]
pub struct Variable {
    pub name: String,
    pub raw_name: [u8; 12],
    pub unk_0c: u32,
    pub value: i32,
}

impl Variable {
    fn read(r: &mut Reader) -> Result<Self> {
        let raw_name: [u8; 12] = r.array()?;
        let end = raw_name.iter().position(|&b| b == 0).unwrap_or(12);
        let name = String::from_utf8_lossy(&raw_name[..end]).into_owned();
        Ok(Self { name, raw_name, unk_0c: r.u32()?, value: r.u32()? as i32 })
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub offset: u32,
    pub sample: i16,
    pub track: u8,
    pub part: u8,
    pub loop_count: u8,
    pub remap: u16,
    pub level_mode: u8,
    pub beats_per_bar: u8,
    pub bars: u8,
    pub section: u16,
    pub beat_unit: u8,
    pub sync_mode: u8,
    pub unk_w2: u16,
    pub word3: u32,
    pub branches: Vec<Branch>,
}

pub const SECTION_START: i16 = 0;
pub const LOOP_MARK: i16 = -1;
pub const RANDOM_LEVEL: i16 = -2;
pub const POST_EVENT: i16 = -3;
pub const FLAG_MASTER_BEAT: u32 = 0x8000_0000;
pub const FLAG_BEAT_SLICED: u32 = 0x4000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Branch {
    pub min: i8,
    pub max: i8,
    pub target: u16,
}

impl Node {
    pub fn choose_branch(&self, level: i32) -> Option<Branch> {
        if let Some(b) = self.branches.iter().find(|b| i32::from(b.min) <= level && level <= i32::from(b.max))
            && (b.target as i16) >= 0
        {
            return Some(*b);
        }
        let dist = |b: &Branch| (i32::from(b.min) - level).abs().min((i32::from(b.max) - level).abs());
        let mut best: Option<(Branch, i32)> = None;
        for b in &self.branches {
            let d = dist(b);
            if d < best.map_or(0x7F, |(_, bd)| bd) {
                best = Some((*b, d));
            }
        }
        best.map(|(b, _)| b).or_else(|| self.branches.first().copied())
    }

    pub fn beat_ms(&self, duration_ms: u32) -> u32 {
        let n = u32::from(self.beats_per_bar) * u32::from(self.bars);
        duration_ms.checked_div(n).unwrap_or(duration_ms)
    }
}

fn parse_nodes(data: &[u8], offsets: &[u32], end: u32) -> Result<Vec<Node>> {
    let mut r = Reader::be(data);
    let mut nodes = Vec::with_capacity(offsets.len());
    for (i, &offset) in offsets.iter().enumerate() {
        let next = offsets.get(i + 1).copied().unwrap_or(end);
        r.seek(offset as usize)?;
        let w = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
        let count = ((w[1] >> 15) & 0x1F) as usize;
        let branches = (0..count)
            .map(|_| {
                let b = r.u32()?;
                Ok(Branch { min: (b >> 24) as i8, max: (b >> 16) as i8, target: b as u16 })
            })
            .collect::<Result<Vec<_>>>()?;
        if r.pos() != next as usize {
            return Err(Error::invalid(format!(
                "PFDx node {i} at {offset:#x}: {count} branches end at {:#x}, next node at {next:#x}",
                r.pos()
            )));
        }
        nodes.push(Node {
            offset,
            sample: (w[0] >> 16) as i16,
            track: ((w[0] >> 11) & 0x1F) as u8,
            part: ((w[0] >> 5) & 0x3F) as u8,
            loop_count: (w[0] & 0x1F) as u8,
            remap: (w[1] >> 20) as u16,
            level_mode: ((w[1] >> 12) & 7) as u8,
            beats_per_bar: ((w[1] >> 8) & 0xF) as u8,
            bars: w[1] as u8,
            section: (w[2] >> 16) as u16,
            beat_unit: ((w[2] >> 4) & 0xF) as u8,
            sync_mode: ((w[2] >> 2) & 3) as u8,
            unk_w2: ((w[2] & 3) | (w[2] & 0xFF00)) as u16,
            word3: w[3],
            branches,
        });
    }
    Ok(nodes)
}

#[derive(Clone, Debug)]
pub struct Event {
    pub offset: u32,
    pub id: u32,
    pub unk_header: [u32; 4],
    pub actions: Vec<Action>,
}

fn parse_events(data: &[u8], offsets: &[u32], start: u32, end: u32) -> Result<Vec<Event>> {
    let mut r = Reader::be(data);
    let mut events = Vec::with_capacity(offsets.len());
    let mut expect = start;
    for (i, &offset) in offsets.iter().enumerate() {
        if offset != expect {
            return Err(Error::invalid(format!("PFDx event {i} at {offset:#x}, expected {expect:#x}")));
        }
        r.seek(offset as usize)?;
        let (h0, h1, h2) = (r.u32()?, r.u32()?, r.u32()?);
        let id_word = r.u32()?;
        let h4 = r.u32()?;
        let actions = (0..id_word & 0xFF)
            .map(|_| Ok(Action { words: [r.u32()?, r.u32()?, r.u32()?] }))
            .collect::<Result<Vec<_>>>()?;
        expect = r.pos() as u32;
        events.push(Event { offset, id: id_word >> 8, unk_header: [h0, h1, h2, h4], actions });
    }
    if expect.next_multiple_of(4) != end {
        return Err(Error::invalid(format!("PFDx events end at {expect:#x}, next section at {end:#x}")));
    }
    Ok(events)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub words: [u32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opcode {
    Control,
    Wait,
    Sync,
    Goto,
    FadeVolume,
    Fade6,
    Fade7,
    Set,
    Post,
    Callback,
    Arith,
    Pause,
    Op16,
    FadePitch,
    Fade18,
    Unknown(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Literal,
    Property,
    Variable,
}

impl Action {
    pub fn players(&self) -> u32 {
        self.words[0] & 0xFF_FFFF
    }
    pub fn projects(&self) -> u8 {
        ((self.words[0] >> 24) & 0xF) as u8
    }
    pub fn player_flags(&self) -> u8 {
        (self.words[0] >> 28) as u8
    }
    pub fn opcode(&self) -> Opcode {
        match (self.words[1] >> 17) & 0x7F {
            1 => Opcode::Control,
            2 => Opcode::Wait,
            3 => Opcode::Sync,
            4 => Opcode::Goto,
            5 => Opcode::FadeVolume,
            6 => Opcode::Fade6,
            7 => Opcode::Fade7,
            8 => Opcode::Set,
            9 => Opcode::Post,
            13 => Opcode::Callback,
            14 => Opcode::Arith,
            15 => Opcode::Pause,
            16 => Opcode::Op16,
            17 => Opcode::FadePitch,
            18 => Opcode::Fade18,
            n => Opcode::Unknown(n as u8),
        }
    }
    fn source(bits: u32) -> Source {
        match bits & 3 {
            1 => Source::Property,
            2 => Source::Variable,
            _ => Source::Literal,
        }
    }
    pub fn source_a(&self) -> Source {
        Self::source(self.words[1] >> 14)
    }
    pub fn source_b(&self) -> Source {
        Self::source(self.words[1] >> 12)
    }
    pub fn source_bits(&self) -> (u8, u8) {
        (((self.words[1] >> 14) & 3) as u8, ((self.words[1] >> 12) & 3) as u8)
    }
    pub fn sub_op(&self) -> u8 {
        ((self.words[1] >> 9) & 7) as u8
    }
    pub fn compare(&self) -> u8 {
        ((self.words[1] >> 6) & 7) as u8
    }
    pub fn runtime_bits(&self) -> u32 {
        self.words[1] & 0x1_003F
    }
    pub fn hi(&self) -> u16 {
        (self.words[2] >> 16) as u16
    }
    pub fn lo(&self) -> u16 {
        self.words[2] as u16
    }
}

#[derive(Clone, Debug)]
pub struct MusFile {
    pub checksum: u32,
    pub entries: Vec<MusEntry>,
}

#[derive(Clone, Copy, Debug)]
pub struct MusEntry {
    pub index: u16,
    pub sub_index: u16,
    pub snr_offset: u32,
    pub sns_offset: u32,
}

impl MusFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        let checksum = r.u32()?;
        if r.u32()? != 0 {
            return Err(Error::invalid("MUS 0x04 not zero"));
        }
        let mut entries = Vec::new();
        let mut limit = data.len();
        while r.pos() + 12 <= limit {
            let e = MusEntry { index: r.u16()?, sub_index: r.u16()?, snr_offset: r.u32()?, sns_offset: r.u32()? };
            if e.snr_offset == 0 {
                break;
            }
            limit = limit.min(e.snr_offset as usize);
            entries.push(e);
        }
        if entries.is_empty() {
            return Err(Error::invalid("MUS without entries"));
        }
        Ok(Self { checksum, entries })
    }

    pub fn index_end(&self) -> usize {
        8 + 12 * self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mus_index_stops_at_padding() {
        let mut d = vec![0u8; 0x30];
        d[..4].copy_from_slice(&0x0CC3_1862u32.to_be_bytes());
        d[8..20].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0x20, 0, 0, 0, 0x80]);
        let m = MusFile::parse(&d).unwrap();
        assert_eq!((m.checksum, m.entries.len(), m.index_end()), (0x0CC3_1862, 1, 20));
        assert_eq!((m.entries[0].snr_offset, m.entries[0].sns_offset), (0x20, 0x80));
    }

    fn node(branches: &[(i8, i8, u16)]) -> Node {
        Node {
            offset: 0,
            sample: 1,
            track: 0,
            part: 1,
            loop_count: 0,
            remap: 0,
            level_mode: 0,
            beats_per_bar: 4,
            bars: 1,
            section: 0,
            beat_unit: 4,
            sync_mode: 0,
            unk_w2: 0,
            word3: 0,
            branches: branches.iter().map(|&(min, max, target)| Branch { min, max, target }).collect(),
        }
    }

    #[test]
    fn branch_choice_first_match_then_nearest() {
        let n = node(&[(-1, 30, 32), (30, 127, 48)]);
        assert_eq!(n.choose_branch(10).unwrap().target, 32);
        assert_eq!(n.choose_branch(30).unwrap().target, 32);
        assert_eq!(n.choose_branch(50).unwrap().target, 48);
        let lp = node(&[(0, 0, 0xFFFF), (1, 127, 30)]);
        assert_eq!(lp.choose_branch(0).unwrap().target, 0xFFFF);
        assert_eq!(lp.choose_branch(31).unwrap().target, 30);
        let gap = node(&[(10, 20, 1), (40, 50, 2)]);
        assert_eq!(gap.choose_branch(33).unwrap().target, 2);
        assert!(node(&[]).choose_branch(0).is_none());
        assert_eq!(n.beat_ms(1548), 387);
    }

    #[test]
    fn action_fields() {
        let a = Action { words: [0x1200_0001, 0x0002_B240, 0x0000_0001] };
        assert_eq!((a.players(), a.projects(), a.player_flags()), (1, 2, 1));
        assert_eq!((a.opcode(), a.sub_op(), a.compare()), (Opcode::Control, 1, 1));
        assert_eq!((a.source_a(), a.source_b(), a.hi(), a.lo()), (Source::Variable, Source::Literal, 0, 1));
        let s = Action { words: [0x1200_0001, 0x0010_7000, 0x0078_1700] };
        assert_eq!((s.opcode(), s.source_a(), s.source_b()), (Opcode::Set, Source::Property, Source::Literal));
        assert_eq!(s.runtime_bits(), 0);
    }
}
