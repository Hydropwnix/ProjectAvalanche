use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::view::ViewTarget;
use ssx_ui::game::fe::globe_scene::GlobeFrame;

use super::super::super::UiCamera;
use super::super::dump::{self, Dumps};
use super::super::midground::MipTexture;
use super::super::{GlobeTexture, GlobeView, uniform};
use super::GlobeGpu;
use super::pipelines::Pipelines;
use super::upload;
use super::upload::{
    buffer, floats_bytes, targets, upload_lut, upload_mesh, upload_midground, upload_mips, upload_texture, upload_waves,
};
use crate::render::post::adapt::{AdaptMode, HdrAdaptation};

fn gaussian_taps(sigma: f32) -> ([f32; 4], [f32; 4]) {
    let mut w = [0.0f32; 9];
    for (i, v) in w.iter_mut().enumerate() {
        let x = i as f32 - 4.0;
        *v = (-x * x / (2.0 * sigma * sigma)).exp() / (2.0 * std::f32::consts::PI * sigma * sigma);
    }
    let sum: f32 = w.iter().sum();
    w.iter_mut().for_each(|v| *v /= sum);
    let outer = 3.0 + w[0] / (w[0] + w[1]);
    let inner = 1.0 + w[2] / (w[2] + w[3]);
    ([w[0] + w[1], w[2] + w[3], w[4], 0.0], [-outer, -inner, inner, outer])
}

fn post_uniform(
    view: &GlobeView,
    frame: &ssx_ui::game::fe::globe_scene::GlobeFrame,
    size: UVec2,
    mode: f32,
) -> Vec<u8> {
    let l = &frame.light;
    let f = |n: &str| l.float(n).unwrap_or(0.0);
    let [a, b, c, d, e, ff] = [
        "toneMappingShoulderStrength",
        "toneMappingLinearStrength",
        "toneMappingLinearAngle",
        "toneMappingToeStrength",
        "toneMappingToeNumerator",
        "toneMappingToeDenominator",
    ]
    .map(f);
    let w = f("toneMappingLinearWhite");
    let hable = |x: f32| ((x * (a * x + c * b) + d * e) / (x * (a * x + b) + d * ff)) - e / ff;
    let post = view.post.as_deref();
    let bloom = post.is_some_and(|p| p.bloom);
    let blur = post.map_or([3.0, 3.0], |p| p.blur);
    let (hs, ho) = gaussian_taps(blur[0]);
    let (vs, vo) = gaussian_taps(blur[1]);
    let (vig_on, vig_colour, vig_bounds) = match post.and_then(|p| p.vignette) {
        Some((c, b)) => (1.0, c, b),
        None => (0.0, [0.0; 3], [0.0, 1.0]),
    };
    let fm = post.map_or([1.0, 0.0], |p| p.final_modifier);
    let mut out = floats_bytes(&[
        a,
        b,
        b * c,
        d * e,
        d * ff,
        e / ff,
        1.0 / hable(w),
        0.0,
        f("globalLightingMult"),
        f("dynamicExposureKeyValue"),
        f("dynamicExposureAdaptionRate"),
        view.dt,
        f("bloom_threshold"),
        f("bloom_weight"),
        if bloom { 1.0 } else { 0.0 },
        mode,
    ]);
    for v in [hs, ho, vs, vo] {
        out.extend(floats_bytes(&v));
    }
    out.extend(floats_bytes(&[vig_colour[0], vig_colour[1], vig_colour[2], vig_on]));
    out.extend(floats_bytes(&[vig_bounds[0], vig_bounds[1], fm[0], fm[1]]));
    out.extend(floats_bytes(&[view.raw_scale.unwrap_or(0.0), 0.0, 0.0, 0.0]));
    let q = UVec2::new((size.x / 4).max(1), (size.y / 4).max(1));
    for v in [size.x, size.y, q.x, q.y] {
        out.extend(v.to_le_bytes());
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    view: Option<Res<GlobeView>>,
    views: Query<&ViewTarget, With<UiCamera>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    pipelines: Option<ResMut<Pipelines>>,
    cache: Res<PipelineCache>,
    mut gpu: ResMut<GlobeGpu>,
    dumps: Res<Dumps>,
    mut adapt: ResMut<HdrAdaptation>,
) {
    gpu.active = false;
    gpu.dump = None;
    let (Some(view), Some(mut p)) = (view, pipelines) else { return };
    let (Some(assets), Some(frame)) = (view.assets.as_ref(), view.frame.as_ref()) else {
        gpu.targets = None;
        return;
    };
    let Some(target) = views.iter().next() else { return };
    let format = target.main_texture_format();
    if !p.composite.contains_key(&format) {
        let (layout, shader) = (p.composite_layout.clone(), p.post.clone());
        let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("ssx_globe_composite".into()),
            layout: vec![layout],
            vertex: VertexState {
                shader: shader.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("vs_full".into()),
                buffers: Vec::new(),
                constants: Vec::new(),
            },
            fragment: Some(FragmentState {
                shader,
                entry_point: Some("fs_composite".into()),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        });
        p.composite.insert(format, id);
    }
    if !p.fade.contains_key(&format) {
        let (layout, shader) = (p.fade_layout.clone(), p.fade_shader.clone());
        let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("ssx_screen_fade".into()),
            layout: vec![layout],
            vertex: VertexState {
                shader: shader.clone(),
                shader_defs: Vec::new(),
                entry_point: Some("vs_full".into()),
                buffers: Vec::new(),
                constants: Vec::new(),
            },
            fragment: Some(FragmentState {
                shader,
                entry_point: Some("fs_fade".into()),
                targets: vec![Some(ColorTargetState { format, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        });
        p.fade.insert(format, id);
    }
    let key = Arc::as_ptr(assets) as usize;
    if gpu.uploaded != Some(key) {
        gpu.earth = Some(upload_mesh(&device, &queue, &assets.earth));
        gpu.halo = Some(upload_mesh(&device, &queue, &assets.halo));
        gpu.diffuse = Some(upload_texture(&device, &queue, "ssx_globe_diffuse", &assets.diffuse));
        gpu.relief = Some(upload_texture(&device, &queue, "ssx_globe_relief", &assets.relief));
        gpu.frost = Some(upload_texture(&device, &queue, "ssx_globe_frost", &assets.frost));
        gpu.background = Some(upload_texture(&device, &queue, "ssx_globe_background", &assets.background));
        gpu.sky_textures =
            assets.sky_textures.iter().map(|(k, t)| (k.clone(), upload_mips(&device, &queue, t))).collect();
        let blank = GlobeTexture { width: 1, height: 1, rgba: vec![0, 0, 0, 0], blocks: None };
        gpu.blank = Some(upload_texture(&device, &queue, "ssx_globe_blank", &blank));
        gpu.cloud_texture = assets.particle_textures.values().next().map(|t| upload_mips(&device, &queue, t));
        gpu.uploaded = Some(key);
    }
    let size = UVec2::new(target.main_texture().width(), target.main_texture().height());
    if gpu.targets.as_ref().is_none_or(|t| t.size != size) {
        gpu.targets = Some(targets(&device, size));
    }
    if let Some(post) = view.post.as_ref()
        && (gpu.lut.is_none() || gpu.lut_version != post.lut_version)
    {
        gpu.lut = Some(upload_lut(&device, &queue, &post.lut));
        gpu.lut_version = post.lut_version;
    }
    if gpu.lut.is_none() {
        let identity = ssx_assets::postfx::ColourCube::identity().rgba8();
        gpu.lut = Some(upload_lut(&device, &queue, &identity));
    }
    let (r, ra) = (frame.globe_r[0], frame.atmos_r[0]);
    let earth = floats_bytes(&uniform(frame, &assets.earth_material, &frame.light, r, ra));
    let halo = floats_bytes(&uniform(frame, &assets.halo_material, &frame.light, r, ra));
    let mut mode = AdaptMode::Clear;
    if let Some(t) = gpu.targets.as_mut() {
        mode = adapt.begin(&device, &queue, upload::quarter_of(size), &t.adapted, t.fresh);
        t.fresh = false;
    }
    if view.raw_scale.is_some() {
        mode = AdaptMode::Current;
    }
    adapt.ran();
    let post = post_uniform(&view, frame, size, mode.code());
    gpu.bloom = view.post.as_ref().is_some_and(|p| p.bloom);
    let mg = frame.midground.as_ref().zip(view.midground.as_ref()).filter(|(f, m)| f.range == m.range);
    gpu.midground = false;
    if let Some((mf, m)) = mg {
        let key = Arc::as_ptr(m) as usize;
        if gpu.mg_uploaded != Some(key) {
            gpu.mg_parts = upload_midground(&device, &queue, m);
            gpu.mg_waves = upload_waves(&device, &queue, m);
            gpu.mg_uploaded = Some(key);
        }
        for (part, (g, _)) in m.waves.iter().zip(gpu.mg_waves.iter_mut()) {
            let data = floats_bytes(&super::super::midground::wave_uniform(mf, &part.material, &frame.light));
            match g.uniform.as_ref() {
                Some(b) => queue.write_buffer(b, 0, &data),
                None => g.uniform = Some(buffer(&device, &queue, "ssx_wave_uniform", BufferUsages::UNIFORM, &data)),
            }
        }
        for (part, g) in m.parts.iter().zip(gpu.mg_parts.iter_mut()) {
            let data = floats_bytes(&super::super::midground_uniform(
                mf,
                &part.material,
                &frame.light,
                view.raw_scale.is_some(),
            ));
            match g.uniform.as_ref() {
                Some(b) => queue.write_buffer(b, 0, &data),
                None => g.uniform = Some(buffer(&device, &queue, "ssx_mg_uniform", BufferUsages::UNIFORM, &data)),
            }
        }
        gpu.midground = true;
    }
    gpu.globe = frame.visible;
    gpu.sky = (!frame.visible).then(|| {
        super::super::backdrop::sky_textures(&frame.light).map(|t| t.map(|p| p.replace('\\', "/")).unwrap_or_default())
    });
    let backdrop = floats_bytes(&super::super::backdrop::uniform(frame, [size.x, size.y]));
    let fade = floats_bytes(&[frame.fade[0], frame.fade[1], 0.0, 0.0]);
    let write = |slot: &mut Option<Buffer>, label: &'static str, data: &[u8]| match slot {
        Some(b) => queue.write_buffer(b, 0, data),
        None => *slot = Some(buffer(&device, &queue, label, BufferUsages::UNIFORM, data)),
    };
    write(&mut gpu.earth_uniform, "ssx_globe_earth_uniform", &earth);
    write(&mut gpu.halo_uniform, "ssx_globe_halo_uniform", &halo);
    write(&mut gpu.backdrop_uniform, "ssx_globe_backdrop_uniform", &backdrop);
    write(&mut gpu.post_uniform, "ssx_globe_post_uniform", &post);
    write(&mut gpu.fade_uniform, "ssx_screen_fade_uniform", &fade);
    let half = UVec2::new((size.x / 2).max(1), (size.y / 2).max(1));
    gpu.clouds.clear();
    for d in &frame.clouds {
        let u = floats_bytes(&cloud_uniform(frame, d, half, size));
        let mut records = Vec::with_capacity(d.particles.len() * 8);
        for p in &d.particles {
            records.extend([p.pos[0], p.pos[1], p.pos[2], p.size, p.ori[0], p.ori[1], p.ori[2], p.birth]);
        }
        let ub = buffer(&device, &queue, "ssx_globe_cloud_uniform", BufferUsages::UNIFORM, &u);
        let rb = buffer(&device, &queue, "ssx_globe_cloud_records", BufferUsages::STORAGE, &floats_bytes(&records));
        gpu.clouds.push((ub, rb, d.particles.len() as u32));
    }
    if let Some(dir) = view.dump.as_deref() {
        let mut done = dumps.done.lock().unwrap_or_else(|e| e.into_inner());
        if done.as_deref() != Some(dir.as_path()) {
            *done = Some(dir.clone());
            gpu.dump = Some(dir.clone());
            let _ = std::fs::remove_file(dir.join("index.txt"));
            let lut = view.post.as_ref().map(|p| p.lut.to_vec());
            let lut = lut.unwrap_or_else(|| ssx_assets::postfx::ColourCube::identity().rgba8());
            let inputs =
                DumpInputs { post: &post, backdrop: &backdrop, fade: &fade, earth: &earth, halo: &halo, lut: &lut };
            dump_inputs(dir, &view, frame, size, &inputs);
        }
    }
    gpu.active = true;
}

struct DumpInputs<'a> {
    post: &'a [u8],
    backdrop: &'a [u8],
    fade: &'a [u8],
    earth: &'a [u8],
    halo: &'a [u8],
    lut: &'a [u8],
}

fn dump_inputs(dir: &std::path::Path, view: &GlobeView, frame: &GlobeFrame, size: UVec2, i: &DumpInputs) {
    let Some(assets) = view.assets.as_ref() else { return };
    let bytes = |name: &str, b: &[u8]| dump::write(dir, name, b.len(), 1, "bytes", b);
    bytes("post_uniform", i.post);
    bytes("backdrop_uniform", i.backdrop);
    bytes("fade_uniform", i.fade);
    bytes("earth_uniform", i.earth);
    bytes("halo_uniform", i.halo);
    dump::write(dir, "lut", 32, 32 * 32, "Rgba8Unorm", i.lut);
    let bg = &assets.background;
    dump::write(dir, "background", bg.width as usize, bg.height as usize, "Rgba8Unorm", &bg.rgba);
    let mips = |name: &str, t: &MipTexture| {
        for (l, data) in t.levels.iter().enumerate() {
            let (w, h) = ((t.width >> l).max(1) as usize, (t.height >> l).max(1) as usize);
            dump::write(dir, &format!("{name}_mip{l}"), w, h, "Rgba8Unorm", data);
        }
    };
    if let Some(t) = assets.particle_textures.values().next() {
        mips("cloud_texture", t);
    }
    let half = UVec2::new((size.x / 2).max(1), (size.y / 2).max(1));
    for (n, d) in frame.clouds.iter().enumerate() {
        bytes(&format!("cloud{n}_uniform"), &floats_bytes(&cloud_uniform(frame, d, half, size)));
        let records: Vec<f32> = d
            .particles
            .iter()
            .flat_map(|p| [p.pos[0], p.pos[1], p.pos[2], p.size, p.ori[0], p.ori[1], p.ori[2], p.birth])
            .collect();
        bytes(&format!("cloud{n}_records"), &floats_bytes(&records));
    }
    let mg = frame.midground.as_ref().zip(view.midground.as_ref()).filter(|(f, m)| f.range == m.range);
    if let Some((mf, m)) = mg {
        for (n, w) in m.waves.iter().enumerate() {
            bytes(
                &format!("wave{n}_uniform"),
                &floats_bytes(&super::super::midground::wave_uniform(mf, &w.material, &frame.light)),
            );
            let flat = |v: &[[f32; 3]]| floats_bytes(&v.iter().flatten().copied().collect::<Vec<f32>>());
            bytes(&format!("wave{n}_positions"), &flat(&w.mesh.positions));
            bytes(&format!("wave{n}_normals"), &flat(&w.mesh.normals));
            bytes(&format!("wave{n}_tangents"), &flat(&w.mesh.tangents));
            bytes(&format!("wave{n}_uvs"), &floats_bytes(&w.mesh.uvs.iter().flatten().copied().collect::<Vec<f32>>()));
            bytes(&format!("wave{n}_colours"), &w.colours.iter().flatten().copied().collect::<Vec<u8>>());
            let idx: Vec<u8> = w.mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            bytes(&format!("wave{n}_indices"), &idx);
            for (k, t) in w.textures.iter().enumerate() {
                mips(&format!("wave{n}_tex{k}"), t);
            }
        }
    }
    let text = format!(
        "dt {}\nsize {} {}\nvisible {}\nclouds {}\n",
        view.dt,
        size.x,
        size.y,
        frame.visible,
        frame.clouds.len()
    );
    dump::write(dir, "frame", text.len(), 1, "text", text.as_bytes());
}

fn cloud_uniform(
    frame: &ssx_ui::game::fe::globe_scene::GlobeFrame,
    d: &ssx_ui::game::fe::globe_clouds::CloudDraw,
    half: UVec2,
    full: UVec2,
) -> Vec<f32> {
    let r = &d.render;
    let l = &frame.light;
    let lv = |n: &str| l.vector(n).unwrap_or([0.0; 4]);
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    let identity: [[f32; 4]; 4] = std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }));
    let (r0, r1) = (lv("rampColor0"), lv("rampColor1"));
    let vectors: [[f32; 4]; 21] = [
        [d.time_ms as f32, r.age_max as f32, 0.0, d.transparency],
        r.color_mult,
        r.color_mult_spread,
        r.color_ramp[0],
        r.color_ramp[1],
        r.color_ramp[2],
        r.color_ramp[3],
        r.color_ramp_times,
        r.size_ramp,
        r.size_ramp_times,
        [b(r.use_color_ramp), b(r.use_color_mult), b(r.use_size_ramp), b(r.use_anim_texture)],
        [r.anim_mode as f32, r.anim_count as f32, r.anim_rows as f32, r.anim_speed],
        [r.anim_random_start as f32, 0.0, 0.0, 0.0],
        r.pivot_shift,
        [r.near_fade[0], r.near_fade[1], r.z_bias, 0.0],
        [r.z_feather[0], r.z_feather[1], 0.0, 0.0],
        [r0[0], r0[1], r0[2], l.float("rampIntensity").unwrap_or(0.0)],
        [r1[0], r1[1], r1[2], 0.0],
        [half.x as f32, half.y as f32, full.x as f32, full.y as f32],
        [frame.proj[2][2], frame.proj[3][2], 0.0, 0.0],
        [0.0; 4],
    ];
    let mut out = Vec::with_capacity(32 + 84);
    out.extend(frame.proj.iter().flatten());
    out.extend(identity.iter().flatten());
    out.extend(vectors.iter().take(20).flatten());
    out
}
