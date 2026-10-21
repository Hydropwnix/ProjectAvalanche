use std::path::Path;

use anyhow::Result;

use crate::avalanche_entities::{AvalancheEntities, load_bound};

pub const SPRAY_TYPES: [u8; 2] = [0, 1];

pub fn load(big_root: &Path, track: &str) -> Result<AvalancheEntities> {
    load_bound(big_root, track, &[], &SPRAY_TYPES)
}
