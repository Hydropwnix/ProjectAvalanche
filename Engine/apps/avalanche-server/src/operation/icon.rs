use std::io::Read;
use std::path::Path;

use bytes::Bytes;
use ssx_net_proto::icon::{MAX_ICON_BYTES, hash_hex, read_icon};

#[derive(Clone, Debug)]
pub struct IconFile {
    pub bytes: Bytes,
    pub hash: String,
}

pub fn load(path: &Path) -> Result<IconFile, String> {
    let shown = path.display().to_string();
    let at = |e: &dyn std::fmt::Display| ssx_core::text::located(&shown, e).to_string();
    let meta = std::fs::metadata(path).map_err(|e| at(&e))?;
    if !meta.is_file() {
        return Err(at(&"not a file"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_ICON_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| at(&e))?;
    if bytes.len() > MAX_ICON_BYTES {
        return Err(at(&format!("larger than {MAX_ICON_BYTES} bytes")));
    }
    read_icon(&bytes).map_err(|e| at(&format!("the icon is refused: {e}")))?;
    let hash = hash_hex(&bytes);
    Ok(IconFile { bytes: Bytes::from(bytes), hash })
}
