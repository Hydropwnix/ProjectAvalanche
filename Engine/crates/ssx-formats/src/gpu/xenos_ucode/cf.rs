use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlFlow {
    pub lo: u32,
    pub hi: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfCondition {
    Always,
    Bool { index: u32, value: bool },
    Predicate { value: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfKind {
    Nop,
    Exec { address: u32, count: u32, sequence: u32, end: bool, condition: CfCondition, predicate_clean: bool },
    LoopStart { skip_address: u32, loop_const: u32, repeat: bool },
    LoopEnd { body_address: u32, loop_const: u32, predicated_break: Option<bool> },
    Call { address: u32, condition: CfCondition },
    Return,
    Jump { address: u32, condition: CfCondition },
    Alloc { kind: u32, size: u32 },
    MarkVsFetchDone,
}

impl ControlFlow {
    pub fn opcode(&self) -> u32 {
        bits(self.hi as u32, 12, 4)
    }

    pub fn decode(&self) -> CfKind {
        let (lo, hi) = (self.lo, self.hi as u32);
        let op = self.opcode();
        match op {
            0 => CfKind::Nop,
            1..=6 | 13 | 14 => {
                let condition = match op {
                    3 | 4 | 13 | 14 => CfCondition::Bool { index: bits(hi, 2, 8), value: bits(hi, 10, 1) == 1 },
                    5 | 6 => CfCondition::Predicate { value: bits(hi, 10, 1) == 1 },
                    _ => CfCondition::Always,
                };
                let predicate_clean = match op {
                    1 | 2 | 5 | 6 => bits(hi, 9, 1) == 1,
                    13 | 14 => true,
                    _ => false,
                };
                CfKind::Exec {
                    address: bits(lo, 0, 12),
                    count: bits(lo, 12, 3),
                    sequence: bits(lo, 16, 12),
                    end: matches!(op, 2 | 4 | 6 | 14),
                    condition,
                    predicate_clean,
                }
            }
            7 => CfKind::LoopStart {
                skip_address: bits(lo, 0, 13),
                loop_const: bits(lo, 16, 5),
                repeat: bits(lo, 13, 1) == 1,
            },
            8 => CfKind::LoopEnd {
                body_address: bits(lo, 0, 13),
                loop_const: bits(lo, 16, 5),
                predicated_break: (bits(lo, 21, 1) == 1).then_some(bits(hi, 10, 1) == 1),
            },
            9 | 11 => {
                let condition = if bits(lo, 13, 1) == 1 {
                    CfCondition::Always
                } else if bits(lo, 14, 1) == 1 {
                    CfCondition::Predicate { value: bits(hi, 10, 1) == 1 }
                } else {
                    CfCondition::Bool { index: bits(hi, 2, 8), value: bits(hi, 10, 1) == 1 }
                };
                let address = bits(lo, 0, 13);
                if op == 9 { CfKind::Call { address, condition } } else { CfKind::Jump { address, condition } }
            }
            10 => CfKind::Return,
            12 => CfKind::Alloc { kind: bits(hi, 9, 2), size: bits(lo, 0, 3) },
            _ => CfKind::MarkVsFetchDone,
        }
    }
}

pub fn unpack_cf_pair(d: [u32; 3]) -> [ControlFlow; 2] {
    [
        ControlFlow { lo: d[0], hi: (d[1] & 0xFFFF) as u16 },
        ControlFlow { lo: (d[1] >> 16) | (d[2] << 16), hi: (d[2] >> 16) as u16 },
    ]
}
