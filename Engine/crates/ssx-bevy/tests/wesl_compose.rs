use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::prelude::AssetId;
use bevy::render::renderer::RenderDevice;
use bevy::shader::{Shader, ShaderCache, ShaderCacheError, ShaderCacheSource, ShaderDefVal, ValidateShader};
use bevy::tasks::block_on;
use ssx_bevy::hooks::compose::{error_key, view_bindings_fit};
use ssx_bevy::hooks::headless::{loaded_shaders, shader_files_on_disk, shader_files_registered};
use ssx_bevy::hooks::sets::{
    CRATE, Kind, Module, combination, condition_flags, counterpart, describe, has_entry_point, kind_of, module_model,
    package_imports, variations, with,
};

const REQUIRED: wgpu::Features = wgpu::Features::POLYGON_MODE_LINE.union(wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER);

struct DeviceClass {
    name: &'static str,
    device: RenderDevice,
    binding_arrays: bool,
    globals: Vec<ShaderDefVal>,
}

fn device_class(name: &'static str, features: Option<wgpu::Features>, limits: Option<wgpu::Limits>) -> DeviceClass {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::NOOP,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        display: None,
        backend_options: wgpu::BackendOptions {
            noop: wgpu::NoopBackendOptions { enable: true, ..Default::default() },
            ..Default::default()
        },
    });
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("noop adapter");
    let features = features.unwrap_or(adapter.features() - wgpu::Features::all_experimental_mask());
    let limits = limits.unwrap_or(adapter.limits());
    let (device, _queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        required_limits: limits.clone(),
        ..Default::default()
    }))
    .expect("noop device");
    let array_len = (16 + bevy::pbr::MAX_VIEW_LIGHT_PROBES) as u32;
    let binding_arrays = limits.max_binding_array_elements_per_shader_stage >= array_len
        && limits.max_binding_array_sampler_elements_per_shader_stage >= array_len
        && features.contains(
            wgpu::Features::TEXTURE_BINDING_ARRAY
                | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        );
    let storage = limits.max_storage_buffers_per_shader_stage;
    let globals = vec![
        ShaderDefVal::UInt("AVAILABLE_STORAGE_BUFFER_BINDINGS".into(), storage),
        ShaderDefVal::Bool("AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_3".into(), storage >= 3),
        ShaderDefVal::Bool("AVAILABLE_STORAGE_BUFFER_BINDINGS__GE_6".into(), storage >= 6),
    ];
    DeviceClass { name, device: RenderDevice::from(device), binding_arrays, globals }
}

fn device_classes() -> Vec<DeviceClass> {
    vec![
        device_class("all features", None, None),
        device_class("required features, downlevel limits", Some(REQUIRED), Some(wgpu::Limits::downlevel_defaults())),
    ]
}

fn composed_wgsl(_: &(), source: ShaderCacheSource, _: &ValidateShader) -> Result<String, ShaderCacheError> {
    match source {
        ShaderCacheSource::Wgsl(wgsl) => Ok(wgsl),
        ShaderCacheSource::SpirV(_) => Err(ShaderCacheError::ProcessShaderError("SPIR-V".into())),
    }
}

fn validate(device: &RenderDevice, wgsl: &str) -> Result<(), String> {
    let scope = device.wgpu_device().push_error_scope(wgpu::ErrorFilter::Validation);
    drop(device.create_and_validate_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(wgsl.into()),
    }));
    match block_on(scope.pop()) {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    }
}

struct Composer {
    cache: ShaderCache<String, ()>,
    shaders: HashMap<String, (AssetId<Shader>, Shader)>,
}

impl Composer {
    fn new(shaders: &[(String, AssetId<Shader>, Shader)], globals: &[ShaderDefVal]) -> Self {
        let mut cache = ShaderCache::new((), composed_wgsl);
        let mut by_path = HashMap::new();
        for (path, id, shader) in shaders {
            let mut shader = shader.clone();
            shader.shader_defs.splice(0..0, globals.iter().cloned());
            cache.set_shader(*id, shader.clone());
            by_path.insert(path.clone(), (*id, shader));
        }
        Self { cache, shaders: by_path }
    }

    fn compose(&mut self, module: &str, defs: &[ShaderDefVal]) -> Result<String, String> {
        let (id, shader) = self.shaders.get(module).expect("a registered module").clone();
        let out = self.cache.get(0, id, defs).map(|wgsl| wgsl.as_ref().clone()).map_err(|e| e.to_string());
        self.cache.set_shader(id, shader);
        out
    }
}

fn modules() -> Vec<Module> {
    let disk = shader_files_on_disk();
    let on_disk: BTreeSet<String> = disk.keys().cloned().collect();
    let registered: BTreeSet<String> = shader_files_registered().into_keys().collect();
    assert_eq!(on_disk, registered, "shader files on disk vs. the ones the code registers");
    let sources: BTreeMap<String, String> =
        disk.iter().map(|(p, f)| (p.clone(), std::fs::read_to_string(f).expect("a shader file"))).collect();
    let wesl: BTreeSet<String> = on_disk.iter().filter(|p| p.ends_with(".wesl")).cloned().collect();
    let closure = |path: &str| {
        let mut seen = BTreeSet::from([path.to_string()]);
        let mut todo = vec![path.to_string()];
        while let Some(p) = todo.pop() {
            for import in package_imports(&sources[&p], &wesl) {
                if seen.insert(import.clone()) {
                    todo.push(import);
                }
            }
        }
        seen
    };
    let mut out = Vec::new();
    let mut imported = BTreeSet::new();
    for (path, src) in &sources {
        if !has_entry_point(src) {
            continue;
        }
        let kind = kind_of(path).unwrap_or_else(|| panic!("{path}: an entry point but no kind in kind_of()"));
        let modules = closure(path);
        let flags: BTreeSet<String> = modules.iter().flat_map(|m| condition_flags(&sources[m])).collect();
        imported.extend(modules.into_iter().filter(|m| m != path));
        out.push(module_model(path, kind, flags));
    }
    for (path, src) in &sources {
        if !has_entry_point(src) {
            assert!(imported.contains(path), "{path}: a library no composed module imports");
        }
    }
    out
}

fn all_but_hooks(module: &Module) -> u64 {
    module
        .dims
        .iter()
        .enumerate()
        .filter(|(_, d)| !d.iter().any(|f| f.starts_with("SSX_HOOK_")))
        .fold(0, |bits, (k, _)| bits | 1 << k)
}

#[derive(Clone, Copy)]
enum Job {
    Combination { bits: u64, compressed: bool },
    Variation { index: usize, all_on: bool, hooks: bool },
    Unproducible { index: usize, bits: u64 },
}

#[derive(Default)]
struct Stats {
    combinations: AtomicUsize,
    not_produced: AtomicUsize,
    variations: AtomicUsize,
    unproducible: AtomicUsize,
    validated: AtomicUsize,
    bevy_too: AtomicUsize,
    failures: AtomicUsize,
    unproducible_failures: AtomicUsize,
    compose_ns: AtomicUsize,
    validate_ns: AtomicUsize,
}

fn hash_of(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

type Outcomes = BTreeMap<(usize, &'static str, String), (usize, String)>;

#[test]
fn every_wesl_module_composes_and_validates_for_every_def_set() {
    let modules = modules();
    let kinds: Vec<Kind> = modules.iter().map(|m| m.kind).collect();
    let shaders = loaded_shaders();
    let devices = device_classes();
    let paths: BTreeSet<&str> = shaders.iter().map(|(p, _, _)| p.as_str()).collect();
    for kind in [Kind::MainVertex, Kind::MainFragment, Kind::ShadowPrepass] {
        let c = counterpart(kind).expect("a counterpart");
        assert!(paths.contains(c), "{c} is not among Bevy's shaders");
    }
    let mut jobs: Vec<(usize, usize, Job)> = Vec::new();
    for (m, module) in modules.iter().enumerate() {
        assert!(module.dims.len() < 24, "{}: {} dims", module.path, module.dims.len());
        let combos = 1u64 << module.dims.len();
        for (d, device) in devices.iter().enumerate() {
            for compressed in if module.vertex_input { &[false, true][..] } else { &[false][..] } {
                jobs.extend((0..combos).map(|bits| (m, d, Job::Combination { bits, compressed: *compressed })));
            }
            for (index, v) in variations(module.kind).iter().enumerate() {
                if v.add.contains(&"CLUSTERED_DECALS_ARE_USABLE") && !device.binding_arrays {
                    continue;
                }
                jobs.extend([false, true].map(|all_on| (m, d, Job::Variation { index, all_on, hooks: false })));
                if all_but_hooks(module) != (1u64 << module.dims.len()) - 1 {
                    jobs.push((m, d, Job::Variation { index, all_on: true, hooks: true }));
                }
            }
        }
        for index in 0..module.unproducible.len() {
            jobs.extend((0..combos).map(|bits| (m, 0, Job::Unproducible { index, bits })));
        }
    }
    let stats: Vec<Stats> = modules.iter().map(|_| Stats::default()).collect();
    let validated: Vec<Mutex<HashMap<u64, Option<String>>>> =
        devices.iter().map(|_| Mutex::new(HashMap::new())).collect();
    let outcomes = Mutex::new(Outcomes::new());
    let dump = std::env::var_os("SSX_WESL_DUMP").map(|path| (path, Mutex::new(Vec::<String>::new())));
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let devices = device_classes();
                let mut composers: Vec<Composer> =
                    devices.iter().map(|d| Composer::new(&shaders, &d.globals)).collect();
                let mut check = |m: usize, d: usize, path: &str, defs: &[ShaderDefVal]| -> Result<(), String> {
                    let t0 = std::time::Instant::now();
                    let wgsl = composers[d].compose(path, defs);
                    stats[m].compose_ns.fetch_add(t0.elapsed().as_nanos() as usize, Ordering::Relaxed);
                    let wgsl = wgsl?;
                    let hash = hash_of(&wgsl);
                    if let Some((_, lines)) = &dump
                        && path.starts_with(CRATE)
                    {
                        let mut sorted: Vec<&str> = wgsl.lines().collect();
                        sorted.sort_unstable();
                        let lines_hash = hash_of(&sorted.join("\n"));
                        lines.lock().expect("lock").push(format!(
                            "{path}\t{}\t{}\t{lines_hash:016x}",
                            devices[d].name,
                            describe(defs)
                        ));
                    }
                    if let Some(known) = validated[d].lock().expect("lock").get(&hash) {
                        return known.clone().map_or(Ok(()), Err);
                    }
                    stats[m].validated.fetch_add(1, Ordering::Relaxed);
                    let t1 = std::time::Instant::now();
                    let result = validate(&devices[d].device, &wgsl).and_then(|()| view_bindings_fit(kinds[m], &wgsl));
                    stats[m].validate_ns.fetch_add(t1.elapsed().as_nanos() as usize, Ordering::Relaxed);
                    validated[d].lock().expect("lock").insert(hash, result.clone().err());
                    result
                };
                loop {
                    let start = next.fetch_add(32, Ordering::Relaxed);
                    if start >= jobs.len() {
                        break;
                    }
                    for &(m, d, job) in &jobs[start..(start + 32).min(jobs.len())] {
                        let (module, s) = (&modules[m], &stats[m]);
                        let device = &devices[d];
                        let defs = match job {
                            Job::Combination { bits, compressed } => {
                                combination(module, device.binding_arrays, bits, compressed)
                            }
                            Job::Variation { index, all_on, hooks } => {
                                let bits = match (all_on, hooks) {
                                    (false, _) => 0,
                                    (true, false) => all_but_hooks(module),
                                    (true, true) => (1u64 << module.dims.len()) - 1,
                                };
                                let v = &variations(module.kind)[index];
                                combination(module, device.binding_arrays, bits, false)
                                    .map(|defs| with(defs, v.add, v.remove))
                            }
                            Job::Unproducible { index, bits } => {
                                let u = &module.unproducible[index];
                                combination(module, device.binding_arrays, bits, false)
                                    .map(|defs| with(defs, &u.add, &u.remove))
                            }
                        };
                        let Some(defs) = defs else {
                            s.not_produced.fetch_add(1, Ordering::Relaxed);
                            continue;
                        };
                        match job {
                            Job::Combination { .. } => s.combinations.fetch_add(1, Ordering::Relaxed),
                            Job::Variation { .. } => s.variations.fetch_add(1, Ordering::Relaxed),
                            Job::Unproducible { .. } => s.unproducible.fetch_add(1, Ordering::Relaxed),
                        };
                        let Err(error) = check(m, d, &module.path, &defs) else { continue };
                        let key = error_key(&error);
                        let bevy_too = counterpart(module.kind)
                            .map(|c| check(m, d, c, &defs))
                            .is_some_and(|bevy| bevy.err().is_some_and(|e| error_key(&e) == key));
                        let outcome = if bevy_too {
                            s.bevy_too.fetch_add(1, Ordering::Relaxed);
                            "Bevy's own module fails with the same error"
                        } else if let Job::Unproducible { index, .. } = job {
                            s.unproducible_failures.fetch_add(1, Ordering::Relaxed);
                            module.unproducible[index].why
                        } else {
                            s.failures.fetch_add(1, Ordering::Relaxed);
                            "FAILED"
                        };
                        let mut o = outcomes.lock().expect("lock");
                        let entry = o.entry((m, outcome, key)).or_insert((0, String::new()));
                        entry.0 += 1;
                        if entry.1.is_empty() {
                            entry.1 = format!("[{}] {}\n{error}", device.name, describe(&defs));
                        }
                    }
                }
            });
        }
    });
    ssx_core::println!(
        "{} shaders; device classes: {}; {threads} threads, {:.1} s",
        shaders.len(),
        devices.iter().map(|d| d.name).collect::<Vec<_>>().join(" | "),
        started.elapsed().as_secs_f64()
    );
    ssx_core::println!(
        "{:<42} {:<13} {:>4} {:>7} {:>6} {:>5} {:>6} {:>9} {:>6} {:>6} {:>6} {:>8}",
        "module",
        "kind",
        "dims",
        "combos",
        "not by",
        "views",
        "unprod",
        "validated",
        "bevy",
        "unp.f",
        "FAILED",
        "cpu s"
    );
    let mut failed = 0;
    for (module, s) in modules.iter().zip(&stats) {
        let n = |a: &AtomicUsize| a.load(Ordering::Relaxed);
        failed += n(&s.failures);
        ssx_core::println!(
            "{:<42} {:<13} {:>4} {:>7} {:>6} {:>5} {:>6} {:>9} {:>6} {:>6} {:>6} {:>8.1}",
            module.path.trim_start_matches("ssx_bevy/"),
            format!("{:?}", module.kind),
            module.dims.len(),
            n(&s.combinations),
            n(&s.not_produced),
            n(&s.variations),
            n(&s.unproducible),
            n(&s.validated),
            n(&s.bevy_too),
            n(&s.unproducible_failures),
            n(&s.failures),
            (n(&s.compose_ns) + n(&s.validate_ns)) as f64 * 1e-9,
        );
        let dims: Vec<String> = module.dims.iter().map(|d| d.join("+")).collect();
        ssx_core::println!("    dims: {}", dims.join(" "));
        if !module.contract.is_empty() {
            ssx_core::println!("    always: {}", module.contract.join(" "));
        }
        for u in &module.unproducible {
            ssx_core::println!("    unproducible: +{:?} -{:?} ({})", u.add, u.remove, u.why);
        }
    }
    for ((m, outcome, key), (count, example)) in outcomes.lock().expect("lock").iter() {
        ssx_core::println!("-- {} | {outcome} | {count}× | {key}", modules[*m].path);
        ssx_core::println!("   e.g. {example}");
        ssx_core::println!();
    }
    if let Some((path, lines)) = dump {
        let mut lines = lines.into_inner().expect("lock");
        lines.sort();
        lines.dedup();
        std::fs::write(path, lines.join("\n") + "\n").expect("the dump");
    }
    assert_eq!(failed, 0, "def sets a key produces failed to compose or validate");
}
