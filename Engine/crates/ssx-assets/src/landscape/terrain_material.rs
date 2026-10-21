use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_formats::attrib::Value;

use crate::vault_db::{fnv1, single, single_text, with_track_db};

pub type TerrainMaterial = crate::vault_db::Material;

const CLASS_MESHSUBD: u32 = fnv1("ge_meshsubd");
const FIELD_MATERIAL: u32 = fnv1("Material");

pub fn load_track_materials(big_root: &Path, track: &str) -> Result<BTreeMap<String, TerrainMaterial>> {
    with_track_db(big_root, track, |db| {
        let mut out = BTreeMap::new();
        for (key, i) in db.collections_of(0, CLASS_MESHSUBD) {
            let fields = &db.vaults[0].decoded[i].fields;
            let path = fields.iter().find_map(|f| single_text(&f.data)).context("ge_meshsubd without a path")?;
            let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
            let Some(Value::Ref(r)) = fields.iter().find(|f| f.key == FIELD_MATERIAL).and_then(|f| single(&f.data))
            else {
                bail!("ge_meshsubd {key:#010x} has no Material reference");
            };
            let m = db.material(r.class, r.collection)?;
            if m.textures.is_empty() {
                bail!("terrain material {} has no textures", m.name);
            }
            out.insert(file, m);
        }
        if out.is_empty() {
            bail!("no ge_meshsubd collections in {track}_track.vlt");
        }
        Ok(out)
    })
}
