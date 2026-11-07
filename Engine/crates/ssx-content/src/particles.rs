use ssx_formats::lynx::{Action, Effect, Group, Param, ParticleSystem, State, Value};
use ssx_formats::lynx_schema::{self, ParamType};

use crate::id::{ContentId, Kind};
use crate::material::Problem;

pub const MAX_EFFECTS: usize = 16;
pub const MAX_GROUPS: usize = 64;
pub const MAX_ACTIONS: usize = 256;
pub const MAX_PARAMETERS: usize = 1024;
pub const MAX_STATES: usize = 16;
pub const MAX_INDEX: u32 = 1024;

pub const TEXTURE_PARAMETER: &str = "sTexture";

pub const SPRAY_PREFIX: &str = "snowspray/";

fn names(t: &toml::Table, keys: &[&str]) -> Result<(), String> {
    match t.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown key {} (one of {})", ssx_core::text::quoted(k), keys.join(", "))),
        None => Ok(()),
    }
}

fn text<'a>(t: &'a toml::Table, key: &str, required: bool) -> Result<Option<&'a str>, String> {
    match t.get(key) {
        Some(toml::Value::String(s)) if !s.is_empty() && s.len() <= 63 && !s.chars().any(char::is_control) => {
            Ok(Some(s))
        }
        Some(_) => Err(format!("{key}: a name of 1 to 63 bytes")),
        None if required => Err(format!("{key}: missing")),
        None => Ok(None),
    }
}

fn list<'a>(t: &'a toml::Table, key: &str, max: usize) -> Result<&'a [toml::Value], String> {
    match t.get(key) {
        Some(toml::Value::Array(a)) if a.len() <= max => Ok(a),
        Some(toml::Value::Array(a)) => Err(format!("{key}: {} entries (at most {max})", a.len())),
        Some(_) => Err(format!("{key}: an array")),
        None => Ok(&[]),
    }
}

fn tables<'a>(t: &'a toml::Table, key: &str, max: usize) -> Result<Vec<&'a toml::Table>, String> {
    list(t, key, max)?
        .iter()
        .enumerate()
        .map(|(i, v)| v.as_table().ok_or_else(|| format!("{key}[{i}]: a table")))
        .collect()
}

fn number(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Float(x) if x.is_finite() && x.abs() <= f64::from(f32::MAX) => Some(*x),
        toml::Value::Integer(i) if i.unsigned_abs() < (1 << 24) => Some(*i as f64),
        _ => None,
    }
}

fn param(class: &str, v: &toml::Value) -> Result<Param, String> {
    let a = v
        .as_array()
        .ok_or("a parameter is [name, value], [name, index, value] or [name, index, value, numElements]")?;
    let name = a.first().and_then(toml::Value::as_str).ok_or("a parameter starts with its name")?;
    if name.is_empty() || name.len() > 63 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{}: a parameter name is 1 to 63 letters, digits or `_`", ssx_core::text::quoted(name)));
    }
    let int = |v: &toml::Value, what: &str| {
        v.as_integer()
            .and_then(|i| u32::try_from(i).ok())
            .filter(|i| *i <= MAX_INDEX)
            .ok_or_else(|| format!("{name}: {what} is 0 to {MAX_INDEX}"))
    };
    let (index, value, num_elements) = match a.len() {
        2 => (0, &a[1], None),
        3 => (int(&a[1], "the index")?, &a[2], None),
        4 => (int(&a[1], "the index")?, &a[2], Some(int(&a[3], "numElements")?)),
        n => return Err(format!("{name}: {n} items (2 to 4)")),
    };
    let schema = lynx_schema::class(class);
    let ty = ssx_formats::lynx::param_type(schema, name)
        .ok_or_else(|| format!("{name}: no type (the class has no such parameter and the name's prefix types none)"))?;
    let value = match ty {
        ParamType::Bool => Value::Bool(value.as_bool().ok_or_else(|| format!("{name}: true or false"))?),
        ParamType::Int => Value::Int(
            value
                .as_integer()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or_else(|| format!("{name}: a 32-bit integer"))?,
        ),
        ParamType::Float => Value::Float(number(value).ok_or_else(|| format!("{name}: a finite number"))? as f32),
        ParamType::Vector2 | ParamType::Vector3 | ParamType::Vector4 => {
            let (lo, hi) = match ty {
                ParamType::Vector2 => (2, 2),
                ParamType::Vector3 => (3, 3),
                _ => (3, 4),
            };
            let xs: Option<Vec<f32>> =
                value.as_array().and_then(|a| a.iter().map(|x| number(x).map(|v| v as f32)).collect());
            match xs.filter(|x| (lo..=hi).contains(&x.len())) {
                Some(x) => Value::Vector(x),
                None => return Err(format!("{name}: {lo} to {hi} finite numbers")),
            }
        }
        ParamType::Reference => {
            let s = value.as_str().filter(|s| !s.is_empty() && s.len() <= 255 && !s.chars().any(char::is_control));
            Value::Reference(s.ok_or_else(|| format!("{name}: a `group/action/parameter` path"))?.to_string())
        }
        ParamType::String64 if name == TEXTURE_PARAMETER => {
            let s = value.as_str().ok_or_else(|| format!("{name}: a texture id"))?;
            let id: ContentId = s.parse().map_err(|e| format!("{name}: {e}"))?;
            if id.kind() != Kind::Texture {
                return Err(format!("{name}: {id} is not a texture id"));
            }
            Value::String(id.to_string())
        }
        ParamType::String64 => {
            let s = value.as_str().filter(|s| s.len() < 64 && !s.chars().any(char::is_control));
            Value::String(s.ok_or_else(|| format!("{name}: a text of at most 63 bytes"))?.to_string())
        }
        ParamType::Matrix33 | ParamType::Matrix44 => {
            return Err(format!("{name}: a matrix (the game's loader sets none from a file)"));
        }
    };
    let raw = raw_of(&value);
    Ok(Param { name: name.to_string(), index, num_elements, raw, value, exact: true })
}

fn raw_of(v: &Value) -> String {
    match v {
        Value::Bool(b) => if *b { "1" } else { "0" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Vector(xs) => format!("{{ {} }}", xs.iter().map(f32::to_string).collect::<Vec<_>>().join(", ")),
        Value::Reference(s) | Value::String(s) => s.clone(),
    }
}

fn params(class: &str, t: &toml::Table, at: &str) -> Result<Vec<Param>, String> {
    list(t, "parameters", MAX_PARAMETERS)
        .map_err(|e| format!("{at}: {e}"))?
        .iter()
        .enumerate()
        .map(|(i, v)| param(class, v).map_err(|e| format!("{at}.parameters[{i}]: {e}")))
        .collect()
}

pub fn system_from_table(t: &toml::Table) -> Result<ParticleSystem, Vec<String>> {
    let run = || -> Result<ParticleSystem, String> {
        names(t, &["version", "effect"])?;
        let version = text(t, "version", false)?.map(str::to_string);
        let mut effects = Vec::new();
        for (e, et) in tables(t, "effect", MAX_EFFECTS)?.into_iter().enumerate() {
            let at = format!("effect[{e}]");
            names(et, &["name", "parameters", "group"]).map_err(|m| format!("{at}: {m}"))?;
            let name = text(et, "name", true).map_err(|m| format!("{at}: {m}"))?.unwrap_or_default().to_string();
            let mut groups = Vec::new();
            for (g, gt) in tables(et, "group", MAX_GROUPS).map_err(|m| format!("{at}: {m}"))?.into_iter().enumerate() {
                let at = format!("{at}.group[{g}]");
                names(gt, &["name", "parameters", "action"]).map_err(|m| format!("{at}: {m}"))?;
                let gname = text(gt, "name", true).map_err(|m| format!("{at}: {m}"))?.unwrap_or_default().to_string();
                let mut actions = Vec::new();
                let list = tables(gt, "action", MAX_ACTIONS).map_err(|m| format!("{at}: {m}"))?;
                for (a, atab) in list.into_iter().enumerate() {
                    let at = format!("{at}.action[{a}]");
                    names(atab, &["class", "instance", "name", "parameters", "state"])
                        .map_err(|m| format!("{at}: {m}"))?;
                    let class = text(atab, "class", true).map_err(|m| format!("{at}: {m}"))?.unwrap_or_default();
                    if lynx_schema::class(class).is_none_or(|c| c.vtable.is_none()) {
                        return Err(format!(
                            "{at}: class {} is not a particle action of the game's library",
                            ssx_core::text::quoted(class)
                        ));
                    }
                    let mut states = Vec::new();
                    for (s, st) in
                        tables(atab, "state", MAX_STATES).map_err(|m| format!("{at}: {m}"))?.into_iter().enumerate()
                    {
                        let at = format!("{at}.state[{s}]");
                        names(st, &["id", "parameters"]).map_err(|m| format!("{at}: {m}"))?;
                        let id =
                            text(st, "id", true).map_err(|m| format!("{at}: {m}"))?.unwrap_or_default().to_string();
                        states.push(State { id, params: params(class, st, &at)? });
                    }
                    actions.push(Action {
                        class_name: class.to_string(),
                        instance_name: text(atab, "instance", false)
                            .map_err(|m| format!("{at}: {m}"))?
                            .map(str::to_string),
                        name: text(atab, "name", false).map_err(|m| format!("{at}: {m}"))?.map(str::to_string),
                        params: params(class, atab, &at)?,
                        states,
                    });
                }
                groups.push(Group { name: gname, params: params("ParticleGroup", gt, &at)?, actions });
            }
            effects.push(Effect { name, params: params("ParticleEffect", et, &at)?, groups });
        }
        if effects.is_empty() {
            return Err("a system has at least one [[effect]]".into());
        }
        Ok(ParticleSystem { version, effects })
    };
    run().map_err(|e| vec![e])
}

pub fn parse_system(bytes: &[u8]) -> Result<ParticleSystem, Vec<Problem>> {
    let table = crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    system_from_table(&table).map_err(|ps| ps.into_iter().map(|m| (0, 0, m)).collect())
}

fn value_toml(name: &str, v: &Value, texture: &dyn Fn(&str) -> Option<String>) -> Result<toml::Value, String> {
    Ok(match v {
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Int(i) => toml::Value::Integer(i64::from(*i)),
        Value::Float(f) => toml::Value::Float(f64::from(*f)),
        Value::Vector(xs) => toml::Value::Array(xs.iter().map(|x| toml::Value::Float(f64::from(*x))).collect()),
        Value::Reference(s) => toml::Value::String(s.clone()),
        Value::String(s) if name == TEXTURE_PARAMETER => toml::Value::String(
            texture(s).ok_or_else(|| format!("{name} {}: no texture id", ssx_core::text::quoted(s)))?,
        ),
        Value::String(s) => toml::Value::String(s.clone()),
    })
}

fn params_toml(ps: &[Param], texture: &dyn Fn(&str) -> Option<String>) -> Result<toml::Value, String> {
    let mut out = Vec::with_capacity(ps.len());
    for p in ps {
        let mut a = vec![toml::Value::String(p.name.clone())];
        if p.index != 0 || p.num_elements.is_some() {
            a.push(toml::Value::Integer(i64::from(p.index)));
        }
        a.push(value_toml(&p.name, &p.value, texture)?);
        if let Some(n) = p.num_elements {
            a.push(toml::Value::Integer(i64::from(n)));
        }
        out.push(toml::Value::Array(a));
    }
    Ok(toml::Value::Array(out))
}

pub fn system_to_table(s: &ParticleSystem, texture: &dyn Fn(&str) -> Option<String>) -> Result<toml::Table, String> {
    let mut t = toml::Table::new();
    if let Some(v) = &s.version {
        t.insert("version".into(), toml::Value::String(v.clone()));
    }
    let mut effects = Vec::new();
    for e in &s.effects {
        let mut et = toml::Table::new();
        et.insert("name".into(), toml::Value::String(e.name.clone()));
        et.insert("parameters".into(), params_toml(&e.params, texture)?);
        let mut groups = Vec::new();
        for g in &e.groups {
            let mut gt = toml::Table::new();
            gt.insert("name".into(), toml::Value::String(g.name.clone()));
            gt.insert("parameters".into(), params_toml(&g.params, texture)?);
            let mut actions = Vec::new();
            for a in &g.actions {
                let mut at = toml::Table::new();
                at.insert("class".into(), toml::Value::String(a.class_name.clone()));
                if let Some(i) = &a.instance_name {
                    at.insert("instance".into(), toml::Value::String(i.clone()));
                }
                if let Some(n) = &a.name {
                    at.insert("name".into(), toml::Value::String(n.clone()));
                }
                at.insert("parameters".into(), params_toml(&a.params, texture)?);
                if !a.states.is_empty() {
                    let states = a
                        .states
                        .iter()
                        .map(|st| {
                            let mut s = toml::Table::new();
                            s.insert("id".into(), toml::Value::String(st.id.clone()));
                            s.insert("parameters".into(), params_toml(&st.params, texture)?);
                            Ok(toml::Value::Table(s))
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    at.insert("state".into(), toml::Value::Array(states));
                }
                actions.push(toml::Value::Table(at));
            }
            gt.insert("action".into(), toml::Value::Array(actions));
            groups.push(toml::Value::Table(gt));
        }
        et.insert("group".into(), toml::Value::Array(groups));
        effects.push(toml::Value::Table(et));
    }
    t.insert("effect".into(), toml::Value::Array(effects));
    Ok(t)
}

pub const SPRAY_SURFACES: [&str; 3] = ["snow", "packed", "ice"];
pub const SPRAY_KINDS: [&str; 6] = ["carve", "impact", "impact_awesome", "kickoff", "kickoff_awesome", "trail"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpraySlot {
    pub node: u32,
    pub particles: Option<ContentId>,
    pub effect: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SprayTable {
    pub slots: [[Option<SpraySlot>; 6]; 3],
}

impl SprayTable {
    pub fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        for (s, row) in self.slots.iter().enumerate() {
            let mut r = toml::Table::new();
            for (k, slot) in row.iter().enumerate() {
                let Some(slot) = slot else { continue };
                let mut st = toml::Table::new();
                st.insert("node".into(), toml::Value::String(format!("0x{:08x}", slot.node)));
                if let Some(p) = &slot.particles {
                    st.insert("particles".into(), toml::Value::String(p.to_string()));
                }
                if let Some(e) = &slot.effect {
                    st.insert("effect".into(), toml::Value::String(e.clone()));
                }
                r.insert(SPRAY_KINDS[k].into(), toml::Value::Table(st));
            }
            t.insert(SPRAY_SURFACES[s].into(), toml::Value::Table(r));
        }
        t
    }

    pub fn from_table(t: &toml::Table) -> Result<SprayTable, Vec<String>> {
        let mut out = SprayTable::default();
        let mut problems = Vec::new();
        if let Err(e) = names(t, &SPRAY_SURFACES) {
            problems.push(e);
        }
        for (s, surface) in SPRAY_SURFACES.iter().enumerate() {
            let Some(row) = t.get(*surface) else { continue };
            let Some(row) = row.as_table() else {
                problems.push(format!("{surface}: a table of kinds"));
                continue;
            };
            if let Err(e) = names(row, &SPRAY_KINDS) {
                problems.push(format!("{surface}: {e}"));
            }
            for (k, kind) in SPRAY_KINDS.iter().enumerate() {
                let Some(slot) = row.get(*kind) else { continue };
                match slot_of(slot) {
                    Ok(v) => out.slots[s][k] = Some(v),
                    Err(e) => problems.push(format!("{surface}.{kind}: {e}")),
                }
            }
        }
        if problems.is_empty() { Ok(out) } else { Err(problems) }
    }
}

struct SlotFields {
    node: Option<u32>,
    particles: Option<ContentId>,
    effect: Option<String>,
}

fn slot_fields(v: &toml::Value, whole: bool) -> Result<SlotFields, String> {
    let t = v.as_table().ok_or("a table { node, particles, effect }")?;
    names(t, &["node", "particles", "effect"])?;
    let node = match t.get("node") {
        Some(v) => Some(
            v.as_str()
                .and_then(crate::lighting::hex_key)
                .ok_or("node: a ge_particlenode's key as \"0x<8 hex digits>\"")?,
        ),
        None if whole => return Err("node: a ge_particlenode's key as \"0x<8 hex digits>\"".into()),
        None => None,
    };
    let particles = match t.get("particles") {
        Some(v) => {
            let id: ContentId =
                v.as_str().ok_or("particles: a particles id")?.parse().map_err(|e| format!("particles: {e}"))?;
            if id.kind() != Kind::Particles || id.path().starts_with(SPRAY_PREFIX) {
                return Err(format!("particles: {id} is not a particle system's id"));
            }
            Some(id)
        }
        None => None,
    };
    let effect = text(t, "effect", false)?.map(str::to_string);
    Ok(SlotFields { node, particles, effect })
}

fn slot_of(v: &toml::Value) -> Result<SpraySlot, String> {
    let SlotFields { node, particles, effect } = slot_fields(v, true)?;
    if particles.is_some() != effect.is_some() {
        return Err("particles and effect go together (the system and the effect in it)".into());
    }
    Ok(SpraySlot { node: node.unwrap_or_default(), particles, effect })
}

pub fn parse_spray_patch(bytes: &[u8]) -> Result<toml::Table, Vec<Problem>> {
    let table = crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let lines = crate::lighting::key_lines(bytes);
    let mut problems = Vec::new();
    if let Err(e) = names(&table, &SPRAY_SURFACES) {
        problems.push(e);
    }
    for surface in SPRAY_SURFACES {
        let Some(row) = table.get(surface) else { continue };
        let Some(row) = row.as_table() else {
            problems.push(format!("{surface}: a table of kinds"));
            continue;
        };
        if let Err(e) = names(row, &SPRAY_KINDS) {
            problems.push(format!("{surface}: {e}"));
        }
        for kind in SPRAY_KINDS {
            if let Some(slot) = row.get(kind)
                && let Err(e) = slot_fields(slot, false)
            {
                problems.push(format!("{surface}.{kind}: {e}"));
            }
        }
    }
    if problems.is_empty() {
        return Ok(table);
    }
    Err(problems
        .into_iter()
        .map(|m| {
            let key = m.split(['.', ':']).next().unwrap_or("");
            let (l, c) = lines.get(key).copied().unwrap_or((0, 0));
            (l, c, m)
        })
        .collect())
}

pub fn parse_spray(bytes: &[u8]) -> Result<SprayTable, Vec<Problem>> {
    let table = crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let lines = crate::lighting::key_lines(bytes);
    SprayTable::from_table(&table).map_err(|ps| {
        ps.into_iter()
            .map(|m| {
                let key = m.split(['.', ':']).next().unwrap_or("");
                let (l, c) = lines.get(key).copied().unwrap_or((0, 0));
                (l, c, m)
            })
            .collect()
    })
}

pub fn is_spray(id: &ContentId) -> bool {
    id.kind() == Kind::Particles && id.path().starts_with(SPRAY_PREFIX)
}

#[cfg(test)]
mod tests;
