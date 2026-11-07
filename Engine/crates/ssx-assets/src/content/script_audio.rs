use std::sync::Arc;

use ssx_content::audio::sounds::SoundDef;
use ssx_content::audio::{Limits, decode_file};
use ssx_content::{ContentId, Kind, Layer};

use super::store::GameContent;
use crate::sfx_override::OverrideSound;

impl GameContent {
    pub fn script_sound(&self, id: &ContentId) -> Result<Arc<OverrideSound>, String> {
        if id.kind() != Kind::Sound {
            return Err(format!("{id} is not a sound id"));
        }
        let (package, layer, def) = self.with_registry(|r| {
            let res = r.load(id).map_err(|e| e.to_string())?;
            if res.layer == Layer::Base {
                return Err(format!("{id} is the game's own sound (fired as its event)"));
            }
            let def = res.asset.value::<SoundDef>().cloned().ok_or_else(|| format!("{id}: not a sound record"))?;
            Ok((res.source, res.layer, def))
        })?;
        let limits = Limits::sound();
        let mut set = Vec::new();
        for rel in &def.files {
            let bytes = self.read_audio_from(id, &package, layer, rel)?;
            let pcm = decode_file(rel, bytes, &limits).map_err(|e| e.to_string())?;
            set.push(super::audio::convert::samples48(rel, pcm));
        }
        if set.is_empty() {
            return Err(format!("{id}: no files"));
        }
        Ok(Arc::new(OverrideSound::new(id.as_str(), &package, set, &def)))
    }
}
