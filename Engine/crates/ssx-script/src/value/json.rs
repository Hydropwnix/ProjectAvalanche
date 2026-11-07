use std::collections::HashSet;
use std::fmt::Write as _;

use super::{MAX_DEPTH, MAX_STRING, NetValue, ValueLimits};

pub const MAX_JSON: usize = 1 << 20;
const EXACT: f64 = 9_007_199_254_740_992.0;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct JsonError(pub String);

fn err<T>(m: impl Into<String>) -> Result<T, JsonError> {
    Err(JsonError(m.into()))
}

struct Writer {
    out: String,
}

impl Writer {
    fn push(&mut self, s: &str) -> Result<(), JsonError> {
        if self.out.len() + s.len() > MAX_JSON {
            return err(format!("the JSON text would be over {MAX_JSON} bytes"));
        }
        self.out.push_str(s);
        Ok(())
    }

    fn string(&mut self, bytes: &[u8]) -> Result<(), JsonError> {
        let Ok(text) = std::str::from_utf8(bytes) else { return err("a string that is not UTF-8") };
        self.push("\"")?;
        let mut start = 0;
        for (i, c) in text.char_indices() {
            let escape = match c {
                '"' => Some("\\\"".to_string()),
                '\\' => Some("\\\\".to_string()),
                '\n' => Some("\\n".to_string()),
                '\r' => Some("\\r".to_string()),
                '\t' => Some("\\t".to_string()),
                c if u32::from(c) < 0x20 => Some(format!("\\u{:04x}", u32::from(c))),
                _ => None,
            };
            if let Some(e) = escape {
                self.push(&text[start..i])?;
                self.push(&e)?;
                start = i + c.len_utf8();
            }
        }
        self.push(&text[start..])?;
        self.push("\"")
    }

    fn number(&mut self, n: f64) -> Result<(), JsonError> {
        if !n.is_finite() {
            return err("NaN and infinities have no JSON form");
        }
        let mut s = String::new();
        if n.fract() == 0.0 && n.abs() <= EXACT {
            let _ = write!(s, "{}", n as i64);
        } else if (1e-5..1e17).contains(&n.abs()) {
            let _ = write!(s, "{n}");
        } else {
            let _ = write!(s, "{n:e}");
        }
        self.push(&s)
    }

    fn value(&mut self, v: &NetValue) -> Result<(), JsonError> {
        match v {
            NetValue::Nil => self.push("null"),
            NetValue::Bool(b) => self.push(if *b { "true" } else { "false" }),
            NetValue::Number(n) => self.number(*n),
            NetValue::String(s) => self.string(s),
            NetValue::Element(_) => err("an element has no JSON form"),
            NetValue::Table(entries) if entries.is_empty() => self.push("{}"),
            NetValue::Table(entries) => {
                if let Some(order) = array_order(entries) {
                    self.push("[")?;
                    for (i, at) in order.into_iter().enumerate() {
                        if i > 0 {
                            self.push(",")?;
                        }
                        self.value(&entries[at].1)?;
                    }
                    return self.push("]");
                }
                self.push("{")?;
                for (i, (k, val)) in entries.iter().enumerate() {
                    let NetValue::String(key) = k else {
                        return err("a table whose keys are neither 1..n nor all strings");
                    };
                    if i > 0 {
                        self.push(",")?;
                    }
                    self.string(key)?;
                    self.push(":")?;
                    self.value(val)?;
                }
                self.push("}")
            }
        }
    }
}

fn array_order(entries: &[(NetValue, NetValue)]) -> Option<Vec<usize>> {
    let n = entries.len();
    let mut at = vec![usize::MAX; n];
    for (pos, (k, _)) in entries.iter().enumerate() {
        let NetValue::Number(i) = k else { return None };
        if i.fract() != 0.0 || *i < 1.0 || *i > n as f64 {
            return None;
        }
        let slot = &mut at[*i as usize - 1];
        if *slot != usize::MAX {
            return None;
        }
        *slot = pos;
    }
    Some(at)
}

pub fn encode(v: &NetValue) -> Result<String, JsonError> {
    let mut w = Writer { out: String::new() };
    w.value(v)?;
    Ok(w.out)
}

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
    left: usize,
}

impl Reader<'_> {
    fn spend(&mut self, n: usize) -> Result<(), JsonError> {
        match self.left.checked_sub(n) {
            Some(l) => {
                self.left = l;
                Ok(())
            }
            None => err(format!("the decoded value would be over {} bytes", ValueLimits::JSON.max_bytes)),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, word: &[u8]) -> Result<(), JsonError> {
        if self.text.get(self.at..self.at + word.len()) == Some(word) {
            self.at += word.len();
            Ok(())
        } else {
            err(format!("unexpected text at byte {}", self.at))
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<f64, JsonError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return err(format!("a malformed number at byte {start}")),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return err(format!("a malformed number at byte {start}"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return err(format!("a malformed number at byte {start}"));
            }
        }
        let text = std::str::from_utf8(&self.text[start..self.at]).map_err(|_| JsonError("a number".into()))?;
        let n: f64 = text.parse().map_err(|_| JsonError(format!("a malformed number at byte {start}")))?;
        if !n.is_finite() {
            return err(format!("the number at byte {start} is beyond a double's range"));
        }
        Ok(n)
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let Some(h) = self.text.get(self.at..self.at + 4) else { return err("a \\u escape ends early") };
        let h = std::str::from_utf8(h).ok().and_then(|s| u32::from_str_radix(s, 16).ok());
        self.at += 4;
        h.ok_or_else(|| JsonError("a malformed \\u escape".into()))
    }

    fn string(&mut self) -> Result<Vec<u8>, JsonError> {
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let Some(b) = self.peek() else { return err("a string ends early") };
            self.at += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(e) = self.peek() else { return err("an escape ends early") };
                    self.at += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                self.expect(b"\\u").map_err(|_| JsonError("a lone surrogate".into()))?;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return err("a lone surrogate");
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(code).ok_or_else(|| JsonError("a lone surrogate".into()))?
                        }
                        _ => return err("an unknown escape"),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                b if b < 0x20 => return err("a control character in a string"),
                b => out.push(b),
            }
            if out.len() > MAX_STRING {
                return err(format!("a string over {MAX_STRING} bytes"));
            }
        }
        Ok(out)
    }

    fn value(&mut self, depth: usize) -> Result<NetValue, JsonError> {
        self.ws();
        Ok(match self.peek() {
            Some(b'n') => {
                self.expect(b"null")?;
                self.spend(1)?;
                NetValue::Nil
            }
            Some(b't') => {
                self.expect(b"true")?;
                self.spend(1)?;
                NetValue::Bool(true)
            }
            Some(b'f') => {
                self.expect(b"false")?;
                self.spend(1)?;
                NetValue::Bool(false)
            }
            Some(b'"') => {
                let s = self.string()?;
                self.spend(5 + s.len())?;
                NetValue::String(s)
            }
            Some(b'-' | b'0'..=b'9') => {
                let n = self.number()?;
                self.spend(9)?;
                NetValue::Number(n)
            }
            Some(b'[') => {
                if depth >= MAX_DEPTH {
                    return err(format!("tables nested deeper than {MAX_DEPTH}"));
                }
                self.at += 1;
                self.spend(6)?;
                let mut entries = Vec::new();
                let mut i = 0u32;
                self.ws();
                if self.peek() == Some(b']') {
                    self.at += 1;
                    return Ok(NetValue::Table(entries));
                }
                loop {
                    let v = self.value(depth + 1)?;
                    i = i.checked_add(1).ok_or_else(|| JsonError("too many items".into()))?;
                    if !matches!(v, NetValue::Nil) {
                        self.spend(9)?;
                        entries.push((NetValue::Number(f64::from(i)), v));
                    }
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            break;
                        }
                        _ => return err(format!("expected , or ] at byte {}", self.at)),
                    }
                }
                NetValue::Table(entries)
            }
            Some(b'{') => {
                if depth >= MAX_DEPTH {
                    return err(format!("tables nested deeper than {MAX_DEPTH}"));
                }
                self.at += 1;
                self.spend(6)?;
                let mut entries = Vec::new();
                let mut keys = HashSet::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.at += 1;
                    return Ok(NetValue::Table(entries));
                }
                loop {
                    self.ws();
                    if self.peek() != Some(b'"') {
                        return err(format!("expected a key at byte {}", self.at));
                    }
                    let k = self.string()?;
                    self.spend(5 + k.len())?;
                    if !keys.insert(k.clone()) {
                        return err("two equal keys in one object");
                    }
                    self.ws();
                    self.expect(b":")?;
                    let v = self.value(depth + 1)?;
                    if !matches!(v, NetValue::Nil) {
                        entries.push((NetValue::String(k), v));
                    }
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            break;
                        }
                        _ => return err(format!("expected , or }} at byte {}", self.at)),
                    }
                }
                NetValue::Table(entries)
            }
            Some(_) => return err(format!("unexpected text at byte {}", self.at)),
            None => return err("the text ends early"),
        })
    }
}

pub fn decode(text: &[u8]) -> Result<NetValue, JsonError> {
    if text.len() > MAX_JSON {
        return err(format!("a JSON text of {} bytes (at most {MAX_JSON})", text.len()));
    }
    if std::str::from_utf8(text).is_err() {
        return err("the text is not UTF-8");
    }
    let mut r = Reader { text, at: 0, left: ValueLimits::JSON.max_bytes };
    let v = r.value(0)?;
    r.ws();
    if r.at != text.len() {
        return err(format!("text after the value at byte {}", r.at));
    }
    Ok(v)
}
