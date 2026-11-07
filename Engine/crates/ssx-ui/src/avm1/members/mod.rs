use super::interp::Frame;
use super::object::{Func, FuncCode, Kind, NativeFn, ObjId, Object, Property, ScopeKind, flags};
use super::value::{Str, Value, number_to_string, primitive_to_bool, primitive_to_number};
use super::{Ctx, Fault};

mod operators;
mod properties;
mod variables;

const MAX_CHAIN: usize = 64;

pub fn array_index(name: &str) -> Option<usize> {
    if name.is_empty() || name.len() > 9 || !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if name.len() > 1 && name.starts_with('0') {
        return None;
    }
    name.parse().ok()
}

impl Ctx<'_> {
    pub fn new_object(&mut self) -> ObjId {
        self.vm.heap.alloc(Object::new(Value::Object(self.vm.protos.object), Kind::Plain))
    }

    pub fn new_object_with(&mut self, proto: ObjId, kind: Kind) -> ObjId {
        self.vm.heap.alloc(Object::new(Value::Object(proto), kind))
    }

    pub fn new_array(&mut self, items: Vec<Value>) -> ObjId {
        self.vm.heap.alloc(Object::new(Value::Object(self.vm.protos.array), Kind::Array(items)))
    }

    pub fn new_function_object(&mut self, func: Func) -> ObjId {
        let f = self.vm.heap.alloc(Object::new(Value::Object(self.vm.protos.function), Kind::Function(Box::new(func))));
        let proto = self.new_object();
        self.define(proto, "constructor", Value::Object(f), flags::DONT_ENUM);
        self.define(f, "prototype", Value::Object(proto), flags::DONT_ENUM);
        f
    }

    pub fn new_native(&mut self, nf: NativeFn) -> ObjId {
        let func = Func { code: FuncCode::Native(nf), scope: None, base: None };
        self.vm.heap.alloc(Object::new(Value::Object(self.vm.protos.function), Kind::Function(Box::new(func))))
    }

    pub fn define(&mut self, obj: ObjId, name: &str, v: Value, fl: u8) {
        let Some(o) = self.vm.heap.get_mut(obj) else { return };
        let new = !o.props.contains(name);
        if new && o.props.len() >= super::sandbox::caps::PROPS {
            return;
        }
        o.props.insert(Str::from(name), Property::new(v, fl));
        if new {
            self.new_property(name);
        }
    }

    pub(crate) fn new_property(&mut self, name: &str) {
        self.vm.sandbox.grow(super::sandbox::PROPERTY_BYTES + name.len() as u64);
    }

    pub fn kind_of(&self, v: &Value) -> Option<&Kind> {
        v.as_object().and_then(|o| self.vm.heap.get(o)).map(|o| &o.kind)
    }

    pub fn is_function(&self, v: &Value) -> bool {
        matches!(self.kind_of(v), Some(Kind::Function(_)))
    }

    pub fn to_bool(&mut self, v: &Value) -> bool {
        match v {
            Value::Object(o) => match self.vm.heap.get(*o).map(|o| &o.kind) {
                Some(Kind::Display(_)) => self.display_of(v).is_some(),
                _ => true,
            },
            p => primitive_to_bool(p, self.version()),
        }
    }

    pub fn to_number(&mut self, v: &Value) -> f64 {
        match v {
            Value::Object(o) => {
                if matches!(self.vm.heap.get(*o).map(|o| &o.kind), Some(Kind::Display(_))) {
                    return f64::NAN;
                }
                match self.to_primitive(v, false) {
                    Ok(p @ Value::Object(_)) | Ok(p @ Value::Undefined) => {
                        let _ = p;
                        f64::NAN
                    }
                    Ok(p) => primitive_to_number(&p, self.version()),
                    Err(_) => f64::NAN,
                }
            }
            p => primitive_to_number(p, self.version()),
        }
    }

    pub fn to_string(&mut self, v: &Value) -> Result<Str, Fault> {
        Ok(match v {
            Value::Undefined => Str::from(if self.version() >= 7 { "undefined" } else { "" }),
            Value::Null => Str::from("null"),
            Value::Bool(b) => Str::from(if *b { "true" } else { "false" }),
            Value::Number(n) => Str::from(number_to_string(*n)),
            Value::String(s) => s.clone(),
            Value::Object(o) => match self.vm.heap.get(*o).map(|o| &o.kind) {
                Some(Kind::Display(_)) => match self.display_of(v) {
                    Some(id) => Str::from(self.player.path(id)),
                    None => Str::from(""),
                },
                _ => match self.to_primitive(v, true)? {
                    Value::Object(_) => Str::from("[object Object]"),
                    p => self.to_string(&p)?,
                },
            },
        })
    }

    pub fn to_primitive(&mut self, v: &Value, string_hint: bool) -> Result<Value, Fault> {
        let Value::Object(_) = v else { return Ok(v.clone()) };
        let order: [&str; 2] = if string_hint { ["toString", "valueOf"] } else { ["valueOf", "toString"] };
        for m in order {
            let (f, home) = self.get_method(v, m)?;
            if self.is_function(&f) {
                let r = self.call(&f, v.clone(), &[], home)?;
                if !matches!(r, Value::Object(_)) {
                    return Ok(r);
                }
            }
        }
        Ok(v.clone())
    }

    pub fn version(&self) -> u8 {
        self.vm.cur_version
    }
}
