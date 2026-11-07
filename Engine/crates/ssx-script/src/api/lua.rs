use mlua::{Lua, MetaMethod, RegistryKey, Table, UserData, UserDataFields, UserDataMethods, Value};

use crate::value::ElementId;
pub use crate::value::lua::{ElementRef, element_of};

pub const MAX_ELEMENT_LIST: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerRef(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LatentRef(pub u64);

impl UserData for TimerRef {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "Timer");
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::AnyUserData| {
            Ok(other.borrow::<TimerRef>().is_ok_and(|o| o.0 == this.0))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("timer: {}", this.0)));
    }
}

impl UserData for LatentRef {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_meta_field(MetaMethod::Type, "LatentHandle");
    }
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::AnyUserData| {
            Ok(other.borrow::<LatentRef>().is_ok_and(|o| o.0 == this.0))
        });
        methods.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("latent: {}", this.0)));
    }
}

struct ElementCache(RegistryKey);

pub fn init(lua: &Lua) -> mlua::Result<()> {
    let cache = lua.create_table()?;
    let meta = lua.create_table()?;
    meta.raw_set("__mode", "v")?;
    cache.set_metatable(Some(meta))?;
    lua.set_app_data(ElementCache(lua.create_registry_value(cache)?));
    Ok(())
}

pub fn element_value(lua: &Lua, id: ElementId) -> mlua::Result<Value> {
    let cache: Option<Table> = match lua.app_data_ref::<ElementCache>() {
        Some(c) => Some(lua.registry_value(&c.0)?),
        None => None,
    };
    let key = id.0 as f64;
    if let Some(cache) = &cache
        && let Value::UserData(ud) = cache.raw_get::<Value>(key)?
    {
        return Ok(Value::UserData(ud));
    }
    let ud = lua.create_userdata(ElementRef(id))?;
    if let Some(cache) = cache {
        cache.raw_set(key, &ud)?;
    }
    Ok(Value::UserData(ud))
}
