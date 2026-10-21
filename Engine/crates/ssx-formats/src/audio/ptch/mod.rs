mod curve;
mod modules;
mod v1;
mod voice;

use crate::reader::Reader;
use crate::{Error, Result};

pub use curve::*;
pub use modules::*;
pub use v1::*;
pub use voice::*;

pub const MAGIC: &[u8; 4] = b"PTCH";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ref(pub u32);

impl Ref {
    pub const NONE: Ref = Ref(0x000F_FFC0);

    pub fn block(self) -> usize {
        (self.0 & 0x3F) as usize
    }

    pub fn offset(self) -> usize {
        ((self.0 >> 4) & 0xFFFC) as usize
    }

    pub fn vtype(self) -> ValueType {
        ValueType((self.0 >> 16) as u16)
    }

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ValueType(pub u16);

impl ValueType {
    pub fn base(self) -> BaseType {
        BaseType::from_bits((self.0 >> 4) & 7)
    }

    pub fn unit(self) -> u8 {
        ((self.0 >> 7) & 0xFF) as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BaseType {
    Word,
    Int,
    UInt,
    Float,
    Double,
    Bool,
    Unk6,
    Unk7,
}

impl BaseType {
    fn from_bits(b: u16) -> Self {
        match b {
            0 => Self::Word,
            1 => Self::Int,
            2 => Self::UInt,
            3 => Self::Float,
            4 => Self::Double,
            5 => Self::Bool,
            6 => Self::Unk6,
            _ => Self::Unk7,
        }
    }

    pub fn size(self) -> usize {
        match self {
            Self::Double => 8,
            _ => 4,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Ptch {
    V4(Patch),
    V1(PatchV1),
}

impl Ptch {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(MAGIC)?;
        match r.u8()? {
            4 => Patch::parse(data).map(Self::V4),
            1 => PatchV1::parse(data).map(Self::V1),
            v => Err(Error::invalid(format!("PTCH version {v} (only 1 and 4 exist)"))),
        }
    }
}

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::invalid(msg))
}

fn cstr_at(data: &[u8], pos: usize) -> Result<String> {
    let mut r = Reader::be(data);
    r.seek(pos)?;
    r.cstr()
}

fn at(data: &[u8], pos: usize) -> Result<Reader<'_>> {
    let mut r = Reader::be(data);
    r.seek(pos)?;
    Ok(r)
}

struct Coverage {
    used: Vec<bool>,
}

impl Coverage {
    fn new(len: usize) -> Self {
        Self { used: vec![false; len] }
    }

    fn mark(&mut self, start: usize, len: usize, what: &str) -> Result<()> {
        let end = start.checked_add(len).filter(|&e| e <= self.used.len());
        let Some(end) = end else {
            return invalid(format!("{what} at {start:#x}+{len:#x} runs past the end"));
        };
        if self.used[start..end].iter().any(|&u| u) {
            return invalid(format!("{what} at {start:#x}+{len:#x} overlaps another section"));
        }
        self.used[start..end].fill(true);
        Ok(())
    }

    fn finish(&self, data: &[u8]) -> Result<usize> {
        let mut padding = 0;
        for (i, (&u, &b)) in self.used.iter().zip(data).enumerate() {
            if !u {
                if b != 0 {
                    return invalid(format!("unexplained non-zero byte {b:#04x} at {i:#x}"));
                }
                padding += 1;
            }
        }
        Ok(padding)
    }
}

#[derive(Clone, Debug)]
pub struct Patch {
    pub debug: bool,
    pub unk_id: [u8; 16],
    pub name: String,
    pub configs: Vec<Config>,
    pub interfaces: Vec<Interface>,
    pub ports: Vec<Port>,
    pub locals: Layout,
    pub locals_init: Vec<u8>,
    pub modules: Vec<Module>,
    pub padding: usize,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub unk_type: u16,
    pub name: String,
    pub options: Vec<ConfigOption>,
}

#[derive(Clone, Debug)]
pub struct ConfigOption {
    pub value: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Interface {
    pub layout: Layout,
    pub hash: u32,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub size: u16,
    pub n_events: u16,
    pub n_signals: u16,
    pub n_fields: u16,
    pub events: Option<Vec<Member>>,
    pub signals: Option<Vec<Member>>,
    pub fields: Option<Vec<Member>>,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub vtype: ValueType,
    pub offset: u16,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Port {
    pub input: bool,
    pub interface: u16,
    pub name: String,
    pub init: Vec<u8>,
}

impl Patch {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut cov = Coverage::new(data.len());
        let mut r = Reader::be(data);
        r.expect(MAGIC)?;
        let (version, loaded, debug, unk_7) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        if version != 4 || loaded != 0 || debug > 1 || unk_7 != 0 {
            return invalid(format!("PTCH v4 header bytes {version} {loaded} {debug} {unk_7}"));
        }
        let unk_id = r.array::<16>()?;
        let (n_cfg, n_if, n_ports, unk_1e) = (r.u16()? as usize, r.u16()? as usize, r.u16()? as usize, r.u16()?);
        let mut o = [0usize; 10];
        for v in &mut o {
            *v = r.u32()? as usize;
        }
        let [o_cfg, o_cfg_names, o_if, o_if_names, o_locals, o_locals_init, o_ports, o_port_names, o_modules, unk_44] =
            o;
        if unk_1e != 0 || unk_44 != 0 || n_ports >= 0x3F {
            return invalid(format!("PTCH v4 header: word 0x1e {unk_1e}, 0x44 {unk_44:#x}, {n_ports} ports"));
        }
        let name = cstr_at(data, 0x48)?;
        cov.mark(0, (0x48 + name.len() + 1).next_multiple_of(4), "header")?;

        let cfg_names = name_table(data, &mut cov, o_cfg_names, n_cfg, "config names")?;
        let mut configs = Vec::with_capacity(n_cfg);
        for (i, name) in cfg_names.into_iter().enumerate() {
            let mut r = at(data, o_cfg + 12 * i)?;
            let (unk_type, unk_2, _name_off, options_off) = (r.u16()?, r.u16()?, r.u32()?, r.u32()? as usize);
            if unk_2 != 0 || (options_off != 0) != (debug == 1) {
                return invalid(format!("config {i}: word {unk_2}, options at {options_off:#x}"));
            }
            cov.mark(o_cfg + 12 * i, 12, "config")?;
            let options = if options_off != 0 { config_options(data, &mut cov, options_off)? } else { Vec::new() };
            configs.push(Config { unk_type, name, options });
        }

        let if_names = name_table(data, &mut cov, o_if_names, n_if, "interface names")?;
        let mut interfaces = Vec::with_capacity(n_if);
        for (i, name) in if_names.into_iter().enumerate() {
            let pos = o_if + 0x1C * i;
            let layout = Layout::parse(data, &mut cov, pos)?;
            let mut r = at(data, pos + 0x14)?;
            let hash = r.u32()?;
            cov.mark(pos, 0x1C, "interface")?;
            interfaces.push(Interface { layout, hash, name });
        }

        let port_names = name_table(data, &mut cov, o_port_names, n_ports, "port names")?;
        let mut ports = Vec::with_capacity(n_ports);
        for (i, name) in port_names.into_iter().enumerate() {
            let pos = o_ports + 12 * i;
            let mut r = at(data, pos)?;
            let (input, unk_1, interface, _name_off, init_off) = (r.u8()?, r.u8()?, r.u16()?, r.u32()?, r.u32()?);
            let Some(iface) = interfaces.get(interface as usize) else {
                return invalid(format!("port {i}: interface {interface} out of range"));
            };
            if input > 1 || unk_1 != 0 {
                return invalid(format!("port {i}: bytes {input} {unk_1}"));
            }
            cov.mark(pos, 12, "port")?;
            let size = iface.layout.size as usize;
            cov.mark(init_off as usize, size, "port data")?;
            let init = at(data, init_off as usize)?.bytes(size)?.to_vec();
            ports.push(Port { input: input == 1, interface, name, init });
        }

        let locals = Layout::parse(data, &mut cov, o_locals)?;
        cov.mark(o_locals, 0x14, "locals")?;
        cov.mark(o_locals_init, locals.size as usize, "locals data")?;
        let locals_init = at(data, o_locals_init)?.bytes(locals.size as usize)?.to_vec();

        let mut modules = Vec::new();
        let mut r = at(data, o_modules)?;
        loop {
            let pos = r.pos();
            let fourcc = r.array::<4>()?;
            if fourcc == [0xFF; 4] {
                cov.mark(pos, 4, "end marker")?;
                if r.remaining() != 0 {
                    return invalid(format!("{} bytes after the module end marker", r.remaining()));
                }
                break;
            }
            let size = r.u32()? as usize;
            let body = r.bytes(size)?;
            cov.mark(pos, 8 + size, "module")?;
            let kind = ModuleKind::parse(fourcc, body, pos + 8)
                .map_err(|e| Error::invalid(format!("module {} at {pos:#x}: {e}", fourcc_str(fourcc))))?;
            modules.push(Module { offset: pos, fourcc, kind });
        }
        let padding = cov.finish(data)?;
        Ok(Self { debug: debug == 1, unk_id, name, configs, interfaces, ports, locals, locals_init, modules, padding })
    }

    pub fn block_layout(&self, block: usize) -> Option<&Layout> {
        match self.ports.get(block) {
            Some(p) => self.interfaces.get(p.interface as usize).map(|i| &i.layout),
            None if block == self.ports.len() => Some(&self.locals),
            None => None,
        }
    }

    pub fn ref_name(&self, r: Ref) -> Option<&str> {
        let l = self.block_layout(r.block())?;
        [&l.events, &l.signals, &l.fields]
            .into_iter()
            .flatten()
            .flatten()
            .find(|m| m.offset as usize == r.offset())
            .map(|m| m.name.as_str())
    }
}

pub fn fourcc_str(f: [u8; 4]) -> String {
    String::from_utf8_lossy(&f).into_owned()
}

fn name_table(data: &[u8], cov: &mut Coverage, off: usize, n: usize, what: &str) -> Result<Vec<String>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let mut r = at(data, off)?;
    let mut by_rank = vec![0usize; n];
    let mut rank_of = vec![0usize; n];
    let mut str_off = vec![0usize; n];
    for v in by_rank.iter_mut().chain(rank_of.iter_mut()).chain(str_off.iter_mut()) {
        *v = r.u16()? as usize;
    }
    cov.mark(off, 6 * n, what)?;
    let base = off + 6 * n;
    let mut sorted = Vec::with_capacity(n);
    for &s in &str_off {
        let name = cstr_at(data, base + s)?;
        cov.mark(base + s, name.len() + 1, what)?;
        sorted.push(name);
    }
    let perm_ok = (0..n).all(|k| by_rank[k] < n && rank_of[by_rank[k]] == k);
    if !perm_ok || !sorted.is_sorted() {
        return invalid(format!("{what} at {off:#x}: inconsistent permutations or unsorted names"));
    }
    Ok(rank_of.iter().map(|&k| sorted[k].clone()).collect())
}

impl Layout {
    fn parse(data: &[u8], cov: &mut Coverage, pos: usize) -> Result<Self> {
        let mut r = at(data, pos)?;
        let (o_ev, o_sig, o_fld) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        let (size, n_events, n_signals, n_fields) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
        let events = member_table(data, cov, o_ev, n_events as usize)?;
        let signals = member_table(data, cov, o_sig, n_signals as usize)?;
        let fields = member_table(data, cov, o_fld, n_fields as usize)?;
        for (m, kind) in [(&events, "event"), (&signals, "signal")] {
            if let Some(m) = m.iter().flatten().find(|m| m.vtype.0 != 0) {
                return invalid(format!("{kind} {} has type {:#x}", m.name, m.vtype.0));
            }
        }
        let l = Self { size, n_events, n_signals, n_fields, events, signals, fields };
        let widths = |m: &Option<Vec<Member>>, w: Option<usize>| -> Vec<(usize, usize)> {
            m.iter().flatten().map(|m| (m.offset as usize, w.unwrap_or(m.vtype.base().size()))).collect()
        };
        let mut spans = widths(&l.events, Some(4));
        spans.extend(widths(&l.signals, Some(8)));
        spans.extend(widths(&l.fields, None));
        if let Some(&(o, w)) = spans.iter().find(|&&(o, w)| o + w > size as usize || o % w.min(8) != 0) {
            return invalid(format!("member at {o:#x} (+{w}) outside or misaligned in a {size:#x}-byte block"));
        }
        Ok(l)
    }
}

fn member_table(data: &[u8], cov: &mut Coverage, off: usize, n: usize) -> Result<Option<Vec<Member>>> {
    if off == 0 || n == 0 {
        return Ok(if n == 0 { Some(Vec::new()) } else { None });
    }
    let mut r = at(data, off)?;
    let mut raw = Vec::with_capacity(n);
    for _ in 0..n {
        raw.push((r.u16()?, r.u16()?));
    }
    let mut order = Vec::with_capacity(n);
    for _ in 0..n {
        order.push(r.u16()? as usize);
    }
    let mut str_off = Vec::with_capacity(n);
    for _ in 0..n {
        str_off.push(r.u16()? as usize);
    }
    cov.mark(off, 8 * n, "member table")?;
    let base = off + 8 * n;
    let mut members = Vec::with_capacity(n);
    for (&(t, o), &s) in raw.iter().zip(&str_off) {
        let name = cstr_at(data, base + s)?;
        cov.mark(base + s, name.len() + 1, "member name")?;
        if t & 0xF != 0 || o & 0xF != 0 {
            return invalid(format!("member {name}: type {t:#x} / offset {o:#x} with low bits set"));
        }
        members.push(Member { vtype: ValueType(t), offset: o >> 4, name });
    }
    let mut expect: Vec<usize> = (0..n).collect();
    expect.sort_by(|&a, &b| (members[a].vtype, &members[a].name).cmp(&(members[b].vtype, &members[b].name)));
    if order != expect || !members.is_sorted_by(|a, b| a.name <= b.name) {
        return invalid(format!("member table at {off:#x}: order {order:?} (expected {expect:?})"));
    }
    Ok(Some(members))
}

fn config_options(data: &[u8], cov: &mut Coverage, off: usize) -> Result<Vec<ConfigOption>> {
    for n in 1..=16usize {
        let Some(opts) = try_options(data, off, n) else { continue };
        let strings: usize = opts.iter().map(|o| o.name.len() + 1).sum();
        cov.mark(off, 8 * n + strings, "config options")?;
        return Ok(opts);
    }
    invalid(format!("config option table at {off:#x}: no consistent count"))
}

fn try_options(data: &[u8], off: usize, n: usize) -> Option<Vec<ConfigOption>> {
    let mut r = at(data, off).ok()?;
    let values: Vec<u32> = (0..n).map(|_| r.u32()).collect::<Result<_>>().ok()?;
    let order: Vec<usize> = (0..n).map(|_| r.u16().map(usize::from)).collect::<Result<_>>().ok()?;
    let offs: Vec<usize> = (0..n).map(|_| r.u16().map(usize::from)).collect::<Result<_>>().ok()?;
    let base = off + 8 * n;
    let mut names = Vec::with_capacity(n);
    let mut expect_off = 0;
    for &o in &offs {
        if o != expect_off {
            return None;
        }
        let s = cstr_at(data, base + o).ok()?;
        if s.is_empty() || !s.bytes().all(|b| (0x20..0x7F).contains(&b)) {
            return None;
        }
        expect_off += s.len() + 1;
        names.push(s);
    }
    let mut by_value: Vec<usize> = (0..n).collect();
    by_value.sort_by_key(|&i| values[i]);
    (order == by_value && names.is_sorted())
        .then(|| values.into_iter().zip(names).map(|(value, name)| ConfigOption { value, name }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(interp: u8, xs: &[f32], points: &[f32]) -> Curve {
        Curve {
            interp,
            wrap: 0,
            uniform: xs.is_empty(),
            min_x: xs.first().copied().unwrap_or(-1.0),
            max_x: xs.last().copied().unwrap_or(1.0),
            min_y: -1e9,
            max_y: 1e9,
            n_segments: (points.len() - 1) as u16,
            stride: 1,
            outputs: Vec::new(),
            xs: xs.to_vec(),
            points: points.to_vec(),
        }
    }

    #[test]
    fn linear_curves_interpolate_and_clamp() {
        let c = curve(1, &[], &[45.0, 0.0, 315.0]);
        assert_eq!(c.eval(-1.0), 45.0);
        assert_eq!(c.eval(0.0), 0.0);
        assert!((c.eval(-0.5) - 22.5).abs() < 1e-5);
        assert!((c.eval(0.5) - 157.5).abs() < 1e-4);
        assert_eq!(c.eval(3.0), 315.0);
        let c = curve(1, &[0.0, 10.0, 20.0], &[0.0, 1.0, 3.0]);
        assert!((c.eval(15.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn nearest_and_steps() {
        let c = curve(0, &[0.0, 1.0, 2.0], &[5.0, 7.0, 9.0]);
        assert_eq!(c.eval(0.4), 5.0);
        assert_eq!(c.eval(0.5), 7.0);
        let c = curve(1, &[0.0, 5000.0, 5000.0, 10000.0], &[0.0, 0.0, 1.0, 1.0]);
        assert_eq!(c.eval(4999.0), 0.0);
        assert_eq!(c.eval(5001.0), 1.0);
    }

    #[test]
    fn cubic_passes_its_points_and_is_smooth() {
        let c = curve(2, &[0.0, 1.0, 2.0, 3.0], &[0.0, 1.0, 0.0, 1.0]);
        for (x, y) in [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0), (3.0, 1.0)] {
            assert!((c.eval(x) - y).abs() < 1e-6, "{x}");
        }
        let (l, r) = (c.eval(0.999), c.eval(1.001));
        assert!((l - r).abs() < 1e-3);
    }

    #[test]
    fn assignments_leave_their_target() {
        assert_eq!(Instr::Op(29).arity(), (2, 1));
        assert_eq!(OPERATORS[18], "eq");
        assert_eq!(FUNCTIONS[0], "abs");
    }
}
