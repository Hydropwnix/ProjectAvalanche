use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use ssx_content::particles::{SpraySlot, SprayTable, system_from_table, system_to_table};
use ssx_content::{Asset, BASE_NAMESPACE, ContentId, Kind, Layer, LoadError, Source};
use ssx_formats::lynx::ParticleSystem;

use super::store::GameContent;
use super::textures::{GAME_EXPLORER, game_archives, game_files};
use super::vaults::rel;
use crate::particles::{BASE_SNOW_SPRAY, LOD_SNOW_SPRAY, LynxVfx, ParticleNode, SnowSpraySet, particle_node};
use crate::vault_db::with_attribdb;

pub fn particles_id(stored: &str) -> Option<ContentId> {
    let p = stored.replace('\\', "/");
    let p = p.strip_suffix(".lnx").unwrap_or(&p);
    ContentId::new(BASE_NAMESPACE, Kind::Particles, &super::path_segments(p)?).ok()
}

pub const SPRAY_TABLES: [(&str, u32); 2] = [("base_snowspray", BASE_SNOW_SPRAY), ("lod_snowspray", LOD_SNOW_SPRAY)];

pub fn spray_id(name: &str) -> ContentId {
    ContentId::new(BASE_NAMESPACE, Kind::Particles, &format!("{}{name}", ssx_content::particles::SPRAY_PREFIX))
        .expect("a valid id")
}

pub fn system_texture_id(name: &str) -> Option<String> {
    crate::particles::texture_id(name).map(|id| id.to_string())
}

pub fn spray_record(set: &SnowSpraySet) -> SprayTable {
    let mut t = SprayTable::default();
    for (s, row) in set.slots.iter().enumerate() {
        for (k, n) in row.iter().enumerate() {
            t.slots[s][k] = n.as_ref().map(|n| SpraySlot { node: n.key, particles: None, effect: None });
        }
    }
    t
}

pub struct DiscParticles {
    big_root: PathBuf,
    files: BTreeMap<ContentId, PathBuf>,
}

impl DiscParticles {
    pub fn index(big_root: &Path) -> Result<Self> {
        let mut files = BTreeMap::new();
        for a in game_archives(big_root)? {
            let ge = a.join(GAME_EXPLORER);
            for f in game_files(&ge, "lnx")? {
                if let Some(id) = particles_id(&rel(&ge, &f)) {
                    files.entry(id).or_insert(f);
                }
            }
        }
        Ok(DiscParticles { big_root: big_root.to_path_buf(), files })
    }

    pub fn file(&self, id: &ContentId) -> Option<&Path> {
        self.files.get(id).map(PathBuf::as_path)
    }

    fn spray(&self, id: &ContentId) -> Option<u32> {
        SPRAY_TABLES.iter().find(|(n, _)| spray_id(n) == *id).map(|(_, k)| *k)
    }
}

impl Source for DiscParticles {
    fn name(&self) -> &str {
        BASE_NAMESPACE
    }

    fn has(&self, id: &ContentId) -> bool {
        self.files.contains_key(id) || self.spray(id).is_some()
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        if let Some(key) = self.spray(id) {
            let set = with_attribdb(&self.big_root, |db| crate::particles::load_snow_spray(db, key))
                .map_err(|e| failed(format!("{e:#}")))?;
            return Ok(Asset::Record(spray_record(&set).to_table()));
        }
        let file = self.files.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        let data = std::fs::read(file).map_err(|e| failed(format!("{}: {e}", file.display())))?;
        let sys = ParticleSystem::parse(&data).map_err(|e| failed(format!("{}: {e}", file.display())))?;
        let table = system_to_table(&sys, &system_texture_id).map_err(failed)?;
        Ok(Asset::Record(table))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind != Kind::Particles {
            return Vec::new();
        }
        let mut out: Vec<ContentId> = self.files.keys().cloned().collect();
        out.extend(SPRAY_TABLES.iter().map(|(n, _)| spray_id(n)));
        out
    }
}

impl GameContent {
    pub fn particle_system(&self, stored: &str) -> Result<Arc<ParticleSystem>> {
        if let Ok(id) = stored.parse::<ContentId>() {
            return self.particle_system_by_id(&id);
        }
        let disc = || -> Result<Arc<ParticleSystem>> {
            let path = crate::particles::fx_file(self.big_root(), stored);
            let data = std::fs::read(&path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
            Ok(Arc::new(ParticleSystem::parse(&data).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?))
        };
        if !self.packages_provide(Kind::Particles) {
            return disc();
        }
        let Some(id) = particles_id(stored) else { return disc() };
        let changed = self.with_registry(|r| {
            r.winner(&id).is_some_and(|w| w.layer > Layer::Base)
                || r.patches_of(&id).map(|p| !p.is_empty()).unwrap_or(true)
        });
        if !changed {
            return disc();
        }
        match self.particle_system_by_id(&id) {
            Ok(s) => Ok(s),
            Err(e) => {
                self.notice(format!("content: {id} not used ({e:#}); the disc's system is"));
                disc()
            }
        }
    }

    fn particle_system_by_id(&self, id: &ContentId) -> Result<Arc<ParticleSystem>> {
        if id.kind() != Kind::Particles || ssx_content::particles::is_spray(id) {
            anyhow::bail!("{id} is not a particle system's id");
        }
        let got = self.with_registry(|r| r.load(id)).map_err(|e| anyhow::anyhow!("{e}"))?;
        let table = got.asset.record().ok_or_else(|| anyhow::anyhow!("{id}: not a record"))?;
        let sys = system_from_table(table).map_err(|e| anyhow::anyhow!("{id}: {}", e.join("; ")))?;
        if got.layer > Layer::Base || !got.patched_by.is_empty() {
            self.info(format!("content: {id}: the particle system from {}", got.source));
        }
        Ok(Arc::new(sys))
    }

    pub fn route_spray(&self, collection: u32, disc: SnowSpraySet) -> SnowSpraySet {
        if !self.packages_provide(Kind::Particles) {
            return disc;
        }
        let Some((name, _)) = SPRAY_TABLES.iter().find(|(_, k)| *k == collection) else { return disc };
        let id = spray_id(name);
        match self.spray_from_registry(&id, &disc) {
            Ok(Some(set)) => set,
            Ok(None) => disc,
            Err(why) => {
                self.notice(format!("content: {id} not used ({why}); the disc's table is"));
                disc
            }
        }
    }

    fn spray_from_registry(&self, id: &ContentId, disc: &SnowSpraySet) -> Result<Option<SnowSpraySet>, String> {
        let (table, source) = self.with_registry(|r| -> Result<_, String> {
            let w = r.winner(id).ok_or_else(|| format!("{id}: no source provides it"))?;
            let patches = r.patches_of(id).map_err(|e| e.to_string())?;
            if w.layer == Layer::Base && patches.is_empty() {
                return Ok((None, String::new()));
            }
            let by: Vec<&str> = patches.iter().map(|(n, _)| n.as_str()).filter(|n| *n != w.name).collect();
            let source =
                if by.is_empty() { w.name.to_string() } else { format!("{}, patched by {}", w.name, by.join(", ")) };
            let mut t = if w.layer == Layer::Base {
                spray_record(disc).to_table()
            } else {
                r.load(id).map_err(|e| e.to_string())?.asset.record().cloned().ok_or("not a record")?
            };
            if w.layer == Layer::Base {
                for (_, p) in &patches {
                    ssx_content::patch::merge(&mut t, p);
                }
            }
            Ok((Some(t), source))
        })?;
        let Some(table) = table else { return Ok(None) };
        let t = SprayTable::from_table(&table).map_err(|ps| ps.join("; "))?;
        let big = self.big_root().to_path_buf();
        let mut set = SnowSpraySet { slots: Default::default() };
        with_attribdb(&big, |db| -> Result<()> {
            for (s, row) in t.slots.iter().enumerate() {
                for (k, slot) in row.iter().enumerate() {
                    let Some(slot) = slot else { continue };
                    let disc_node = disc.slots[s][k].as_ref().filter(|n| n.key == slot.node).cloned();
                    let mut node: ParticleNode = match disc_node {
                        Some(n) => n,
                        None => particle_node(db, slot.node)?,
                    };
                    if let (Some(p), Some(e)) = (&slot.particles, &slot.effect) {
                        let collection = node.lynx.as_ref().map(|l| l.collection.clone()).unwrap_or_default();
                        node.lynx =
                            Some(LynxVfx { collection, name: e.clone(), path: p.to_string(), definition: e.clone() });
                    }
                    set.slots[s][k] = Some(node);
                }
            }
            Ok(())
        })
        .map_err(|e| format!("{e:#}"))?;
        self.info(format!("content: {id}: the spray table from {source}"));
        Ok(Some(set))
    }
}
