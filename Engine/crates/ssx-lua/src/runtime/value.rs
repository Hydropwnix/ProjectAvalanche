use std::fmt;
use std::rc::Rc;

pub struct LStr {
    pub hash: u32,
    pub bytes: Box<[u8]>,
}

#[derive(Clone)]
pub struct Str(pub Rc<LStr>);

pub fn str_hash(s: &[u8]) -> u32 {
    let l = s.len();
    let mut h = l as u32;
    let step = (l >> 5) + 1;
    let mut l1 = l;
    while l1 >= step {
        h ^= (h << 5).wrapping_add(h >> 2).wrapping_add(u32::from(s[l1 - 1]));
        l1 -= step;
    }
    h
}

impl Str {
    pub fn new(b: &[u8]) -> Str {
        Str(Rc::new(LStr { hash: str_hash(b), bytes: b.into() }))
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }
    pub fn hash(&self) -> u32 {
        self.0.hash
    }
    pub fn to_str_lossy(&self) -> String {
        String::from_utf8_lossy(&self.0.bytes).into_owned()
    }
}

impl PartialEq for Str {
    fn eq(&self, o: &Str) -> bool {
        Rc::ptr_eq(&self.0, &o.0) || (self.0.hash == o.0.hash && self.0.bytes == o.0.bytes)
    }
}

impl fmt::Debug for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_str_lossy())
    }
}

impl From<&str> for Str {
    fn from(s: &str) -> Str {
        Str::new(s.as_bytes())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GcRef(pub u32);

#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    Num(f64),
    Str(Str),
    Table(GcRef),
    Func(GcRef),
    UserData(GcRef),
    Thread(GcRef),
    LightUserData(usize),
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Str::from(s))
    }
    pub fn bytes(b: &[u8]) -> Value {
        Value::Str(Str::new(b))
    }
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Table(_) => "table",
            Value::Func(_) => "function",
            Value::UserData(_) | Value::LightUserData(_) => "userdata",
            Value::Thread(_) => "thread",
        }
    }
    pub fn gc(&self) -> Option<GcRef> {
        match self {
            Value::Table(r) | Value::Func(r) | Value::UserData(r) | Value::Thread(r) => Some(*r),
            _ => None,
        }
    }
    pub fn raw_eq(&self, o: &Value) -> bool {
        match (self, o) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::LightUserData(a), Value::LightUserData(b)) => a == b,
            (Value::Table(a), Value::Table(b))
            | (Value::Func(a), Value::Func(b))
            | (Value::UserData(a), Value::UserData(b))
            | (Value::Thread(a), Value::Thread(b)) => a == b,
            _ => false,
        }
    }
    pub fn as_num(&self) -> Option<f64> {
        if let Value::Num(n) = self { Some(*n) } else { None }
    }
    pub fn to_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Str(s) => crate::number::str2d(s.bytes()),
            _ => None,
        }
    }
    pub fn to_str_coerce(&self) -> Option<Str> {
        match self {
            Value::Str(s) => Some(s.clone()),
            Value::Num(n) => Some(Str::from(crate::number::tostring(*n).as_str())),
            _ => None,
        }
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Value {
        Value::Num(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::str(s)
    }
}

impl From<Str> for Value {
    fn from(s: Str) -> Value {
        Value::Str(s)
    }
}
