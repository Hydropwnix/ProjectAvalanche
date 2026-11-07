use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, RwLock};

use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapter, RenderDevice};
use bevy::shader::Shader;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use ssx_content::shader::{Accepted, HookFamily, Limits, check_hook};

use bevy::shader::ShaderDefVal;

use super::compose::{self, Composer, DeviceCaps, Scope, def_key, entries, variant_modules, variant_path};
use super::sets::CRATE;

#[derive(Clone, Debug)]
pub enum HookState {
    Composing,
    Ready { shaders: BTreeMap<&'static str, Handle<Shader>>, accepted: Accepted },
    Refused(String),
}

static HOOKS: RwLock<BTreeMap<u64, HookState>> = RwLock::new(BTreeMap::new());

pub fn state(hash: u64) -> Option<HookState> {
    HOOKS.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(&hash).cloned()
}

fn set_state(hash: u64, s: HookState) {
    HOOKS.write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(hash, s);
}

static SETTLED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static LAST_GOOD: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

pub fn settled() -> u64 {
    SETTLED.load(std::sync::atomic::Ordering::Acquire)
}

pub fn drawn(hash: u64, key: &str) -> Option<u64> {
    let mut last = LAST_GOOD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match state(hash) {
        Some(HookState::Ready { .. }) => {
            last.insert(key.to_string(), hash);
            Some(hash)
        }
        _ => last.get(key).copied().filter(|h| matches!(state(*h), Some(HookState::Ready { .. }))),
    }
}

type SetKey = (String, String);

struct Sets {
    admitted: BTreeMap<u64, BTreeSet<SetKey>>,
    denied: BTreeMap<u64, BTreeSet<SetKey>>,
    wanted: Vec<(u64, String, Vec<ShaderDefVal>)>,
    in_flight: BTreeSet<(u64, SetKey)>,
    checking: usize,
    epochs: BTreeMap<u64, u64>,
}

static SETS: Mutex<Sets> = Mutex::new(Sets {
    admitted: BTreeMap::new(),
    denied: BTreeMap::new(),
    wanted: Vec::new(),
    in_flight: BTreeSet::new(),
    checking: 0,
    epochs: BTreeMap::new(),
});

fn sets() -> std::sync::MutexGuard<'static, Sets> {
    SETS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn admit(hash: u64, entry: &str, defs: &[ShaderDefVal]) -> bool {
    let key = (entry.to_string(), def_key(defs));
    let mut s = sets();
    if s.admitted.get(&hash).is_some_and(|a| a.contains(&key)) {
        return true;
    }
    if s.denied.get(&hash).is_some_and(|d| d.contains(&key)) || s.in_flight.contains(&(hash, key.clone())) {
        return false;
    }
    if !s.wanted.iter().any(|(h, e, d)| *h == hash && *e == entry && def_key(d) == key.1) {
        s.wanted.push((hash, entry.to_string(), defs.to_vec()));
    }
    false
}

pub fn epoch(hash: u64) -> u64 {
    sets().epochs.get(&hash).copied().unwrap_or(0)
}

pub fn hook_defs(hash: u64) -> Option<Vec<ShaderDefVal>> {
    match state(hash)? {
        HookState::Ready { accepted, .. } => Some(compose::hook_flags(&accepted)),
        _ => None,
    }
}

pub fn composing() -> bool {
    let s = sets();
    if !s.wanted.is_empty() || s.checking > 0 {
        return true;
    }
    drop(s);
    HOOKS.read().unwrap_or_else(std::sync::PoisonError::into_inner).values().any(|s| matches!(s, HookState::Composing))
}

pub fn summary() -> String {
    let hooks = HOOKS.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    if hooks.is_empty() {
        return String::new();
    }
    let (mut composing, mut ready, mut refused, mut why) = (0, 0, 0, None);
    for s in hooks.values() {
        match s {
            HookState::Composing => composing += 1,
            HookState::Ready { .. } => ready += 1,
            HookState::Refused(w) => {
                refused += 1;
                why = Some(w.clone());
            }
        }
    }
    let mut line = format!("shader hooks: {ready} drawn, {composing} composing, {refused} refused");
    if let Some(w) = why {
        line.push_str(&format!("\n  SHADER REFUSED: {}", w.chars().take(150).collect::<String>()));
    }
    line
}

pub fn variant_shader(hash: u64, entry: &str) -> Option<(Handle<Shader>, bool, bool)> {
    match state(hash)? {
        HookState::Ready { shaders, accepted } => {
            Some((shaders.get(entry)?.clone(), accepted.surface.is_some(), accepted.vertex.is_some()))
        }
        _ => None,
    }
}

pub fn hook_hash(family: HookFamily, text: &str) -> u64 {
    let h = ssx_content::ContentHash::of(format!("{}\n{text}", family.as_str()).as_bytes());
    let bytes = h.to_string();
    u64::from_str_radix(&bytes[..16], 16).unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct HookRequest {
    pub family: HookFamily,
    pub file: String,
    pub text: Arc<str>,
    pub always: BTreeSet<&'static str>,
    pub optional: BTreeSet<&'static str>,
}

impl HookRequest {
    pub fn object(file: &str, text: Arc<str>) -> Self {
        HookRequest {
            family: HookFamily::Object,
            file: file.to_string(),
            text,
            always: ["VERTEX_POSITIONS", "VERTEX_NORMALS"].into(),
            optional: ["VERTEX_UVS_A", "VERTEX_UVS_B", "VERTEX_TANGENTS"].into(),
        }
    }

    pub fn sky(file: &str, text: Arc<str>) -> Self {
        HookRequest {
            family: HookFamily::Sky,
            file: file.to_string(),
            text,
            always: BTreeSet::new(),
            optional: BTreeSet::new(),
        }
    }
}

struct Running {
    hash: u64,
    file: String,
    accepted: Accepted,
    shaders: BTreeMap<&'static str, Handle<Shader>>,
    exact: Option<Vec<SetKey>>,
    task: Task<Result<compose::CheckStats, Vec<String>>>,
}

#[derive(Resource, Default)]
pub struct HookQueue {
    pending: Vec<(u64, HookRequest)>,
    running: Vec<Running>,
    modules: HashMap<u64, Vec<Handle<Shader>>>,
    entries: BTreeMap<String, Handle<Shader>>,
}

static REQUESTS: Mutex<Vec<(u64, HookRequest)>> = Mutex::new(Vec::new());

pub fn request(r: HookRequest) -> u64 {
    let hash = hook_hash(r.family, &r.text);
    if state(hash).is_some() {
        return hash;
    }
    match check_hook(r.family, r.text.as_bytes(), &Limits::default()) {
        Ok(_) => {
            set_state(hash, HookState::Composing);
            REQUESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((hash, r));
        }
        Err(problems) => {
            let why = problems
                .iter()
                .map(|(l, c, m)| if *l == 0 { format!("{}: {m}", r.file) } else { format!("{}:{l}:{c}: {m}", r.file) })
                .collect::<Vec<_>>()
                .join("; ");
            set_state(hash, HookState::Refused(why));
        }
    }
    hash
}

pub fn refusal(hash: u64) -> Option<String> {
    match state(hash)? {
        HookState::Refused(why) => Some(why),
        _ => None,
    }
}

fn entry_sources(
    family: HookFamily,
    shaders: &Assets<Shader>,
    server: &AssetServer,
    held: &mut BTreeMap<String, Handle<Shader>>,
) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut ready = true;
    for (entry, _) in entries(family) {
        let handle =
            held.entry(entry.to_string()).or_insert_with(|| server.load(format!("embedded://{CRATE}/{entry}")));
        match shaders.get(&*handle) {
            Some(shader) => {
                out.insert(entry.to_string(), shader.source.as_str().to_string());
            }
            None => ready = false,
        }
    }
    ready.then_some(out)
}

pub(super) fn start(
    mut queue: ResMut<HookQueue>,
    mut shaders: ResMut<Assets<Shader>>,
    server: Res<AssetServer>,
    device: Option<Res<RenderDevice>>,
    adapter: Option<Res<RenderAdapter>>,
) {
    let taken = std::mem::take(&mut *REQUESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    queue.pending.extend(taken);
    let (Some(device), Some(adapter)) = (device, adapter) else { return };
    let mut waiting = Vec::new();
    for (hash, r) in std::mem::take(&mut queue.pending) {
        let accepted = match check_hook(r.family, r.text.as_bytes(), &Limits::default()) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let Some(sources) = entry_sources(r.family, &shaders, &server, &mut queue.entries) else {
            waiting.push((hash, r));
            continue;
        };
        let Some(modules) = variant_modules(r.family, hash, &r.text, &|e| sources.get(e).cloned()) else {
            waiting.push((hash, r));
            continue;
        };
        let mut handles = Vec::new();
        let mut entry_shaders = BTreeMap::new();
        for (path, src) in modules {
            let handle = shaders.add(Shader::from_wesl(src, path.clone()));
            if let Some((entry, _)) = entries(r.family).iter().find(|(e, _)| variant_path(hash, e) == path) {
                entry_shaders.insert(*entry, handle.clone());
            }
            handles.push(handle);
        }
        queue.modules.insert(hash, handles);
        let scope = Scope::Mesh { always: r.always.clone(), optional: r.optional.clone() };
        let task = spawn_check(&shaders, &device, &adapter, hash, &accepted, scope);
        info!("shader hook {}: checked ({}); composing with its family on a worker", r.file, accepted.summary());
        queue.running.push(Running { hash, file: r.file.clone(), accepted, shaders: entry_shaders, exact: None, task });
    }
    queue.pending.extend(waiting);
    let wanted = std::mem::take(&mut sets().wanted);
    let mut by_hook: BTreeMap<u64, Vec<(String, Vec<ShaderDefVal>)>> = BTreeMap::new();
    for (hash, entry, defs) in wanted {
        by_hook.entry(hash).or_default().push((entry, defs));
    }
    for (hash, list) in by_hook {
        let Some(HookState::Ready { shaders: entry_shaders, accepted }) = state(hash) else {
            continue;
        };
        let keys: Vec<SetKey> = list.iter().map(|(e, d)| (e.clone(), def_key(d))).collect();
        sets().in_flight.extend(keys.iter().map(|k| (hash, k.clone())));
        info!("shader hook h{hash:016x}: {} def set(s) a pipeline asked for: composing them on a worker", keys.len());
        let task = spawn_check(&shaders, &device, &adapter, hash, &accepted, Scope::Exact(list));
        sets().checking += 1;
        let file = format!("h{hash:016x}");
        queue.running.push(Running { hash, file, accepted, shaders: entry_shaders, exact: Some(keys), task });
    }
}

fn spawn_check(
    shaders: &Assets<Shader>,
    device: &RenderDevice,
    adapter: &RenderAdapter,
    hash: u64,
    accepted: &Accepted,
    scope: Scope,
) -> Task<Result<compose::CheckStats, Vec<String>>> {
    let snapshot: Vec<(String, AssetId<Shader>, Shader)> = shaders
        .iter()
        .map(|(id, s)| (s.path.strip_prefix("embedded://").unwrap_or(&s.path).to_string(), id, s.clone()))
        .filter(|(p, _, _)| !p.is_empty())
        .collect();
    let crate_sources: BTreeMap<String, String> = snapshot
        .iter()
        .filter(|(p, _, _)| p.starts_with(CRATE))
        .map(|(p, _, s)| (p.clone(), s.source.as_str().to_string()))
        .collect();
    let caps =
        DeviceCaps::of("this device", device.features(), &device.limits(), adapter.get_downlevel_capabilities().flags);
    let (family, checked) = (accepted.family, accepted.clone());
    AsyncComputeTaskPool::get().spawn(async move {
        let mut composer = Composer::new(&snapshot, &caps.globals);
        compose::check_variant(&mut composer, family, hash, &checked, &crate_sources, &caps, &scope)
    })
}

pub(super) fn finish(mut queue: ResMut<HookQueue>) {
    let mut still = Vec::new();
    for mut run in std::mem::take(&mut queue.running) {
        let Some(result) = block_on(poll_once(&mut run.task)) else {
            still.push(run);
            continue;
        };
        if let Some(keys) = run.exact {
            finish_exact(run.hash, &run.file, keys, result);
            continue;
        }
        match result {
            Ok(stats) => {
                let composed = stats.analysed.as_ref().map_or_else(|| "none".to_string(), |a| a.summary());
                info!(
                    "shader hook {}: composed, validated and analysed for {} def sets ({} distinct) on this device; as \
                     composed: {composed}; the entry points' function memory at most {} bytes",
                    run.file, stats.sets, stats.validated, stats.entry_frame
                );
                sets().admitted.entry(run.hash).or_default().extend(stats.admitted);
                set_state(run.hash, HookState::Ready { shaders: run.shaders, accepted: run.accepted });
                SETTLED.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
            Err(failures) => {
                let why = format!("{}: does not compose with its family: {}", run.file, failures.join("; "));
                warn!("shader hook refused: {why}; the family's own shader draws");
                set_state(run.hash, HookState::Refused(why));
                SETTLED.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
        }
    }
    queue.running = still;
}

fn finish_exact(hash: u64, file: &str, keys: Vec<SetKey>, result: Result<compose::CheckStats, Vec<String>>) {
    let mut s = sets();
    s.checking = s.checking.saturating_sub(1);
    for k in &keys {
        s.in_flight.remove(&(hash, k.clone()));
    }
    match result {
        Ok(stats) => {
            let admitted: BTreeSet<SetKey> = stats.admitted.into_iter().collect();
            for k in keys.iter().filter(|k| !admitted.contains(*k)) {
                warn!("shader hook {file}: def set of {} not admitted ({}); the family's own shader draws", k.0, k.1);
                s.denied.entry(hash).or_default().insert(k.clone());
            }
            info!("shader hook {file}: {} def set(s) a pipeline asked for admitted", admitted.len());
            s.admitted.entry(hash).or_default().extend(admitted);
        }
        Err(failures) => {
            warn!(
                "shader hook {file}: def sets a pipeline asked for refused: {}; the family's own shader draws there",
                failures.join("; ")
            );
            s.denied.entry(hash).or_default().extend(keys);
        }
    }
    *s.epochs.entry(hash).or_default() += 1;
    drop(s);
    SETTLED.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_checked_at_once() {
        let sky = "fn surface(input: SurfaceIn, base: Surface, sky_texture: texture_2d<f32>, sky_sampler: sampler) -> \
                   Surface {\n    var out = base;\n    var t = input.time;\n    while (t > 0.0) { t -= 1.0; }\n    return out;\n}\n";
        let refused = request(HookRequest::sky("shaders/while.wgsl", Arc::from(sky)));
        assert!(
            matches!(state(refused), Some(HookState::Refused(why)) if why.contains("loops must be")),
            "{:?}",
            state(refused)
        );
        let object = request(HookRequest::object("shaders/glow.wgsl", Arc::from(ssx_content::shader::OBJECT_TEMPLATE)));
        assert!(matches!(state(object), Some(HookState::Composing)));
        assert!(composing());
    }
}
