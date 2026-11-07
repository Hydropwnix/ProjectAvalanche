use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::id::{ContentId, Kind};
use crate::patch;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    Base,
    Port,
    ClientMods,
    Server,
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Layer::Base => "L0 base",
            Layer::Port => "L1 port",
            Layer::ClientMods => "L2 client mods",
            Layer::Server => "L3 server",
        })
    }
}

#[derive(Clone)]
pub enum Asset {
    Bytes(Arc<[u8]>),
    Files(BTreeMap<String, Arc<[u8]>>),
    Record(toml::Table),
    Value(Arc<dyn Any + Send + Sync>),
}

impl Asset {
    pub fn record(&self) -> Option<&toml::Table> {
        if let Asset::Record(t) = self { Some(t) } else { None }
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        if let Asset::Bytes(b) = self { Some(b) } else { None }
    }

    pub fn value<T: Any + Send + Sync>(&self) -> Option<&T> {
        if let Asset::Value(v) = self { v.downcast_ref() } else { None }
    }

    pub fn into_value<T: Any + Send + Sync>(self) -> Option<Arc<T>> {
        if let Asset::Value(v) = self { v.downcast::<T>().ok() } else { None }
    }
}

impl fmt::Debug for Asset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Asset::Bytes(b) => write!(f, "Bytes({} bytes)", b.len()),
            Asset::Files(m) => write!(f, "Files({:?})", m.keys().collect::<Vec<_>>()),
            Asset::Record(t) => write!(f, "Record({t:?})"),
            Asset::Value(_) => f.write_str("Value(..)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("{0}: no source provides it")]
    Missing(ContentId),
    #[error("{id}: {}", ssx_core::text::shown_max(.message, ssx_core::text::LINE_CHARS))]
    Failed { id: ContentId, message: String },
    #[error(
        "{id}: patched by {}, but the winning asset ({}) is not a data record",
        ssx_core::text::shown_max(.by, 256),
        ssx_core::text::shown_max(.winner, 256)
    )]
    NotARecord { id: ContentId, winner: String, by: String },
}

pub trait Source: Send + Sync {
    fn name(&self) -> &str;
    fn has(&self, id: &ContentId) -> bool;
    fn load(&self, id: &ContentId) -> Result<Asset, LoadError>;
    fn list(&self, kind: Kind) -> Vec<ContentId>;
    fn patches(&self) -> Vec<ContentId> {
        Vec::new()
    }
    fn patch(&self, _id: &ContentId) -> Result<Option<toml::Table>, LoadError> {
        Ok(None)
    }
    fn file_of(&self, _id: &ContentId) -> Option<String> {
        None
    }
    fn model_cost(&self, _id: &ContentId) -> Option<Result<crate::model::ModelCost, LoadError>> {
        None
    }
    fn texture_probe(&self, _id: &ContentId) -> Option<Result<crate::texture::TextureProbe, LoadError>> {
        None
    }
    fn sky_cost(&self, _id: &ContentId) -> Option<Result<(crate::sky::SkyDef, crate::sky::SkyCost), LoadError>> {
        None
    }
    fn audio_probe(&self, _id: &ContentId) -> Option<AudioProbe> {
        None
    }
    fn audio_file(&self, _id: &ContentId, _rel: &str) -> Option<Result<Arc<[u8]>, LoadError>> {
        None
    }
    fn rider_entry(&self, _id: &ContentId) -> Option<Result<crate::rider::RiderEntry, LoadError>> {
        None
    }
    fn rider_model_cost(
        &self,
        _id: &ContentId,
    ) -> Option<Result<Option<(String, crate::model::ModelCost)>, LoadError>> {
        None
    }
    fn rider_model(&self, _id: &ContentId) -> Option<Result<Option<Arc<crate::model::Model>>, LoadError>> {
        None
    }
    fn movie_file(&self, _id: &ContentId) -> Option<Result<(std::fs::File, u64), LoadError>> {
        None
    }
    fn map_record(&self, _id: &ContentId) -> Option<Result<crate::map::MapToml, LoadError>> {
        None
    }
}

pub type AudioProbe = Result<Vec<(String, crate::audio::AudioInfo)>, LoadError>;

struct Mount {
    layer: Layer,
    priority: i32,
    seq: u64,
    source: Box<dyn Source>,
}

pub struct SetAside(Vec<Mount>);

impl SetAside {
    pub fn names(&self) -> Vec<String> {
        self.0.iter().map(|m| m.source.name().to_string()).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MountInfo<'a> {
    pub layer: Layer,
    pub priority: i32,
    pub name: &'a str,
}

impl Mount {
    fn info(&self) -> MountInfo<'_> {
        MountInfo { layer: self.layer, priority: self.priority, name: self.source.name() }
    }
}

#[derive(Debug)]
pub struct Resolved {
    pub asset: Asset,
    pub layer: Layer,
    pub source: String,
    pub patched_by: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub id: ContentId,
    pub layer: Layer,
    pub winner: String,
    pub losers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Override {
    pub id: ContentId,
    pub winner: (Layer, String),
    pub over: Vec<(Layer, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DanglingPatch {
    pub id: ContentId,
    pub source: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub conflicts: Vec<Conflict>,
    pub overrides: Vec<Override>,
    pub dangling_patches: Vec<DanglingPatch>,
}

#[derive(Default)]
pub struct Registry {
    mounts: Vec<Mount>,
    next_seq: u64,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mount(&mut self, layer: Layer, priority: i32, source: Box<dyn Source>) {
        self.mounts.push(Mount { layer, priority, seq: self.next_seq, source });
        self.next_seq += 1;
        self.mounts.sort_by_key(|m| (m.layer, m.priority, m.seq));
    }

    pub fn unmount(&mut self, layer: Layer) -> usize {
        let before = self.mounts.len();
        self.mounts.retain(|m| m.layer != layer);
        before - self.mounts.len()
    }

    pub fn unmount_named(&mut self, layer: Layer, name: &str) -> usize {
        let before = self.mounts.len();
        self.mounts.retain(|m| m.layer != layer || m.source.name() != name);
        before - self.mounts.len()
    }

    pub fn set_aside(&mut self, layer: Layer) -> SetAside {
        let (taken, kept) = std::mem::take(&mut self.mounts).into_iter().partition(|m| m.layer == layer);
        self.mounts = kept;
        SetAside(taken)
    }

    pub fn put_back(&mut self, aside: SetAside) {
        self.mounts.extend(aside.0);
        self.mounts.sort_by_key(|m| (m.layer, m.priority, m.seq));
    }

    pub fn ids_of(&self, layer: Layer, name: &str) -> BTreeSet<ContentId> {
        let mounts = self.mounts.iter().filter(|m| m.layer == layer && m.source.name() == name);
        mounts.flat_map(|m| Kind::ALL.iter().flat_map(|&k| m.source.list(k)).chain(m.source.patches())).collect()
    }

    pub fn mounts(&self) -> impl Iterator<Item = MountInfo<'_>> {
        self.mounts.iter().map(Mount::info)
    }

    fn winning(&self, id: &ContentId) -> Option<&Mount> {
        self.mounts.iter().rev().find(|m| m.source.has(id))
    }

    pub fn has(&self, id: &ContentId) -> bool {
        self.winning(id).is_some()
    }

    pub fn winner(&self, id: &ContentId) -> Option<MountInfo<'_>> {
        self.winning(id).map(Mount::info)
    }

    pub fn providers(&self, id: &ContentId) -> impl Iterator<Item = (MountInfo<'_>, &dyn Source)> {
        self.mounts.iter().rev().filter(move |m| m.source.has(id)).map(|m| (m.info(), m.source.as_ref()))
    }

    pub fn load(&self, id: &ContentId) -> Result<Resolved, LoadError> {
        let m = self.winning(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        let mut asset = m.source.load(id)?;
        let mut patched_by = Vec::new();
        for p in &self.mounts {
            let Some(patch) = p.source.patch(id)? else { continue };
            match &mut asset {
                Asset::Record(t) => patch::merge(t, &patch),
                _ => {
                    return Err(LoadError::NotARecord {
                        id: id.clone(),
                        winner: m.source.name().to_string(),
                        by: p.source.name().to_string(),
                    });
                }
            }
            patched_by.push(p.source.name().to_string());
        }
        Ok(Resolved { asset, layer: m.layer, source: m.source.name().to_string(), patched_by })
    }

    pub fn list_above(&self, layer: Layer, kind: Kind) -> Vec<ContentId> {
        let ids: BTreeSet<ContentId> =
            self.mounts.iter().filter(|m| m.layer > layer).flat_map(|m| m.source.list(kind)).collect();
        ids.into_iter().collect()
    }

    pub fn patched_above(&self, layer: Layer) -> Vec<ContentId> {
        let ids: BTreeSet<ContentId> =
            self.mounts.iter().filter(|m| m.layer > layer).flat_map(|m| m.source.patches()).collect();
        ids.into_iter().collect()
    }

    pub fn patches_of(&self, id: &ContentId) -> Result<Vec<(String, toml::Table)>, LoadError> {
        let mut out = Vec::new();
        for m in &self.mounts {
            if let Some(p) = m.source.patch(id)? {
                out.push((m.source.name().to_string(), p));
            }
        }
        Ok(out)
    }

    pub fn patches_in(
        &self,
        id: &ContentId,
        keep: impl Fn(Layer) -> bool,
    ) -> Result<Vec<(String, Layer, toml::Table)>, LoadError> {
        let mut out = Vec::new();
        for m in self.mounts.iter().filter(|m| keep(m.layer)) {
            if let Some(p) = m.source.patch(id)? {
                out.push((m.source.name().to_string(), m.layer, p));
            }
        }
        Ok(out)
    }

    pub fn list(&self, kind: Kind) -> Vec<ContentId> {
        let ids: BTreeSet<ContentId> = self.mounts.iter().flat_map(|m| m.source.list(kind)).collect();
        ids.into_iter().collect()
    }

    pub fn report(&self) -> Report {
        let mut providers: BTreeMap<ContentId, Vec<&Mount>> = BTreeMap::new();
        for m in &self.mounts {
            for kind in Kind::ALL {
                for id in m.source.list(kind) {
                    providers.entry(id).or_default().push(m);
                }
            }
        }
        let mut report = Report::default();
        for (id, ms) in &providers {
            let Some((top, rest)) = ms.split_last() else { continue };
            let same: Vec<String> =
                rest.iter().rev().filter(|m| m.layer == top.layer).map(|m| m.source.name().to_string()).collect();
            if !same.is_empty() {
                let winner = top.source.name().to_string();
                report.conflicts.push(Conflict { id: id.clone(), layer: top.layer, winner, losers: same });
            }
            let lower: Vec<(Layer, String)> =
                rest.iter().rev().filter(|m| m.layer < top.layer).map(|m| (m.layer, m.source.name().into())).collect();
            if !lower.is_empty() {
                let winner = (top.layer, top.source.name().to_string());
                report.overrides.push(Override { id: id.clone(), winner, over: lower });
            }
        }
        for m in &self.mounts {
            for id in m.source.patches() {
                if !providers.contains_key(&id) {
                    report.dangling_patches.push(DanglingPatch { id, source: m.source.name().to_string() });
                }
            }
        }
        report
    }
}

#[derive(Default)]
pub struct MemorySource {
    pub name: String,
    pub assets: BTreeMap<ContentId, Asset>,
    pub patches: BTreeMap<ContentId, toml::Table>,
}

impl MemorySource {
    pub fn new(name: &str) -> Self {
        MemorySource { name: name.to_string(), ..Default::default() }
    }

    pub fn with(mut self, id: ContentId, asset: Asset) -> Self {
        self.assets.insert(id, asset);
        self
    }

    pub fn with_patch(mut self, id: ContentId, patch: toml::Table) -> Self {
        self.patches.insert(id, patch);
        self
    }
}

impl Source for MemorySource {
    fn name(&self) -> &str {
        &self.name
    }

    fn has(&self, id: &ContentId) -> bool {
        self.assets.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        self.assets.get(id).cloned().ok_or_else(|| LoadError::Missing(id.clone()))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        self.assets.keys().filter(|id| id.kind() == kind).cloned().collect()
    }

    fn patches(&self) -> Vec<ContentId> {
        self.patches.keys().cloned().collect()
    }

    fn patch(&self, id: &ContentId) -> Result<Option<toml::Table>, LoadError> {
        Ok(self.patches.get(id).cloned())
    }
}

#[cfg(test)]
mod tests;
