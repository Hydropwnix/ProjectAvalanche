use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use ssx_content::anim::Clip;
use ssx_content::data::SimRow;
use ssx_content::hash::ContentHash;
use ssx_content::policy::ClientMods;
use ssx_content::registry::Asset;
use ssx_content::trick::{MAX_ADDED_TRICKS, ROLES, TrickToml};
use ssx_content::{ContentId, Kind, Layer};
use ssx_formats::ant::Guid;

use super::anims::{DiscAnims, DiscClip};
use super::store::GameContent;
use crate::anim_clip::{ModClip, Replaced, bind};

#[derive(Default)]
pub struct TrickCache {
    routes: Mutex<Option<TrickRoutesAt>>,
    noticed: Mutex<BTreeSet<String>>,
}

type TrickRoutesAt = ((u64, u8), Arc<TrickRoutes>);

#[derive(Debug)]
pub struct RoutedTrick {
    pub id: ContentId,
    pub source: String,
    pub layer: Layer,
    pub trick: TrickToml,
    pub clips: Vec<(Guid, Arc<ModClip>)>,
    pub hash: ContentHash,
}

#[derive(Debug, Default)]
pub struct TrickRoutes {
    pub by_input: [Option<Arc<RoutedTrick>>; MAX_ADDED_TRICKS],
    pub by_guid: HashMap<Guid, Arc<ModClip>>,
}

impl TrickRoutes {
    pub fn tricks(&self) -> impl Iterator<Item = &Arc<RoutedTrick>> {
        self.by_input.iter().flatten()
    }

    pub fn is_empty(&self) -> bool {
        self.by_input.iter().all(Option::is_none)
    }

    pub fn by_value(&self, v: u32) -> Option<&Arc<RoutedTrick>> {
        let i = v.checked_sub(u32::from(ssx_content::trick::FIRST_VALUE))?;
        self.by_input.get(usize::try_from(i).ok()?)?.as_ref()
    }
}

pub fn clip_guid(trick: &ContentId, role: usize) -> Guid {
    let mut bytes = b"project-avalanche/trick-clip/1\n".to_vec();
    bytes.extend_from_slice(trick.to_string().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&(role as u64).to_le_bytes());
    let mut g = [0; 16];
    g.copy_from_slice(&ContentHash::of(&bytes).0[..16]);
    Guid(g)
}

pub fn base_clip<'a>(disc: &'a DiscAnims, t: &TrickToml, role: usize) -> Option<&'a DiscClip> {
    let name = t.base_clip(role);
    disc.clips.values().find(|c| c.canonical.is_none() && c.id.path().rsplit('/').next() == Some(name.as_str()))
}

impl GameContent {
    pub fn trick_routes(&self) -> Arc<TrickRoutes> {
        let key = (self.generation(), self.sim_policy() as u8);
        let mut slot = self.tricks.routes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((k, r)) = slot.as_ref()
            && *k == key
        {
            return r.clone();
        }
        let r = Arc::new(self.route_tricks());
        *slot = Some((key, r.clone()));
        r
    }

    fn route_tricks(&self) -> TrickRoutes {
        let mut routes = TrickRoutes::default();
        if !self.packages_provide(Kind::Trick) {
            return routes;
        }
        let Some(disc) = self.disc_anims() else { return routes };
        let skeleton = match self.riders.skeletons.get(self.big_root(), None) {
            Ok(s) => s,
            Err(e) => {
                self.notice(format!("content: the riders' skeleton for the packages' tricks: {e:#}"));
                return routes;
            }
        };
        let client_mods = self.sim_policy() == ClientMods::Any;
        let eligible = |layer: Layer| match layer {
            Layer::Base => false,
            Layer::ClientMods => client_mods,
            _ => true,
        };
        let changed = self.changed_ids(&[Kind::Trick]);
        let mut ids: Vec<(usize, ContentId)> = {
            let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            let order: Vec<(Layer, String)> = registry.mounts().map(|m| (m.layer, m.name.to_string())).collect();
            changed
                .into_iter()
                .filter_map(|id| {
                    let (info, _) = registry.providers(&id).find(|(m, _)| eligible(m.layer))?;
                    let rank = order.iter().rposition(|(l, n)| *l == info.layer && n == info.name)?;
                    Some((rank, id))
                })
                .collect()
        };
        ids.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        for (_, id) in ids {
            let mut i = 0;
            loop {
                let loaded = {
                    let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut providers = registry.providers(&id).skip(i);
                    let Some((info, source)) = providers.next() else { break };
                    let next = providers.next().map(|(n, _)| n.name.to_string());
                    let asset = eligible(info.layer).then(|| self.cached_load(&id, info.name, info.layer, source));
                    (info.name.to_string(), info.layer, next, asset)
                };
                i += 1;
                let (source, layer, next, asset) = loaded;
                let Some(asset) = asset else { continue };
                let routed = asset
                    .map_err(|e| e.to_string())
                    .and_then(|a| a.into_value::<TrickToml>().ok_or_else(|| format!("{id}: not a decoded trick")))
                    .and_then(|t| {
                        let clip_of = |c: &ContentId| self.package_clip(c, &source);
                        bind_trick(&id, &source, layer, (*t).clone(), (&disc, &skeleton.rig), clip_of)
                    })
                    .and_then(|r| match &routes.by_input[r.trick.input.index()] {
                        Some(other) => {
                            let whose = if other.source == r.source {
                                "the same package's, first by id".to_string()
                            } else {
                                format!("from {}, a stronger package", other.source)
                            };
                            Err(format!(
                                "input {} is {}'s ({whose}; one trick an input)",
                                r.trick.input.name(),
                                other.id
                            ))
                        }
                        None => Ok(r),
                    });
                match routed {
                    Ok(r) => {
                        for (g, m) in &r.clips {
                            routes.by_guid.insert(*g, m.clone());
                        }
                        let slot = r.trick.input.index();
                        routes.by_input[slot] = Some(Arc::new(r));
                        break;
                    }
                    Err(why) => self.passed_over(&id, &source, layer, &why, next.as_deref()),
                }
            }
        }
        routes
    }

    pub(in crate::content) fn dev_trick_problem(
        &self,
        asset: &Asset,
        package: &str,
        id: &ContentId,
        clips: &[(ContentId, Asset)],
        changed: &BTreeSet<ContentId>,
    ) -> Option<String> {
        let t = asset.clone().into_value::<TrickToml>()?;
        if !changed.contains(id) && !t.clips.iter().any(|c| changed.contains(c)) {
            return None;
        }
        let disc = self.disc_anims()?;
        let skeleton = match self.riders.skeletons.get(self.big_root(), None) {
            Ok(s) => s,
            Err(e) => return Some(format!("the riders' skeleton: {e:#}")),
        };
        let clip_of = |c: &ContentId| {
            clips
                .iter()
                .find(|(k, _)| k == c)
                .and_then(|(_, a)| a.clone().into_value::<Clip>())
                .ok_or_else(|| format!("{c}: {package} has no clip of that id"))
        };
        bind_trick(id, package, Layer::ClientMods, (*t).clone(), (&disc, &skeleton.rig), clip_of)
            .err()
            .map(|e| format!("{id}: {e}"))
    }

    fn package_clip(&self, id: &ContentId, source: &str) -> Result<Arc<Clip>, String> {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some((info, src)) = registry.providers(id).find(|(info, _)| info.name == source) else {
            return Err(format!("{id}: {source} has no clip of that id"));
        };
        self.cached_load(id, info.name, info.layer, src)
            .map_err(|e| e.to_string())
            .and_then(|a| a.into_value::<Clip>().ok_or_else(|| format!("{id}: not a decoded clip")))
    }

    pub(in crate::content) fn trick_rows(&self) -> Vec<SimRow> {
        let routes = self.trick_routes();
        routes
            .tricks()
            .map(|r| SimRow {
                id: r.id.to_string(),
                class: "trick".into(),
                hash: r.hash,
                layer: format!("{} {}", r.layer, r.source),
            })
            .collect()
    }

    pub(in crate::content) fn trick_clip_ids(&self) -> BTreeSet<ContentId> {
        self.trick_routes().tricks().flat_map(|r| r.trick.clips.iter().cloned()).collect()
    }
}

fn bind_trick(
    id: &ContentId,
    source: &str,
    layer: Layer,
    t: TrickToml,
    (disc, rig): (&DiscAnims, &ssx_formats::crsf::SceneGraph),
    clip_of: impl Fn(&ContentId) -> Result<Arc<Clip>, String>,
) -> Result<RoutedTrick, String> {
    if id.namespace() != source {
        return Err(format!("an added trick is an id of its package's namespace ({source}:trick/…)"));
    }
    for (name, field) in t.names.iter().zip(["name", "name_uber", "name_super_uber"]) {
        if name.namespace() != source {
            return Err(format!("{field}: {name} is not a string of {source}"));
        }
    }
    let mut clips = Vec::with_capacity(ROLES.len());
    let mut hashes = Vec::with_capacity(ROLES.len());
    for (r, clip_id) in t.clips.iter().enumerate() {
        let role = ROLES[r].0;
        if clip_id.namespace() != source {
            return Err(format!("clips.{role}: {clip_id} is not a clip of {source} (a trick plays its own clips)"));
        }
        let dc = base_clip(disc, &t, r)
            .ok_or_else(|| format!("clips.{role}: the disc has no clip {} to stand in", t.base_clip(r)))?;
        let clip = clip_of(clip_id).map_err(|e| format!("clips.{role}: {e}"))?;
        hashes.push(ssx_content::anim::sim_hash(&clip));
        let replaced =
            Replaced { id: &dc.id, end_frame: dc.end_frame, additive: dc.additive, frame_rate: dc.frame_rate() };
        let m = bind(clip, source, replaced, &disc.dofs, rig).map_err(|e| format!("clips.{role}: {e}"))?;
        clips.push((clip_guid(id, r), Arc::new(m)));
    }
    let hashes: [ContentHash; 12] = hashes.try_into().map_err(|_| "the clips are not 12".to_string())?;
    let hash = ssx_content::trick::sim_hash(&t, &hashes);
    Ok(RoutedTrick { id: id.clone(), source: source.to_string(), layer, trick: t, clips, hash })
}

pub fn tricks_for(root: Option<&Path>) -> Option<Arc<TrickRoutes>> {
    let content = GameContent::of(root?);
    if !content.packages_provide(Kind::Trick) {
        return None;
    }
    let r = content.trick_routes();
    (!r.is_empty()).then_some(r)
}

pub fn notice(root: Option<&Path>, line: String) {
    if let Some(root) = root {
        let content = GameContent::of(root);
        let mut seen = content.tricks.noticed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if seen.insert(line.clone()) {
            content.notice(format!("content: {line}"));
        }
    }
}
