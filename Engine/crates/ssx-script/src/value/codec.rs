use std::collections::HashSet;

use super::{CANONICAL_NAN, ElementId, KeyRepr, MAX_ELEMENT_ID, MAX_LIST, NetValue, ValueError, ValueLimits};

const NIL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const NUMBER: u8 = 3;
const STRING: u8 = 4;
const TABLE: u8 = 5;
const ELEMENT: u8 = 6;

pub(crate) fn varint_len(n: usize) -> usize {
    let mut len = 1;
    let mut n = n >> 7;
    while n > 0 {
        len += 1;
        n >>= 7;
    }
    len
}

fn put_varint(out: &mut Vec<u8>, mut n: usize) {
    while n >= 0x80 {
        out.push((n as u8 & 0x7F) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

pub fn encode(v: &NetValue, limits: &ValueLimits) -> Result<Vec<u8>, ValueError> {
    let mut out = Vec::new();
    put(&mut out, v, limits, 0)?;
    if out.len() > limits.max_bytes {
        return Err(ValueError::TooLarge(limits.max_bytes));
    }
    Ok(out)
}

pub fn encode_list(values: &[NetValue], limits: &ValueLimits) -> Result<Vec<u8>, ValueError> {
    if values.len() > MAX_LIST {
        return Err(ValueError::TooMany);
    }
    let mut out = Vec::new();
    put_varint(&mut out, values.len());
    for v in values {
        put(&mut out, v, limits, 0)?;
        if out.len() > limits.max_bytes {
            return Err(ValueError::TooLarge(limits.max_bytes));
        }
    }
    Ok(out)
}

fn put(out: &mut Vec<u8>, v: &NetValue, limits: &ValueLimits, depth: usize) -> Result<(), ValueError> {
    if out.len() > limits.max_bytes {
        return Err(ValueError::TooLarge(limits.max_bytes));
    }
    match v {
        NetValue::Nil => out.push(NIL),
        NetValue::Bool(b) => out.push(if *b { TRUE } else { FALSE }),
        NetValue::Number(n) => {
            out.push(NUMBER);
            out.extend_from_slice(&super::carried(*n).to_bits().to_le_bytes());
        }
        NetValue::String(s) => {
            if s.len() > limits.max_string {
                return Err(ValueError::StringTooLong { len: s.len(), max: limits.max_string });
            }
            out.push(STRING);
            put_varint(out, s.len());
            out.extend_from_slice(s);
        }
        NetValue::Element(e) => {
            if !limits.elements {
                return Err(ValueError::NoElements);
            }
            if e.0 > MAX_ELEMENT_ID {
                return Err(ValueError::NonCanonical("an element id at or above 2^53"));
            }
            out.push(ELEMENT);
            out.extend_from_slice(&e.0.to_le_bytes());
        }
        NetValue::Table(t) => {
            if depth >= limits.max_depth {
                return Err(ValueError::TooDeep(limits.max_depth));
            }
            out.push(TABLE);
            put_varint(out, t.len());
            let mut seen = HashSet::new();
            for (k, val) in t {
                let key = KeyRepr::of(k).ok_or(ValueError::BadKey(k.type_name()))?;
                if !seen.insert(key) {
                    return Err(ValueError::DuplicateKey);
                }
                if matches!(val, NetValue::Nil) {
                    return Err(ValueError::NonCanonical("a nil table value"));
                }
                match k {
                    NetValue::Number(n) if *n == 0.0 => put(out, &NetValue::Number(0.0), limits, depth + 1)?,
                    k => put(out, k, limits, depth + 1)?,
                }
                put(out, val, limits, depth + 1)?;
            }
        }
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    limits: ValueLimits,
}

impl Reader<'_> {
    fn left(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn byte(&mut self) -> Result<u8, ValueError> {
        let b = *self.bytes.get(self.at).ok_or(ValueError::Truncated)?;
        self.at += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&[u8], ValueError> {
        if n > self.left() {
            return Err(ValueError::Truncated);
        }
        let s = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn u64(&mut self) -> Result<u64, ValueError> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(b))
    }

    fn varint(&mut self) -> Result<usize, ValueError> {
        let mut n: u64 = 0;
        for i in 0..5 {
            let b = self.byte()?;
            n |= u64::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                if i > 0 && b == 0 {
                    return Err(ValueError::NonCanonical("a length with a needless byte"));
                }
                return usize::try_from(n)
                    .ok()
                    .filter(|&n| n <= u32::MAX as usize)
                    .ok_or(ValueError::NonCanonical("a length over 32 bits"));
            }
        }
        Err(ValueError::NonCanonical("a length over 32 bits"))
    }

    fn value(&mut self, depth: usize) -> Result<NetValue, ValueError> {
        Ok(match self.byte()? {
            NIL => NetValue::Nil,
            FALSE => NetValue::Bool(false),
            TRUE => NetValue::Bool(true),
            NUMBER => {
                let bits = self.u64()?;
                let n = f64::from_bits(bits);
                if n.is_nan() && bits != CANONICAL_NAN {
                    return Err(ValueError::NonCanonical("a NaN other than the canonical one"));
                }
                NetValue::Number(n)
            }
            STRING => {
                let len = self.varint()?;
                if len > self.limits.max_string {
                    return Err(ValueError::StringTooLong { len, max: self.limits.max_string });
                }
                NetValue::String(self.take(len)?.to_vec())
            }
            ELEMENT => {
                if !self.limits.elements {
                    return Err(ValueError::NoElements);
                }
                let id = self.u64()?;
                if id > MAX_ELEMENT_ID {
                    return Err(ValueError::NonCanonical("an element id at or above 2^53"));
                }
                NetValue::Element(ElementId(id))
            }
            TABLE => {
                if depth >= self.limits.max_depth {
                    return Err(ValueError::TooDeep(self.limits.max_depth));
                }
                let count = self.varint()?;
                if count > self.left() / 2 {
                    return Err(ValueError::Truncated);
                }
                let mut entries = Vec::new();
                for _ in 0..count {
                    let k = self.value(depth + 1)?;
                    let v = self.value(depth + 1)?;
                    if matches!(k, NetValue::Number(n) if n == 0.0 && n.is_sign_negative()) {
                        return Err(ValueError::NonCanonical("a key of -0"));
                    }
                    if matches!(v, NetValue::Nil) {
                        return Err(ValueError::NonCanonical("a nil table value"));
                    }
                    KeyRepr::of(&k).ok_or(ValueError::BadKey(k.type_name()))?;
                    entries.push((k, v));
                }
                let mut seen = HashSet::new();
                for (k, _) in &entries {
                    if !seen.insert(KeyRepr::of(k).ok_or(ValueError::BadKey(k.type_name()))?) {
                        return Err(ValueError::DuplicateKey);
                    }
                }
                NetValue::Table(entries)
            }
            tag => return Err(ValueError::BadTag(tag)),
        })
    }

    fn end(&self) -> Result<(), ValueError> {
        match self.left() {
            0 => Ok(()),
            n => Err(ValueError::Trailing(n)),
        }
    }
}

fn reader<'a>(bytes: &'a [u8], limits: &ValueLimits) -> Result<Reader<'a>, ValueError> {
    if bytes.len() > limits.max_bytes {
        return Err(ValueError::TooLarge(limits.max_bytes));
    }
    Ok(Reader { bytes, at: 0, limits: *limits })
}

pub fn decode(bytes: &[u8], limits: &ValueLimits) -> Result<NetValue, ValueError> {
    let mut r = reader(bytes, limits)?;
    let v = r.value(0)?;
    r.end()?;
    Ok(v)
}

pub fn decode_list(bytes: &[u8], limits: &ValueLimits) -> Result<Vec<NetValue>, ValueError> {
    let mut r = reader(bytes, limits)?;
    let count = r.varint()?;
    if count > MAX_LIST {
        return Err(ValueError::TooMany);
    }
    if count > r.left() {
        return Err(ValueError::Truncated);
    }
    let mut out = Vec::new();
    for _ in 0..count {
        out.push(r.value(0)?);
    }
    r.end()?;
    Ok(out)
}
