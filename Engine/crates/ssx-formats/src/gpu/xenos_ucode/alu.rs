use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addressing {
    Absolute,
    Loop,
    A0,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegFile {
    Temp,
    Const,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operand {
    pub file: RegFile,
    pub index: u32,
    pub addressing: Addressing,
    pub abs: bool,
    pub negate: bool,
    pub swizzle: [u8; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alu(pub [u32; 3]);

impl Alu {
    pub fn vector_dest(&self) -> u32 {
        bits(self.0[0], 0, 6)
    }
    pub fn vector_dest_relative(&self) -> bool {
        bits(self.0[0], 6, 1) == 1
    }
    pub fn abs_constants(&self) -> bool {
        bits(self.0[0], 7, 1) == 1
    }
    pub fn scalar_dest(&self) -> u32 {
        bits(self.0[0], 8, 6)
    }
    pub fn scalar_dest_relative(&self) -> bool {
        bits(self.0[0], 14, 1) == 1
    }
    pub fn is_export(&self) -> bool {
        bits(self.0[0], 15, 1) == 1
    }
    pub fn vector_mask(&self) -> u32 {
        bits(self.0[0], 16, 4)
    }
    pub fn scalar_mask(&self) -> u32 {
        bits(self.0[0], 20, 4)
    }
    pub fn vector_clamp(&self) -> bool {
        bits(self.0[0], 24, 1) == 1
    }
    pub fn scalar_clamp(&self) -> bool {
        bits(self.0[0], 25, 1) == 1
    }
    pub fn scalar_opcode(&self) -> u32 {
        bits(self.0[0], 26, 6)
    }
    pub fn vector_opcode(&self) -> u32 {
        bits(self.0[2], 24, 5)
    }
    pub fn predicated(&self) -> Option<bool> {
        (bits(self.0[1], 28, 1) == 1).then_some(bits(self.0[1], 27, 1) == 1)
    }
    fn src_swizzle_raw(&self, i: u32) -> u32 {
        bits(self.0[1], 8 * (3 - i), 8)
    }
    fn src_negate(&self, i: u32) -> bool {
        bits(self.0[1], 27 - i, 1) == 1
    }
    fn src_reg_raw(&self, i: u32) -> u32 {
        bits(self.0[2], 8 * (3 - i), 8)
    }
    fn src_is_temp(&self, i: u32) -> bool {
        bits(self.0[2], 32 - i, 1) == 1
    }
    fn src_const_addressed(&self, i: u32) -> bool {
        let c0 = bits(self.0[1], 31, 1) == 1;
        let c1 = bits(self.0[1], 30, 1) == 1;
        match i {
            1 => c0,
            2 => {
                if self.src_is_temp(1) {
                    c0
                } else {
                    c1
                }
            }
            _ => {
                if self.src_is_temp(1) && self.src_is_temp(2) {
                    c0
                } else {
                    c1
                }
            }
        }
    }
    fn const_addressing(&self, i: u32) -> Addressing {
        if !self.src_const_addressed(i) {
            Addressing::Absolute
        } else if bits(self.0[1], 29, 1) == 1 {
            Addressing::A0
        } else {
            Addressing::Loop
        }
    }

    pub fn source(&self, i: u32) -> Operand {
        let reg = self.src_reg_raw(i);
        let sw = self.src_swizzle_raw(i);
        let swizzle = std::array::from_fn(|c| (((sw >> (2 * c)) + c as u32) & 3) as u8);
        if self.src_is_temp(i) {
            Operand {
                file: RegFile::Temp,
                index: reg & 0x3F,
                addressing: if reg & 0x40 != 0 { Addressing::Loop } else { Addressing::Absolute },
                abs: reg & 0x80 != 0,
                negate: self.src_negate(i),
                swizzle,
            }
        } else {
            Operand {
                file: RegFile::Const,
                index: reg,
                addressing: self.const_addressing(i),
                abs: self.abs_constants(),
                negate: self.src_negate(i),
                swizzle,
            }
        }
    }

    pub fn scalar_operands(&self) -> Vec<Operand> {
        let op = self.scalar_opcode();
        let form = SCALAR_OPCODES.get(op as usize).map_or(ScalarOperands::None, |e| e.1);
        let s3 = self.source(3);
        let one = |o: &Operand, lane: usize| Operand { swizzle: [o.swizzle[lane]; 4], ..*o };
        match form {
            ScalarOperands::None => vec![],
            ScalarOperands::A => vec![one(&s3, 3)],
            ScalarOperands::AB => {
                let b_lane = if VECTOR_OPERAND_COUNTS[self.vector_opcode() as usize] == 3 { 2 } else { 0 };
                vec![one(&s3, 3), one(&s3, b_lane)]
            }
            ScalarOperands::ConstTemp => {
                let sw = self.src_swizzle_raw(3);
                let comp = |lane: u32| (((sw >> (2 * lane)) + lane) & 3) as u8;
                let constant = Operand {
                    file: RegFile::Const,
                    index: self.src_reg_raw(3),
                    addressing: self.const_addressing(3),
                    abs: self.abs_constants(),
                    negate: self.src_negate(3),
                    swizzle: [comp(3); 4],
                };
                let temp_index = (op & 1) | ((bits(self.0[2], 29, 1)) << 1) | (sw & 0x3C);
                let temp = Operand {
                    file: RegFile::Temp,
                    index: temp_index,
                    addressing: Addressing::Absolute,
                    abs: self.abs_constants(),
                    negate: self.src_negate(3),
                    swizzle: [comp(0); 4],
                };
                vec![constant, temp]
            }
        }
    }

    pub fn result_masks(&self) -> (u32, u32, u32, u32) {
        let (v, s) = (self.vector_mask(), self.scalar_mask());
        if !self.is_export() {
            return (v, s, 0, 0);
        }
        let zero = if self.scalar_dest_relative() { 0b1111 & !(v | s) } else { 0 };
        (v & !s, s & !v, zero, v & s)
    }
}
