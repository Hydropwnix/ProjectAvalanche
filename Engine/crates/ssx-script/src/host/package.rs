use ssx_content::PackageSource;
use ssx_content::glob::{Budget, Pattern};
use ssx_content::manifest::{Files, Scripts};

use super::Side;
use super::limits::HostLimits;
use crate::{FileMap, MAX_SOURCE};

const PATTERN_BUDGET: u64 = 50_000_000;

struct Input<'a> {
    name: &'a str,
    scripts: &'a Scripts,
    files_cfg: &'a Files,
    files: &'a [String],
    size: &'a dyn Fn(&str) -> u64,
    read: &'a dyn Fn(&str) -> Result<Vec<u8>, String>,
}

#[derive(Clone, Debug)]
pub struct PackageScripts {
    pub name: String,
    pub api: u32,
    pub settings: toml::Table,
    pub scripts: Vec<(String, String)>,
    pub modules: FileMap,
    pub client_scripts: bool,
}

pub fn check_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        && name.as_bytes()[0].is_ascii_alphanumeric();
    if !ok {
        return Err(format!("{:?} is not a package name", crate::env::cap(name, 64)));
    }
    ssx_content::path::check_package_path(name, false).map_err(|e| format!("{name:?} is not a package name: {e}"))
}

fn patterns(list: &[String]) -> Result<Vec<Pattern>, String> {
    list.iter().map(|p| Pattern::parse(p).map_err(|e| format!("[scripts] / [files] pattern {p:?}: {e}"))).collect()
}

fn matching(pats: &[Pattern], files: &[String], budget: &mut Budget) -> Result<Vec<Vec<String>>, String> {
    pats.iter()
        .map(|p| {
            let mut hit = Vec::new();
            for f in files {
                if p.matches(f, budget).map_err(|e| e.to_string())? {
                    hit.push(f.clone());
                }
            }
            hit.sort();
            Ok(hit)
        })
        .collect()
}

impl PackageScripts {
    pub fn from_source(src: &PackageSource, side: Side, limits: &HostLimits) -> Result<Self, String> {
        let m = &src.manifest;
        let files: Vec<String> = src.tree().files().map(str::to_string).collect();
        let size = |path: &str| src.tree().size(path).unwrap_or(u64::MAX);
        let read = |path: &str| src.tree().read(path, MAX_SOURCE as u64).map_err(|e| e.to_string());
        let input = Input {
            name: &m.resource.name,
            scripts: &m.scripts,
            files_cfg: &m.files,
            files: &files,
            size: &size,
            read: &read,
        };
        let mut p = Self::build(input, m.settings.clone(), side, limits)?;
        p.api = m.resource.api;
        Ok(p)
    }

    pub fn from_memory(
        name: &str,
        side: Side,
        scripts: &Scripts,
        files: &Files,
        settings: toml::Table,
        contents: &[(String, String)],
        limits: &HostLimits,
    ) -> Result<Self, String> {
        let paths: Vec<String> = contents.iter().map(|(p, _)| p.clone()).collect();
        let find = |path: &str| contents.iter().find(|(p, _)| p == path).map(|(_, s)| s);
        let size = |path: &str| find(path).map_or(u64::MAX, |s| s.len() as u64);
        let read =
            |path: &str| find(path).map(|s| s.as_bytes().to_vec()).ok_or_else(|| format!("{path}: no such file"));
        let input = Input { name, scripts, files_cfg: files, files: &paths, size: &size, read: &read };
        Self::build(input, settings, side, limits)
    }

    fn build(input: Input, settings: toml::Table, side: Side, limits: &HostLimits) -> Result<Self, String> {
        let Input { name, scripts, files_cfg, files, size, read } = input;
        check_name(name)?;
        let mut budget = Budget(PATTERN_BUDGET);
        let luau: Vec<String> =
            files.iter().filter(|f| f.ends_with(".luau") && !f.ends_with(".d.luau")).cloned().collect();
        let side_list = match side {
            Side::Server => &scripts.server,
            Side::Client => &scripts.client,
        };
        let shared = matching(&patterns(&scripts.shared)?, &luau, &mut budget)?;
        let own = matching(&patterns(side_list)?, &luau, &mut budget)?;
        let client_scripts = match side {
            Side::Client => own.iter().chain(&shared).any(|l| !l.is_empty()),
            Side::Server => {
                shared.iter().any(|l| !l.is_empty())
                    || matching(&patterns(&scripts.client)?, &luau, &mut budget)?.iter().any(|l| !l.is_empty())
            }
        };
        let mut order: Vec<String> = Vec::new();
        for f in shared.into_iter().chain(own).flatten() {
            if !order.contains(&f) {
                order.push(f);
            }
        }
        let modules: Vec<String> = match side {
            Side::Server => luau.clone(),
            Side::Client => {
                let mut visible = Vec::new();
                for list in [&scripts.client, &scripts.shared, &files_cfg.include] {
                    visible.extend(matching(&patterns(list)?, &luau, &mut budget)?.into_iter().flatten());
                }
                let hidden: Vec<String> =
                    matching(&patterns(&files_cfg.server_only)?, &luau, &mut budget)?.into_iter().flatten().collect();
                if let Some(f) = order.iter().find(|f| hidden.contains(f)) {
                    return Err(format!("{f}: a client script is also server_only"));
                }
                visible.sort();
                visible.dedup();
                visible.into_iter().filter(|f| !hidden.contains(f)).collect()
            }
        };
        let mut lower: Vec<(String, &String)> = modules.iter().map(|m| (m.to_ascii_lowercase(), m)).collect();
        lower.sort();
        if let Some(w) = lower.windows(2).find(|w| w[0].0 == w[1].0) {
            return Err(format!("{} and {}: two spellings of one path", w[0].1, w[1].1));
        }
        if modules.len() > limits.source_files {
            return Err(format!("{} script files (at most {})", modules.len(), limits.source_files));
        }
        let mut total = 0u64;
        for m in &modules {
            let n = size(m);
            if n > MAX_SOURCE as u64 {
                return Err(format!("{m}: over {MAX_SOURCE} bytes"));
            }
            total += n;
        }
        if total > limits.source_bytes as u64 {
            return Err(format!("{total} bytes of scripts (at most {})", limits.source_bytes));
        }
        let mut map = FileMap::new();
        for m in &modules {
            let bytes = read(m)?;
            let text = String::from_utf8(bytes).map_err(|_| format!("{m}: not UTF-8"))?;
            map.insert(m.clone(), text);
        }
        let scripts = order.iter().filter_map(|p| map.get(p).map(|s| (p.clone(), s.clone()))).collect();
        Ok(PackageScripts {
            name: name.to_string(),
            api: crate::api::API_LEVEL,
            settings,
            scripts,
            modules: map,
            client_scripts,
        })
    }
}

pub fn setting_value(v: &toml::Value, depth: usize) -> Result<crate::value::NetValue, String> {
    use crate::value::{MAX_DEPTH, MAX_STRING, NetValue};
    const EXACT: i64 = 1 << 53;
    let nested = matches!(v, toml::Value::Array(_) | toml::Value::Table(_));
    if nested && depth >= MAX_DEPTH {
        return Err(format!("tables nested deeper than {MAX_DEPTH}"));
    }
    Ok(match v {
        toml::Value::String(s) if s.len() > MAX_STRING => return Err(format!("a string over {MAX_STRING} bytes")),
        toml::Value::String(s) => NetValue::String(s.as_bytes().to_vec()),
        toml::Value::Integer(i) if !(-EXACT..=EXACT).contains(i) => {
            return Err(format!("the integer {i} is beyond ±2^53 (a script's numbers would round it)"));
        }
        toml::Value::Integer(i) => NetValue::Number(*i as f64),
        toml::Value::Float(f) => NetValue::Number(*f),
        toml::Value::Boolean(b) => NetValue::Bool(*b),
        toml::Value::Datetime(d) => NetValue::String(d.to_string().into_bytes()),
        toml::Value::Array(a) => {
            let mut entries = Vec::new();
            for (i, x) in a.iter().enumerate() {
                let key = u32::try_from(i + 1).map_err(|_| "an array of more than 2^32 items")?;
                entries.push((NetValue::Number(f64::from(key)), setting_value(x, depth + 1)?));
            }
            NetValue::Table(entries)
        }
        toml::Value::Table(t) => {
            let mut entries = Vec::new();
            for (k, x) in t {
                entries.push((NetValue::String(k.as_bytes().to_vec()), setting_value(x, depth + 1)?));
            }
            NetValue::Table(entries)
        }
    })
}
