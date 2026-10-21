use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum ConstSymbol {
    Literal([f32; 4]),
    Named { name: String, element: u32, count: u32 },
}

#[derive(Clone, Debug, Default)]
pub struct Symbols {
    pub floats: std::collections::BTreeMap<u32, ConstSymbol>,
    pub textures: std::collections::BTreeMap<u32, String>,
    pub bools: std::collections::BTreeMap<u32, String>,
}

fn fmt_f32(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e7 { format!("{v:.1}") } else { format!("{v:?}") }
}

fn lane_chars(sw: &[u8; 4], lanes: &[usize]) -> String {
    lanes.iter().map(|&l| COMPONENTS[sw[l] as usize]).collect()
}

fn mask_lanes(mask: u32) -> Vec<usize> {
    (0..4).filter(|l| mask & (1 << l) != 0).collect()
}

fn mask_text(mask: u32) -> String {
    (0..4).map(|l| if mask & (1 << l) != 0 { COMPONENTS[l] } else { '_' }).collect()
}

impl Symbols {
    fn reg_text(&self, op: &Operand) -> String {
        let rel = match op.addressing {
            Addressing::Absolute => "",
            Addressing::Loop => "+aL",
            Addressing::A0 => "+a0",
        };
        match op.file {
            RegFile::Temp if rel.is_empty() => format!("r{}", op.index),
            RegFile::Temp => format!("r[{}{rel}]", op.index),
            RegFile::Const => match self.floats.get(&op.index) {
                Some(ConstSymbol::Named { name, element, count }) if *count > 1 || !rel.is_empty() => {
                    format!("{name}[{element}{rel}]")
                }
                Some(ConstSymbol::Named { name, .. }) => name.clone(),
                _ if rel.is_empty() => format!("c{}", op.index),
                _ => format!("c[{}{rel}]", op.index),
            },
        }
    }

    pub fn operand_expr(&self, op: &Operand, lanes: &[usize]) -> String {
        if op.file == RegFile::Const
            && op.addressing == Addressing::Absolute
            && let Some(ConstSymbol::Literal(v)) = self.floats.get(&op.index)
        {
            let vals: Vec<f32> = lanes
                .iter()
                .map(|&l| {
                    let mut x = v[op.swizzle[l] as usize];
                    if op.abs {
                        x = x.abs();
                    }
                    if op.negate { -x } else { x }
                })
                .collect();
            return if vals.iter().all(|&x| x == vals[0]) {
                fmt_f32(vals[0])
            } else {
                format!("float{}({})", vals.len(), vals.iter().map(|&x| fmt_f32(x)).collect::<Vec<_>>().join(", "))
            };
        }
        let mut s = format!("{}.{}", self.reg_text(op), lane_chars(&op.swizzle, lanes));
        if op.abs {
            s = format!("abs({s})");
        }
        if op.negate {
            s = format!("-{s}");
        }
        s
    }

    fn operand_asm(&self, op: &Operand) -> String {
        let mut s = match op.file {
            RegFile::Temp => self.reg_text(op),
            RegFile::Const => match op.addressing {
                Addressing::Absolute => format!("c{}", op.index),
                Addressing::Loop => format!("c[{}+aL]", op.index),
                Addressing::A0 => format!("c[{}+a0]", op.index),
            },
        };
        if op.abs {
            s = format!("|{s}|");
        }
        if op.swizzle != [0, 1, 2, 3] {
            s.push('.');
            s += &lane_chars(&op.swizzle, &[0, 1, 2, 3]);
        }
        if op.negate { format!("-{s}") } else { s }
    }
}

impl Alu {
    fn dest_name(&self, shader_type: ShaderType, scalar: bool) -> String {
        if self.is_export() {
            let r = self.vector_dest();
            return match (shader_type, r) {
                (ShaderType::Vertex, 0..=15) => format!("o{r}"),
                (ShaderType::Vertex, 62) => "oPos".into(),
                (ShaderType::Vertex, 63) => "oPts".into(),
                (ShaderType::Pixel, 0..=3) => format!("oC{r}"),
                (ShaderType::Pixel, 61) => "oDepth".into(),
                (_, 32) => "eA".into(),
                (_, 33..=37) => format!("eM{}", r - 33),
                _ => format!("export{r}"),
            };
        }
        let (r, rel) = if scalar {
            (self.scalar_dest(), self.scalar_dest_relative())
        } else {
            (self.vector_dest(), self.vector_dest_relative())
        };
        if rel { format!("r[{r}+aL]") } else { format!("r{r}") }
    }

    fn vector_expr(&self, syms: &Symbols, lanes: &[usize]) -> String {
        let op = self.vector_opcode();
        let n = VECTOR_OPERAND_COUNTS[op as usize] as u32;
        let src: Vec<Operand> = (1..=n).map(|i| self.source(i)).collect();
        let e = |i: usize| syms.operand_expr(&src[i], lanes);
        let full = |i: usize, k: usize| syms.operand_expr(&src[i], &(0..k).collect::<Vec<_>>());
        match op {
            0 => format!("{} + {}", e(0), e(1)),
            1 => format!("{} * {}", e(0), e(1)),
            2 => format!("max({}, {})", e(0), e(1)),
            3 => format!("min({}, {})", e(0), e(1)),
            4 => format!("({} == {})", e(0), e(1)),
            5 => format!("({} > {})", e(0), e(1)),
            6 => format!("({} >= {})", e(0), e(1)),
            7 => format!("({} != {})", e(0), e(1)),
            8 => format!("frac({})", e(0)),
            9 => format!("trunc({})", e(0)),
            10 => format!("floor({})", e(0)),
            11 => format!("{} * {} + {}", e(0), e(1), e(2)),
            12 => format!("({} == 0) ? {} : {}", e(0), e(1), e(2)),
            13 => format!("({} >= 0) ? {} : {}", e(0), e(1), e(2)),
            14 => format!("({} > 0) ? {} : {}", e(0), e(1), e(2)),
            15 => format!("dot({}, {})", full(0, 4), full(1, 4)),
            16 => format!("dot({}, {})", full(0, 3), full(1, 3)),
            17 => format!("dot({}, {}) + {}", full(0, 2), full(1, 2), full(2, 1)),
            18 => format!("cube({}, {})", full(0, 4), full(1, 4)),
            19 => format!("max4({})", full(0, 4)),
            20..=23 => format!("{}({}, {})", VECTOR_OPCODE_NAMES[op as usize].unwrap_or("?"), full(0, 4), full(1, 4)),
            24..=27 => {
                let cmp = ["==", ">", ">=", "!="][op as usize - 24];
                format!("kill if any({} {cmp} {})", full(0, 4), full(1, 4))
            }
            28 => format!("dst({}, {})", full(0, 4), full(1, 4)),
            29 => format!("max({}, {}) [a0 = {}]", e(0), e(1), syms.operand_expr(&src[0], &[3])),
            _ => format!("vector_opcode_{op}"),
        }
    }

    fn scalar_expr(&self, syms: &Symbols) -> String {
        let op = self.scalar_opcode();
        let ops = self.scalar_operands();
        let a = || syms.operand_expr(&ops[0], &[0]);
        let b = || syms.operand_expr(&ops[1], &[0]);
        match op {
            0 => format!("{} + {}", a(), b()),
            1 => format!("{} + ps", a()),
            2 => format!("{} * {}", a(), b()),
            3 => format!("{} * ps", a()),
            4 => format!("lit_mul({}, ps, {})", a(), b()),
            5 => format!("max({}, {})", a(), b()),
            6 => format!("min({}, {})", a(), b()),
            7 => format!("({} == 0)", a()),
            8 => format!("({} > 0)", a()),
            9 => format!("({} >= 0)", a()),
            10 => format!("({} != 0)", a()),
            11 => format!("frac({})", a()),
            12 => format!("trunc({})", a()),
            13 => format!("floor({})", a()),
            14 => format!("exp2({})", a()),
            15 => format!("log2_clamped({})", a()),
            16 => format!("log2({})", a()),
            17..=19 => format!("1 / {}", a()),
            20..=22 => format!("rsqrt({})", a()),
            23 | 24 => format!("max({}, {}) [a0 = {}]", a(), b(), a()),
            25 => format!("{} - {}", a(), b()),
            26 => format!("{} - ps", a()),
            27 => format!("p0 = ({} == 0)", a()),
            28 => format!("p0 = ({} != 0)", a()),
            29 => format!("p0 = ({} > 0)", a()),
            30 => format!("p0 = ({} >= 0)", a()),
            31 => format!("setp_inv({})", a()),
            32 => format!("setp_pop({})", a()),
            33 => "setp_clr".into(),
            34 => format!("setp_rstr({})", a()),
            35..=39 => {
                let cmp = ["== 0", "> 0", ">= 0", "!= 0", "== 1"][op as usize - 35];
                format!("kill if ({} {cmp})", a())
            }
            40 => format!("sqrt({})", a()),
            42 | 43 => format!("{} * {}", a(), b()),
            44 | 45 => format!("{} + {}", a(), b()),
            46 | 47 => format!("{} - {}", a(), b()),
            48 => format!("sin({})", a()),
            49 => format!("cos({})", a()),
            50 => "ps".into(),
            _ => format!("scalar_opcode_{op}"),
        }
    }

    pub fn disassemble(&self, shader_type: ShaderType, syms: &Symbols) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let pred = match self.predicated() {
            Some(true) => "(p0) ",
            Some(false) => "(!p0) ",
            None => "",
        };
        let (vmask, smask, zero, one) = self.result_masks();
        let vop = self.vector_opcode();
        let vname = VECTOR_OPCODE_NAMES[vop as usize].unwrap_or("vector_op?");
        let vector_changes_state = matches!(vop, 20..=27 | 29);
        if vmask != 0 || zero != 0 || one != 0 || vector_changes_state {
            let n = VECTOR_OPERAND_COUNTS[vop as usize] as u32;
            let dest = self.dest_name(shader_type, false);
            let srcs: Vec<String> = (1..=n).map(|i| syms.operand_asm(&self.source(i))).collect();
            let sat = if self.vector_clamp() { "_sat" } else { "" };
            let asm = format!("{pred}{vname}{sat} {dest}.{}, {}", mask_text(vmask | zero | one), srcs.join(", "));
            let mut expr = String::new();
            if vmask != 0 || vector_changes_state {
                let lanes = mask_lanes(vmask);
                let mut v = self.vector_expr(syms, if lanes.is_empty() { &[0, 1, 2, 3] } else { &lanes });
                if self.vector_clamp() {
                    v = format!("saturate({v})");
                }
                expr = if lanes.is_empty() { v } else { format!("{dest}.{} = {v}", lane_chars(&[0, 1, 2, 3], &lanes)) };
            }
            for (m, val) in [(zero, "0"), (one, "1")] {
                if m != 0 {
                    expr += &format!(
                        "{}{dest}.{} = {val}",
                        if expr.is_empty() { "" } else { "; " },
                        lane_chars(&[0, 1, 2, 3], &mask_lanes(m))
                    );
                }
            }
            out.push((asm, expr));
        }
        let sop = self.scalar_opcode();
        if sop != SCALAR_RETAIN_PREV || smask != 0 {
            let name = SCALAR_OPCODES.get(sop as usize).map_or("scalar_op?", |e| e.0);
            let dest = self.dest_name(shader_type, true);
            let srcs: Vec<String> = self
                .scalar_operands()
                .iter()
                .map(|o| {
                    let mut s = syms.operand_asm(&Operand { swizzle: [0, 1, 2, 3], negate: false, ..*o });
                    s.push('.');
                    s.push(COMPONENTS[o.swizzle[0] as usize]);
                    if o.negate { format!("-{s}") } else { s }
                })
                .collect();
            let sat = if self.scalar_clamp() { "_sat" } else { "" };
            let asm = format!("{pred}{name}{sat} {dest}.{}, {}", mask_text(smask), srcs.join(", "));
            let mut v = self.scalar_expr(syms);
            if self.scalar_clamp() {
                v = format!("saturate({v})");
            }
            let expr = if smask != 0 {
                format!("{dest}.{} = ps = {v}", lane_chars(&[0, 1, 2, 3], &mask_lanes(smask)))
            } else {
                format!("ps = {v}")
            };
            out.push((asm, expr));
        }
        out
    }
}

fn cond_text(c: &CfCondition, syms: &Symbols) -> String {
    match c {
        CfCondition::Always => String::new(),
        CfCondition::Bool { index, value } => {
            let name = syms.bools.get(index).map(|n| format!(" ({n})")).unwrap_or_default();
            format!(" {}b{index}{name}", if *value { "" } else { "!" })
        }
        CfCondition::Predicate { value } => format!(" {}p0", if *value { "" } else { "!" }),
    }
}

impl Program {
    pub fn disassemble(&self, syms: &Symbols) -> String {
        let mut out = String::new();
        let mut line = |left: String, right: &str| {
            if right.is_empty() {
                out += &format!("{left}\n");
            } else {
                out += &format!("{left:<64} ; {right}\n");
            }
        };
        let tex = |tf: u32| syms.textures.get(&tf).cloned();
        for (ci, c) in self.cf.iter().enumerate() {
            let kind = c.decode();
            let name = CF_OPCODE_NAMES[c.opcode() as usize];
            match kind {
                CfKind::Nop => line(format!("{ci:>3}  cnop"), ""),
                CfKind::Exec { address, count, sequence, condition, predicate_clean, .. } => {
                    let clean = if predicate_clean { "" } else { "  // PredicateClean=false" };
                    line(format!("{ci:>3}  {name}{}{clean}", cond_text(&condition, syms)), "");
                    for k in 0..count {
                        let slot = address + k;
                        let fetch = (sequence >> (2 * k)) & 1 == 1;
                        let serialize = if (sequence >> (2 * k + 1)) & 1 == 1 { "s" } else { " " };
                        match self.slot(slot, fetch) {
                            Slot::Fetch(f) => line(format!("    {slot:>3}{serialize}  {}", f.disassemble(&tex)), ""),
                            Slot::Alu(a) => {
                                for (j, (asm, expr)) in a.disassemble(self.shader_type, syms).into_iter().enumerate() {
                                    let lead = if j == 0 {
                                        format!("    {slot:>3}{serialize}  ")
                                    } else {
                                        "         + ".into()
                                    };
                                    line(format!("{lead}{asm}"), &expr);
                                }
                            }
                        }
                    }
                }
                CfKind::LoopStart { skip_address, loop_const, repeat } => line(
                    format!("{ci:>3}  loop i{loop_const}, L{skip_address}{}", if repeat { ", Repeat" } else { "" }),
                    "",
                ),
                CfKind::LoopEnd { body_address, loop_const, predicated_break } => {
                    let p = match predicated_break {
                        Some(true) => " (p0 break)",
                        Some(false) => " (!p0 break)",
                        None => "",
                    };
                    line(format!("{ci:>3}  endloop i{loop_const}, L{body_address}{p}"), "")
                }
                CfKind::Call { address, condition } => {
                    line(format!("{ci:>3}  call{} L{address}", cond_text(&condition, syms)), "")
                }
                CfKind::Return => line(format!("{ci:>3}  ret"), ""),
                CfKind::Jump { address, condition } => {
                    line(format!("{ci:>3}  jmp{} L{address}", cond_text(&condition, syms)), "")
                }
                CfKind::Alloc { kind, size } => {
                    let what = match (kind, self.shader_type) {
                        (1, _) => "position".to_string(),
                        (2, ShaderType::Vertex) => "interpolators".to_string(),
                        (2, ShaderType::Pixel) => "colors".to_string(),
                        (3, _) => format!("export = {size}"),
                        _ => format!("type{kind}"),
                    };
                    line(format!("{ci:>3}  alloc {what}"), "")
                }
                CfKind::MarkVsFetchDone => line(format!("{ci:>3}  vfetch_done"), ""),
            }
        }
        out
    }
}
