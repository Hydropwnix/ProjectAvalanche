use std::collections::HashMap;

use bevy::prelude::*;
use bevy::render::render_phase::TrackedRenderPass;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;

use super::super::{UiFrame, UiLayer, blur_passes};
use super::pipelines::UiPipelines;
use super::{FILTER_PARAMS_BYTES, Full, SAMPLES, UI_FORMAT, UiGpu, fullscreen, replay, texture_2d_desc};

pub(super) struct PendingLayer {
    key: u64,
    index: usize,
    out: LayerTexture,
}

#[derive(Clone)]
pub(super) struct LayerTexture {
    size: UVec2,
    samples: u32,
    view: TextureView,
    full: Option<BindGroup>,
    paint: Option<BindGroup>,
}

#[derive(Default)]
pub(super) struct LayerPool {
    free: HashMap<(UVec2, u32), Vec<(LayerTexture, u64)>>,
    blur_params: HashMap<[u32; 12], BindGroup>,
}

const LAYER_CACHE_BUDGET: u64 = 48 << 20;

fn layer_bytes(t: &LayerTexture) -> u64 {
    u64::from(t.size.x) * u64::from(t.size.y) * u64::from(t.samples) * 4
}

impl LayerPool {
    const KEEP: u64 = 120;
    const BUDGET: u64 = 48 << 20;

    fn take(&mut self, kit: &LayerKit, label: &'static str, size: UVec2, samples: u32) -> LayerTexture {
        if let Some((t, _)) = self.free.get_mut(&(size, samples)).and_then(Vec::pop) {
            return t;
        }
        let mut usage = TextureUsages::RENDER_ATTACHMENT;
        if samples == 1 {
            usage |= TextureUsages::TEXTURE_BINDING;
        }
        let view = kit
            .device
            .create_texture(&texture_2d_desc(label, size, UI_FORMAT, samples, usage))
            .create_view(&TextureViewDescriptor::default());
        LayerTexture { size, samples, view, full: None, paint: None }
    }

    fn give(&mut self, t: LayerTexture, now: u64) {
        self.free.entry((t.size, t.samples)).or_default().push((t, now));
    }

    fn trim(&mut self, now: u64) {
        self.free.retain(|_, v| {
            v.retain(|(_, freed)| now - *freed < Self::KEEP);
            !v.is_empty()
        });
        let mut by_age: Vec<(u64, u64)> = self.free.values().flatten().map(|(t, f)| (*f, layer_bytes(t))).collect();
        let mut left: u64 = by_age.iter().map(|(_, bytes)| bytes).sum();
        if left > Self::BUDGET {
            by_age.sort_unstable();
            let mut cut = now + 1;
            for (freed, bytes) in by_age {
                if left <= Self::BUDGET {
                    cut = freed;
                    break;
                }
                left -= bytes;
            }
            self.free.retain(|_, v| {
                v.retain(|(_, freed)| *freed >= cut);
                !v.is_empty()
            });
        }
        if self.blur_params.len() > 256 {
            self.blur_params.clear();
        }
    }
}

impl LayerTexture {
    fn full(&mut self, kit: &LayerKit) -> BindGroup {
        self.full
            .get_or_insert_with(|| {
                kit.device.create_bind_group(
                    "ssx_ui_full",
                    &kit.full_layout,
                    &BindGroupEntries::sequential((&self.view, kit.linear)),
                )
            })
            .clone()
    }

    fn paint(&mut self, kit: &LayerKit) -> BindGroup {
        self.paint
            .get_or_insert_with(|| {
                kit.device.create_bind_group(
                    "ssx_ui_layer_paint",
                    &kit.paint_layout,
                    &BindGroupEntries::sequential((&self.view, kit.linear)),
                )
            })
            .clone()
    }
}

pub(super) struct LayerKit<'a> {
    device: &'a RenderDevice,
    full_layout: BindGroupLayout,
    filter_layout: BindGroupLayout,
    paint_layout: BindGroupLayout,
    linear: &'a Sampler,
    blur: RenderPipeline,
    compose: RenderPipeline,
    glyphs: RenderPipeline,
}

impl LayerKit<'_> {
    fn params(&self, words: [f32; 12], second: &TextureView) -> BindGroup {
        let mut bytes = Vec::with_capacity(48);
        for w in words {
            bytes.extend_from_slice(&w.to_le_bytes());
        }
        debug_assert_eq!(bytes.len() as u64, FILTER_PARAMS_BYTES);
        let buffer = self.device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("ssx_ui_filter"),
            contents: &bytes,
            usage: BufferUsages::UNIFORM,
        });
        self.device.create_bind_group(
            "ssx_ui_filter",
            &self.filter_layout,
            &BindGroupEntries::sequential((buffer.as_entire_binding(), second)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn blur(
        &self,
        encoder: &mut CommandEncoder,
        pool: &mut LayerPool,
        held: &mut Vec<LayerTexture>,
        src: usize,
        size: UVec2,
        half: Vec2,
        passes: u32,
        alpha_only: bool,
        dummy: &TextureView,
    ) -> usize {
        let mut cur = src;
        let flag = if alpha_only { 1.0 } else { 0.0 };
        for _ in 0..passes {
            for (dir, h) in [(Vec2::X, half.x), (Vec2::Y, half.y)] {
                if h <= 0.0 {
                    continue;
                }
                let dst = pool.take(self, "ssx_ui_blur", size, 1);
                let words = [dir.x, dir.y, h, flag, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
                let params = match pool.blur_params.get(&words.map(f32::to_bits)) {
                    Some(g) => g.clone(),
                    None => {
                        let g = self.params(words, dummy);
                        pool.blur_params.insert(words.map(f32::to_bits), g.clone());
                        g
                    }
                };
                let source = held[cur].full(self);
                fullscreen(
                    encoder,
                    "ssx_ui_blur",
                    &dst.view,
                    true,
                    Full { pipeline: &self.blur, groups: vec![source, params] },
                );
                held.push(dst);
                cur = held.len() - 1;
            }
        }
        cur
    }
}

fn render_layer(
    encoder: &mut CommandEncoder,
    kit: &LayerKit,
    gpu: &UiGpu,
    pool: &mut LayerPool,
    now: u64,
    l: &UiLayer,
    out: &LayerTexture,
) -> Option<()> {
    let (vb, pb, db, ib, draws) = (
        gpu.vertices.as_ref()?,
        gpu.paint_coords.as_ref()?,
        gpu.draw_ids.as_ref()?,
        gpu.indices.as_ref()?,
        gpu.draws_group.as_ref()?,
    );
    let dummy = gpu.clear_texture.as_ref()?;
    let mut held = vec![pool.take(kit, "ssx_ui_layer", l.size, SAMPLES)];
    const TEXT: usize = 0;
    {
        let pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("ssx_ui_layer"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &held[TEXT].view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(default()), store: StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut rp = TrackedRenderPass::new(kit.device, pass);
        rp.set_vertex_buffer(0, vb.slice(..));
        rp.set_vertex_buffer(1, pb.slice(..));
        rp.set_vertex_buffer(2, db.slice(..));
        rp.set_index_buffer(ib.slice(..), IndexFormat::Uint32);
        rp.set_bind_group(0, draws, &[]);
        let glyphs = &kit.glyphs;
        replay(&mut rp, &l.ops, gpu, &[], &|_, mode| (mode == 0).then_some(glyphs));
    }
    let f = &l.filter;
    let k = l.px_per_twip / 2.0;
    let blurred = if f.blur.0 > 0.0 || f.blur.1 > 0.0 {
        let half = Vec2::new(f.blur.0, f.blur.1) * k;
        kit.blur(encoder, pool, &mut held, TEXT, l.size, half, blur_passes(f.blur_high_quality), false, dummy)
    } else {
        TEXT
    };
    let (shadow, words) = match f.shadow {
        Some(s) => {
            let half = Vec2::new(s.blur_x, s.blur_y) * k;
            let src = kit.blur(encoder, pool, &mut held, TEXT, l.size, half, blur_passes(s.high_quality), true, dummy);
            let flags = f32::from(u8::from(s.knockout) | (u8::from(s.hide_object) << 1));
            let o = Vec2::new(s.offset.0, s.offset.1) * l.px_per_twip;
            let words =
                [0.0, 0.0, 0.0, 0.0, s.color[0], s.color[1], s.color[2], s.color[3], o.x, o.y, s.strength, flags];
            (Some(src), words)
        }
        None => (None, [0.0; 12]),
    };
    let params = kit.params(words, shadow.map_or(dummy, |i| &held[i].view));
    let source = held[blurred].full(kit);
    fullscreen(
        encoder,
        "ssx_ui_compose",
        &out.view,
        true,
        Full { pipeline: &kit.compose, groups: vec![source, params] },
    );
    for t in held {
        pool.give(t, now);
    }
    Some(())
}

pub(super) fn layer_kit<'a>(
    p: &'a UiPipelines,
    cache: &PipelineCache,
    device: &'a RenderDevice,
) -> Option<LayerKit<'a>> {
    let get = |id: CachedRenderPipelineId| cache.get_render_pipeline(id).cloned();
    Some(LayerKit {
        device,
        full_layout: cache.get_bind_group_layout(&p.full_layout),
        filter_layout: cache.get_bind_group_layout(&p.filter_layout),
        paint_layout: cache.get_bind_group_layout(&p.paint_layout),
        linear: &p.linear,
        blur: get(p.blur)?,
        compose: get(p.compose)?,
        glyphs: get(p.layer)?,
    })
}

pub(super) fn prepare_layers(
    frame: &UiFrame,
    p: &UiPipelines,
    cache: &PipelineCache,
    device: &RenderDevice,
    gpu: &mut UiGpu,
) {
    gpu.frame_counter += 1;
    gpu.layer_groups.clear();
    let now = gpu.frame_counter;
    for stale in std::mem::take(&mut gpu.pending_layers) {
        gpu.layer_cache.remove(&stale.key);
        gpu.layer_pool.give(stale.out, now);
    }
    if frame.layers.is_empty() {
        return;
    }
    let Some(kit) = layer_kit(p, cache, device) else { return };
    let mut pool = std::mem::take(&mut gpu.layer_pool);
    for (index, l) in frame.layers.iter().enumerate() {
        let group = match gpu.layer_cache.get_mut(&l.key) {
            Some((t, used)) => {
                *used = now;
                t.paint.clone()
            }
            None => {
                let mut out = pool.take(&kit, "ssx_ui_layer_out", l.size, 1);
                let g = out.paint(&kit);
                gpu.layer_cache.insert(l.key, (out.clone(), now));
                gpu.pending_layers.push(PendingLayer { key: l.key, index, out });
                Some(g)
            }
        };
        let fallback = gpu.white.as_ref().map(|w| w.1.clone());
        if let Some(g) = group.or(fallback) {
            gpu.layer_groups.push(g);
        }
    }
    // TODO(guess): filtered layers kept 120 frames after their last use and within a byte budget (least recently used first); settle: Scaleform's glyph cache size and eviction (not traced).
    gpu.layer_cache.retain(|_, (t, used)| {
        let keep = now - *used < 120;
        if !keep {
            pool.give(t.clone(), now);
        }
        keep
    });
    let mut cached: u64 = gpu.layer_cache.values().map(|(t, _)| layer_bytes(t)).sum();
    if cached > LAYER_CACHE_BUDGET {
        let mut old: Vec<(u64, u64)> =
            gpu.layer_cache.iter().filter(|(_, (_, used))| *used < now).map(|(k, (_, used))| (*used, *k)).collect();
        old.sort_unstable();
        for (_, key) in old {
            if cached <= LAYER_CACHE_BUDGET {
                break;
            }
            if let Some((t, _)) = gpu.layer_cache.remove(&key) {
                cached -= layer_bytes(&t);
                pool.give(t, now);
            }
        }
    }
    pool.trim(now);
    if now.is_multiple_of(300) && std::env::var_os("SSX_UI_LAYER_LOG").is_some() {
        let mb = |size: UVec2, samples: u32| f64::from(size.x * size.y * samples) * 4.0 / 1048576.0;
        let cached: f64 = gpu.layer_cache.values().map(|(t, _)| mb(t.size, t.samples)).sum();
        let free: f64 = pool.free.values().flatten().map(|(t, _)| mb(t.size, t.samples)).sum();
        let largest = gpu.layer_cache.values().map(|(t, _)| t.size).max_by_key(|s| s.x * s.y);
        info!(
            "ui layers at frame {now}: {} this frame, {} cached ({cached:.0} MiB, the largest {largest:?}), {} free \
             ({free:.0} MiB)",
            frame.layers.len(),
            gpu.layer_cache.len(),
            pool.free.values().map(Vec::len).sum::<usize>()
        );
    }
    gpu.layer_pool = pool;
}

pub(super) fn draw_pending_layers(encoder: &mut CommandEncoder, kit: &LayerKit, frame: &UiFrame, gpu: &mut UiGpu) {
    let pending = std::mem::take(&mut gpu.pending_layers);
    let mut pool = std::mem::take(&mut gpu.layer_pool);
    let now = gpu.frame_counter;
    for p in &pending {
        let drawn = frame.layers.get(p.index).and_then(|l| render_layer(encoder, kit, gpu, &mut pool, now, l, &p.out));
        if drawn.is_none() {
            gpu.layer_cache.remove(&p.key);
        }
    }
    gpu.layer_pool = pool;
}
