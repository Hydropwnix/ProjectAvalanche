pub mod scenario;

use anyhow::{Result, bail};

pub fn load_script_tracks(big_root: &std::path::Path, script: &str) -> Result<Vec<String>> {
    let Some(path) = crate::track_objects::asset_file(big_root, script) else {
        bail!("controller script {script} not found under {}", big_root.display())
    };
    let text = std::fs::read_to_string(&path)?;
    Ok(scenario::describe(&scenario::state_sets(&text)))
}

pub fn object_world_ao(schemas: &ssx_lua::game::scene::Schemas, object_key: u32) -> bool {
    let has = |s: &ssx_lua::game::scene::Schema| s.factories.iter().any(|f| f.kind == "WorldAo");
    match schemas.schemas.iter().find(|(n, _)| crate::vault_db::fnv1(n) == object_key) {
        Some((_, s)) => has(s),
        None => schemas.schemas.get("DefaultInstance").is_some_and(has),
    }
}
