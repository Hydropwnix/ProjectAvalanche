use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use anyhow::Context;
use ssx_assets::anim_db::{AssetDb, field};
use ssx_lua::game::{LIBS_ANIM, open_libs};
use ssx_lua::heap::{NativeFn, downcast};
use ssx_lua::value::GcRef;
use ssx_lua::{Lua, LuaResult, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum GsValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Vector([f32; 3]),
    BoolExpression(String),
}

impl GsValue {
    fn as_f32(&self) -> Option<f32> {
        match self {
            GsValue::Float(v) => Some(*v),
            GsValue::Int(i) => Some(*i as f32),
            GsValue::Bool(b) => Some(f32::from(u8::from(*b))),
            GsValue::Vector(_) | GsValue::BoolExpression(_) => None,
        }
    }
}

#[derive(Default)]
struct Store {
    values: HashMap<Vec<u8>, GsValue>,
    defaults: HashMap<Vec<u8>, GsValue>,
    written: Vec<Vec<u8>>,
    gs_ro: Value,
    funcs: HashMap<String, Value>,
    posted: Vec<String>,
}

struct GsHandle {
    writable: bool,
}

pub struct AnimScripts {
    lua: Lua,
    store: Rc<RefCell<Store>>,
    gs_ro: Value,
    animatable: Value,
    strict: GcRef,
    pub errors: BTreeMap<String, String>,
    pub seconds: f64,
    pub calls: u64,
}

unsafe impl Send for AnimScripts {}

impl std::fmt::Debug for AnimScripts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnimScripts").field("values", &self.store.borrow().values.len()).finish()
    }
}

fn lua_err<T>(r: LuaResult<T>, what: &str) -> anyhow::Result<T> {
    r.map_err(|e| anyhow::anyhow!("{what}: {}", e.message()))
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(String::from_utf8_lossy(s.bytes()).into_owned()),
        _ => None,
    }
}

fn key_of(v: Option<&Value>) -> ssx_lua::Str {
    v.and_then(Value::to_str_coerce).unwrap_or_else(|| ssx_lua::Str::new(b""))
}

fn not_a_value(l: &Lua, key: &[u8]) -> ssx_lua::LuaError {
    l.arg_error(2, &format!("'{}' is not a value asset name.", String::from_utf8_lossy(key)))
}

fn default_of(db: &AssetDb, a: ssx_assets::anim_db::Asset<'_>) -> Option<GsValue> {
    if a.is("BoolScriptExpressionAsset") {
        let script = a.guid("ScriptAsset").and_then(|g| db.get(g))?;
        let ordinal = usize::try_from(a.int("ReadFunctionOrdinal")?).ok()?;
        let name = script.array("SymbolTable").get(ordinal).and_then(|v| v.as_str())?;
        return Some(GsValue::BoolExpression(name.to_string()));
    }
    Some(if a.is("BoolAsset") {
        GsValue::Bool(a.bool("Default").unwrap_or(false))
    } else if a.is("FloatAsset") {
        GsValue::Float(a.f32("Default").unwrap_or(0.0))
    } else if a.is("IntegerAsset") || a.is("GameStateEnumerationAsset") {
        GsValue::Int(a.int("Default").unwrap_or(0) as i32)
    } else if a.is("Vector3Asset") {
        match a.value("Default") {
            Some(ssx_formats::ant::Value::Vector3(v)) => GsValue::Vector(*v),
            _ => GsValue::Vector([0.0; 3]),
        }
    } else {
        return None;
    })
}

impl AnimScripts {
    pub fn from_db(db: &AssetDb) -> anyhow::Result<Self> {
        let engine = db.by_name("LuaScriptEngine").context("no LuaScriptEngine asset")?;
        let mut values = Vec::new();
        for e in engine.array("ValueAssetList") {
            let Some(asset) = field(engine.schema, e, "AntAsset").and_then(|v| v.as_guid()).and_then(|g| db.get(g))
            else {
                continue;
            };
            if let Some(v) = default_of(db, asset) {
                values.push((asset.name().to_string(), v));
            }
        }
        let mut scripts = Vec::new();
        for g in engine.guids("LuaScriptList") {
            let Some(a) = db.get(g) else { continue };
            let Some(code) = a.value("ScriptCode").and_then(|v| v.as_bytes()) else { continue };
            let code = code.split(|b| *b == 0).next().unwrap_or(code);
            scripts.push((a.name().to_string(), code.to_vec()));
        }
        Self::new(values, &scripts)
    }

    pub fn new(values: Vec<(String, GsValue)>, scripts: &[(String, Vec<u8>)]) -> anyhow::Result<Self> {
        let mut store = Store::default();
        store.values = values.into_iter().map(|(n, v)| (n.into_bytes(), v)).collect();
        store.defaults = store.values.clone();
        let store = Rc::new(RefCell::new(store));
        let mut lua = Lua::new();
        open_libs(&mut lua, &LIBS_ANIM);
        let trace = lua.create_function("Trace", |_, _| Ok(Vec::new()));
        lua.set_global("print", trace.clone());
        let ant = lua.create_table();
        let animatable_lib = lua.create_table();
        let message = lua.create_table();
        let sink = store.clone();
        let send = lua.create_function("Send", move |l: &mut Lua, a: &[Value]| {
            if let (Some(Value::Table(t)), Some("MsgPostObjectiveEvent")) =
                (a.get(1), a.first().and_then(text).as_deref())
                && let Some(e) = text(&l.raw_get(*t, &Value::str("mEventName")))
            {
                sink.borrow_mut().posted.push(e);
            }
            Ok(Vec::new())
        });
        if let (Value::Table(m), Value::Table(t)) = (&message, &ant) {
            lua_err(lua.raw_set(*m, Value::str("Send"), send), "Message.Send")?;
            lua_err(lua.raw_set(*t, Value::str("Trace"), trace), "Ant.Trace")?;
        }
        lua.set_global("Message", message);
        lua.set_global("Ant", ant.clone());
        lua.set_global("Animatable", animatable_lib);
        let (gs_meta, gs_rw, gs_ro) = Self::game_state_type(&mut lua, &store)?;
        let animatable = lua.create_table();
        if let Value::Table(t) = &animatable {
            lua_err(lua.raw_set(*t, Value::str("nativeptr"), Value::LightUserData(1)), "nativeptr")?;
            lua_err(lua.raw_set(*t, Value::str("gs"), gs_rw.clone()), "gs")?;
        }
        for v in [gs_rw.clone(), gs_ro.clone(), animatable.clone(), Value::Table(gs_meta)] {
            lua.pin(v);
        }
        for (name, code) in scripts {
            lua_err(lua.exec(code, name), name)?;
        }
        let mut funcs = HashMap::new();
        let g = lua.globals;
        let mut k = Value::Nil;
        while let Ok(Some((key, v))) = lua.heap.table(g).next(&k) {
            if let (Some(n), Value::Func(_)) = (text(&key), &v) {
                funcs.insert(n, v.clone());
            }
            k = key;
        }
        let strict = Self::strict_globals(&mut lua)?;
        lua.pin(Value::Table(strict));
        {
            let mut s = store.borrow_mut();
            s.funcs = funcs;
            s.gs_ro = gs_ro.clone();
        }
        Ok(AnimScripts { lua, store, gs_ro, animatable, strict, errors: BTreeMap::new(), seconds: 0.0, calls: 0 })
    }

    fn game_state_type(lua: &mut Lua, store: &Rc<RefCell<Store>>) -> anyhow::Result<(GcRef, Value, Value)> {
        let st = store.clone();
        let index: NativeFn = Rc::new(move |l: &mut Lua, a: &[Value]| {
            let key = key_of(a.get(1));
            let v = st.borrow().values.get(key.bytes()).cloned();
            Ok(vec![match v {
                None => return Err(not_a_value(l, key.bytes())),
                Some(GsValue::Float(x)) => Value::Num(f64::from(x)),
                Some(GsValue::Int(i)) => Value::Num(f64::from(i)),
                Some(GsValue::Bool(b)) => Value::Bool(b),
                Some(GsValue::Vector(v)) => {
                    let t = l.new_table(0, 4);
                    for (k, c) in ["x", "y", "z"].iter().zip(v) {
                        l.raw_set(t, Value::str(k), Value::Num(f64::from(c)))?;
                    }
                    Value::Table(t)
                }
                Some(GsValue::BoolExpression(f)) => {
                    let (func, gs) = {
                        let s = st.borrow();
                        (s.funcs.get(&f).cloned(), s.gs_ro.clone())
                    };
                    let func = func.ok_or_else(|| l.rt_error(&format!("no function '{f}'")))?;
                    let r = l.call(func, &[gs])?;
                    Value::Bool(r.first().is_some_and(Value::truthy))
                }
            }])
        });
        let st = store.clone();
        let newindex: NativeFn = Rc::new(move |l: &mut Lua, a: &[Value]| {
            let key = key_of(a.get(1));
            let old = st.borrow().values.get(key.bytes()).cloned();
            let Some(old) = old else { return Err(not_a_value(l, key.bytes())) };
            let writable = match a.first() {
                Some(Value::UserData(u)) => downcast::<GsHandle>(l.heap.userdata(*u)).is_some_and(|h| h.writable),
                _ => false,
            };
            if !writable {
                return Err(l.rt_error("GameState table is read only in this context"));
            }
            let new = match old {
                GsValue::Float(_) => GsValue::Float(l.check_num(a, 3)? as f32),
                GsValue::Int(_) => GsValue::Int(l.check_int(a, 3)?),
                GsValue::Bool(_) => GsValue::Bool(l.check_any(a, 3)?.truthy()),
                GsValue::Vector(_) | GsValue::BoolExpression(_) => {
                    return Err(l.rt_error("a write of this value type through gs is not ported"));
                }
            };
            let mut s = st.borrow_mut();
            s.values.insert(key.bytes().to_vec(), new);
            s.written.push(key.bytes().to_vec());
            Ok(Vec::new())
        });
        let meta = lua.new_table(0, 4);
        let index = lua.create_function_with("__index", index, Vec::new());
        let newindex = lua.create_function_with("__newindex", newindex, Vec::new());
        lua_err(lua.raw_set(meta, Value::str("__index"), index), "__index")?;
        lua_err(lua.raw_set(meta, Value::str("__newindex"), newindex), "__newindex")?;
        let rw = lua.create_userdata(Box::new(GsHandle { writable: true }), Some(meta));
        let ro = lua.create_userdata(Box::new(GsHandle { writable: false }), Some(meta));
        Ok((meta, rw, ro))
    }

    fn strict_globals(lua: &mut Lua) -> anyhow::Result<GcRef> {
        let strict = lua.new_table(0, 2);
        let read = lua.create_function("__index", |l: &mut Lua, a: &[Value]| {
            let n = a.get(1).and_then(text).unwrap_or_default();
            Err(l.rt_error(&format!("attempt to read undefined global variable '{n}'")))
        });
        let write = lua.create_function("__newindex", |l: &mut Lua, a: &[Value]| {
            let n = a.get(1).and_then(text).unwrap_or_default();
            Err(l.rt_error(&format!("attempt to create global variable '{n}' during update (use gamestate instead)")))
        });
        lua_err(lua.raw_set(strict, Value::str("__index"), read), "strict")?;
        lua_err(lua.raw_set(strict, Value::str("__newindex"), write), "strict")?;
        Ok(strict)
    }

    pub fn reset(&mut self) {
        let mut s = self.store.borrow_mut();
        s.values = s.defaults.clone();
        s.written.clear();
        s.posted.clear();
    }

    pub fn set(&mut self, name: &str, v: f32) -> bool {
        let mut s = self.store.borrow_mut();
        let (base, lane) = match name.split_once('.') {
            Some((b, l)) => (b, Some(l)),
            None => (name, None),
        };
        let Some(slot) = s.values.get_mut(base.as_bytes()) else { return false };
        match (slot, lane) {
            (GsValue::Float(x), None) => *x = v,
            (GsValue::Int(i), None) => *i = ssx_lua::number::to_int(f64::from(v)),
            (GsValue::Bool(b), None) => *b = v != 0.0,
            (GsValue::Vector(x), Some(l)) => match l {
                "x" => x[0] = v,
                "y" => x[1] = v,
                "z" => x[2] = v,
                _ => return false,
            },
            _ => return false,
        }
        true
    }

    pub fn get(&self, name: &str) -> Option<f32> {
        self.store.borrow().values.get(name.as_bytes()).and_then(GsValue::as_f32)
    }

    pub fn take_written(&mut self) -> Vec<(String, f32)> {
        let mut s = self.store.borrow_mut();
        let mut names = std::mem::take(&mut s.written);
        names.reverse();
        let mut seen = std::collections::HashSet::new();
        names.retain(|n| seen.insert(n.clone()));
        names
            .into_iter()
            .filter_map(|n| {
                let v = s.values.get(&n).and_then(GsValue::as_f32)?;
                Some((String::from_utf8_lossy(&n).into_owned(), v))
            })
            .collect()
    }

    pub fn update_players(&mut self) -> bool {
        let t0 = std::time::Instant::now();
        let ant = self.lua.get_global("Ant");
        let list = self.lua.new_table(1, 0);
        let _ = self.lua.raw_set(list, Value::Num(1.0), self.animatable.clone());
        if let Value::Table(a) = &ant {
            let _ = self.lua.raw_set(*a, Value::str("animatables"), Value::Table(list));
        }
        let g = self.lua.globals;
        self.lua.heap.table_mut(g).meta = Some(self.strict);
        let f = self.store.borrow().funcs.get("UpdatePlayers").cloned();
        let r = match f {
            Some(f) => self.lua.call(f, &[]).map(|_| ()),
            None => Err(self.lua.rt_error("no function 'UpdatePlayers'")),
        };
        self.lua.heap.table_mut(g).meta = None;
        if let Value::Table(a) = &ant {
            let _ = self.lua.raw_set(*a, Value::str("self"), Value::Nil);
            let _ = self.lua.raw_set(*a, Value::str("animatables"), Value::Nil);
        }
        self.seconds += t0.elapsed().as_secs_f64();
        self.calls += 1;
        match r {
            Ok(()) => true,
            Err(e) => {
                self.errors.entry("UpdatePlayers".into()).or_insert_with(|| e.message());
                false
            }
        }
    }

    pub fn expression(&mut self, function: &str, float: bool) -> Option<f32> {
        let t0 = std::time::Instant::now();
        let f = self.store.borrow().funcs.get(function).cloned();
        let r = match f {
            Some(f) => self.lua.call(f, std::slice::from_ref(&self.gs_ro)),
            None => Err(self.lua.rt_error(&format!("no function '{function}'"))),
        };
        self.seconds += t0.elapsed().as_secs_f64();
        self.calls += 1;
        let v = r.and_then(|r| {
            let v = r.into_iter().next().unwrap_or_default();
            if float {
                self.lua.check_num(std::slice::from_ref(&v), 1).map(|n| n as f32)
            } else {
                Ok(f32::from(u8::from(v.truthy())))
            }
        });
        match v {
            Ok(v) => Some(v),
            Err(e) => {
                self.errors.entry(function.to_string()).or_insert_with(|| e.message());
                None
            }
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.store.borrow().funcs.contains_key(name)
    }

    pub fn take_posted(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.borrow_mut().posted)
    }

    pub fn collect(&mut self) {
        self.lua.maybe_collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
function UpdatePlayers()
    for _, a in ipairs(Ant.animatables) do
        Ant.self = a
        UpdatePlayer(a)
        Ant.self = nil
    end
end
function UpdatePlayer(a)
    local gs = a.gs
    gs.FLT_a = gs.FLT_a / 3
    gs.INT_b = -2.7
    gs.BLN_c = 0
end
function Scaled(gs) return gs.FLT_a * 1.2 + gs.FLT_d * 0.4 end
function Goofy(gs) if UNDEFINED_GLOBAL then return 1 else return -1 end end
function Write(gs) gs.FLT_a = 1 end
function Unknown(gs) return gs.FLT_nope end
function Flag(gs) return gs.BLN_c end
function ViaExpression(gs) if gs.BLNX_flag then return 2 else return 3 end end
function Strict() return UNDEFINED_GLOBAL end
"#;

    fn scripts(update: &str) -> AnimScripts {
        let values = vec![
            ("FLT_a".to_string(), GsValue::Float(0.7)),
            ("FLT_d".to_string(), GsValue::Float(0.3)),
            ("INT_b".to_string(), GsValue::Int(5)),
            ("BLN_c".to_string(), GsValue::Bool(false)),
            ("BLNX_flag".to_string(), GsValue::BoolExpression("Flag".into())),
        ];
        let code = SCRIPT.replace("UpdatePlayer(a)\n", &format!("{update}(a)\n"));
        AnimScripts::new(values, &[("test".into(), code.into_bytes())]).expect("the test script loads")
    }

    #[test]
    fn expressions_compute_in_double_and_round_once() {
        let mut s = scripts("UpdatePlayer");
        let want = (f64::from(0.7f32) * 1.2 + f64::from(0.3f32) * 0.4) as f32;
        assert_eq!(s.expression("Scaled", true), Some(want));
        assert!(s.errors.is_empty());
    }

    #[test]
    fn strict_globals_only_in_the_scene_op() {
        let mut s = scripts("Strict");
        assert_eq!(s.expression("Goofy", true), Some(-1.0));
        assert!(!s.update_players());
        let e = &s.errors["UpdatePlayers"];
        assert!(e.contains("attempt to read undefined global variable 'UNDEFINED_GLOBAL'"), "{e}");
        assert_eq!(s.expression("Goofy", true), Some(-1.0), "the metatable is removed after the call");
    }

    #[test]
    fn game_state_writes_and_errors() {
        let mut s = scripts("UpdatePlayer");
        assert!(s.update_players(), "{:?}", s.errors);
        assert_eq!(s.get("FLT_a"), Some((f64::from(0.7f32) / 3.0) as f32));
        assert_eq!(s.get("INT_b"), Some(-2.0));
        assert_eq!(s.get("BLN_c"), Some(1.0), "lua_toboolean(0) is true");
        let mut w = s.take_written();
        w.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(w.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["BLN_c", "FLT_a", "INT_b"]);
        assert_eq!(s.expression("Write", true), None);
        assert!(s.errors["Write"].contains("GameState table is read only in this context"));
        assert_eq!(s.expression("Unknown", true), None);
        assert!(s.errors["Unknown"].contains("'FLT_nope' is not a value asset name."));
        assert_eq!(s.expression("ViaExpression", true), Some(2.0), "BLNX_flag evaluates Flag: BLN_c is true");
        s.reset();
        assert_eq!(s.get("FLT_a"), Some(0.7));
        assert_eq!(s.expression("ViaExpression", true), Some(3.0));
        assert_eq!(s.expression("Flag", false), Some(0.0));
    }
}
