use super::settings::TextureSettings;
use super::{Limits, Shape, Texture, TextureError, TextureKind, dds, jpeg, ktx2, mips, png};
use crate::hash::ContentHash;

pub const FORMATS: [&str; 5] = ["png", "jpg", "jpeg", "ktx2", "dds"];

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = b"\xFF\xD8\xFF";
const KTX2_MAGIC: &[u8] = b"\xABKTX 20\xBB\r\n\x1A\n";
const DDS_MAGIC: &[u8] = b"DDS ";

fn sniff(bytes: &[u8]) -> &'static str {
    [(PNG_MAGIC, "PNG"), (JPEG_MAGIC, "JPEG"), (KTX2_MAGIC, "KTX2"), (DDS_MAGIC, "DDS")]
        .into_iter()
        .find(|(m, _)| bytes.starts_with(m))
        .map_or("of no texture format", |(_, n)| n)
}

pub fn image_size(name: &str, bytes: &[u8]) -> Result<(u32, u32), TextureError> {
    if bytes.starts_with(PNG_MAGIC) {
        png::size(bytes, name)
    } else if bytes.starts_with(JPEG_MAGIC) {
        jpeg::size(bytes, name)
    } else {
        Err(TextureError::new(name, format!("neither PNG nor JPEG: its content is {}", sniff(bytes))))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureProbe {
    pub width: u32,
    pub height: u32,
    pub faces: u32,
}

impl TextureProbe {
    pub fn decoded_bytes(&self) -> u64 {
        super::chain_bytes(self.width, self.height) * u64::from(self.faces)
    }
}

const PROBE_BYTES: u64 = 64 << 10;
const JPEG_PROBE_BYTES: u64 = 1 << 20;

pub fn probe_file(
    name: &str,
    size: u64,
    read: &mut dyn FnMut(u64) -> Result<Vec<u8>, String>,
    limits: &Limits,
) -> Result<TextureProbe, TextureError> {
    if size > limits.max_file_bytes {
        let cap = limits.max_file_bytes;
        return Err(TextureError::new(name, format!("{size} bytes, over the texture file cap of {cap}")));
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let head = read(size.min(PROBE_BYTES)).map_err(|e| TextureError::new(name, e))?;
    let (width, height, faces) = match ext.as_str() {
        "png" | "jpg" | "jpeg" => {
            let wh = match image_size(name, &head) {
                Err(_) if head.starts_with(JPEG_MAGIC) && size > PROBE_BYTES => {
                    let more = read(size.min(JPEG_PROBE_BYTES)).map_err(|e| TextureError::new(name, e))?;
                    image_size(name, &more)?
                }
                other => other?,
            };
            (wh.0, wh.1, 1)
        }
        "ktx2" => ktx2::size(&head, name)?,
        "dds" => dds::size(&head, name)?,
        _ => return Err(TextureError::new(name, format!("not a texture file ({})", FORMATS.join(", ")))),
    };
    super::check_size(name, width, height, faces, limits)?;
    Ok(TextureProbe { width, height, faces })
}

pub fn decode_file(
    name: &str,
    bytes: &[u8],
    settings: &TextureSettings,
    limits: &Limits,
) -> Result<Texture, TextureError> {
    if bytes.len() as u64 > limits.max_file_bytes {
        let cap = limits.max_file_bytes;
        return Err(TextureError::new(name, format!("{} bytes, over the texture file cap of {cap}", bytes.len())));
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let (format, magic) = match ext.as_str() {
        "png" => ("PNG", PNG_MAGIC),
        "jpg" | "jpeg" => ("JPEG", JPEG_MAGIC),
        "ktx2" => ("KTX2", KTX2_MAGIC),
        "dds" => ("DDS", DDS_MAGIC),
        _ => return Err(TextureError::new(name, format!("not a texture file ({})", FORMATS.join(", ")))),
    };
    if !bytes.starts_with(magic) {
        return Err(TextureError::new(name, format!("named .{ext}, but its content is {}", sniff(bytes))));
    }
    let (shape, mut levels, mut blocks) = match format {
        "PNG" => (Shape::Plane, vec![png::decode(bytes, name, limits)?], None),
        "JPEG" => (Shape::Plane, vec![jpeg::decode(bytes, name, limits)?], None),
        "KTX2" => ktx2::decode(bytes, name, limits)?,
        _ => dds::decode(bytes, name, limits)?,
    };
    if settings.kind == Some(TextureKind::Normal) {
        for t in levels.iter_mut().flat_map(|l| &mut l.images).flat_map(|i| i.as_chunks_mut::<4>().0.iter_mut()) {
            (t[2], t[3]) = (0, 255);
        }
    }
    if levels.len() == 1
        && let Some(level0) = levels.pop()
    {
        levels = mips::build_chain(level0);
        if levels.len() > 1 {
            blocks = None;
        }
    }
    // TODO(guess): a mod's colour values are kept as stored, sRGB-encoded or not (no colour profile, gamma or sRGB
    // flag is applied), as the game keeps its texels raw and squares colours in its shaders; the disc's UI and splash
    // textures shown raw look as authored, so the disc stores display-encoded colours as an author's image does;
    // settle: EA's texture pipeline (whether it converted colours before block compression).
    Ok(Texture { shape, levels, blocks, kind: settings.kind, source: Some(ContentHash::of(bytes)) })
}
