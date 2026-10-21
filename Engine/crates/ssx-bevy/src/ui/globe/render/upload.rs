use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};

use super::super::midground::MipTexture;
use super::super::{GlobeMesh, GlobeTexture};
use super::{DEPTH_FORMAT, HDR_FORMAT, LDR_FORMAT};

pub(super) struct MeshBuffers {
    pub(super) positions: Buffer,
    pub(super) uvs: Buffer,
    pub(super) indices: Buffer,
    pub(super) count: u32,
}

pub(super) struct Targets {
    pub(super) size: UVec2,
    pub(super) depth: Texture,
    pub(super) depth_view: TextureView,
    pub(super) hdr: Texture,
    pub(super) hdr_view: TextureView,
    pub(super) frame: Texture,
    pub(super) frame_view: TextureView,
    pub(super) quarter: Texture,
    pub(super) quarter_view: TextureView,
    pub(super) bloom_a: Texture,
    pub(super) bloom_a_view: TextureView,
    pub(super) bloom_b: Texture,
    pub(super) bloom_b_view: TextureView,
    pub(super) ldr: Texture,
    pub(super) ldr_view: TextureView,
    pub(super) ldr_aa: Texture,
    pub(super) ldr_aa_view: TextureView,
    pub(super) half_depth: Texture,
    pub(super) half_depth_view: TextureView,
    pub(super) half_colour: Texture,
    pub(super) half_colour_view: TextureView,
    pub(super) adapted: Buffer,
    pub(super) exposure: Buffer,
    pub(super) fresh: bool,
}

pub(super) struct MidgroundGpu {
    pub(super) positions: Buffer,
    pub(super) normals: Buffer,
    pub(super) tangents: Buffer,
    pub(super) uvs: Buffer,
    pub(super) indices: Buffer,
    pub(super) count: u32,
    pub(super) textures: Vec<TextureView>,
    pub(super) uniform: Option<Buffer>,
}

pub(super) fn floats_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub(super) fn buffer(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: &'static str,
    usage: BufferUsages,
    data: &[u8],
) -> Buffer {
    let b = device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size: (data.len().max(16) as u64).next_multiple_of(16),
        usage: usage | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&b, 0, data);
    b
}

pub(super) fn upload_mesh(device: &RenderDevice, queue: &RenderQueue, m: &GlobeMesh) -> MeshBuffers {
    let pos: Vec<u8> = m.positions.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    let uv: Vec<u8> = m.uvs.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    let idx: Vec<u8> = m.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    MeshBuffers {
        positions: buffer(device, queue, "ssx_globe_positions", BufferUsages::VERTEX, &pos),
        uvs: buffer(device, queue, "ssx_globe_uvs", BufferUsages::VERTEX, &uv),
        indices: buffer(device, queue, "ssx_globe_indices", BufferUsages::INDEX, &idx),
        count: m.indices.len() as u32,
    }
}

// TODO(guess): the host GPU's block decode stands in for the Xenos texture unit's, as for the terrain (`terrain::textures::block_format`); settle: the Xenos' own DXT interpolation (a capture of decoded texels from the console).
fn block_format(device: &RenderDevice, b: &super::super::Blocks) -> Option<TextureFormat> {
    use ssx_formats::xenos::TextureFormat as X;
    let off = std::env::var("SSX_BC_TEXTURES").is_ok_and(|v| matches!(v.as_str(), "off" | "0" | "false" | "fallback"));
    if off || !device.features().contains(WgpuFeatures::TEXTURE_COMPRESSION_BC) {
        return None;
    }
    match b.format {
        X::Dxt1 => Some(TextureFormat::Bc1RgbaUnorm),
        X::Dxt23 => Some(TextureFormat::Bc2RgbaUnorm),
        X::Dxt45 => Some(TextureFormat::Bc3RgbaUnorm),
        X::Dxn => Some(TextureFormat::Bc5RgUnorm),
        _ => None,
    }
}

fn upload_levels(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: &'static str,
    (width, height): (u32, u32),
    rgba: &[&[u8]],
    blocks: Option<&super::super::Blocks>,
) -> TextureView {
    let block = blocks.and_then(|b| block_format(device, b).map(|f| (f, b.levels[..rgba.len()].concat())));
    let (format, data) = block.unwrap_or_else(|| (TextureFormat::Rgba8Unorm, rgba.concat()));
    let desc = TextureDescriptor {
        label: Some(label),
        size: Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: rgba.len().max(1) as u32,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    };
    device
        .create_texture_with_data(queue, &desc, TextureDataOrder::LayerMajor, &data)
        .create_view(&TextureViewDescriptor::default())
}

pub(super) fn upload_texture(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: &'static str,
    t: &GlobeTexture,
) -> TextureView {
    upload_levels(device, queue, label, (t.width, t.height), &[&t.rgba], t.blocks.as_ref())
}

pub(super) fn upload_mips(device: &RenderDevice, queue: &RenderQueue, t: &MipTexture) -> TextureView {
    let levels: Vec<&[u8]> = t.levels.iter().map(Vec::as_slice).collect();
    upload_levels(device, queue, "ssx_globe_mip_texture", (t.width, t.height), &levels, t.blocks.as_ref())
}

pub(super) fn upload_lut(device: &RenderDevice, queue: &RenderQueue, rgba: &[u8]) -> TextureView {
    let n = 32;
    let desc = TextureDescriptor {
        label: Some("ssx_globe_colour_cube"),
        size: Extent3d { width: n, height: n, depth_or_array_layers: n },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D3,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    };
    device
        .create_texture_with_data(queue, &desc, TextureDataOrder::LayerMajor, rgba)
        .create_view(&TextureViewDescriptor::default())
}

pub(super) fn upload_midground(
    device: &RenderDevice,
    queue: &RenderQueue,
    m: &super::super::MidgroundAssets,
) -> Vec<MidgroundGpu> {
    let bytes = |v: &[[f32; 3]]| -> Vec<u8> { v.iter().flatten().flat_map(|f| f.to_le_bytes()).collect() };
    m.parts
        .iter()
        .map(|p| {
            let uv: Vec<u8> = p.mesh.uvs.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
            let idx: Vec<u8> = p.mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            MidgroundGpu {
                positions: buffer(device, queue, "ssx_mg_positions", BufferUsages::VERTEX, &bytes(&p.mesh.positions)),
                normals: buffer(device, queue, "ssx_mg_normals", BufferUsages::VERTEX, &bytes(&p.mesh.normals)),
                tangents: buffer(device, queue, "ssx_mg_tangents", BufferUsages::VERTEX, &bytes(&p.mesh.tangents)),
                uvs: buffer(device, queue, "ssx_mg_uvs", BufferUsages::VERTEX, &uv),
                indices: buffer(device, queue, "ssx_mg_indices", BufferUsages::INDEX, &idx),
                count: p.mesh.indices.len() as u32,
                textures: p.textures.iter().map(|t| upload_mips(device, queue, t)).collect(),
                uniform: None,
            }
        })
        .collect()
}

pub(super) fn upload_waves(
    device: &RenderDevice,
    queue: &RenderQueue,
    m: &super::super::MidgroundAssets,
) -> Vec<(MidgroundGpu, Buffer)> {
    let bytes = |v: &[[f32; 3]]| -> Vec<u8> { v.iter().flatten().flat_map(|f| f.to_le_bytes()).collect() };
    m.waves
        .iter()
        .map(|p| {
            let uv: Vec<u8> = p.mesh.uvs.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
            let idx: Vec<u8> = p.mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let colours: Vec<u8> = p.colours.iter().flatten().copied().collect();
            let g = MidgroundGpu {
                positions: buffer(device, queue, "ssx_wave_positions", BufferUsages::VERTEX, &bytes(&p.mesh.positions)),
                normals: buffer(device, queue, "ssx_wave_normals", BufferUsages::VERTEX, &bytes(&p.mesh.normals)),
                tangents: buffer(device, queue, "ssx_wave_tangents", BufferUsages::VERTEX, &bytes(&p.mesh.tangents)),
                uvs: buffer(device, queue, "ssx_wave_uvs", BufferUsages::VERTEX, &uv),
                indices: buffer(device, queue, "ssx_wave_indices", BufferUsages::INDEX, &idx),
                count: p.mesh.indices.len() as u32,
                textures: p.textures.iter().map(|t| upload_mips(device, queue, t)).collect(),
                uniform: None,
            };
            (g, buffer(device, queue, "ssx_wave_colours", BufferUsages::VERTEX, &colours))
        })
        .collect()
}

pub(super) fn target_texture(
    device: &RenderDevice,
    label: &'static str,
    size: UVec2,
    format: TextureFormat,
    usage: TextureUsages,
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

pub(super) fn targets(device: &RenderDevice, size: UVec2) -> Targets {
    let usage = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC;
    let view = |t: &Texture| t.create_view(&TextureViewDescriptor::default());
    let hdr = target_texture(device, "ssx_globe_hdr", size, HDR_FORMAT, usage);
    let frame = target_texture(device, "ssx_globe_frame_buffer", size, HDR_FORMAT, usage | TextureUsages::COPY_DST);
    let q = UVec2::new((size.x / 4).max(1), (size.y / 4).max(1));
    let quarter = target_texture(device, "ssx_globe_quarter", q, HDR_FORMAT, usage);
    let bloom_a = target_texture(device, "ssx_globe_bloom_a", q, LDR_FORMAT, usage);
    let bloom_b = target_texture(device, "ssx_globe_bloom_b", q, LDR_FORMAT, usage);
    let ldr = target_texture(device, "ssx_globe_ldr", size, LDR_FORMAT, usage);
    let ldr_aa = target_texture(device, "ssx_globe_ldr_aa", size, LDR_FORMAT, usage);
    let h = UVec2::new((size.x / 2).max(1), (size.y / 2).max(1));
    let half_depth = target_texture(device, "ssx_globe_half_depth", h, TextureFormat::R32Float, usage);
    let half_colour = target_texture(device, "ssx_globe_half_colour", h, LDR_FORMAT, usage);
    let (mut w, mut h, mut n) = (q.x, q.y, (q.x * q.y) as u64);
    while w > 1 || h > 1 {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        n += (w * h) as u64;
    }
    let storage = |label: &'static str, size: u64| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: size.next_multiple_of(16),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    };
    let depth = device.create_texture(&TextureDescriptor {
        label: Some("ssx_globe_depth"),
        size: Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    Targets {
        size,
        depth_view: view(&depth),
        depth,
        hdr_view: view(&hdr),
        hdr,
        frame_view: view(&frame),
        frame,
        quarter_view: view(&quarter),
        quarter,
        bloom_a_view: view(&bloom_a),
        bloom_a,
        bloom_b_view: view(&bloom_b),
        bloom_b,
        ldr_view: view(&ldr),
        ldr,
        ldr_aa_view: view(&ldr_aa),
        ldr_aa,
        half_depth_view: view(&half_depth),
        half_depth,
        half_colour_view: view(&half_colour),
        half_colour,
        adapted: storage("ssx_globe_adapted", n * 4),
        exposure: storage("ssx_globe_exposure", 16),
        fresh: true,
    }
}
