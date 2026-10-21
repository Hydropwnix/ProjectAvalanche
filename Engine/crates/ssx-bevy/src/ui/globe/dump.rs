use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bevy::prelude::*;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;

const ROW_ALIGNMENT: u32 = 256;

#[derive(Resource, Default)]
pub(super) struct Dumps {
    pub done: Mutex<Option<PathBuf>>,
    pending: Mutex<Vec<Readback>>,
}

struct Readback {
    dir: PathBuf,
    name: String,
    buffer: Buffer,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
    row: u32,
    format: String,
}

impl Dumps {
    fn push(&self, r: Readback) {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).push(r);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn texture(
        &self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        dir: &Path,
        name: &str,
        texture: &Texture,
        size: UVec2,
        format: TextureFormat,
    ) {
        let aspect = if format.is_depth_stencil_format() { TextureAspect::DepthOnly } else { TextureAspect::All };
        let bpp = format.block_copy_size(Some(aspect)).unwrap_or(4);
        let row = (size.x * bpp).next_multiple_of(ROW_ALIGNMENT);
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("ssx_globe_dump"),
            size: u64::from(row * size.y),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo { texture, mip_level: 0, origin: Origin3d::ZERO, aspect },
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
        );
        self.push(Readback {
            dir: dir.to_path_buf(),
            name: name.into(),
            buffer,
            width: size.x,
            height: size.y,
            bytes_per_pixel: bpp,
            row,
            format: format!("{format:?}"),
        });
    }

    pub fn buffer(
        &self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        dir: &Path,
        name: &str,
        source: &Buffer,
        size: u64,
    ) {
        let size = size.next_multiple_of(4);
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("ssx_globe_dump"),
            size,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(source, 0, &buffer, 0, size);
        self.push(Readback {
            dir: dir.to_path_buf(),
            name: name.into(),
            buffer,
            width: size as u32,
            height: 1,
            bytes_per_pixel: 1,
            row: size as u32,
            format: "bytes".into(),
        });
    }
}

pub(super) fn index(dir: &Path, line: &str) {
    let file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("index.txt"));
    if let Err(e) = file.and_then(|mut f| writeln!(f, "{line}")) {
        error!("globe dump: {}: {e}", dir.display());
    }
}

pub(super) fn write(dir: &Path, name: &str, width: usize, height: usize, format: &str, bytes: &[u8]) {
    if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(format!("{name}.raw")), bytes)) {
        error!("globe dump: {name}: {e}");
        return;
    }
    index(dir, &format!("{name} {width} {height} {format}"));
}

pub(super) fn write_dumps(dumps: Res<Dumps>, device: Res<RenderDevice>) {
    let pending = std::mem::take(&mut *dumps.pending.lock().unwrap_or_else(|e| e.into_inner()));
    if pending.is_empty() {
        return;
    }
    for r in &pending {
        r.buffer.slice(..).map_async(MapMode::Read, |_| {});
    }
    if let Err(e) = device.poll(PollType::wait_indefinitely()) {
        error!("globe dump: waiting for the GPU: {e}");
        return;
    }
    for r in &pending {
        let data = r.buffer.slice(..).get_mapped_range();
        let line = (r.width * r.bytes_per_pixel) as usize;
        let mut out = Vec::with_capacity(line * r.height as usize);
        for y in 0..r.height as usize {
            let start = y * r.row as usize;
            out.extend_from_slice(&data[start..start + line]);
        }
        drop(data);
        r.buffer.unmap();
        write(&r.dir, &r.name, r.width as usize, r.height as usize, &r.format, &out);
    }
    info!("globe dump: {} targets written to {}", pending.len(), pending[0].dir.display());
}
