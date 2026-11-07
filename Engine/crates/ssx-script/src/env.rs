use std::sync::{Arc, Mutex};

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, LuaString, MultiValue, StdLib, Table, Value, Variadic};

use crate::text::{BoundedText, display_bounded};

pub(crate) fn libs() -> StdLib {
    StdLib::COROUTINE
        | StdLib::TABLE
        | StdLib::OS
        | StdLib::STRING
        | StdLib::UTF8
        | StdLib::BIT
        | StdLib::MATH
        | StdLib::BUFFER
        | StdLib::DEBUG
}

const OS_KEEP: [&str; 3] = ["clock", "time", "date"];
const DEBUG_KEEP: [&str; 1] = ["traceback"];
const GLOBALS_REMOVED: [&str; 4] = ["getfenv", "setfenv", "loadstring", "print"];
const MAX_CHUNK_NAME: usize = 64;
pub const MAX_LOADSTRING: usize = 1 << 20;
pub const MAX_OUTPUT_LINES: usize = 1000;
pub const MAX_OUTPUT_LINE: usize = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub lines: Vec<String>,
    pub dropped: usize,
}

pub(crate) fn cut_globals(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    for (lib, keep) in [("os", &OS_KEEP[..]), ("debug", &DEBUG_KEEP[..])] {
        let full: Table = globals.get(lib)?;
        let cut = lua.create_table()?;
        for k in keep {
            cut.set(*k, full.get::<Value>(*k)?)?;
        }
        globals.set(lib, cut)?;
    }
    for g in GLOBALS_REMOVED {
        globals.set(g, Value::Nil)?;
    }
    Ok(())
}

pub(crate) fn cap(s: &str, max: usize) -> String {
    let mut t = BoundedText::new(max);
    t.push_str(s);
    t.finish()
}

pub(crate) fn lua_text(s: &LuaString, max: usize) -> String {
    let mut t = BoundedText::new(max);
    t.push_bytes(&s.as_bytes());
    t.finish()
}

pub(crate) struct EnvParts {
    pub globals: Table,
    loadstring_into: Function,
    print: Function,
}

impl EnvParts {
    pub fn new(lua: &Lua, globals: Table, output: Arc<Mutex<Output>>) -> mlua::Result<Self> {
        let loadstring_into = lua.create_function(|lua, (env, src, name): (Table, LuaString, Option<LuaString>)| {
            let source = src.as_bytes();
            if source.len() > MAX_LOADSTRING {
                return Err(mlua::Error::runtime(format!("loadstring: source over {MAX_LOADSTRING} bytes")));
            }
            let name = name.map_or_else(|| "loadstring".into(), |n| lua_text(&n, MAX_CHUNK_NAME));
            let code = match crate::compile::bytecode(&name, &source) {
                Ok(code) => code,
                Err(e) => return Ok((Value::Nil, Value::String(lua.create_string(cap(&e.0, MAX_OUTPUT_LINE))?))),
            };
            let chunk = lua.load(code).set_name(format!("={name}")).set_mode(ChunkMode::Binary);
            match chunk.set_environment(env).into_function() {
                Ok(f) => Ok((Value::Function(f), Value::Nil)),
                Err(e) => {
                    let message = display_bounded(&e, MAX_OUTPUT_LINE);
                    Ok((Value::Nil, Value::String(lua.create_string(message)?)))
                }
            }
        })?;
        let tostring: Function = globals.get("tostring")?;
        let print = lua.create_function(move |_, args: Variadic<Value>| {
            {
                let mut o = output.lock().unwrap_or_else(|e| e.into_inner());
                if o.lines.len() >= MAX_OUTPUT_LINES {
                    o.dropped = o.dropped.saturating_add(1);
                    return Ok(MultiValue::new());
                }
            }
            let mut line = BoundedText::new(MAX_OUTPUT_LINE);
            for (i, v) in args.iter().enumerate() {
                let tab = usize::from(i > 0);
                if line.is_full() {
                    match v {
                        Value::String(s) => line.cut(tab + s.as_bytes().len()),
                        _ => line.skip_value(),
                    }
                    continue;
                }
                if tab == 1 {
                    line.push_str("\t");
                }
                match v {
                    Value::String(s) => line.push_bytes(&s.as_bytes()),
                    other => line.push_bytes(&tostring.call::<LuaString>(other.clone())?.as_bytes()),
                }
            }
            let text = line.finish();
            let mut o = output.lock().unwrap_or_else(|e| e.into_inner());
            if o.lines.len() < MAX_OUTPUT_LINES {
                o.lines.push(text);
            } else {
                o.dropped = o.dropped.saturating_add(1);
            }
            Ok(MultiValue::new())
        })?;
        Ok(EnvParts { globals, loadstring_into, print })
    }

    pub fn script_env(&self, lua: &Lua) -> mlua::Result<Table> {
        let env = lua.create_table()?;
        let meta = lua.create_table()?;
        meta.set("__index", &self.globals)?;
        meta.set_readonly(true);
        env.set_metatable(Some(meta))?;
        env.raw_set("loadstring", self.loadstring_into.bind(&env)?)?;
        env.raw_set("print", &self.print)?;
        env.set_safeenv(true);
        Ok(env)
    }
}
