use std::path::{Path, PathBuf};

use super::encode::{ENCODER, encode_chain};
use super::{BlockChain, BlockFormat, BlockLevel, Texture, TextureKind};
use crate::hash::ContentHash;

const MAGIC: &[u8; 4] = b"AVTC";
const VERSION: u32 = 1;
const MAX_LEVELS: usize = 16;

fn format_code(f: BlockFormat) -> u32 {
    match f {
        BlockFormat::Bc1 => 1,
        BlockFormat::Bc2 => 2,
        BlockFormat::Bc3 => 3,
        BlockFormat::Bc4 => 4,
        BlockFormat::Bc5 => 5,
        BlockFormat::Bc7 => 7,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Hit,
    Stored,
    Encoded,
}

#[derive(Clone, Debug)]
pub struct TextureCache {
    dir: PathBuf,
}

impl TextureCache {
    pub fn new(dir: &Path) -> Self {
        TextureCache { dir: dir.to_path_buf() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn key(source: &ContentHash, format: BlockFormat, kind: TextureKind, shapes: &[(u32, u32)]) -> ContentHash {
        let mut h = blake3::Hasher::new();
        h.update(b"ava-texture-cache\0");
        h.update(&source.0);
        h.update(&format_code(format).to_le_bytes());
        h.update(kind.as_str().as_bytes());
        h.update(b"\0");
        h.update(ENCODER.as_bytes());
        for (w, ht) in shapes {
            h.update(&w.to_le_bytes());
            h.update(&ht.to_le_bytes());
        }
        ContentHash(*h.finalize().as_bytes())
    }

    fn path(&self, key: &ContentHash) -> PathBuf {
        self.dir.join(format!("{key}.bc"))
    }

    fn read(&self, key: &ContentHash, format: BlockFormat, shapes: &[(u32, u32)]) -> Option<BlockChain> {
        let expected: u64 = shapes.iter().map(|&(w, h)| format.level_bytes(w, h)).sum();
        let len = 16 + 16 * shapes.len() as u64 + expected + 32;
        let bytes = read_capped(&self.path(key), len)?;
        let (body, hash) = bytes.split_at_checked(bytes.len().checked_sub(32)?)?;
        if blake3::hash(body).as_bytes() != hash || body.get(..4)? != MAGIC {
            return None;
        }
        let word = |at: usize| body.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        if word(4)? != VERSION || word(8)? != format_code(format) || word(12)? as usize != shapes.len() {
            return None;
        }
        let mut at = 16 + 16 * shapes.len();
        let mut levels = Vec::with_capacity(shapes.len());
        for (i, &(w, h)) in shapes.iter().enumerate() {
            let head = 16 + 16 * i;
            let n = format.level_bytes(w, h) as usize;
            if (word(head)?, word(head + 4)?) != (w, h) || u64::from(word(head + 8)?) != n as u64 {
                return None;
            }
            levels.push(BlockLevel { width: w, height: h, data: body.get(at..at + n)?.to_vec() });
            at += n;
        }
        (at == body.len()).then_some(BlockChain { format, levels })
    }

    fn write(&self, key: &ContentHash, chain: &BlockChain) -> Result<(), String> {
        let mut body = MAGIC.to_vec();
        for v in [VERSION, format_code(chain.format), chain.levels.len() as u32] {
            body.extend(v.to_le_bytes());
        }
        for l in &chain.levels {
            for v in [l.width, l.height, l.data.len() as u32, 0] {
                body.extend(v.to_le_bytes());
            }
        }
        chain.levels.iter().for_each(|l| body.extend_from_slice(&l.data));
        body.extend_from_slice(blake3::hash(&body).as_bytes());
        ssx_files::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        ssx_files::replace(&self.path(key), &body, &ssx_files::Options::new()).map_err(|e| e.to_string())
    }
}

fn read_capped(path: &Path, len: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() != len {
        return None;
    }
    let mut out = Vec::with_capacity(len as usize);
    file.take(len + 1).read_to_end(&mut out).ok()?;
    (out.len() as u64 == len).then_some(out)
}

pub fn blocks_for(
    cache: Option<&TextureCache>,
    texture: &Texture,
    format: BlockFormat,
    kind: TextureKind,
) -> Result<(BlockChain, Outcome, Option<String>), String> {
    let shapes: Vec<(u32, u32)> = texture.levels.iter().map(|l| (l.width, l.height)).collect();
    if shapes.len() > MAX_LEVELS {
        return Err(format!("{} levels", shapes.len()));
    }
    let key = match (cache, &texture.source) {
        (Some(c), Some(source)) => Some((c, TextureCache::key(source, format, kind, &shapes))),
        _ => None,
    };
    if let Some((c, key)) = &key
        && let Some(chain) = c.read(key, format, &shapes)
    {
        return Ok((chain, Outcome::Hit, None));
    }
    let chain = encode_chain(texture, format, kind)?;
    match key {
        Some((c, key)) => match c.write(&key, &chain) {
            Ok(()) => Ok((chain, Outcome::Stored, None)),
            Err(e) => Ok((chain, Outcome::Encoded, Some(e))),
        },
        None => Ok((chain, Outcome::Encoded, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;
    use crate::texture::{Level, Shape, mips};

    fn texture(seed: u8) -> Texture {
        let image: Vec<u8> = (0..32u32 * 32).flat_map(|i| [(i as u8).wrapping_mul(seed), seed, 9, 255]).collect();
        let levels = mips::build_chain(Level { width: 32, height: 32, images: vec![image.clone()] });
        Texture { shape: Shape::Plane, levels, blocks: None, kind: None, source: Some(ContentHash::of(&image)) }
    }

    #[test]
    fn stores_reads_and_rejects_damaged_entries() {
        let dir = TestDir::new("texcache");
        let cache = TextureCache::new(&dir.0.join("Cache"));
        let t = texture(3);
        let (a, o, e) = blocks_for(Some(&cache), &t, BlockFormat::Bc1, TextureKind::Color).unwrap();
        assert_eq!((o, e), (Outcome::Stored, None));
        let (b, o, _) = blocks_for(Some(&cache), &t, BlockFormat::Bc1, TextureKind::Color).unwrap();
        assert_eq!((o, &b), (Outcome::Hit, &a));
        let (_, o, _) = blocks_for(Some(&cache), &t, BlockFormat::Bc1, TextureKind::Data).unwrap();
        assert_eq!(o, Outcome::Stored);
        let shapes: Vec<_> = t.levels.iter().map(|l| (l.width, l.height)).collect();
        let key = TextureCache::key(t.source.as_ref().unwrap(), BlockFormat::Bc1, TextureKind::Color, &shapes);
        let path = cache.path(&key);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[40] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        let (c, o, _) = blocks_for(Some(&cache), &t, BlockFormat::Bc1, TextureKind::Color).unwrap();
        assert_eq!((o, &c), (Outcome::Stored, &a));
        std::fs::write(&path, &bytes[..bytes.len() - 1]).unwrap();
        assert_eq!(blocks_for(Some(&cache), &t, BlockFormat::Bc1, TextureKind::Color).unwrap().1, Outcome::Stored);
        assert_eq!(blocks_for(None, &t, BlockFormat::Bc1, TextureKind::Color).unwrap().1, Outcome::Encoded);
    }
}
