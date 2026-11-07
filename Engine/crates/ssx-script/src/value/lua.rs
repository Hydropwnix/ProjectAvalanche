use std::ffi::c_void;

use mlua::{AnyUserData, Lua, MetaMethod, UserData, UserDataFields, UserDataMethods, Value};

use super::codec::varint_len;
use super::{ElementId, KeyRepr, NetValue, ValueError, ValueLimits};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementRef(pub ElementId);

impl UserData for ElementRef {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "Element");
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: AnyUserData| {
            Ok(other.borrow::<ElementRef>().is_ok_and(|o| o.0 == this.0))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("element: {}", this.0)));
    }
}

pub fn element_of(ud: &AnyUserData) -> Option<ElementId> {
    ud.borrow::<ElementRef>().ok().map(|e| e.0)
}

struct Conv {
    left: usize,
    limits: ValueLimits,
    path: Vec<*const c_void>,
}

impl Conv {
    fn spend(&mut self, n: usize) -> Result<(), ValueError> {
        self.left = self.left.checked_sub(n).ok_or(ValueError::TooLarge(self.limits.max_bytes))?;
        Ok(())
    }

    fn value(&mut self, v: &Value, depth: usize) -> Result<NetValue, ValueError> {
        Ok(match v {
            Value::Nil => {
                self.spend(1)?;
                NetValue::Nil
            }
            Value::Boolean(b) => {
                self.spend(1)?;
                NetValue::Bool(*b)
            }
            Value::Integer(i) => {
                self.spend(9)?;
                NetValue::Number(*i as f64)
            }
            Value::Number(n) => {
                self.spend(9)?;
                NetValue::Number(super::carried(*n))
            }
            Value::String(s) => {
                let bytes = s.as_bytes();
                let len = bytes.len();
                if len > self.limits.max_string {
                    return Err(ValueError::StringTooLong { len, max: self.limits.max_string });
                }
                self.spend(1 + varint_len(len) + len)?;
                NetValue::String(bytes.to_vec())
            }
            Value::UserData(ud) => match element_of(ud) {
                Some(id) if self.limits.elements => {
                    self.spend(9)?;
                    NetValue::Element(id)
                }
                Some(_) => return Err(ValueError::NoElements),
                None => return Err(ValueError::NotAValue("userdata")),
            },
            Value::Table(t) => {
                if depth >= self.limits.max_depth {
                    return Err(ValueError::TooDeep(self.limits.max_depth));
                }
                if t.metatable().is_some() {
                    return Err(ValueError::Metatable);
                }
                let ptr = t.to_pointer();
                if self.path.contains(&ptr) {
                    return Err(ValueError::Cycle);
                }
                self.spend(1)?;
                self.path.push(ptr);
                let mut entries = Vec::new();
                for pair in t.pairs::<Value, Value>() {
                    let (k, val) = pair.map_err(|_| ValueError::NotAValue("table entry"))?;
                    let key = self.value(&k, depth + 1)?;
                    let key = match key {
                        NetValue::Number(0.0) => NetValue::Number(0.0),
                        NetValue::Nil | NetValue::Table(_) => return Err(ValueError::BadKey(key.type_name())),
                        key => key,
                    };
                    if KeyRepr::of(&key).is_none() {
                        return Err(ValueError::BadKey(key.type_name()));
                    }
                    entries.push((key, self.value(&val, depth + 1)?));
                }
                self.path.pop();
                self.spend(varint_len(entries.len()))?;
                NetValue::Table(entries)
            }
            Value::Function(_) => return Err(ValueError::NotAValue("function")),
            Value::Thread(_) => return Err(ValueError::NotAValue("thread")),
            Value::LightUserData(_) => return Err(ValueError::NotAValue("light userdata")),
            Value::Buffer(_) => return Err(ValueError::NotAValue("buffer")),
            Value::Vector(_) => return Err(ValueError::NotAValue("vector")),
            Value::Error(_) => return Err(ValueError::NotAValue("error")),
            Value::Other(_) => return Err(ValueError::NotAValue("value of an unknown type")),
        })
    }
}

pub fn from_lua(v: &Value, limits: &ValueLimits) -> Result<NetValue, ValueError> {
    Conv { left: limits.max_bytes, limits: *limits, path: Vec::new() }.value(v, 0)
}

pub fn from_lua_multi(values: &[Value], limits: &ValueLimits) -> Result<Vec<NetValue>, ValueError> {
    if values.len() > super::MAX_LIST {
        return Err(ValueError::TooMany);
    }
    let mut c = Conv { left: limits.max_bytes, limits: *limits, path: Vec::new() };
    c.spend(varint_len(values.len()))?;
    values.iter().map(|v| c.value(v, 0)).collect()
}

pub fn to_lua(
    lua: &Lua,
    v: &NetValue,
    element: &dyn Fn(&Lua, ElementId) -> mlua::Result<Value>,
) -> mlua::Result<Value> {
    Ok(match v {
        NetValue::Nil => Value::Nil,
        NetValue::Bool(b) => Value::Boolean(*b),
        NetValue::Number(n) => Value::Number(*n),
        NetValue::String(s) => Value::String(lua.create_string(s)?),
        NetValue::Element(e) => element(lua, *e)?,
        NetValue::Table(entries) => {
            let t = lua.create_table()?;
            for (k, val) in entries {
                t.raw_set(to_lua(lua, k, element)?, to_lua(lua, val, element)?)?;
            }
            Value::Table(t)
        }
    })
}
