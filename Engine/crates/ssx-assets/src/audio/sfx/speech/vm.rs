use ssx_formats::speech::gas::{Record, Script};

pub trait VmHost {
    fn game_var(&self, id: i32) -> i32;
    fn stat(&self, call: i16, id: u32) -> i32;
    fn this_context(&self) -> u32;
}

const THIS: u32 = 0xFEED_BABE;

struct Vm<'a> {
    stmts: &'a [ssx_formats::speech::gas::Statement],
    params: Vec<i32>,
    locals: Vec<i32>,
    host: &'a dyn VmHost,
    next: usize,
    failed: bool,
    result: i32,
}

pub fn run(script: &Script, args: &[i32], host: &dyn VmHost) -> i32 {
    let mut params = vec![0; script.params.len()];
    for (k, &(slot, _)) in script.params.iter().enumerate() {
        if let (Some(p), Some(&a)) = (params.get_mut(slot as usize), args.get(k)) {
            *p = a;
        }
    }
    let mut vm = Vm {
        stmts: &script.statements,
        params,
        locals: vec![0; script.locals.len()],
        host,
        next: 0,
        failed: false,
        result: 0,
    };
    for i in 0..vm.stmts.len() {
        if i != vm.next {
            continue;
        }
        let head = vm.stmts[i].records[0];
        if head.a == 0 {
            vm.step(i);
        } else if head.a != 5 {
            vm.failed = false;
            vm.next += 1;
        }
    }
    vm.result
}

impl Vm<'_> {
    fn rec(&self, s: usize, k: usize) -> Record {
        self.stmts[s].records.get(k).copied().unwrap_or(Record { a: -1, op: -1, kind: -1, index: -1, value: -1 })
    }

    fn step(&mut self, s: usize) {
        let head = self.rec(s, 0);
        match head.op {
            5 => {
                self.expression(s);
                self.failed = false;
                self.next += 1;
            }
            10 => {
                let ok = self.condition(s, 1);
                self.next = if ok { self.next + 1 } else { head.kind as usize };
                self.failed = !ok;
            }
            11 => {
                if self.failed {
                    let ok = self.condition(s, 1);
                    self.next = if ok { self.next + 1 } else { head.kind as usize };
                    self.failed = !ok;
                } else {
                    self.next = head.kind as usize;
                    self.failed = false;
                }
            }
            12 => {
                self.next = if self.failed { self.next + 1 } else { head.kind as usize };
                self.failed = false;
            }
            0x28 => {
                self.result = self.operand(s, 1);
                self.next = self.stmts.len();
                self.failed = false;
            }
            _ => {
                self.next += 1;
                self.failed = false;
            }
        }
    }

    fn operand(&self, s: usize, k: usize) -> i32 {
        let r = self.rec(s, k);
        match r.kind {
            25 => {
                let id = ((r.value as u32 & 0xFFFF) << 16) | (r.index as u16 as u32);
                match r.op {
                    0x1F6..=0x1F8 => self.host.stat(r.op, if id == THIS { self.host.this_context() } else { id }),
                    0x200..=0x202 | 0x20A..=0x20C => self.host.stat(r.op, id),
                    _ => r.value,
                }
            }
            10 => self.params.get(r.index as usize).copied().unwrap_or(0),
            15 => self.locals.get(r.index as usize).copied().unwrap_or(0),
            _ => r.value,
        }
    }

    fn write(&mut self, s: usize, k: usize, v: i32) {
        let r = self.rec(s, k);
        let slot = match r.kind {
            10 => self.params.get_mut(r.index as usize),
            15 => self.locals.get_mut(r.index as usize),
            _ => None,
        };
        if let Some(x) = slot {
            *x = v;
        }
    }

    fn expression(&mut self, s: usize) {
        match self.rec(s, 2).op {
            0xCA => {
                let v = self.host.game_var(i32::from(self.rec(s, 3).op) - 1000);
                self.write(s, 1, v);
            }
            0xCB => self.write(s, 1, 0),
            op => {
                let dest = self.operand(s, 1);
                let rhs = self.arith(s, 3);
                let v = match op {
                    0xC9 => rhs,
                    0xCD | 0xD7 => dest.wrapping_add(rhs),
                    0xCE | 0xD8 => dest.wrapping_sub(rhs),
                    0xCF => dest.wrapping_mul(rhs),
                    0xD0 if rhs != 0 => dest.wrapping_div(rhs),
                    0xDC => dest.max(rhs),
                    0xDD => dest.min(rhs),
                    _ => dest,
                };
                self.write(s, 1, v);
            }
        }
    }

    fn arith(&self, s: usize, k: usize) -> i32 {
        let op = self.rec(s, k + 1).op;
        if op == 0xFB {
            return self.operand(s, k);
        }
        let (a, b) = (self.operand(s, k), self.operand(s, k + 2));
        match op {
            0xFF => a.wrapping_add(b),
            0x100 => a.wrapping_sub(b),
            0x101 => a.wrapping_mul(b),
            0x102 if b != 0 => a.wrapping_div(b),
            _ => 0,
        }
    }

    fn condition(&self, s: usize, k: usize) -> bool {
        let op = self.rec(s, k + 1).op;
        let v = self.operand(s, k);
        match op {
            0x136 => v < 0,
            0x137 => v <= 0,
            0x138 => v == 0,
            0x139 => v != 0,
            0x13A => v >= 0,
            0x13B => v > 0,
            0x14B..=0x151 => {
                let r = self.operand(s, k + 2);
                match op {
                    0x14B => v < r,
                    0x14C => v <= r,
                    0x14D => v == r,
                    0x14E => v != r,
                    0x14F => v >= r,
                    0x150 => v > r,
                    _ if v == 0 => r == 0,
                    _ => v & r != 0,
                }
            }
            0x155 | 0x156 => {
                let (a, b) = (self.operand(s, k + 2), self.operand(s, k + 3));
                let (lo, hi) = if b < a { (b, a) } else { (a, b) };
                let inside = lo <= v && v <= hi;
                if op == 0x155 { inside } else { !inside }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_formats::speech::gas::Statement;

    struct H(i32);
    impl VmHost for H {
        fn game_var(&self, id: i32) -> i32 {
            if id == 102 { self.0 } else { -1 }
        }
        fn stat(&self, _: i16, _: u32) -> i32 {
            i32::MAX
        }
        fn this_context(&self) -> u32 {
            7
        }
    }

    fn r(a: i16, op: i16, kind: i16, index: i16, value: i32) -> Record {
        Record { a, op, kind, index, value }
    }

    fn voice_free() -> Script {
        let st = |records: Vec<Record>| Statement { records };
        Script {
            id: 178,
            unk_04: 0,
            params: Vec::new(),
            locals: vec![(0, 0)],
            unk_10: 0,
            statements: vec![
                st(vec![r(-1, 5, -1, -1, -1)]),
                st(vec![r(-1, 7, -1, -1, -1), r(-1, -1, -1, -1, -1)]),
                st(vec![r(0, 5, -1, -1, 0), r(-1, 401, 15, 0, 0), r(-1, 202, 0, 0, 0), r(-1, 1102, 0, 0, 0)]),
                st(vec![r(0, 10, 5, -1, 0), r(-1, 400, 15, 0, 0), r(-1, 333, 2, -1, -1), r(-1, 400, 0, 0, 0)]),
                st(vec![r(0, 40, -1, -1, 0), r(-1, 418, 25, 0, 1)]),
                st(vec![r(0, 13, -1, -1, 0)]),
                st(vec![r(0, 40, -1, -1, 0), r(-1, 418, 25, 0, 0)]),
            ],
        }
    }

    #[test]
    fn a_condition_script_reads_its_game_variable() {
        assert_eq!(run(&voice_free(), &[], &H(0)), 1);
        assert_eq!(run(&voice_free(), &[], &H(2500)), 0);
    }

    #[test]
    fn between_swaps_its_bounds() {
        let mut s = voice_free();
        s.statements[3] = Statement {
            records: vec![
                r(0, 10, 5, -1, 0),
                r(-1, 400, 15, 0, 0),
                r(-1, 341, 3, -1, -1),
                r(-1, 400, 0, 0, 70),
                r(-1, 400, 0, 0, 20),
            ],
        };
        assert_eq!(run(&s, &[], &H(45)), 1);
        assert_eq!(run(&s, &[], &H(71)), 0);
    }
}
