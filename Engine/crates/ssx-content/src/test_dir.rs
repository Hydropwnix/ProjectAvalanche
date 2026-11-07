use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct TestDir(pub PathBuf);

impl TestDir {
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("ssx-content-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("a temporary folder");
        TestDir(p)
    }

    pub fn write(&self, rel: &str, text: &str) {
        self.write_bytes(rel, text.as_bytes());
    }

    pub fn write_bytes(&self, rel: &str, bytes: &[u8]) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("folders");
        std::fs::write(p, bytes).expect("a file");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn png_rgba(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(png::ColorType::Rgba);
    let mut writer = e.write_header().expect("a PNG header");
    let data: Vec<u8> = rgba.iter().copied().cycle().take((w * h * 4) as usize).collect();
    writer.write_image_data(&data).expect("PNG data");
    writer.finish().expect("a PNG");
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

pub fn png_header_only(w: u32, h: u32) -> Vec<u8> {
    let mut chunk = b"IHDR".to_vec();
    chunk.extend(w.to_be_bytes());
    chunk.extend(h.to_be_bytes());
    chunk.extend([8, 6, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(13u32.to_be_bytes());
    out.extend(&chunk);
    out.extend(crc32(&chunk).to_be_bytes());
    out
}
