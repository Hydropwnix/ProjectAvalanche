mod client;
pub mod defs;
mod draw;
mod elements;
mod events;
pub mod lua;
mod meta;
mod names;
mod server;
mod world;

use std::sync::{Arc, OnceLock, Weak};

use mlua::{AnyUserData, Function, Lua, MultiValue, Table, Value};

use crate::host::Side;
use crate::host::world::HostShared;
use crate::value::{ElementId, NetValue, from_lua, to_lua};

pub use names::{
    check_command_name, check_data_key, check_element_kind, check_element_name, check_event_name, check_export_name,
    check_server_command_name,
};

pub const API_LEVEL: u32 = ssx_content::API_LEVEL;

#[derive(Clone)]
pub struct Ctx {
    pub shared: Weak<HostShared>,
    pub package: String,
    pub side: Side,
    pub root: ElementId,
}

impl Ctx {
    pub fn host(&self) -> Result<Arc<HostShared>, String> {
        self.shared.upgrade().ok_or_else(|| "the script host is gone".to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sides {
    Both,
    Server,
    Client,
}

impl Sides {
    pub fn has(self, side: Side) -> bool {
        matches!((self, side), (Sides::Both, _) | (Sides::Server, Side::Server) | (Sides::Client, Side::Client))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Event,
    ElementKind,
    NewElementKind,
    ElementName,
    DataKey,
    Command,
    Export,
    StorageKey,
    Setting,
    Package,
    ContentId,
}

#[derive(Clone, Copy, Debug)]
pub enum Ty {
    Nil,
    Bool,
    Number { min: f64, max: f64 },
    Int { min: i64, max: i64 },
    Str { max: usize },
    Name(NameKind),
    Enum(&'static [&'static str]),
    Element,
    Elements,
    Function,
    Timer,
    Latent,
    Value,
    Any,
    Opt(&'static Ty),
    List(&'static Ty),
    Record(&'static str),
    Vec3 { max: f64 },
}

pub struct Param {
    pub name: &'static str,
    pub ty: Ty,
}

pub enum Arg {
    Nil,
    Bool(bool),
    Num(f64),
    Int(i64),
    Str(String),
    Element(ElementId),
    Elements(Vec<ElementId>),
    Func(Function),
    Timer(u64),
    Latent(u64),
    Value(Value),
    Vec3([f64; 3]),
    Record(Vec<(&'static str, Arg)>),
}

pub struct Args {
    pub fixed: Vec<Arg>,
    pub rest: Vec<Value>,
}

impl Args {
    pub fn str(&self, i: usize) -> Option<&str> {
        match self.fixed.get(i) {
            Some(Arg::Str(s)) => Some(s),
            _ => None,
        }
    }
    pub fn element(&self, i: usize) -> Option<ElementId> {
        match self.fixed.get(i) {
            Some(Arg::Element(e)) => Some(*e),
            _ => None,
        }
    }
    pub fn bool(&self, i: usize) -> Option<bool> {
        match self.fixed.get(i) {
            Some(Arg::Bool(b)) => Some(*b),
            _ => None,
        }
    }
    pub fn int(&self, i: usize) -> Option<i64> {
        match self.fixed.get(i) {
            Some(Arg::Int(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn num(&self, i: usize) -> Option<f64> {
        match self.fixed.get(i) {
            Some(Arg::Num(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn func(&self, i: usize) -> Option<&Function> {
        match self.fixed.get(i) {
            Some(Arg::Func(f)) => Some(f),
            _ => None,
        }
    }
    pub fn value(&self, i: usize) -> Value {
        match self.fixed.get(i) {
            Some(Arg::Value(v)) => v.clone(),
            _ => Value::Nil,
        }
    }
    pub fn vec3(&self, i: usize) -> Option<[f64; 3]> {
        match self.fixed.get(i) {
            Some(Arg::Vec3(v)) => Some(*v),
            _ => None,
        }
    }
    pub fn field(&self, i: usize, name: &str) -> Option<&Arg> {
        match self.fixed.get(i) {
            Some(Arg::Record(fields)) => fields.iter().find(|(n, _)| *n == name).map(|(_, a)| a),
            _ => None,
        }
    }
}

pub enum Ret {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Element(ElementId),
    Elements(Vec<ElementId>),
    Strings(Vec<String>),
    Timer(u64),
    Latent(u64),
    Latents(Vec<u64>),
    Value(NetValue),
    Lua(MultiValue),
}

pub enum ApiError {
    Refused(String),
    Lua(mlua::Error),
}

impl From<String> for ApiError {
    fn from(s: String) -> Self {
        ApiError::Refused(s)
    }
}

impl From<&str> for ApiError {
    fn from(s: &str) -> Self {
        ApiError::Refused(s.to_string())
    }
}

impl From<mlua::Error> for ApiError {
    fn from(e: mlua::Error) -> Self {
        ApiError::Lua(e)
    }
}

pub type Imp = fn(&Ctx, &Lua, Args) -> Result<Vec<Ret>, ApiError>;

pub struct Spec {
    pub name: &'static str,
    pub sides: Sides,
    pub api: u32,
    pub params: &'static [Param],
    pub rest: Option<Ty>,
    pub returns: &'static [Ty],
    pub any_results: bool,
    pub call: Imp,
}

const fn num(max: f64) -> Ty {
    Ty::Number { min: -max, max }
}

pub const WORLD: f64 = 1.0e6;
const VEC3: Ty = Ty::Record("Vec3");
const OPT_NUM: Ty = Ty::Opt(&Ty::Number { min: -1.0e9, max: 1.0e9 });

pub const RECORDS: &[(&str, &[(&str, Ty)])] = &[
    (
        "LatentStatus",
        &[
            ("total", Ty::Number { min: 0.0, max: f64::MAX }),
            ("sent", Ty::Number { min: 0.0, max: f64::MAX }),
            ("percent", Ty::Number { min: 0.0, max: 100.0 }),
        ],
    ),
    ("Vec3", &[("x", num(WORLD)), ("y", num(WORLD)), ("z", num(WORLD))]),
    (
        "RiderState",
        &[
            ("name", Ty::Str { max: 64 }),
            ("position", VEC3),
            ("velocity", Ty::Opt(&VEC3)),
            ("speed", OPT_NUM),
            ("air", Ty::Opt(&Ty::Bool)),
            ("grind", Ty::Opt(&Ty::Bool)),
            ("trick", Ty::Opt(&Ty::Str { max: 128 })),
            ("score", OPT_NUM),
            ("health", Ty::Opt(&Ty::Number { min: 0.0, max: 1.0 })),
            ("finished", Ty::Opt(&Ty::Bool)),
            ("isLocal", Ty::Bool),
        ],
    ),
    (
        "Instance",
        &[
            ("mode", Ty::Str { max: 32 }),
            ("drop", Ty::Str { max: 256 }),
            ("state", Ty::Enum(&["lobby", "countdown", "running", "results"])),
            ("clock", Ty::Number { min: 0.0, max: 1.0e9 }),
        ],
    ),
    (
        "Standing",
        &[
            ("rider", Ty::Opt(&Ty::Element)),
            ("name", Ty::Str { max: 64 }),
            ("place", Ty::Int { min: 1, max: 65_536 }),
            ("value", Ty::Number { min: -1.0e12, max: 1.0e12 }),
            ("finished", Ty::Bool),
        ],
    ),
    (
        "ContentInfo",
        &[
            ("id", Ty::Str { max: 256 }),
            ("kind", Ty::Str { max: 16 }),
            ("source", Ty::Str { max: 64 }),
            ("layer", Ty::Str { max: 16 }),
        ],
    ),
    (
        "InstanceInfo",
        &[
            ("id", Ty::Int { min: 1, max: 65_535 }),
            ("mode", Ty::Str { max: 32 }),
            ("drop", Ty::Str { max: 256 }),
            ("state", Ty::Enum(&["lobby", "loading", "countdown", "running", "results"])),
            ("round", Ty::Int { min: 0, max: 65_535 }),
            ("clock", Ty::Number { min: 0.0, max: 1.0e9 }),
        ],
    ),
    (
        "ServerStanding",
        &[
            ("rider", Ty::Opt(&Ty::Element)),
            ("name", Ty::Str { max: 64 }),
            ("place", Ty::Int { min: 1, max: 65_536 }),
            ("state", Ty::Enum(&["racing", "finished", "failed", "left", "invalidated"])),
            ("clock", Ty::Number { min: 0.0, max: 1.0e9 }),
            ("points", Ty::Number { min: 0.0, max: 1.0e12 }),
            ("split", Ty::Opt(&Ty::Int { min: 0, max: 255 })),
        ],
    ),
    (
        "ServerRiderState",
        &[
            ("name", Ty::Str { max: 64 }),
            ("number", Ty::Int { min: 0, max: 65_535 }),
            ("progress", Ty::Enum(&["joined", "dressed", "loaded", "ready", "away"])),
            ("status", Ty::Enum(&["joining", "loading", "intro", "riding", "away", "spectating", "finished"])),
            ("ping", Ty::Int { min: 0, max: 1 << 31 }),
            ("position", Ty::Opt(&VEC3)),
            ("velocity", Ty::Opt(&VEC3)),
            ("speed", OPT_NUM),
            ("split", Ty::Opt(&Ty::Int { min: 0, max: 255 })),
            ("clock", OPT_NUM),
            ("points", OPT_NUM),
            ("finished", Ty::Bool),
        ],
    ),
    (
        "MarkerOptions",
        &[
            ("heading", Ty::Opt(&Ty::Number { min: -360.0, max: 360.0 })),
            ("size", Ty::Opt(&Ty::Number { min: 0.1, max: 100.0 })),
            ("color", Ty::Opt(&COLOUR)),
            ("visible", Ty::Opt(&Ty::Bool)),
        ],
    ),
    (
        "ServerMarkerOptions",
        &[
            ("heading", Ty::Opt(&Ty::Number { min: -360.0, max: 360.0 })),
            ("size", Ty::Opt(&Ty::Number { min: 0.1, max: 100.0 })),
            ("color", Ty::Opt(&COLOUR)),
            ("visible", Ty::Opt(&Ty::Bool)),
            ("instance", Ty::Opt(&Ty::Element)),
        ],
    ),
    (
        "TextOptions",
        &[
            ("size", Ty::Opt(&Ty::Number { min: 6.0, max: 128.0 })),
            ("align", Ty::Opt(&Ty::Enum(&["left", "center", "right"]))),
            ("shadow", Ty::Opt(&Ty::Bool)),
        ],
    ),
];

pub const COLOUR: Ty = Ty::Int { min: 0, max: 0xFFFF_FFFF };

pub fn registry() -> impl Iterator<Item = &'static Spec> {
    meta::SPECS
        .iter()
        .chain(elements::SPECS)
        .chain(events::SPECS)
        .chain(client::SPECS)
        .chain(world::SPECS)
        .chain(draw::SPECS)
        .chain(server::SPECS)
}

pub fn specs(side: Side, api: u32) -> impl Iterator<Item = &'static Spec> {
    registry().filter(move |s| s.sides.has(side) && s.api <= api)
}

pub fn server_calls() -> &'static [&'static Spec] {
    static CALLS: OnceLock<Vec<&'static Spec>> = OnceLock::new();
    CALLS.get_or_init(|| specs(Side::Server, API_LEVEL).collect())
}

fn type_of(v: &Value) -> &'static str {
    match v {
        Value::UserData(ud) if lua::element_of(ud).is_some() => "Element",
        Value::UserData(ud) if ud.borrow::<lua::TimerRef>().is_ok() => "Timer",
        Value::UserData(ud) if ud.borrow::<lua::LatentRef>().is_ok() => "LatentHandle",
        other => other.type_name(),
    }
}

fn expected(ty: &Ty) -> String {
    defs::type_text(ty)
}

fn check_one(ty: &Ty, v: Value) -> Result<Arg, String> {
    let wrong = |v: &Value| format!("{} expected, got {}", expected(ty), type_of(v));
    Ok(match ty {
        Ty::Opt(inner) => match v {
            Value::Nil => Arg::Nil,
            v => check_one(inner, v)?,
        },
        Ty::Nil => match v {
            Value::Nil => Arg::Nil,
            v => return Err(wrong(&v)),
        },
        Ty::Bool => match v {
            Value::Boolean(b) => Arg::Bool(b),
            v => return Err(wrong(&v)),
        },
        Ty::Number { min, max } => {
            let n = number(&v).ok_or_else(|| wrong(&v))?;
            if !n.is_finite() || n < *min || n > *max {
                return Err(format!("a number in {min}..{max} expected, got {n}"));
            }
            Arg::Num(n)
        }
        Ty::Int { min, max } => {
            let n = number(&v).ok_or_else(|| wrong(&v))?;
            if !n.is_finite() || n.fract() != 0.0 || n < *min as f64 || n > *max as f64 {
                return Err(format!("a whole number in {min}..{max} expected, got {n}"));
            }
            Arg::Int(n as i64)
        }
        Ty::Str { max } => Arg::Str(string(&v, *max).ok_or_else(|| wrong(&v))??),
        Ty::Name(kind) => {
            let s = string(&v, names::max_len(*kind)).ok_or_else(|| wrong(&v))??;
            names::check(*kind, &s)?;
            Arg::Str(s)
        }
        Ty::Enum(options) => {
            let s = string(&v, 64).ok_or_else(|| wrong(&v))??;
            if !options.contains(&s.as_str()) {
                return Err(format!("one of {} expected, got {s:?}", options.join(", ")));
            }
            Arg::Str(s)
        }
        Ty::Element => match &v {
            Value::UserData(ud) => Arg::Element(lua::element_of(ud).ok_or_else(|| wrong(&v))?),
            _ => return Err(wrong(&v)),
        },
        Ty::Elements => match &v {
            Value::UserData(ud) => Arg::Elements(vec![lua::element_of(ud).ok_or_else(|| wrong(&v))?]),
            Value::Table(t) => {
                let n = t.raw_len();
                if n > lua::MAX_ELEMENT_LIST || t.metatable().is_some() {
                    return Err(format!("a list of at most {} elements expected", lua::MAX_ELEMENT_LIST));
                }
                let mut out = Vec::new();
                for i in 1..=n {
                    match t.raw_get::<Value>(i).map_err(|e| e.to_string())? {
                        Value::UserData(ud) => out.push(lua::element_of(&ud).ok_or("a list of elements expected")?),
                        _ => return Err("a list of elements expected".into()),
                    }
                }
                Arg::Elements(out)
            }
            _ => return Err(wrong(&v)),
        },
        Ty::Function => match v {
            Value::Function(f) => Arg::Func(f),
            v => return Err(wrong(&v)),
        },
        Ty::Timer => match &v {
            Value::UserData(ud) => Arg::Timer(userdata::<lua::TimerRef>(ud).ok_or_else(|| wrong(&v))?.0),
            _ => return Err(wrong(&v)),
        },
        Ty::Latent => match &v {
            Value::UserData(ud) => Arg::Latent(userdata::<lua::LatentRef>(ud).ok_or_else(|| wrong(&v))?.0),
            _ => return Err(wrong(&v)),
        },
        Ty::Value | Ty::Any => Arg::Value(v),
        Ty::Vec3 { max } => {
            let t = plain_table(&v).ok_or_else(|| wrong(&v))?;
            let mut out = [f64::NAN; 3];
            for (k, x) in fields_of(t, 3)? {
                let i = ["x", "y", "z"].iter().position(|n| *n == k).ok_or_else(|| format!("a key {k:?} in a Vec3"))?;
                let n = number(&x).ok_or("a Vec3 of numbers expected")?;
                if !n.is_finite() || n.abs() > *max {
                    return Err(format!("a Vec3 within ±{max} expected, got {k} = {n}"));
                }
                out[i] = n;
            }
            if out.iter().any(|n| n.is_nan()) {
                return Err("a Vec3 needs x, y and z".into());
            }
            Arg::Vec3(out)
        }
        Ty::Record(name) => {
            let t = plain_table(&v).ok_or_else(|| wrong(&v))?;
            let Some((_, fields)) = RECORDS.iter().find(|(n, _)| n == name) else { return Err("not a record".into()) };
            let mut out = Vec::new();
            for (k, x) in fields_of(t, fields.len())? {
                let (fname, fty) =
                    fields.iter().find(|(f, _)| *f == k).ok_or_else(|| format!("{name} has no field {k:?}"))?;
                out.push((*fname, check_one(fty, x).map_err(|e| format!("{name}.{k}: {e}"))?));
            }
            Arg::Record(out)
        }
        Ty::List(_) => return Err("not a parameter type".into()),
    })
}

fn plain_table(v: &Value) -> Option<&mlua::Table> {
    match v {
        Value::Table(t) if t.metatable().is_none() => Some(t),
        _ => None,
    }
}

fn fields_of(t: &mlua::Table, max: usize) -> Result<Vec<(String, Value)>, String> {
    let mut out = Vec::new();
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        if out.len() == max {
            return Err(format!("a table of at most {max} fields expected"));
        }
        let key = match &k {
            Value::String(s) if s.as_bytes().len() <= 32 => {
                String::from_utf8(s.as_bytes().to_vec()).map_err(|_| "a field name that is not UTF-8")?
            }
            _ => return Err("fields named by short strings expected".into()),
        };
        out.push((key, v));
    }
    Ok(out)
}

fn userdata<T: Copy + 'static>(ud: &AnyUserData) -> Option<T> {
    ud.borrow::<T>().ok().map(|r| *r)
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(i) => Some(*i as f64),
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

fn string(v: &Value, max: usize) -> Option<Result<String, String>> {
    let Value::String(s) = v else { return None };
    let bytes = s.as_bytes();
    if bytes.len() > max {
        return Some(Err(format!("a string of at most {max} bytes expected, got {} bytes", bytes.len())));
    }
    Some(String::from_utf8(bytes.to_vec()).map_err(|_| "a UTF-8 string expected".to_string()))
}

pub fn check_args(spec: &Spec, args: MultiValue) -> Result<Args, String> {
    let mut it = args.into_iter();
    let mut fixed = Vec::with_capacity(spec.params.len());
    for (i, p) in spec.params.iter().enumerate() {
        let v = it.next().unwrap_or(Value::Nil);
        fixed.push(check_one(&p.ty, v).map_err(|e| format!("bad argument #{} to '{}' ({e})", i + 1, spec.name))?);
    }
    let rest: Vec<Value> = it.collect();
    match &spec.rest {
        None if !rest.is_empty() => Err(format!(
            "bad argument #{} to '{}' (at most {} arguments expected)",
            spec.params.len() + 1,
            spec.name,
            spec.params.len()
        )),
        Some(ty) => {
            if rest.len() > crate::value::MAX_LIST {
                return Err(format!("'{}': more than {} arguments", spec.name, crate::value::MAX_LIST));
            }
            for (i, v) in rest.iter().enumerate() {
                if !matches!(ty, Ty::Value | Ty::Any) {
                    check_one(ty, v.clone())
                        .map_err(|e| format!("bad argument #{} to '{}' ({e})", spec.params.len() + i + 1, spec.name))?;
                }
            }
            Ok(Args { fixed, rest })
        }
        None => Ok(Args { fixed, rest }),
    }
}

fn net_matches(ty: &Ty, v: Option<&NetValue>) -> bool {
    match (ty, v) {
        (Ty::Opt(_) | Ty::Nil | Ty::Value, None | Some(NetValue::Nil)) => true,
        (Ty::Opt(inner), v) => net_matches(inner, v),
        (Ty::Value, Some(_)) => true,
        (Ty::Bool, Some(NetValue::Bool(_))) => true,
        (Ty::Number { min, max }, Some(NetValue::Number(n))) => n.is_finite() && *n >= *min && *n <= *max,
        (Ty::Int { min, max }, Some(NetValue::Number(n))) => n.fract() == 0.0 && *n >= *min as f64 && *n <= *max as f64,
        (Ty::Str { max }, Some(NetValue::String(s))) => s.len() <= *max,
        (Ty::Name(_), Some(NetValue::String(_))) => true,
        (Ty::Enum(options), Some(NetValue::String(s))) => options.iter().any(|o| o.as_bytes() == s.as_slice()),
        (Ty::Element, Some(NetValue::Element(_))) => true,
        (Ty::Record(name), Some(NetValue::Table(entries))) => {
            let Some((_, fields)) = RECORDS.iter().find(|(n, _)| n == name) else { return false };
            let named =
                |k: &NetValue| fields.iter().any(|(f, _)| matches!(k, NetValue::String(s) if s == f.as_bytes()));
            let table = NetValue::Table(entries.clone());
            entries.iter().all(|(k, _)| named(k)) && fields.iter().all(|(f, t)| net_matches(t, table.field(f)))
        }
        (Ty::List(inner), Some(NetValue::Table(entries))) => entries
            .iter()
            .enumerate()
            .all(|(i, (k, v))| *k == NetValue::Number((i + 1) as f64) && net_matches(inner, Some(v))),
        _ => false,
    }
}

fn ret_matches(ty: &Ty, r: &Ret) -> bool {
    match (ty, r) {
        (Ty::Opt(_) | Ty::Nil | Ty::Value, Ret::Nil) => true,
        (Ty::Opt(inner), r) => ret_matches(inner, r),
        (Ty::Bool, Ret::Bool(_)) => true,
        (Ty::Number { min, max }, Ret::Num(n)) => n.is_finite() && *n >= *min && *n <= *max,
        (Ty::Int { min, max }, Ret::Num(n)) => n.fract() == 0.0 && *n >= *min as f64 && *n <= *max as f64,
        (Ty::Str { .. } | Ty::Name(_), Ret::Str(_)) => true,
        (Ty::Enum(options), Ret::Str(s)) => options.contains(&s.as_str()),
        (Ty::Element, Ret::Element(_)) | (Ty::Timer, Ret::Timer(_)) | (Ty::Latent, Ret::Latent(_)) => true,
        (Ty::List(inner), Ret::Elements(_)) => matches!(inner, Ty::Element),
        (Ty::List(inner), Ret::Strings(_)) => matches!(inner, Ty::Str { .. } | Ty::Name(_)),
        (Ty::List(inner), Ret::Latents(_)) => matches!(inner, Ty::Latent),
        (Ty::Value, Ret::Value(_)) => true,
        (Ty::Record(_) | Ty::List(_), Ret::Value(v)) => net_matches(ty, Some(v)),
        _ => false,
    }
}

fn into_lua(lua: &Lua, rets: Vec<Ret>) -> mlua::Result<MultiValue> {
    let mut out = MultiValue::new();
    for r in rets {
        match r {
            Ret::Nil => out.push_back(Value::Nil),
            Ret::Bool(b) => out.push_back(Value::Boolean(b)),
            Ret::Num(n) => out.push_back(Value::Number(n)),
            Ret::Str(s) => out.push_back(Value::String(lua.create_string(s)?)),
            Ret::Element(e) => out.push_back(lua::element_value(lua, e)?),
            Ret::Timer(t) => out.push_back(Value::UserData(lua.create_userdata(lua::TimerRef(t))?)),
            Ret::Latent(t) => out.push_back(Value::UserData(lua.create_userdata(lua::LatentRef(t))?)),
            Ret::Elements(list) => {
                let t = lua.create_table_with_capacity(list.len(), 0)?;
                for (i, e) in list.into_iter().enumerate() {
                    t.raw_set(i + 1, lua::element_value(lua, e)?)?;
                }
                out.push_back(Value::Table(t));
            }
            Ret::Strings(list) => out.push_back(Value::Table(lua.create_sequence_from(list)?)),
            Ret::Latents(list) => {
                let t = lua.create_table_with_capacity(list.len(), 0)?;
                for (i, h) in list.into_iter().enumerate() {
                    t.raw_set(i + 1, lua.create_userdata(lua::LatentRef(h))?)?;
                }
                out.push_back(Value::Table(t));
            }
            Ret::Value(v) => out.push_back(to_lua(lua, &v, &lua::element_value)?),
            Ret::Lua(values) => out.extend(values),
        }
    }
    Ok(out)
}

fn call_right(ctx: &Ctx, index: usize) -> Result<(), String> {
    let host = ctx.host()?;
    let mut w = host.lock();
    crate::host::server::rights::call_allowed(&mut w, &ctx.package, index)
}

fn bind(lua: &Lua, spec: &'static Spec, ctx: &Ctx, right: Option<usize>) -> mlua::Result<Function> {
    let ctx = ctx.clone();
    lua.create_function(move |lua, args: MultiValue| {
        if let Some(index) = right {
            call_right(&ctx, index).map_err(|e| mlua::Error::runtime(format!("{}: {e}", spec.name)))?;
        }
        let args = check_args(spec, args).map_err(mlua::Error::runtime)?;
        let rets = match (spec.call)(&ctx, lua, args) {
            Ok(r) => r,
            Err(ApiError::Refused(e)) => return Err(mlua::Error::runtime(format!("{}: {e}", spec.name))),
            Err(ApiError::Lua(e)) => return Err(e),
        };
        let fits = spec.any_results
            || (rets.len() == spec.returns.len() && rets.iter().zip(spec.returns).all(|(r, t)| ret_matches(t, r)));
        if !fits {
            let message = format!("{}: the host returned values its definition does not allow", spec.name);
            return Err(mlua::Error::runtime(message));
        }
        into_lua(lua, rets)
    })
}

pub fn install(lua: &Lua, globals: &Table, ctx: &Ctx) -> mlua::Result<()> {
    lua::init(lua)?;
    let privileged = crate::host::server::rights::PRIVILEGED;
    for (index, spec) in specs(ctx.side, API_LEVEL).enumerate() {
        let right = (ctx.side == Side::Server && !privileged.contains(&spec.name)).then_some(index);
        let f = bind(lua, spec, ctx, right)?;
        match spec.name.split_once('.') {
            Some((lib, member)) => {
                let t = match globals.raw_get::<Value>(lib)? {
                    Value::Table(t) => t,
                    _ => {
                        let t = lua.create_table()?;
                        globals.raw_set(lib, &t)?;
                        t
                    }
                };
                t.raw_set(member, f)?;
            }
            None => globals.raw_set(spec.name, f)?,
        }
    }
    globals.raw_set("root", lua::element_value(lua, crate::host::elements::ROOT)?)?;
    globals.raw_set("resourceRoot", lua::element_value(lua, ctx.root)?)?;
    if ctx.side == Side::Client {
        let player = ctx.host().ok().and_then(|h| h.lock().local_player);
        let v = match player {
            Some(p) => lua::element_value(lua, p)?,
            None => Value::Nil,
        };
        globals.raw_set("localPlayer", v)?;
    }
    globals.raw_set("exports", meta::exports_table(lua, ctx)?)?;
    for lib in ["json", "storage", "draw"] {
        if let Value::Table(t) = globals.raw_get::<Value>(lib)? {
            t.set_readonly(true);
        }
    }
    Ok(())
}

pub(crate) fn copy(v: &Value, limits: &crate::value::ValueLimits) -> Result<NetValue, String> {
    from_lua(v, limits).map_err(|e| format!("the value cannot be copied: {e}"))
}

#[cfg(test)]
mod tests;
