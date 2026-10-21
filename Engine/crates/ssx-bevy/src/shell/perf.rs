use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bevy::app::MainScheduleOrder;
use bevy::core_pipeline::upscaling::upscaling;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::diagnostic::DiagnosticsStore;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::pbr::{
    EARLY_SHADOW_PASS, LATE_SHADOW_PASS, clear_indirect_parameters_metadata, early_gpu_preprocess,
    main_build_indirect_parameters, per_view_shadow_pass, shared_shadow_pass,
};
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy::render::render_resource::{
    Buffer, BufferDescriptor, BufferUsages, CachedPipelineState, CommandEncoderDescriptor, MapMode, PipelineCache,
    PipelineDescriptor,
};
use bevy::render::renderer::{
    CurrentView, PendingCommandBuffers, RenderContext, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue,
};
use bevy::render::settings::WgpuFeatures;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::window::PrimaryWindow;

use crate::render::terrain::TrackStats;

const MAX_MARKS: u32 = 64;
const SLOTS: usize = 4;

const FRAME_BEGIN: u8 = 0;
const VIEW_BEGIN: u8 = 1;
const SHADOW_BEGIN: u8 = 2;
const SHADOW_END: u8 = 3;
const MAIN_BEGIN: u8 = 4;
const MAIN_END: u8 = 5;
const VIEW_END: u8 = 6;
const FRAME_END: u8 = 7;

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct PerfFrameStart;
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct PerfFrameEnd;

#[derive(Clone, Copy, Debug, Default)]
struct GpuFrame {
    frame: f32,
    views: f32,
    cull: f32,
    shadow: f32,
    main: f32,
    post: f32,
}

#[derive(Default)]
struct Shared {
    render_begin: Option<Instant>,
    render_ms: Vec<f32>,
    gpu: Vec<GpuFrame>,
    render_entities: u32,
    pipelines: (u32, u32, u32),
}

fn sh_render_entities(shared: &PerfShared) -> u32 {
    shared.0.lock().unwrap_or_else(|e| e.into_inner()).render_entities
}

#[derive(bevy::ecs::system::SystemParam)]
struct Counts<'w> {
    entities: &'w bevy::ecs::entity::Entities,
    archetypes: &'w bevy::ecs::archetype::Archetypes,
    meshes: Res<'w, Assets<Mesh>>,
    images: Res<'w, Assets<Image>>,
    particles: Option<Res<'w, crate::particles::ParticleFrame>>,
    lights: Option<Res<'w, crate::render::lighting::local::DynamicLocalLights>>,
    snow: Option<Res<'w, crate::render::snow_tracks::SnowTracks>>,
}

impl Counts<'_> {
    const HEADER: &'static str = "ents\tarchetypes\trender_ents\tmeshes\timages\tp_draws\tp_verts\tdyn_lights\tsnow_ms\t\
                                  sfx_ms\tsfx_lock\tsfx_max\taud_load\taud_lock\taud_max\taud_demand\taud_silence\tvoices\tgpu_mem_mb\ttex_bc_mb\ttex_rgba_mb";

    fn line(&self, render_entities: u32) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        let (pd, pv) = self
            .particles
            .as_ref()
            .map_or((0, 0), |p| (p.draws.len(), p.draws.iter().map(|d| d.vertices.len()).sum::<usize>()));
        let ms = |ns: u64| ns as f64 * 1e-6;
        let mb = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
        let calls = crate::audio::sfx::stats::CALLS.swap(0, Relaxed).max(1) as f64;
        let sfx = ms(crate::audio::sfx::stats::NS.swap(0, Relaxed)) / calls;
        let sfx_lock = ms(crate::audio::sfx::stats::LOCK_NS.swap(0, Relaxed)) / calls;
        let sfx_max = ms(crate::audio::sfx::stats::MAX_NS.swap(0, Relaxed));
        let audio_s =
            crate::audio::music::stats::FRAMES.swap(0, Relaxed) as f64 / f64::from(ssx_assets::sfx_aems::MIXER_RATE);
        let load = ms(crate::audio::music::stats::RENDER_NS.swap(0, Relaxed)) / (audio_s * 1000.0).max(1e-9) * 100.0;
        let lock = ms(crate::audio::music::stats::LOCK_NS.swap(0, Relaxed));
        let aud_max = ms(crate::audio::music::stats::RENDER_MAX_NS.swap(0, Relaxed));
        format!(
            "{}\t{}\t{render_entities}\t{}\t{}\t{pd}\t{pv}\t{}\t{:.2}\t{sfx:.2}\t{sfx_lock:.2}\t{sfx_max:.1}\t\
             {load:.1}\t{lock:.1}\t{aud_max:.1}\t{}\t{}\t{}\t{:.0}\t{:.1}\t{:.1}",
            self.entities.count_spawned(),
            self.archetypes.len(),
            self.meshes.len(),
            self.images.len(),
            self.lights.as_ref().map_or(0, |l| l.lights.len()),
            self.snow.as_ref().map_or(0.0, |t| t.update_ms),
            crate::audio::music::stats::ON_DEMAND.swap(0, Relaxed),
            crate::audio::music::stats::SILENCE.swap(0, Relaxed),
            crate::audio::sfx::stats::VOICES.load(Relaxed),
            gpu_memory_mb(),
            mb(crate::render::terrain::textures::BLOCK_BYTES.load(Relaxed)),
            mb(crate::render::terrain::textures::DECODED_BYTES.load(Relaxed)),
        )
    }
}

fn gpu_memory_mb() -> f64 {
    let Ok(dir) = std::fs::read_dir("/proc/self/fdinfo") else { return 0.0 };
    let mut clients = std::collections::HashSet::new();
    let mut kib = 0.0;
    for entry in dir.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
        let Some(client) = text.lines().find_map(|l| l.strip_prefix("drm-client-id:")) else { continue };
        if !clients.insert(client.trim().to_string()) {
            continue;
        }
        for line in text.lines().filter(|l| l.starts_with("drm-total-")) {
            let Some((_, v)) = line.split_once(':') else { continue };
            let mut it = v.split_whitespace();
            let n: f64 = it.next().and_then(|n| n.parse().ok()).unwrap_or(0.0);
            kib += match it.next() {
                Some("MiB") => n * 1024.0,
                Some("GiB") => n * 1024.0 * 1024.0,
                Some("KiB") => n,
                _ => n / 1024.0,
            };
        }
    }
    kib / 1024.0
}

#[derive(Resource, Clone)]
struct PerfShared(Arc<Mutex<Shared>>);

pub struct PerfPlugin {
    out: PathBuf,
    window: f32,
}

impl PerfPlugin {
    pub fn from_env() -> Option<PerfPlugin> {
        let out = std::env::var_os("SSX_PERF").filter(|v| !v.is_empty())?;
        let window = std::env::var("SSX_PERF_WINDOW").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
        Some(PerfPlugin { out: out.into(), window })
    }
}

impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        let file = std::fs::File::create(&self.out).unwrap_or_else(|e| panic!("SSX_PERF {}: {e}", self.out.display()));
        let shared = PerfShared(Arc::default());
        if !app.is_plugin_added::<RenderDiagnosticsPlugin>() {
            app.add_plugins(RenderDiagnosticsPlugin);
        }
        app.init_schedule(PerfFrameStart).init_schedule(PerfFrameEnd);
        let mut order = app.world_mut().resource_mut::<MainScheduleOrder>();
        order.insert_before(First, PerfFrameStart);
        order.insert_after(Last, PerfFrameEnd);
        let now = Instant::now();
        app.insert_resource(PerfState {
            out: std::io::BufWriter::new(file),
            window: self.window,
            start: now,
            window_start: now,
            update_begin: now,
            frame_ms: Vec::new(),
            main_ms: Vec::new(),
            header: false,
            exit: std::env::var("SSX_PERF_EXIT").ok().and_then(|v| v.parse().ok()),
            hitch_ms: std::env::var("SSX_PERF_HITCH").ok().and_then(|v| v.parse().ok()).unwrap_or(50.0),
            hitches: std::fs::File::create(self.out.with_extension("hitches.tsv")).ok().map(|f| {
                let mut w = std::io::BufWriter::new(f);
                let _ = writeln!(
                    w,
                    "t\tscene\tframe_ms\tmain_ms\tpipes_asked\tpipes_created\tpipes_waiting\tcreated_since"
                );
                w
            }),
            hitch_pipes: 0,
        })
        .insert_resource(shared.clone())
        .add_systems(PerfFrameStart, |mut s: ResMut<PerfState>| s.update_begin = Instant::now())
        .add_systems(PerfFrameEnd, frame_end)
        .add_systems(Startup, (window_size, continuous));
        let Some(render) = app.get_sub_app_mut(RenderApp) else { return };
        let pipeline_log = std::fs::File::create(self.out.with_extension("pipelines.tsv")).ok().map(|f| {
            let mut w = std::io::BufWriter::new(f);
            let _ = writeln!(w, "id\tasked_s\tcreated_s\twait_ms\tlabel");
            w
        });
        render
            .insert_resource(PipelineLog { out: pipeline_log, start: now, seen: Vec::new() })
            .insert_resource(shared)
            .add_systems(RenderStartup, init_marks)
            .add_systems(
                Render,
                (render_begin.in_set(RenderSystems::ExtractCommands), render_end.in_set(RenderSystems::PostCleanup)),
            )
            .add_systems(
                RenderGraph,
                (
                    frame_begin.in_set(RenderGraphSystems::Begin),
                    frame_end_marks.after(RenderGraphSystems::Render).before(RenderGraphSystems::Submit),
                    map_marks.in_set(RenderGraphSystems::Finish),
                ),
            )
            .add_systems(
                Core3d,
                (
                    mark::<VIEW_BEGIN>.before(clear_indirect_parameters_metadata),
                    mark::<SHADOW_BEGIN>
                        .after(early_gpu_preprocess)
                        .before(per_view_shadow_pass::<EARLY_SHADOW_PASS>)
                        .before(shared_shadow_pass::<EARLY_SHADOW_PASS>),
                    mark::<SHADOW_END>
                        .after(per_view_shadow_pass::<LATE_SHADOW_PASS>)
                        .after(shared_shadow_pass::<LATE_SHADOW_PASS>)
                        .before(main_build_indirect_parameters),
                    mark::<MAIN_BEGIN>.after(main_build_indirect_parameters).before(Core3dSystems::MainPass),
                    mark::<MAIN_END>.after(Core3dSystems::MainPass).before(Core3dSystems::EarlyPostProcess),
                    mark::<VIEW_END>.after(Core3dSystems::PostProcess).after(upscaling),
                ),
            );
    }
}

#[derive(Resource)]
struct PerfState {
    out: std::io::BufWriter<std::fs::File>,
    window: f32,
    start: Instant,
    window_start: Instant,
    update_begin: Instant,
    frame_ms: Vec<f32>,
    main_ms: Vec<f32>,
    header: bool,
    exit: Option<f32>,
    hitch_ms: f32,
    hitches: Option<std::io::BufWriter<std::fs::File>>,
    hitch_pipes: u32,
}

fn window_size(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let Some((w, h)) = std::env::var("SSX_PERF_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    }) else {
        return;
    };
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(w, h);
    }
}

fn continuous(mut commands: Commands) {
    commands.insert_resource(bevy::winit::WinitSettings::continuous());
}

fn stats(v: &mut [f32], q: f32) -> (f32, f32) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    v.sort_by(f32::total_cmp);
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    (mean, v[((v.len() - 1) as f32 * q).round() as usize])
}

#[allow(clippy::too_many_arguments)]
fn frame_end(
    mut s: ResMut<PerfState>,
    time: Res<Time<Real>>,
    shared: Res<PerfShared>,
    diagnostics: Res<DiagnosticsStore>,
    track: Option<Res<TrackStats>>,
    counts: Counts,
    mut exit: MessageWriter<AppExit>,
) {
    let now = Instant::now();
    let main = (now - s.update_begin).as_secs_f32() * 1000.0;
    s.main_ms.push(main);
    let frame_ms = time.delta_secs() * 1000.0;
    s.frame_ms.push(frame_ms);
    if frame_ms > s.hitch_ms {
        let (asked, created, waiting) = shared.0.lock().unwrap_or_else(|e| e.into_inner()).pipelines;
        let since = created.saturating_sub(std::mem::replace(&mut s.hitch_pipes, created));
        let t = (now - s.start).as_secs_f32();
        let scene = track.as_ref().map_or("fe", |t| t.name.as_str()).to_string();
        if let Some(w) = s.hitches.as_mut() {
            let _ = writeln!(w, "{t:.2}\t{scene}\t{frame_ms:.1}\t{main:.1}\t{asked}\t{created}\t{waiting}\t{since}");
            let _ = w.flush();
        }
    }
    if s.exit.is_some_and(|e| (now - s.start).as_secs_f32() >= e) {
        exit.write(AppExit::Success);
    }
    if (now - s.window_start).as_secs_f32() < s.window {
        return;
    }
    let (mut render, gpu) = {
        let mut sh = shared.0.lock().unwrap_or_else(|e| e.into_inner());
        (std::mem::take(&mut sh.render_ms), std::mem::take(&mut sh.gpu))
    };
    let scene = track.map_or_else(|| "fe".to_string(), |t| t.name.clone());
    let n = s.frame_ms.len();
    let mut frames = std::mem::take(&mut s.frame_ms);
    let mut mains = std::mem::take(&mut s.main_ms);
    let (ft, ft50) = stats(&mut frames, 0.5);
    let ft95 = frames[((n - 1) as f32 * 0.95).round() as usize];
    let ft99 = frames[((n - 1) as f32 * 0.99).round() as usize];
    let ftmax = frames.last().copied().unwrap_or(0.0);
    let (main, main95) = stats(&mut mains, 0.95);
    let (rend, rend95) = stats(&mut render, 0.95);
    let g = gpu.len().max(1) as f32;
    let sum = |f: fn(&GpuFrame) -> f32| gpu.iter().map(f).sum::<f32>() / g;
    let mut gframes: Vec<f32> = gpu.iter().map(|f| f.frame).collect();
    let (gf, gf95) = stats(&mut gframes, 0.95);
    let pass = |name: &str| {
        diagnostics
            .iter()
            .find(|d| d.path().as_str() == format!("render/{name}/elapsed_gpu"))
            .and_then(|d| d.average())
            .unwrap_or(0.0)
    };
    if !s.header {
        s.header = true;
        let _ = writeln!(
            s.out,
            "t\tscene\tframes\tft_avg\tft_p50\tft_p95\tft_p99\tft_max\tmain_avg\tmain_p95\trender_avg\trender_p95\t\
             gpu_n\tgpu_frame\tgpu_p95\tgpu_views\tgpu_cull\tgpu_shadow\tgpu_main\tgpu_post\tgpu_rest\topaque\ttransparent\t\
             {}\tpipes_created\tpipes_waiting",
            Counts::HEADER
        );
    }
    let t = (now - s.start).as_secs_f32();
    let line = format!(
        "{t:.1}\t{scene}\t{n}\t{ft:.2}\t{ft50:.2}\t{ft95:.2}\t{ft99:.2}\t{ftmax:.2}\t{main:.2}\t{main95:.2}\t{rend:.2}\t\
         {rend95:.2}\t{}\t{gf:.2}\t{gf95:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{}",
        gpu.len(),
        sum(|f| f.views),
        sum(|f| f.cull),
        sum(|f| f.shadow),
        sum(|f| f.main),
        sum(|f| f.post),
        sum(|f| f.frame - f.views),
        pass("main_opaque_pass_3d"),
        pass("main_transparent_pass_3d"),
        counts.line(sh_render_entities(&shared)),
    );
    let (_, created, waiting) = shared.0.lock().unwrap_or_else(|e| e.into_inner()).pipelines;
    let line = format!("{line}\t{created}\t{waiting}");
    let _ = writeln!(s.out, "{line}");
    let _ = s.out.flush();
    s.window_start = now;
}

fn render_begin(shared: Res<PerfShared>) {
    shared.0.lock().unwrap_or_else(|e| e.into_inner()).render_begin = Some(Instant::now());
}

#[derive(Resource)]
struct PipelineLog {
    out: Option<std::io::BufWriter<std::fs::File>>,
    start: Instant,
    seen: Vec<(f32, bool)>,
}

fn render_end(
    shared: Res<PerfShared>,
    entities: &bevy::ecs::entity::Entities,
    cache: Res<PipelineCache>,
    mut log: ResMut<PipelineLog>,
    mut frames: Local<u32>,
) {
    let mut sh = shared.0.lock().unwrap_or_else(|e| e.into_inner());
    let (mut asked, mut created) = (0, 0);
    let now = log.start.elapsed().as_secs_f32();
    let PipelineLog { out, seen, .. } = &mut *log;
    for (id, p) in cache.pipelines().enumerate() {
        asked += 1;
        let ok = matches!(p.state, CachedPipelineState::Ok(_));
        created += u32::from(ok);
        if seen.len() <= id {
            seen.resize(id + 1, (now, false));
        }
        if ok && !seen[id].1 {
            seen[id].1 = true;
            let label = match &p.descriptor {
                PipelineDescriptor::RenderPipelineDescriptor(d) => d.label.as_deref().unwrap_or("-").to_string(),
                PipelineDescriptor::ComputePipelineDescriptor(d) => d.label.as_deref().unwrap_or("-").to_string(),
            };
            if let Some(w) = out.as_mut() {
                let _ = writeln!(w, "{id}\t{:.2}\t{now:.2}\t{:.0}\t{label}", seen[id].0, (now - seen[id].0) * 1000.0);
                let _ = w.flush();
            }
        }
    }
    sh.pipelines = (asked, created, cache.waiting_pipelines().count() as u32);
    *frames = frames.wrapping_add(1);
    if *frames % 30 == 1 {
        sh.render_entities = entities.count_spawned();
    }
    if let Some(begin) = sh.render_begin.take() {
        sh.render_ms.push(begin.elapsed().as_secs_f32() * 1000.0);
    }
}

#[derive(Clone, Copy)]
struct Record {
    mark: u8,
    view: Option<Entity>,
    index: u32,
}

struct Slot {
    read: Buffer,
    mapped: Arc<AtomicBool>,
    pending: Option<Vec<Record>>,
}

#[derive(Default)]
struct Current {
    slot: Option<usize>,
    records: Vec<Record>,
    map: Option<usize>,
}

#[derive(Resource)]
struct GpuMarks {
    queries: wgpu::QuerySet,
    resolve: Buffer,
    slots: Vec<Slot>,
    period: f32,
    current: Mutex<Current>,
}

fn init_marks(mut commands: Commands, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let needed = WgpuFeatures::TIMESTAMP_QUERY | WgpuFeatures::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if !device.features().contains(needed) {
        warn!("SSX_PERF: the adapter has no timestamp queries inside encoders; no GPU times");
        return;
    }
    let size = u64::from(MAX_MARKS) * 8;
    let buffer = |label, usage| {
        device.create_buffer(&BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
    };
    let queries = device.wgpu_device().create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("ssx_perf_marks"),
        ty: wgpu::QueryType::Timestamp,
        count: MAX_MARKS,
    });
    let slots = (0..SLOTS)
        .map(|_| Slot {
            read: buffer("ssx_perf_read", BufferUsages::COPY_DST | BufferUsages::MAP_READ),
            mapped: Arc::default(),
            pending: None,
        })
        .collect();
    commands.insert_resource(GpuMarks {
        queries,
        resolve: buffer("ssx_perf_resolve", BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC),
        slots,
        period: queue.get_timestamp_period(),
        current: Mutex::default(),
    });
}

fn frame_begin(
    marks: Option<ResMut<GpuMarks>>,
    shared: Res<PerfShared>,
    device: Res<RenderDevice>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let Some(mut marks) = marks else { return };
    let period = marks.period;
    for slot in &mut marks.slots {
        if !slot.mapped.load(Ordering::Acquire) {
            continue;
        }
        if let Some(records) = slot.pending.take() {
            let ticks: Vec<u64> = {
                let view = slot.read.slice(..).get_mapped_range();
                view.as_chunks::<8>().0.iter().map(|b| u64::from_le_bytes(*b)).collect()
            };
            let frame = gpu_frame(&records, &ticks, period);
            shared.0.lock().unwrap_or_else(|e| e.into_inner()).gpu.push(frame);
        }
        slot.read.unmap();
        slot.mapped.store(false, Ordering::Release);
    }
    let free = marks.slots.iter().position(|s| s.pending.is_none() && !s.mapped.load(Ordering::Acquire));
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("ssx_perf_begin") });
    let GpuMarks { queries, current, .. } = &mut *marks;
    let current = current.get_mut().unwrap_or_else(|e| e.into_inner());
    *current = Current { slot: free, ..default() };
    if free.is_some() {
        encoder.write_timestamp(queries, 0);
        current.records.push(Record { mark: FRAME_BEGIN, view: None, index: 0 });
        pending.push([encoder.finish()]);
    }
}

fn mark<const M: u8>(mut ctx: RenderContext, marks: Option<Res<GpuMarks>>, view: Res<CurrentView>) {
    let Some(marks) = marks else { return };
    let mut current = marks.current.lock().unwrap_or_else(|e| e.into_inner());
    let index = current.records.len() as u32;
    if current.slot.is_none() || index + 1 >= MAX_MARKS {
        return;
    }
    ctx.command_encoder().write_timestamp(&marks.queries, index);
    current.records.push(Record { mark: M, view: Some(view.0), index });
}

fn frame_end_marks(
    marks: Option<ResMut<GpuMarks>>,
    device: Res<RenderDevice>,
    mut pending: ResMut<PendingCommandBuffers>,
) {
    let Some(mut marks) = marks else { return };
    let GpuMarks { queries, resolve, slots, current, .. } = &mut *marks;
    let current = current.get_mut().unwrap_or_else(|e| e.into_inner());
    let Some(slot) = current.slot.take() else { return };
    let index = current.records.len() as u32;
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: Some("ssx_perf_end") });
    encoder.write_timestamp(queries, index);
    current.records.push(Record { mark: FRAME_END, view: None, index });
    let n = index + 1;
    encoder.resolve_query_set(queries, 0..n, resolve, 0);
    encoder.copy_buffer_to_buffer(resolve, 0, &slots[slot].read, 0, u64::from(n) * 8);
    pending.push([encoder.finish()]);
    slots[slot].pending = Some(std::mem::take(&mut current.records));
    current.map = Some(slot);
}

fn map_marks(marks: Option<ResMut<GpuMarks>>) {
    let Some(mut marks) = marks else { return };
    let GpuMarks { slots, current, .. } = &mut *marks;
    let Some(slot) = current.get_mut().unwrap_or_else(|e| e.into_inner()).map.take() else { return };
    let mapped = slots[slot].mapped.clone();
    slots[slot].read.slice(..).map_async(MapMode::Read, move |r| mapped.store(r.is_ok(), Ordering::Release));
}

fn gpu_frame(records: &[Record], ticks: &[u64], period: f32) -> GpuFrame {
    let at = |r: &Record| ticks.get(r.index as usize).copied().unwrap_or(0);
    let ms = |a: u64, b: u64| b.saturating_sub(a) as f32 * period * 1e-6;
    let find = |mark: u8, view: Option<Entity>| records.iter().find(|r| r.mark == mark && r.view == view).map(at);
    let mut out = GpuFrame::default();
    if let (Some(a), Some(b)) = (find(FRAME_BEGIN, None), find(FRAME_END, None)) {
        out.frame = ms(a, b);
    }
    let mut views: Vec<Entity> = records.iter().filter_map(|r| r.view).collect();
    views.dedup();
    for v in views {
        let t = |m| find(m, Some(v));
        let seg = |a: u8, b: u8| t(a).zip(t(b)).map_or(0.0, |(a, b)| ms(a, b));
        out.views += seg(VIEW_BEGIN, VIEW_END);
        out.cull += seg(VIEW_BEGIN, SHADOW_BEGIN) + seg(SHADOW_END, MAIN_BEGIN);
        out.shadow += seg(SHADOW_BEGIN, SHADOW_END);
        out.main += seg(MAIN_BEGIN, MAIN_END);
        out.post += seg(MAIN_END, VIEW_END);
    }
    out
}
