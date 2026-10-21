mod alu;
mod cf;
mod disasm;
mod fetch;

use crate::{Error, Result};

pub use alu::*;
pub use cf::*;
pub use disasm::*;
pub use fetch::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderType {
    Vertex,
    Pixel,
}

pub const CF_OPCODE_NAMES: [&str; 16] = [
    "cnop",
    "exec",
    "exece",
    "cexec",
    "cexece",
    "(p)exec",
    "(p)exece",
    "loop",
    "endloop",
    "ccall",
    "ret",
    "cjmp",
    "alloc",
    "cexec_pc",
    "cexece_pc",
    "vfetch_done",
];

pub const VECTOR_OPCODE_NAMES: [Option<&str>; 32] = [
    Some("add"),
    Some("mul"),
    Some("max"),
    Some("min"),
    Some("seq"),
    Some("sgt"),
    Some("sge"),
    Some("sne"),
    Some("frc"),
    Some("trunc"),
    Some("floor"),
    Some("mad"),
    Some("cndeq"),
    Some("cndge"),
    Some("cndgt"),
    Some("dp4"),
    Some("dp3"),
    Some("dp2add"),
    Some("cube"),
    Some("max4"),
    Some("setp_eq_push"),
    Some("setp_ne_push"),
    Some("setp_gt_push"),
    Some("setp_ge_push"),
    Some("kill_eq"),
    Some("kill_gt"),
    Some("kill_ge"),
    Some("kill_ne"),
    Some("dst"),
    Some("maxa"),
    None,
    None,
];

pub const VECTOR_OPERAND_COUNTS: [u8; 32] =
    [2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 3, 3, 3, 3, 2, 2, 3, 2, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarOperands {
    None,
    A,
    AB,
    ConstTemp,
}

pub const SCALAR_OPCODES: [(&str, ScalarOperands); 51] = {
    use ScalarOperands::*;
    [
        ("adds", AB),
        ("adds_prev", A),
        ("muls", AB),
        ("muls_prev", A),
        ("muls_prev2", AB),
        ("maxs", AB),
        ("mins", AB),
        ("seqs", A),
        ("sgts", A),
        ("sges", A),
        ("snes", A),
        ("frcs", A),
        ("truncs", A),
        ("floors", A),
        ("exp", A),
        ("logc", A),
        ("log", A),
        ("rcpc", A),
        ("rcpf", A),
        ("rcp", A),
        ("rsqc", A),
        ("rsqf", A),
        ("rsq", A),
        ("maxas", AB),
        ("maxasf", AB),
        ("subs", AB),
        ("subs_prev", A),
        ("setp_eq", A),
        ("setp_ne", A),
        ("setp_gt", A),
        ("setp_ge", A),
        ("setp_inv", A),
        ("setp_pop", A),
        ("setp_clr", None),
        ("setp_rstr", A),
        ("kills_eq", A),
        ("kills_gt", A),
        ("kills_ge", A),
        ("kills_ne", A),
        ("kills_one", A),
        ("sqrt", A),
        ("opcode_41", None),
        ("mulsc", ConstTemp),
        ("mulsc", ConstTemp),
        ("addsc", ConstTemp),
        ("addsc", ConstTemp),
        ("subsc", ConstTemp),
        ("subsc", ConstTemp),
        ("sin", A),
        ("cos", A),
        ("retain_prev", None),
    ]
};

pub const SCALAR_RETAIN_PREV: u32 = 50;

const COMPONENTS: [char; 4] = ['x', 'y', 'z', 'w'];

#[inline]
fn bits(v: u32, lo: u32, n: u32) -> u32 {
    (v >> lo) & ((1u32 << n) - 1)
}

#[inline]
fn sbits(v: u32, lo: u32, n: u32) -> i32 {
    let x = bits(v, lo, n) as i32;
    (x << (32 - n)) >> (32 - n)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Alu(Alu),
    Fetch(Fetch),
}

#[derive(Clone, Debug)]
pub struct Program {
    pub shader_type: ShaderType,
    pub cf: Vec<ControlFlow>,
    pub words: Vec<u32>,
}

impl Program {
    pub fn parse(words: &[u32], shader_type: ShaderType) -> Result<Self> {
        let slot_count = words.len() / 3;
        let mut bound = slot_count;
        let mut cf = Vec::new();
        let mut i = 0;
        while i < bound {
            let pair = unpack_cf_pair([words[3 * i], words[3 * i + 1], words[3 * i + 2]]);
            for c in pair {
                if let CfKind::Exec { address, count, .. } = c.decode() {
                    if (address + count) as usize > slot_count {
                        return Err(Error::invalid(format!(
                            "exec references slots {address}..{} beyond the {slot_count}-slot microcode",
                            address + count
                        )));
                    }
                    if count > 0 {
                        bound = bound.min(address as usize);
                    }
                }
                cf.push(c);
            }
            i += 1;
        }
        Ok(Self { shader_type, cf, words: words.to_vec() })
    }

    pub fn slot(&self, index: u32, fetch: bool) -> Slot {
        let i = index as usize * 3;
        let d = [self.words[i], self.words[i + 1], self.words[i + 2]];
        if fetch { Slot::Fetch(Fetch(d)) } else { Slot::Alu(Alu(d)) }
    }

    pub fn executed_slots(&self) -> Vec<(u32, Slot)> {
        let mut out = Vec::new();
        for c in &self.cf {
            if let CfKind::Exec { address, count, sequence, .. } = c.decode() {
                for k in 0..count {
                    out.push((address + k, self.slot(address + k, (sequence >> (2 * k)) & 1 == 1)));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_cf(a: (u32, u16), b: (u32, u16)) -> [u32; 3] {
        [a.0, (a.1 as u32) | (b.0 << 16), (b.0 >> 16) | ((b.1 as u32) << 16)]
    }

    #[test]
    fn cf_pair_round_trip() {
        let seq = 0b11_00_01u32;
        let exec = (2 | (3 << 12) | (seq << 16), (1u16 << 12) | (1 << 9));
        let alloc = (0u32, (12u16 << 12) | (2 << 9));
        let cf = unpack_cf_pair(pack_cf(exec, alloc));
        assert_eq!(
            cf[0].decode(),
            CfKind::Exec {
                address: 2,
                count: 3,
                sequence: seq,
                end: false,
                condition: CfCondition::Always,
                predicate_clean: true
            }
        );
        assert_eq!(cf[1].decode(), CfKind::Alloc { kind: 2, size: 0 });
        let cexece = (5 | (1 << 12), (4u16 << 12) | (7 << 2));
        let cf = unpack_cf_pair(pack_cf(cexece, (0, 0)));
        assert!(matches!(
            cf[0].decode(),
            CfKind::Exec { end: true, condition: CfCondition::Bool { index: 7, value: false }, .. }
        ));
        assert_eq!(cf[1].decode(), CfKind::Nop);
    }

    fn sample_alu() -> Alu {
        let rel = |abs: [u32; 4]| (0..4).fold(0, |acc, i| acc | (((abs[i] + 4 - i as u32) & 3) << (2 * i)));
        let d0 = 1 | (6 << 8) | (0b0011 << 16) | (0b0100 << 20) | (19 << 26);
        let d1 = rel([3, 3, 3, 3]) | (rel([1, 1, 1, 1]) << 8) | (rel([2, 2, 2, 2]) << 16) | (1 << 24);
        let d2 = 4 | (3 << 8) | (2 << 16) | (11 << 24) | (1 << 29) | (1 << 31);
        Alu([d0, d1, d2])
    }

    #[test]
    fn alu_operands_and_listing() {
        let a = sample_alu();
        assert_eq!(a.vector_opcode(), 11);
        assert_eq!(a.scalar_opcode(), 19);
        let s1 = a.source(1);
        assert_eq!((s1.file, s1.index, s1.swizzle, s1.negate), (RegFile::Temp, 2, [2, 2, 2, 2], false));
        let s2 = a.source(2);
        assert_eq!((s2.file, s2.index, s2.swizzle), (RegFile::Const, 3, [1, 1, 1, 1]));
        let s3 = a.source(3);
        assert_eq!((s3.file, s3.index, s3.swizzle, s3.negate), (RegFile::Temp, 4, [3, 3, 3, 3], true));

        let mut syms = Symbols::default();
        syms.floats.insert(3, ConstSymbol::Literal([0.0, 0.5, 0.0, 0.0]));
        let lines = a.disassemble(ShaderType::Pixel, &syms);
        assert_eq!(lines[0].0, "mad r1.xy__, r2.zzzz, c3.yyyy, -r4.wwww");
        assert_eq!(lines[0].1, "r1.xy = r2.zz * 0.5 + -r4.ww");
        assert_eq!(lines[1].1, "r6.z = ps = 1 / -r4.w");
    }

    #[test]
    fn texture_fetch_fields() {
        let dst_sw = (1 << 3) | (7 << 6) | (7 << 9);
        let d0 = 1 | (5 << 12) | (1 << 19) | (2 << 20) | ((1 << 2) << 26);
        let d1 = dst_sw | (3 << 12) | (3 << 14) | (3 << 16) | (7 << 18) | (3 << 24) | (3 << 26) | (1 << 28);
        let d2 = 1 << 14;
        let f = Fetch([d0, d1, d2]);
        assert!(!f.is_vertex());
        assert_eq!((f.dst_reg(), f.const_index(), f.tex_dimension()), (5, 2, 1));
        assert_eq!(f.disassemble(&|tf| (tf == 2).then(|| "s".to_string())), "tfetch2D r5.xy__, r0.xy, tf2 (s)");
    }
}
