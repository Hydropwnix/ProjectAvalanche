use super::*;

#[derive(Clone, Copy, Debug)]
enum Entry {
    Ref(Ref),
    Raw(u64),
}

impl Remix {
    fn pop_f64(&self, i: usize, st: &mut Vec<Entry>) -> f64 {
        match st.pop() {
            Some(Entry::Ref(r)) => self.f64(i, r),
            Some(Entry::Raw(x)) => f64::from_bits(x),
            None => 0.0,
        }
    }

    fn pop_bool(&self, i: usize, st: &mut Vec<Entry>) -> bool {
        match st.pop() {
            Some(Entry::Ref(r)) => as_bool(self.get(i, r)),
            Some(Entry::Raw(x)) => (x >> 32) as u32 != 0,
            None => false,
        }
    }

    fn pop_i32(&self, i: usize, st: &mut Vec<Entry>) -> i32 {
        match st.pop() {
            Some(Entry::Ref(r)) => as_i32(self.get(i, r)),
            Some(Entry::Raw(x)) => (x >> 32) as i32,
            None => 0,
        }
    }

    pub(super) fn expression(&mut self, i: usize, program: &[Instr]) {
        let mut st: Vec<Entry> = Vec::with_capacity(8);
        let f = |x: f64| Entry::Raw(x.to_bits());
        let b = |x: bool| Entry::Raw(u64::from(x) << 32);
        for ins in program {
            match *ins {
                Instr::Ref(r) => st.push(Entry::Ref(r)),
                Instr::Const(c) => st.push(Entry::Raw(c)),
                Instr::Op(0) => {
                    let x = self.pop_bool(i, &mut st);
                    st.push(b(!x));
                }
                Instr::Op(op @ 1..=6 | op @ 9 | op @ 12 | op @ 15 | op @ 18 | op @ 22) => {
                    let y = self.pop_f64(i, &mut st);
                    let x = self.pop_f64(i, &mut st);
                    st.push(match op {
                        1 => f(x * y),
                        2 => f(x / y),
                        3 => f(x - (x / y).floor()),
                        4 => f(x + y),
                        5 => f(x - y),
                        6 => b(x < y),
                        9 => b(x <= y),
                        12 => b(y < x),
                        15 => b(y <= x),
                        18 => b(x == y),
                        _ => b(x != y),
                    });
                }
                Instr::Op(op @ (8 | 11 | 14 | 17 | 21 | 25)) => {
                    let y = self.pop_i32(i, &mut st);
                    let x = self.pop_i32(i, &mut st);
                    st.push(b(match op {
                        8 => x < y,
                        11 => x <= y,
                        14 => x > y,
                        17 => x >= y,
                        21 => x == y,
                        _ => x != y,
                    }));
                }
                Instr::Op(op @ (20 | 24 | 26 | 27 | 28)) => {
                    let y = self.pop_bool(i, &mut st);
                    let x = self.pop_bool(i, &mut st);
                    st.push(b(match op {
                        20 => x == y,
                        24 => x != y,
                        26 => x && y,
                        27 => x != y,
                        _ => x || y,
                    }));
                }
                Instr::Op(op @ 29..=32) => {
                    let v = match op {
                        29 => Some(Val::D(self.pop_f64(i, &mut st))),
                        31 => Some(Val::B(self.pop_bool(i, &mut st))),
                        32 => Some(Val::I(self.pop_i32(i, &mut st))),
                        _ => {
                            st.pop();
                            None
                        }
                    };
                    if let (Some(v), Some(Entry::Ref(r))) = (v, st.last().copied()) {
                        self.set(i, r, v);
                    }
                }
                Instr::Op(_) => {
                    st.pop();
                    st.pop();
                    st.push(b(false));
                }
                Instr::Func(func) => {
                    let (pops, _) = ins.arity();
                    let y = if pops >= 2 { self.pop_f64(i, &mut st) } else { 0.0 };
                    if pops == 3 {
                        st.pop();
                    }
                    let x = self.pop_f64(i, &mut st);
                    st.push(f(match func {
                        0 => x.abs(),
                        5 => x.ceil(),
                        6 => x.cos(),
                        8 => x.exp(),
                        9 => x.floor(),
                        12 => x.max(y),
                        13 => x.min(y),
                        14 => x.powf(y),
                        16 => {
                            if x < 0.0 {
                                -1.0
                            } else {
                                1.0
                            }
                        }
                        19 => x.sqrt(),
                        20 => x.tan(),
                        22 => {
                            if x <= 0.0 {
                                x.ceil()
                            } else {
                                x.floor()
                            }
                        }
                        _ => 0.0,
                    }));
                }
            }
        }
    }
}
