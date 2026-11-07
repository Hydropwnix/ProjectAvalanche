use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, Value};

use crate::env::EnvParts;

pub type FileMap = BTreeMap<String, String>;

pub type Compiled = Arc<BTreeMap<String, Arc<[u8]>>>;

pub const MAX_REQUIRE_NAME: usize = 256;

pub fn resolve(files: &FileMap, name: &str) -> Result<String, String> {
    let shown = crate::env::cap(name, 64);
    let rel = name.strip_prefix("./").unwrap_or(name);
    let segment_ok = |s: &str| {
        !s.is_empty() && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    };
    if name.len() > MAX_REQUIRE_NAME || !rel.split('/').all(segment_ok) {
        return Err(format!("require({shown:?}): only the package's own modules can be required"));
    }
    [format!("{rel}.luau"), format!("{rel}/init.luau"), rel.to_string()]
        .into_iter()
        .find(|c| c.ends_with(".luau") && files.contains_key(c))
        .ok_or_else(|| format!("require({shown:?}): no such module in the package"))
}

#[derive(Default)]
pub(crate) struct Modules {
    ready: HashMap<String, mlua::RegistryKey>,
    loaded: HashMap<String, mlua::RegistryKey>,
    loading: BTreeSet<String>,
    code: Compiled,
}

pub(crate) type SharedModules = Arc<Mutex<Modules>>;

impl Modules {
    pub(crate) fn shared() -> SharedModules {
        Arc::new(Mutex::new(Modules::default()))
    }
}

pub(crate) fn preload(lua: &Lua, parts: &EnvParts, modules: &SharedModules, compiled: &Compiled) -> mlua::Result<()> {
    modules.lock().unwrap_or_else(|e| e.into_inner()).code = compiled.clone();
    for (path, code) in compiled.iter() {
        let env = parts.script_env(lua)?;
        let chunk = lua.load(&code[..]).set_name(format!("@{path}")).set_mode(ChunkMode::Binary);
        let key = lua.create_registry_value(chunk.set_environment(env).into_function()?)?;
        modules.lock().unwrap_or_else(|e| e.into_inner()).ready.insert(path.clone(), key);
    }
    Ok(())
}

pub(crate) fn make_require(
    lua: &Lua,
    parts: Arc<EnvParts>,
    files: Arc<FileMap>,
    modules: SharedModules,
) -> mlua::Result<Function> {
    lua.create_function(move |lua, name: mlua::LuaString| {
        let bytes = name.as_bytes();
        if bytes.len() > MAX_REQUIRE_NAME {
            let n = bytes.len();
            return Err(mlua::Error::runtime(format!("require: a name of {n} bytes (at most {MAX_REQUIRE_NAME})")));
        }
        let name = std::str::from_utf8(&bytes).map_err(|_| mlua::Error::runtime("require: the name is not UTF-8"))?;
        let name = name.to_string();
        let path = resolve(&files, &name).map_err(mlua::Error::runtime)?;
        let ready = {
            let mut m = modules.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(key) = m.loaded.get(&path) {
                return lua.registry_value::<Value>(key);
            }
            if !m.loading.insert(path.clone()) {
                return Err(mlua::Error::runtime(format!("require({name:?}): {path} requires itself")));
            }
            (m.ready.remove(&path), m.code.get(&path).cloned())
        };
        let (ready, code) = ready;
        let source = files.get(&path).map(String::as_str).unwrap_or_default();
        let run = || -> mlua::Result<Value> {
            let f: Function = match ready {
                Some(key) => {
                    let f = lua.registry_value(&key)?;
                    lua.remove_registry_value(key)?;
                    f
                }
                None => {
                    let code: Arc<[u8]> = match code {
                        Some(c) => c,
                        None => crate::compile::bytecode(&path, source.as_bytes())
                            .map_err(|e| mlua::Error::runtime(e.0))?
                            .into(),
                    };
                    let env = parts.script_env(lua)?;
                    let chunk = lua.load(&code[..]).set_name(format!("@{path}")).set_mode(ChunkMode::Binary);
                    chunk.set_environment(env).into_function()?
                }
            };
            f.call(())
        };
        let result = run();
        let mut m = modules.lock().unwrap_or_else(|e| e.into_inner());
        m.loading.remove(&path);
        let value = result?;
        m.loaded.insert(path, lua.create_registry_value(value.clone())?);
        Ok(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_inside_the_package_only() {
        let files: FileMap = ["a.luau", "lib/init.luau", "lib/x.luau", "Up.luau"]
            .into_iter()
            .map(|k| (k.to_string(), String::new()))
            .collect();
        assert_eq!(resolve(&files, "a").unwrap(), "a.luau");
        assert_eq!(resolve(&files, "./a").unwrap(), "a.luau");
        assert_eq!(resolve(&files, "a.luau").unwrap(), "a.luau");
        assert_eq!(resolve(&files, "lib").unwrap(), "lib/init.luau");
        assert_eq!(resolve(&files, "lib/x").unwrap(), "lib/x.luau");
        assert_eq!(resolve(&files, "Up").unwrap(), "Up.luau");
        assert!(resolve(&files, "up").is_err(), "names match exactly, case included");
        for bad in [
            "",
            "./",
            "../a",
            "lib/../a",
            "/a",
            "\\a",
            "lib\\x",
            "C:/a",
            "c:a",
            "@self/a",
            "lib//x",
            "./lib/./x",
            "a ",
            "a\0",
            "lib/x/",
            "~/a",
            "%2e%2e/a",
        ] {
            assert!(resolve(&files, bad).is_err(), "{bad:?}");
        }
        assert!(resolve(&files, &"a/".repeat(200)).unwrap_err().contains("only the package's own"));
    }
}
