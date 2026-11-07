mod codec;
pub mod json;
pub mod lua;

use std::fmt;

pub use codec::{decode, decode_list, encode, encode_list};
pub use lua::{from_lua, from_lua_multi, to_lua};

pub const MAX_DEPTH: usize = 16;
pub const MAX_STRING: usize = 64 << 10;
pub const MAX_BYTES: usize = 16 << 20;
pub const MAX_LIST: usize = 256;
pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000;
pub const MAX_ELEMENT_ID: u64 = (1 << 53) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub enum NetValue {
    Nil,
    Bool(bool),
    Number(f64),
    String(Vec<u8>),
    Table(Vec<(NetValue, NetValue)>),
    Element(ElementId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueLimits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_string: usize,
    pub elements: bool,
}

impl ValueLimits {
    const fn of(max_bytes: usize, elements: bool) -> Self {
        ValueLimits { max_bytes, max_depth: MAX_DEPTH, max_string: MAX_STRING, elements }
    }
    pub const REMOTE: Self = Self::of(64 << 10, true);
    pub const LOCAL: Self = Self::of(1 << 20, true);
    pub const LATENT: Self = Self::of(MAX_BYTES, true);
    pub const STORAGE: Self = Self::of(64 << 10, false);
    pub const JSON: Self = Self::of(1 << 20, false);
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("tables nested deeper than {0}")]
    TooDeep(usize),
    #[error("a string of {len} bytes (at most {max})")]
    StringTooLong { len: usize, max: usize },
    #[error("over {0} bytes in all")]
    TooLarge(usize),
    #[error("more than {MAX_LIST} values")]
    TooMany,
    #[error("a table that contains itself")]
    Cycle,
    #[error("a {0} cannot be copied (only nil, booleans, numbers, strings, tables and elements)")]
    NotAValue(&'static str),
    #[error("a table with a metatable cannot be copied")]
    Metatable,
    #[error("a {0} cannot be a table key (only booleans, numbers, strings and elements)")]
    BadKey(&'static str),
    #[error("element references cannot be carried here")]
    NoElements,
    #[error("the encoding ends early")]
    Truncated,
    #[error("{0} bytes after the encoding")]
    Trailing(usize),
    #[error("unknown tag {0:#04x}")]
    BadTag(u8),
    #[error("not the canonical encoding: {0}")]
    NonCanonical(&'static str),
    #[error("two equal keys in one table")]
    DuplicateKey,
}

impl NetValue {
    pub fn encoded_len(&self) -> usize {
        match self {
            NetValue::Nil | NetValue::Bool(_) => 1,
            NetValue::Number(_) | NetValue::Element(_) => 9,
            NetValue::String(s) => 1 + codec::varint_len(s.len()) + s.len(),
            NetValue::Table(t) => {
                1 + codec::varint_len(t.len()) + t.iter().map(|(k, v)| k.encoded_len() + v.encoded_len()).sum::<usize>()
            }
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            NetValue::Nil => "nil",
            NetValue::Bool(_) => "boolean",
            NetValue::Number(_) => "number",
            NetValue::String(_) => "string",
            NetValue::Table(_) => "table",
            NetValue::Element(_) => "element",
        }
    }

    pub fn field(&self, key: &str) -> Option<&NetValue> {
        match self {
            NetValue::Table(t) => {
                t.iter().find(|(k, _)| matches!(k, NetValue::String(s) if s == key.as_bytes())).map(|(_, v)| v)
            }
            _ => None,
        }
    }
}

pub(crate) fn carried(n: f64) -> f64 {
    if n.is_nan() { f64::from_bits(CANONICAL_NAN) } else { n }
}

#[derive(PartialEq, Eq, Hash)]
pub(crate) enum KeyRepr<'a> {
    Bool(bool),
    Number(u64),
    String(&'a [u8]),
    Element(u64),
}

impl<'a> KeyRepr<'a> {
    pub(crate) fn of(v: &'a NetValue) -> Option<Self> {
        Some(match v {
            NetValue::Bool(b) => KeyRepr::Bool(*b),
            NetValue::Number(n) if n.is_nan() => return None,
            NetValue::Number(n) => KeyRepr::Number(if *n == 0.0 { 0 } else { n.to_bits() }),
            NetValue::String(s) => KeyRepr::String(s),
            NetValue::Element(e) => KeyRepr::Element(e.0),
            NetValue::Nil | NetValue::Table(_) => return None,
        })
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[cfg(test)]
mod tests;
