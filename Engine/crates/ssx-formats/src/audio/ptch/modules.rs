use super::*;

#[derive(Clone, Debug)]
pub struct Module {
    pub offset: usize,
    pub fourcc: [u8; 4],
    pub kind: ModuleKind,
}

#[derive(Clone, Debug)]
pub enum ModuleKind {
    Copy(Copy),
    BoolTrigger(BoolTrigger),
    Envelope(Envelope),
    Expression(Expression),
    Merge(Merge),
    Metronome(Metronome),
    RangeTrigger(RangeTrigger),
    Route(Route),
    Switch(Switch),
    Curve(Curve),
    Voice(Voice),
}

impl ModuleKind {
    pub(super) fn parse(fourcc: [u8; 4], body: &[u8], pos: usize) -> Result<Self> {
        Ok(match &fourcc {
            b"Cpy0" => Self::Copy(Copy::parse(blob(body, pos, 4)?)?),
            b"BTr0" => Self::BoolTrigger(BoolTrigger::parse(fixed(body, 0x24)?)?),
            b"Env0" => Self::Envelope(Envelope::parse(fixed(body, 0x38)?)?),
            b"Exp0" => Self::Expression(Expression::parse(blob(body, pos, 8)?)?),
            b"Mrg0" => Self::Merge(Merge::parse(blob(body, pos, 4)?)?),
            b"Met0" => Self::Metronome(Metronome::parse(fixed(body, 0x20)?)?),
            b"RTr0" => Self::RangeTrigger(RangeTrigger::parse(fixed(body, 0x40)?)?),
            b"Rot0" => Self::Route(Route::parse(blob(body, pos, 4)?)?),
            b"Swt0" => Self::Switch(Switch::parse(blob(body, pos, 4)?)?),
            b"Crv0" => Self::Curve(Curve::parse(body, pos)?),
            b"ACV2" => Self::Voice(Voice::parse(blob(body, pos, 8)?)?),
            _ => return invalid("module type not seen on the disc"),
        })
    }
}

pub(super) fn fixed(body: &[u8], len: usize) -> Result<Reader<'_>> {
    if body.len() != len {
        return invalid(format!("body is {:#x} bytes, expected {len:#x}", body.len()));
    }
    Ok(Reader::be(body))
}

fn blob(body: &[u8], pos: usize, align: usize) -> Result<&[u8]> {
    let n = Reader::be(body).u32()? as usize;
    let start = (pos + 4).next_multiple_of(align) - pos;
    let Some(end) = start.checked_add(n).filter(|&e| e <= body.len()) else {
        return invalid(format!("blob of {n:#x} bytes at +{start:#x} exceeds the {:#x}-byte body", body.len()));
    };
    if body[4..start].iter().chain(&body[end..]).any(|&b| b != 0) {
        return invalid("non-zero padding around the blob");
    }
    Ok(&body[start..end])
}

pub(super) fn refs<const N: usize>(r: &mut Reader) -> Result<[Ref; N]> {
    let mut out = [Ref::NONE; N];
    for v in &mut out {
        *v = Ref(r.u32()?);
    }
    Ok(out)
}

fn ref_pairs(r: &mut Reader, n: usize) -> Result<Vec<[Ref; 2]>> {
    (0..n).map(|_| refs::<2>(r)).collect()
}

#[derive(Clone, Debug)]
pub struct Copy {
    pub values: Vec<[Ref; 2]>,
    pub events: Vec<[Ref; 2]>,
    pub signals: Vec<[Ref; 2]>,
}

impl Copy {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::be(b);
        let (z, nv, ne, ns) = (r.u16()?, r.u16()? as usize, r.u16()? as usize, r.u16()? as usize);
        if z != 0 || b.len() != 8 + 8 * (nv + ne + ns) {
            return invalid(format!("Copy blob of {:#x} bytes for {nv}+{ne}+{ns} pairs", b.len()));
        }
        Ok(Self { values: ref_pairs(&mut r, nv)?, events: ref_pairs(&mut r, ne)?, signals: ref_pairs(&mut r, ns)? })
    }
}

#[derive(Clone, Debug)]
pub struct BoolTrigger {
    pub unk_0: u32,
    pub output: Ref,
    pub input: Ref,
    pub min_retrigger_period: Ref,
    pub max_trigger_count: Ref,
    pub on_true: Ref,
    pub on_false: Ref,
    pub while_true: Ref,
    pub while_false: Ref,
}

impl BoolTrigger {
    pub(super) fn parse(mut r: Reader) -> Result<Self> {
        let unk_0 = r.u32()?;
        let [output, input, min_retrigger_period, max_trigger_count, on_true, on_false, while_true, while_false] =
            refs::<8>(&mut r)?;
        Ok(Self {
            unk_0,
            output,
            input,
            min_retrigger_period,
            max_trigger_count,
            on_true,
            on_false,
            while_true,
            while_false,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Envelope {
    pub unk_0: u16,
    pub voices: u16,
    pub start: Ref,
    pub stop: Ref,
    pub start_time: Ref,
    pub fade_out_time: Ref,
    pub attack: Ref,
    pub decay: Ref,
    pub sustain: Ref,
    pub release: Ref,
    pub infinite_sustain: Ref,
    pub attack_level: Ref,
    pub sustain_level: Ref,
    pub release_level: Ref,
    pub output: Ref,
}

impl Envelope {
    pub(super) fn parse(mut r: Reader) -> Result<Self> {
        let (unk_0, voices) = (r.u16()?, r.u16()?);
        let [start, stop, start_time, fade_out_time, attack, decay, sustain, release, infinite_sustain] =
            refs::<9>(&mut r)?;
        let [attack_level, sustain_level, release_level, output] = refs::<4>(&mut r)?;
        if voices == 0 {
            return invalid("envelope with 0 voices");
        }
        Ok(Self {
            unk_0,
            voices,
            start,
            stop,
            start_time,
            fade_out_time,
            attack,
            decay,
            sustain,
            release,
            infinite_sustain,
            attack_level,
            sustain_level,
            release_level,
            output,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Metronome {
    pub unk_0: u32,
    pub start: Ref,
    pub stop: Ref,
    pub click: Ref,
    pub bpm: Ref,
    pub beat_time: Ref,
    pub clicks_per_beat: Ref,
    pub click_time: Ref,
}

impl Metronome {
    pub(super) fn parse(mut r: Reader) -> Result<Self> {
        let unk_0 = r.u32()?;
        let [start, stop, click, bpm, beat_time, clicks_per_beat, click_time] = refs::<7>(&mut r)?;
        Ok(Self { unk_0, start, stop, click, bpm, beat_time, clicks_per_beat, click_time })
    }
}

#[derive(Clone, Debug)]
pub struct RangeTrigger {
    pub unk_0: u32,
    pub unk_mode: u32,
    pub output: Ref,
    pub input: Ref,
    pub max: Ref,
    pub min: Ref,
    pub includes_max: Ref,
    pub includes_min: Ref,
    pub min_retrigger_period: Ref,
    pub max_trigger_count: Ref,
    pub on_enter_from_above: Ref,
    pub on_enter_from_below: Ref,
    pub on_exit_to_above: Ref,
    pub on_exit_to_below: Ref,
    pub while_in_range: Ref,
    pub while_not_in_range: Ref,
}

impl RangeTrigger {
    pub(super) fn parse(mut r: Reader) -> Result<Self> {
        let (unk_0, unk_mode) = (r.u32()?, r.u32()?);
        let [output, input, max, min, includes_max, includes_min, min_retrigger_period, max_trigger_count] =
            refs::<8>(&mut r)?;
        let [
            on_enter_from_above,
            on_enter_from_below,
            on_exit_to_above,
            on_exit_to_below,
            while_in_range,
            while_not_in_range,
        ] = refs::<6>(&mut r)?;
        Ok(Self {
            unk_0,
            unk_mode,
            output,
            input,
            max,
            min,
            includes_max,
            includes_min,
            min_retrigger_period,
            max_trigger_count,
            on_enter_from_above,
            on_enter_from_below,
            on_exit_to_above,
            on_exit_to_below,
            while_in_range,
            while_not_in_range,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Expression {
    pub max_stack: u16,
    pub program: Vec<Instr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instr {
    Ref(Ref),
    Const(u64),
    Op(u32),
    Func(u32),
}

pub const OPERATORS: [&str; 33] = [
    "not", "mul", "div", "mod", "add", "sub", "lt", "lt_str", "lt_int", "le", "le_str", "le_int", "gt", "gt_str",
    "gt_int", "ge", "ge_str", "ge_int", "eq", "eq_str", "eq_bool", "eq_int", "ne", "ne_str", "ne_bool", "ne_int",
    "and", "xor", "or", "set", "set_str", "set_bool", "set_int",
];

pub const FUNCTIONS: [&str; 37] = [
    "abs", "acos", "asin", "atan", "atan2", "ceil", "cos", "cosh", "exp", "floor", "log", "log10", "max", "min", "pow",
    "rand", "sign", "sin", "sinh", "sqrt", "tan", "tanh", "trunc", "str23", "str24", "str25", "str26", "str27",
    "str28", "str29", "str30", "str31", "str32", "str33", "str34", "str35", "str36",
];

impl Instr {
    pub fn arity(self) -> (usize, usize) {
        match self {
            Self::Ref(_) | Self::Const(_) => (0, 1),
            Self::Op(0) => (1, 1),
            Self::Op(_) => (2, 1),
            Self::Func(4 | 12..=15 | 23..=26 | 29) => (2, 1),
            Self::Func(28 | 30 | 33..=36) => (3, 1),
            Self::Func(_) => (1, 1),
        }
    }
}

impl Expression {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::be(b);
        let (z0, n, max_stack, z8, prog, stack) =
            (r.u32()?, r.u16()? as usize, r.u16()?, r.u32()?, r.u32()? as usize, r.u32()? as usize);
        if z0 != 0 || z8 != 0 || prog != 0x18 || stack != prog + 16 * n || b.len() != stack + 16 * max_stack as usize {
            return invalid(format!("Expression header {z0} {n} {max_stack} {z8} {prog:#x} {stack:#x}"));
        }
        if b[stack..].iter().any(|&x| x != 0) || r.u32()? != 0 {
            return invalid("Expression stack area / header word 0x14 not zero");
        }
        let mut program = Vec::with_capacity(n);
        let mut depth = 0usize;
        for i in 0..n {
            let kind = r.u8()?;
            let pad = r.bytes(7)?;
            let (a, lo) = (r.u32()?, r.u32()?);
            let ins = match kind {
                0 => Instr::Ref(Ref(a)),
                1 => Instr::Const((a as u64) << 32 | lo as u64),
                2 if a <= 32 => Instr::Op(a),
                3 if a <= 36 => Instr::Func(a),
                _ => return invalid(format!("instruction {i}: kind {kind} operand {a:#x}")),
            };
            if pad.iter().any(|&x| x != 0) || (kind != 1 && lo != 0) {
                return invalid(format!("instruction {i}: non-zero padding"));
            }
            let (pops, pushes) = ins.arity();
            if depth < pops {
                return invalid(format!("instruction {i} underflows the stack"));
            }
            depth = depth - pops + pushes;
            if depth > max_stack as usize {
                return invalid(format!("instruction {i} exceeds max_stack {max_stack}"));
            }
            program.push(ins);
        }
        Ok(Self { max_stack, program })
    }
}

#[derive(Clone, Debug)]
pub struct Gate {
    pub event: Ref,
    pub values: Vec<Ref>,
}

fn gate(r: &mut Reader, n: usize, with_counter: bool) -> Result<Gate> {
    let event = Ref(r.u32()?);
    if with_counter && r.u32()? != 0 {
        return invalid("event counter slot not zero");
    }
    let values = (0..n).map(|_| r.u32().map(Ref)).collect::<Result<_>>()?;
    Ok(Gate { event, values })
}

#[derive(Clone, Debug)]
pub struct Merge {
    pub use_priority: bool,
    pub prefer_latest: bool,
    pub output: Gate,
    pub inputs: Vec<(u16, Gate)>,
}

impl Merge {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::be(b);
        let (z0, prio, latest, z3) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let (ni, nv, o_out, o_in) = (r.u16()? as usize, r.u16()? as usize, r.u32()? as usize, r.u32()? as usize);
        if z0 != 0
            || z3 != 0
            || prio > 1
            || latest > 1
            || o_out != 0x10
            || o_in != o_out + 4 + 4 * nv
            || b.len() != o_in + ni * (12 + 4 * nv)
        {
            return invalid(format!("Merge header {z0} {prio} {latest} {z3} {ni} {nv} {o_out:#x} {o_in:#x}"));
        }
        let output = gate(&mut r, nv, false)?;
        let mut inputs = Vec::with_capacity(ni);
        for _ in 0..ni {
            let event = Ref(r.u32()?);
            let (last, priority, z) = (r.u32()?, r.u16()?, r.u16()?);
            if last != 0 || z != 0 {
                return invalid("Merge input padding not zero");
            }
            let values = (0..nv).map(|_| r.u32().map(Ref)).collect::<Result<_>>()?;
            inputs.push((priority, Gate { event, values }));
        }
        Ok(Self { use_priority: prio == 1, prefer_latest: latest == 1, output, inputs })
    }
}

#[derive(Clone, Debug)]
pub struct Route {
    pub selector: Ref,
    pub input: Gate,
    pub outputs: Vec<Gate>,
}

#[derive(Clone, Debug)]
pub struct Switch {
    pub selector: Ref,
    pub output: Gate,
    pub inputs: Vec<Gate>,
}

fn route_header(b: &[u8], one_counter: bool) -> Result<(Reader<'_>, usize, usize, Ref)> {
    let mut r = Reader::be(b);
    let (z0, n, nv, o_one, o_many, sel) =
        (r.u32()?, r.u16()? as usize, r.u16()? as usize, r.u32()? as usize, r.u32()? as usize, Ref(r.u32()?));
    let one = 4 * nv + if one_counter { 8 } else { 4 };
    let many = 4 * nv + if one_counter { 4 } else { 8 };
    if z0 != 0 || o_one != 0x14 || o_many != o_one + one || b.len() != o_many + n * many {
        return invalid(format!("Route/Switch header {z0} {n} {nv} {o_one:#x} {o_many:#x}"));
    }
    Ok((r, n, nv, sel))
}

impl Route {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let (mut r, n, nv, selector) = route_header(b, true)?;
        let input = gate(&mut r, nv, true)?;
        let outputs = (0..n).map(|_| gate(&mut r, nv, false)).collect::<Result<_>>()?;
        Ok(Self { selector, input, outputs })
    }
}

impl Switch {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let (mut r, n, nv, selector) = route_header(b, false)?;
        let output = gate(&mut r, nv, false)?;
        let inputs = (0..n).map(|_| gate(&mut r, nv, true)).collect::<Result<_>>()?;
        Ok(Self { selector, output, inputs })
    }
}
