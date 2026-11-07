use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Push {
    Str(String),
    Int(i32),
    Num(f64),
    Bool(bool),
    Null,
    Undefined,
    Reg(u8),
    Const(u16),
}

impl From<&str> for Push {
    fn from(s: &str) -> Self {
        Push::Str(s.to_string())
    }
}

impl From<i32> for Push {
    fn from(i: i32) -> Self {
        Push::Int(i)
    }
}

pub mod op {
    pub const END: u8 = 0x00;
    pub const STOP: u8 = 0x07;
    pub const ADD2: u8 = 0x47;
    pub const SUBTRACT: u8 = 0x0B;
    pub const MULTIPLY: u8 = 0x0C;
    pub const LESS2: u8 = 0x48;
    pub const EQUALS2: u8 = 0x49;
    pub const NOT: u8 = 0x12;
    pub const POP: u8 = 0x17;
    pub const GET_VARIABLE: u8 = 0x1C;
    pub const SET_VARIABLE: u8 = 0x1D;
    pub const TRACE: u8 = 0x26;
    pub const THROW: u8 = 0x2A;
    pub const DEFINE_LOCAL: u8 = 0x3C;
    pub const CALL_FUNCTION: u8 = 0x3D;
    pub const RETURN: u8 = 0x3E;
    pub const NEW_OBJECT: u8 = 0x40;
    pub const INIT_ARRAY: u8 = 0x42;
    pub const INIT_OBJECT: u8 = 0x43;
    pub const GET_MEMBER: u8 = 0x4E;
    pub const SET_MEMBER: u8 = 0x4F;
    pub const INCREMENT: u8 = 0x50;
    pub const DECREMENT: u8 = 0x51;
    pub const CALL_METHOD: u8 = 0x52;
    pub const NEW_METHOD: u8 = 0x53;
    pub const EXTENDS: u8 = 0x69;
    pub const PUSH_DUPLICATE: u8 = 0x4C;
    pub const STRING_ADD: u8 = 0x21;
    pub const TO_STRING: u8 = 0x4B;
    pub const GREATER: u8 = 0x67;
    pub const STRICT_EQUALS: u8 = 0x66;
    pub const TYPE_OF: u8 = 0x44;
}

#[derive(Default)]
pub struct Asm {
    b: Vec<u8>,
    labels: HashMap<String, usize>,
    fixups: Vec<(usize, String)>,
}

impl Asm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn op(mut self, code: u8) -> Self {
        self.b.push(code);
        self
    }

    pub fn data(mut self, code: u8, d: &[u8]) -> Self {
        let len = u16::try_from(d.len()).expect("an action record is at most 65 535 bytes");
        self.b.push(code);
        self.b.extend_from_slice(&len.to_le_bytes());
        self.b.extend_from_slice(d);
        self
    }

    pub fn pool(self, strings: &[&str]) -> Self {
        let mut d = (strings.len() as u16).to_le_bytes().to_vec();
        for s in strings {
            d.extend_from_slice(s.as_bytes());
            d.push(0);
        }
        self.data(0x88, &d)
    }

    pub fn push(self, values: &[Push]) -> Self {
        let mut d = Vec::new();
        for v in values {
            match v {
                Push::Str(s) => {
                    d.push(0);
                    d.extend_from_slice(s.as_bytes());
                    d.push(0);
                }
                Push::Num(x) => {
                    let b = x.to_le_bytes();
                    d.push(6);
                    d.extend_from_slice(&b[4..8]);
                    d.extend_from_slice(&b[0..4]);
                }
                Push::Null => d.push(2),
                Push::Undefined => d.push(3),
                Push::Reg(r) => d.extend_from_slice(&[4, *r]),
                Push::Bool(b) => d.extend_from_slice(&[5, u8::from(*b)]),
                Push::Int(i) => {
                    d.push(7);
                    d.extend_from_slice(&i.to_le_bytes());
                }
                Push::Const(c) if *c < 256 => d.extend_from_slice(&[8, *c as u8]),
                Push::Const(c) => {
                    d.push(9);
                    d.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
        self.data(0x96, &d)
    }

    pub fn strs(self, values: &[&str]) -> Self {
        self.push(&values.iter().map(|s| Push::Str(s.to_string())).collect::<Vec<_>>())
    }

    pub fn store(self, r: u8) -> Self {
        self.data(0x87, &[r])
    }

    pub fn label(mut self, name: &str) -> Self {
        self.labels.insert(name.to_string(), self.b.len());
        self
    }

    pub fn jump(self, to: &str) -> Self {
        self.branch(0x99, to)
    }

    pub fn if_true(self, to: &str) -> Self {
        self.branch(0x9D, to)
    }

    fn branch(mut self, code: u8, to: &str) -> Self {
        self.b.extend_from_slice(&[code, 2, 0, 0, 0]);
        self.fixups.push((self.b.len() - 2, to.to_string()));
        self
    }

    pub fn function2(mut self, name: &str, params: &[(u8, &str)], regs: u8, flags: u16, body: Asm) -> Self {
        let body = body.finish_body();
        let mut d = name.as_bytes().to_vec();
        d.push(0);
        d.extend_from_slice(&(params.len() as u16).to_le_bytes());
        d.push(regs);
        d.extend_from_slice(&flags.to_be_bytes());
        for (r, p) in params {
            d.push(*r);
            d.extend_from_slice(p.as_bytes());
            d.push(0);
        }
        let len = u16::try_from(body.len()).expect("a function body is at most 65 535 bytes");
        d.extend_from_slice(&len.to_le_bytes());
        self = self.data(0x8E, &d);
        self.b.extend_from_slice(&body);
        self
    }

    pub fn function(mut self, name: &str, params: &[&str], body: Asm) -> Self {
        let body = body.finish_body();
        let mut d = name.as_bytes().to_vec();
        d.push(0);
        d.extend_from_slice(&(params.len() as u16).to_le_bytes());
        for p in params {
            d.extend_from_slice(p.as_bytes());
            d.push(0);
        }
        let len = u16::try_from(body.len()).expect("a function body is at most 65 535 bytes");
        d.extend_from_slice(&len.to_le_bytes());
        self = self.data(0x9B, &d);
        self.b.extend_from_slice(&body);
        self
    }

    pub fn call_method(self, object: Asm, member: &str, args: Vec<Asm>) -> Self {
        let n = args.len() as i32;
        let mut s = self;
        for a in args.into_iter().rev() {
            s = s.then(a);
        }
        s = s.push(&[Push::Int(n)]).then(object);
        s.strs(&[member]).op(op::CALL_METHOD)
    }

    pub fn then(mut self, other: Asm) -> Self {
        let base = self.b.len();
        for (name, at) in other.labels {
            self.labels.insert(name, base + at);
        }
        for (at, name) in other.fixups {
            self.fixups.push((base + at, name));
        }
        self.b.extend(other.b);
        self
    }

    pub fn finish_body(mut self) -> Vec<u8> {
        for (at, l) in std::mem::take(&mut self.fixups) {
            let target = *self.labels.get(&l).unwrap_or_else(|| panic!("the label {l} is not in its block"));
            let delta = target as i64 - (at as i64 + 2);
            let delta = i16::try_from(delta).expect("a branch within ±32 KiB");
            self.b[at..at + 2].copy_from_slice(&delta.to_le_bytes());
        }
        self.b
    }

    pub fn finish(self) -> Vec<u8> {
        let mut b = self.finish_body();
        b.push(op::END);
        b
    }
}

pub fn path(dotted: &str) -> Asm {
    let mut parts = dotted.split('.');
    let mut a = Asm::new().strs(&[parts.next().unwrap_or("")]).op(op::GET_VARIABLE);
    for p in parts {
        a = a.strs(&[p]).op(op::GET_MEMBER);
    }
    a
}

pub fn value(v: Push) -> Asm {
    Asm::new().push(&[v])
}
