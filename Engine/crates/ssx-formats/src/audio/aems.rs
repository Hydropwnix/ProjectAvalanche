use crate::reader::Reader;
use crate::{Error, Result};

pub const INSTANCE_HEADER: u32 = 0x18;
pub const NUM_OPS: u8 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Op {
    Stopped = 0,
    Params = 1,
    GlobalRead = 2,
    Start = 3,
    End = 4,
    FireEvent = 5,
    Stepper = 6,
    Random = 7,
    Shuffle = 8,
    WeightedRandom = 9,
    RangeTrigger = 10,
    Timer = 11,
    SelectOnTrigger = 12,
    AnyOf = 13,
    Envelope = 14,
    Curve = 15,
    Delay = 16,
    Switch = 17,
    Demux = 18,
    MinOf = 19,
    MaxOf = 20,
    ProductOf = 21,
    SumOf = 22,
    Sub = 23,
    Mul = 24,
    Div = 25,
    Mod = 26,
    Player = 27,
    Lfo = 28,
    Glide = 29,
    SumMax = 30,
    SubFloor = 31,
    MulCeiling = 32,
    Min = 33,
    Max = 34,
    Scale = 35,
    Add = 36,
    EventReceived = 37,
    CallFunction = 38,
    GlobalWrite = 39,
}

impl Op {
    pub fn from_u8(v: u8) -> Option<Self> {
        use Op::*;
        const ALL: [Op; 40] = [
            Stopped,
            Params,
            GlobalRead,
            Start,
            End,
            FireEvent,
            Stepper,
            Random,
            Shuffle,
            WeightedRandom,
            RangeTrigger,
            Timer,
            SelectOnTrigger,
            AnyOf,
            Envelope,
            Curve,
            Delay,
            Switch,
            Demux,
            MinOf,
            MaxOf,
            ProductOf,
            SumOf,
            Sub,
            Mul,
            Div,
            Mod,
            Player,
            Lfo,
            Glide,
            SumMax,
            SubFloor,
            MulCeiling,
            Min,
            Max,
            Scale,
            Add,
            EventReceived,
            CallFunction,
            GlobalWrite,
        ];
        ALL.get(v as usize).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wire {
    pub src: Option<i32>,
    pub dst: i32,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub offset: u32,
    pub op: Op,
    pub unk_2: u16,
    pub wires: Vec<Wire>,
    pub state: u32,
    pub state_size: u32,
}

pub fn parse_program(data: &[u8], offset: usize) -> Result<(Vec<Node>, usize)> {
    let mut r = Reader::be(data);
    r.seek(offset)?;
    let mut nodes = Vec::new();
    let mut state = 0u32;
    loop {
        let at = r.pos();
        let op = r.u8()?;
        if op == 0xFF {
            r.skip(3)?;
            return Ok((nodes, r.pos()));
        }
        let op = Op::from_u8(op).ok_or_else(|| Error::invalid(format!("AEMS node {at:#x}: operator {op}")))?;
        let n = r.u8()? as usize;
        let unk_2 = r.u16()?;
        let mut wires = Vec::with_capacity(n);
        for _ in 0..n {
            let src = r.i32()?;
            let dst = r.i32()?;
            wires.push(Wire { src: (src != -1).then_some(src), dst });
        }
        let size = r.i32()?;
        if size <= 0 || size % 4 != 0 {
            return Err(Error::invalid(format!("AEMS node {at:#x}: state size {size}")));
        }
        nodes.push(Node { offset: at as u32, op, unk_2, wires, state, state_size: size as u32 });
        state += size as u32;
    }
}
