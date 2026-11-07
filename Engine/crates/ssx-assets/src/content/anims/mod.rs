pub mod export;
pub mod index;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use ssx_content::anim::Clip;
use ssx_content::data::SimRow;
use ssx_content::policy::ClientMods;
use ssx_content::{Asset, ContentId, Kind, Layer, LoadError, Registry, Source};
use ssx_formats::ant::Guid;
use ssx_formats::ant_anim::Animation;

pub use index::{DiscAnims, DiscClip};

use super::store::{GameContent, Shared, provides_kind};
use crate::anim_clip::{ClipData, ModClip, Replaced, bind};

#[derive(Default)]
pub struct AnimCache {
    disc: OnceLock<Option<Arc<DiscAnims>>>,
    mounted: AtomicBool,
    routes: Mutex<Option<RoutesAt>>,
}

type RoutesAt = ((u64, u8), Arc<Routes>);

#[derive(Debug, Default)]
pub struct Routes {
    pub by_guid: HashMap<Guid, (Arc<ModClip>, Layer)>,
}

impl Source for DiscAnims {
    fn name(&self) -> &str {
        "disc"
    }

    fn has(&self, id: &ContentId) -> bool {
        self.clips.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let c = self.clips.get(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        Ok(Asset::Value(Arc::new(c.clone())))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Anim { self.clips.keys().cloned().collect() } else { Vec::new() }
    }
}

impl GameContent {
    pub fn disc_anims(&self) -> Option<Arc<DiscAnims>> {
        self.anims
            .disc
            .get_or_init(|| match DiscAnims::index(self.big_root()) {
                Ok(d) => {
                    for n in &d.notes {
                        self.notice(format!("content: {n}; left out"));
                    }
                    Some(Arc::new(d))
                }
                Err(e) => {
                    self.notice(format!("content: the disc's animation clips: {e:#}"));
                    None
                }
            })
            .clone()
    }

    pub(in crate::content) fn mount_anims(&self, registry: &mut Registry) -> bool {
        if !provides_kind(registry, Kind::Anim) || self.anims.mounted.load(Ordering::Acquire) {
            return false;
        }
        let Some(d) = self.disc_anims() else { return false };
        if self.anims.mounted.swap(true, Ordering::AcqRel) {
            return false;
        }
        registry.mount(Layer::Base, 0, Box::new(Shared(d)));
        true
    }

    pub(in crate::content) fn mount_anims_preview(&self, registry: &mut Registry) -> bool {
        if !provides_kind(registry, Kind::Anim) {
            return false;
        }
        let Some(d) = self.disc_anims() else { return false };
        registry.mount(Layer::Base, 0, Box::new(Shared(d)));
        true
    }

    pub fn anim_routes(&self) -> Arc<Routes> {
        let key = (self.generation(), self.sim_policy() as u8);
        let mut slot = self.anims.routes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, r)) = slot.as_ref()
            && *k == key
        {
            return r.clone();
        }
        let r = Arc::new(self.route_anims());
        *slot = Some((key, r.clone()));
        r
    }

    fn route_anims(&self) -> Routes {
        let mut routes = Routes::default();
        if !self.packages_provide(Kind::Anim) {
            return routes;
        }
        let Some(disc) = self.disc_anims() else { return routes };
        let skeleton = match self.riders.skeletons.get(self.big_root(), None) {
            Ok(s) => s,
            Err(e) => {
                self.notice(format!("content: the riders' skeleton for the packages' clips: {e:#}"));
                return routes;
            }
        };
        let client_mods = self.sim_policy() == ClientMods::Any;
        let trick_clips = self.trick_clip_ids();
        for id in self.changed_ids(&[Kind::Anim]) {
            if id.namespace() != ssx_content::BASE_NAMESPACE {
                if !trick_clips.contains(&id) {
                    self.notice(format!(
                        "content: {id}: no trick mounted names it; a package's own clip plays through an added trick \
                         (`trick.toml`), a disc clip is replaced by its id (`ssx:anim/…`)"
                    ));
                }
                continue;
            }
            let Some(dc) = disc.clips.get(&id) else {
                self.notice(format!("content: {id}: no clip of the disc has that id"));
                continue;
            };
            if let Some(why) = dc.not_offered() {
                self.notice(format!("content: {why}"));
                continue;
            }
            let replaced =
                Replaced { id: &dc.id, end_frame: dc.end_frame, additive: dc.additive, frame_rate: dc.frame_rate() };
            let mut i = 0;
            loop {
                let loaded = {
                    let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut providers = registry.providers(&id).skip(i);
                    let Some((info, source)) = providers.next() else { break };
                    let next = providers.next().map(|(n, _)| n.name.to_string());
                    let asset = match info.layer {
                        Layer::Base => None,
                        Layer::ClientMods if !client_mods => None,
                        _ => Some(self.cached_load(&id, info.name, info.layer, source)),
                    };
                    (info.name.to_string(), info.layer, next, asset)
                };
                i += 1;
                let (source, layer, next, asset) = loaded;
                let Some(asset) = asset else { continue };
                let bound = asset
                    .map_err(|e| e.to_string())
                    .and_then(|a| a.into_value::<Clip>().ok_or_else(|| format!("{id}: not a decoded clip")))
                    .and_then(|clip| bind(clip, &source, replaced, &disc.dofs, &skeleton.rig));
                match bound {
                    Ok(m) => {
                        routes.by_guid.insert(dc.guid, (Arc::new(m), layer));
                        break;
                    }
                    Err(why) => self.passed_over(&id, &source, layer, &why, next.as_deref()),
                }
            }
        }
        routes
    }

    pub(in crate::content) fn anim_rows(&self) -> Vec<SimRow> {
        let routes = self.anim_routes();
        let mut rows: Vec<SimRow> = routes
            .by_guid
            .values()
            .map(|(m, layer)| SimRow {
                id: m.id.to_string(),
                class: "anim".into(),
                hash: ssx_content::anim::sim_hash(&m.clip),
                layer: format!("{layer} {}", m.source),
            })
            .collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows
    }
}

pub(in crate::content) fn clip_files(tree: &ssx_content::path::PackageTree, f: &str) -> Vec<String> {
    let caps = ssx_content::anim::gltf_limits();
    let gltf = if f.to_ascii_lowercase().ends_with(".toml") {
        let Ok(bytes) = tree.read(f, ssx_content::limits::MAX_RECORD_BYTES) else { return Vec::new() };
        let Ok(t) = ssx_content::anim::parse_toml(&bytes) else { return Vec::new() };
        let Some(g) = t.gltf else { return Vec::new() };
        g
    } else {
        f.to_string()
    };
    let mut out = vec![gltf.clone()];
    out.extend(ssx_content::model::files_named(&gltf, tree, &caps).unwrap_or_default());
    out.retain(|x| x != f);
    out
}

impl GameContent {
    pub(in crate::content) fn dev_anim_problem(&self, asset: &Asset, package: &str, id: &ContentId) -> Option<String> {
        let clip = asset.clone().into_value::<Clip>()?;
        if id.namespace() != ssx_content::BASE_NAMESPACE {
            return None;
        }
        let disc = self.disc_anims()?;
        let Some(dc) = disc.clips.get(id) else { return Some(format!("{id}: no clip of the disc has that id")) };
        if let Some(why) = dc.not_offered() {
            return Some(why);
        }
        let skeleton = match self.riders.skeletons.get(self.big_root(), None) {
            Ok(s) => s,
            Err(e) => return Some(format!("the riders' skeleton: {e:#}")),
        };
        let replaced =
            Replaced { id: &dc.id, end_frame: dc.end_frame, additive: dc.additive, frame_rate: dc.frame_rate() };
        bind(clip, package, replaced, &disc.dofs, &skeleton.rig).err().map(|e| format!("{id}: {e}"))
    }

    pub(in crate::content) fn hold_dev_anims(&self, name: &str, clips: Vec<(ContentId, Asset)>) {
        self.hold_dev_assets(name, clips);
    }
}

pub fn clip_for(root: Option<&Path>, guid: Guid, anim: Animation, table: Vec<u16>) -> (ClipData, Vec<u16>) {
    if let Some(root) = root {
        let content = GameContent::of(root);
        if content.packages_provide(Kind::Anim)
            && let Some((m, _)) = content.anim_routes().by_guid.get(&guid)
        {
            return (ClipData::Mod(m.clone()), m.channel_to_dof.clone());
        }
    }
    (ClipData::Disc(Box::new(anim)), table)
}
