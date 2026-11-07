use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use ssx_content::ContentHash;
use ssx_content::delivery::{self, ClientManifest};
use ssx_content::package::{Origin, PackageSource};

use super::set::{Started, groups};
use super::{Context, Ids};

pub struct Snapshot {
    pub manifest: ClientManifest,
    pub json: Bytes,
    pub hash: ContentHash,
    pub files: HashMap<ContentHash, u64>,
    pub store: PathBuf,
}

impl Snapshot {
    pub fn summary(&self) -> ssx_net_proto::resources::ResourceSummary {
        let mut needed: HashMap<ContentHash, u64> = HashMap::new();
        for f in self.manifest.resources.iter().flat_map(|r| &r.files).filter(|f| !f.lazy) {
            needed.insert(f.hash, f.size);
        }
        ssx_net_proto::resources::ResourceSummary {
            content_id: self.manifest.content_id.map(|c| ssx_net_proto::resources::hex32(&c.0)).unwrap_or_default(),
            client_mods: self.manifest.client_mods.name().into(),
            client_scripts: self.manifest.client_scripts,
            count: u32::try_from(self.manifest.resources.len()).unwrap_or(u32::MAX),
            download: needed.values().sum(),
            manifest: ssx_net_proto::resources::hex32(&self.hash.0),
            manifest_bytes: self.json.len() as u64,
        }
    }
}

pub struct Published(RwLock<Arc<Snapshot>>);

impl Published {
    pub fn new(s: Snapshot) -> Self {
        Published(RwLock::new(Arc::new(s)))
    }

    pub fn empty(mods: super::config::Mods) -> Self {
        let manifest = ClientManifest {
            client_mods: mods.client_mods,
            client_scripts: mods.client_scripts,
            mirrors: Vec::new(),
            content_id: None,
            resources: Vec::new(),
            levels: Vec::new(),
        };
        let json = manifest.to_json();
        let hash = ContentHash::of(json.as_bytes());
        Published::new(Snapshot {
            manifest,
            json: Bytes::from(json),
            hash,
            files: HashMap::new(),
            store: PathBuf::new(),
        })
    }

    pub fn get(&self) -> Arc<Snapshot> {
        Arc::clone(&self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    pub(super) fn set(&self, s: Snapshot) {
        *self.0.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(s);
    }
}

pub fn sources(running: &[Started]) -> Result<Vec<PackageSource>, String> {
    running
        .iter()
        .map(|s| PackageSource::open_as(&s.loaded.dir, Origin::Server).map_err(|e| format!("{}: {e}", s.loaded.name)))
        .collect()
}

pub fn snapshot(ctx: &Context, running: &[Started], ids: &Ids) -> Result<Snapshot, String> {
    let groups = groups(running);
    let resources = running
        .iter()
        .map(|s| delivery::Resource {
            name: s.loaded.name.clone(),
            version: s.loaded.manifest.resource.version,
            title: s.loaded.manifest.resource.title.clone(),
            group: groups.get(&s.loaded.name).copied().unwrap_or(0),
            scripts: s.loaded.client_scripts(),
            files: s.loaded.files.clone(),
        })
        .collect();
    let manifest = ClientManifest {
        client_mods: ctx.mods.client_mods,
        client_scripts: ctx.mods.client_scripts,
        mirrors: ctx.mirrors.clone(),
        content_id: ids.content_id,
        resources,
        levels: if ids.content_id.is_some() { ids.levels.clone() } else { Vec::new() },
    };
    let json = manifest.to_json();
    let read = delivery::from_json(json.as_bytes()).map_err(|e| format!("the client manifest: {e}"))?;
    if read != manifest {
        return Err("the client manifest does not read back as written".into());
    }
    let files = manifest.resources.iter().flat_map(|r| &r.files).map(|f| (f.hash, f.size)).collect();
    let hash = ContentHash::of(json.as_bytes());
    Ok(Snapshot { manifest, json: Bytes::from(json), hash, files, store: ctx.store.root().to_path_buf() })
}
