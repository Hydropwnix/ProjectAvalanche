use std::sync::Arc;
use std::sync::atomic::Ordering;

use mlua::{FromLuaMulti, Function, IntoLuaMulti, Lua, LuaString, MultiValue, Table, Value};

use crate::env::lua_text;

pub(crate) const MEMORY_ERROR: &[u8] = b"not enough memory";
const PART: usize = crate::MAX_ERROR_LEN / 2;

pub(crate) struct Calls {
    xpcall: Function,
    handler: Function,
}

impl Calls {
    pub fn new(lua: &Lua, globals: &Table, clock: Arc<crate::vm::Clock>) -> mlua::Result<Self> {
        let xpcall: Function = globals.get("xpcall")?;
        let tostring: Function = globals.get("tostring")?;
        let traceback: Function = globals.get::<Table>("debug")?.get("traceback")?;
        let handler = lua.create_function(move |lua, e: Value| {
            if clock.expired.load(Ordering::Relaxed) || clock.unwinding.load(Ordering::Relaxed) {
                return Ok(e);
            }
            let message = match &e {
                Value::Error(_) => return Ok(e),
                Value::String(s) if s.as_bytes().as_ref() == MEMORY_ERROR => return Ok(e),
                Value::String(s) => lua_text(s, PART),
                other => lua_text(&tostring.call::<LuaString>(other.clone())?, PART),
            };
            let trace = lua_text(&traceback.call::<LuaString>((Value::Nil, 1))?, PART);
            Ok(Value::String(lua.create_string(format!("{message}\n{trace}"))?))
        })?;
        Ok(Calls { xpcall, handler })
    }

    pub fn call<R: FromLuaMulti>(
        &self,
        lua: &Lua,
        f: &Function,
        args: impl IntoLuaMulti,
    ) -> mlua::Result<Result<R, Value>> {
        let mut call = MultiValue::new();
        call.push_back(Value::Function(f.clone()));
        call.push_back(Value::Function(self.handler.clone()));
        call.extend(args.into_lua_multi(lua)?);
        let mut out: MultiValue = self.xpcall.call(call)?;
        match out.pop_front() {
            Some(Value::Boolean(true)) => Ok(Ok(R::from_lua_multi(out, lua)?)),
            _ => Ok(Err(out.pop_front().unwrap_or(Value::Nil))),
        }
    }
}
