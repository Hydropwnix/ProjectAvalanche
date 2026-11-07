pub mod bots;
pub mod budget;
pub mod career;
mod disc;
pub mod dress;
pub mod look;
pub mod ride;
pub mod skeleton;
mod voice;

use std::sync::{Arc, OnceLock};

use ssx_content::rider::{CharacterDef, ItemDef, RiderEntry, parse_character_table, parse_item_table};
use ssx_content::{ContentId, Kind, Layer, LoadError};

pub use disc::DiscRiders;

use super::store::{GameContent, Shared, provides_kind};

pub const RIDER_KINDS: [Kind; 3] = [Kind::Character, Kind::Gear, Kind::Board];

#[derive(Default)]
pub struct RiderCache {
    disc: OnceLock<Option<Arc<DiscRiders>>>,
    mounted: std::sync::atomic::AtomicBool,
    pub skeletons: skeleton::SkeletonCache,
    decisions: std::sync::atomic::AtomicU64,
    latest: std::sync::Mutex<Option<(u64, budget::RiderLooks)>>,
}

#[derive(Clone, Debug)]
pub struct Resolved<T> {
    pub id: ContentId,
    pub value: T,
    pub source: String,
    pub layer: Layer,
    pub patched_by: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookSource {
    pub source: String,
    pub layer: Layer,
}

impl GameContent {
    pub fn provides_riders(&self) -> bool {
        self.with_registry(|r| RIDER_KINDS.iter().any(|&k| provides_kind(r, k)))
    }

    pub(super) fn mount_riders(&self, registry: &mut ssx_content::Registry) -> bool {
        use std::sync::atomic::Ordering::AcqRel;
        if !RIDER_KINDS.iter().any(|&k| provides_kind(registry, k))
            || self.riders.mounted.load(std::sync::atomic::Ordering::Acquire)
        {
            return false;
        }
        let Some(d) = self.disc_riders() else { return false };
        if self.riders.mounted.swap(true, AcqRel) {
            return false;
        }
        registry.mount(Layer::Base, 0, Box::new(Shared(d)));
        true
    }

    pub fn disc_riders(&self) -> Option<Arc<DiscRiders>> {
        self.riders
            .disc
            .get_or_init(|| match DiscRiders::index(self.big_root()) {
                Ok(d) => {
                    self.info(format!(
                        "content: the disc's characters, gear and boards named for the mounted packages in {:.2} s ({} ids)",
                        d.seconds,
                        d.len()
                    ));
                    Some(Arc::new(d))
                }
                Err(e) => {
                    self.notice(format!("content: the disc's characters, gear and boards: {e:#}"));
                    None
                }
            })
            .clone()
    }

    fn record_table(&self, id: &ContentId) -> Result<(toml::Table, String, Layer, Vec<String>), LoadError> {
        let (table, source, layer) = self.with_registry(|r| {
            for (m, s) in r.providers(id) {
                match s.rider_entry(id) {
                    Some(Ok(RiderEntry { data: Some((_, t)), .. })) => return Ok((t, m.name.to_string(), m.layer)),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e),
                    None => continue,
                }
            }
            Err(LoadError::Missing(id.clone()))
        })?;
        let patches = self.with_registry(|r| r.patches_of(id))?;
        let mut table = table;
        let mut by = Vec::new();
        for (name, p) in patches {
            ssx_content::patch::merge(&mut table, &p);
            by.push(name);
        }
        Ok((table, source, layer, by))
    }

    pub fn character_def(&self, id: &ContentId) -> Result<Resolved<CharacterDef>, LoadError> {
        let (t, source, layer, patched_by) = self.record_table(id)?;
        let value = parse_character_table(&t).map_err(|ps| failed(id, &source, ps))?;
        Ok(Resolved { id: id.clone(), value, source, layer, patched_by })
    }

    pub fn item_def(&self, id: &ContentId) -> Result<Resolved<ItemDef>, LoadError> {
        let (t, source, layer, patched_by) = self.record_table(id)?;
        let value = parse_item_table(&t, id.kind() == Kind::Board).map_err(|ps| failed(id, &source, ps))?;
        Ok(Resolved { id: id.clone(), value, source, layer, patched_by })
    }

    pub fn look_sources(&self, id: &ContentId) -> Vec<LookSource> {
        self.with_registry(|r| {
            r.providers(id)
                .filter(|(m, _)| m.layer != Layer::Base)
                .filter(|(_, s)| matches!(s.rider_entry(id), Some(Ok(RiderEntry { model: Some(_), .. }))))
                .map(|(m, _)| LookSource { source: m.name.to_string(), layer: m.layer })
                .collect()
        })
    }

    pub fn look_cost(
        &self,
        id: &ContentId,
        at: &LookSource,
    ) -> Result<(String, ssx_content::model::ModelCost), LoadError> {
        let none = || LoadError::Failed { id: id.clone(), message: format!("{}: no model", at.source) };
        self.with_source(id, &at.source, at.layer, |s| s.rider_model_cost(id))
            .flatten()
            .unwrap_or_else(|| Err(none()))?
            .ok_or_else(none)
    }

    pub fn look_model(&self, id: &ContentId, at: &LookSource) -> Result<Arc<ssx_content::model::Model>, LoadError> {
        let none = || LoadError::Failed { id: id.clone(), message: format!("{}: no model", at.source) };
        self.with_source(id, &at.source, at.layer, |s| s.rider_model(id))
            .flatten()
            .unwrap_or_else(|| Err(none()))?
            .ok_or_else(none)
    }

    pub fn rider_look_model(&self, id: &ContentId) -> Result<(Arc<ssx_content::model::Model>, LookSource), LoadError> {
        let sources = self.look_sources(id);
        let mut last = LoadError::Missing(id.clone());
        for (i, at) in sources.iter().enumerate() {
            match self.look_model(id, at) {
                Ok(m) => return Ok((m, at.clone())),
                Err(e) => {
                    let next = sources.get(i + 1).map(|n| n.source.as_str());
                    self.passed_over(id, &at.source, at.layer, &e.to_string(), next);
                    last = e;
                }
            }
        }
        Err(last)
    }

    pub fn package_item_keys(&self) -> std::collections::BTreeMap<u32, ContentId> {
        let mut out = std::collections::BTreeMap::new();
        if !self.provides_riders() {
            return out;
        }
        for kind in [Kind::Gear, Kind::Board] {
            for id in self.added(kind) {
                out.insert(career::content_key(&id), id.clone());
                out.insert(career::pants_key(&id), id);
            }
        }
        if let Some(disc) = self.disc_riders() {
            for (&key, id) in &disc.looks {
                if !self.look_sources(id).is_empty() {
                    out.insert(key, id.clone());
                }
            }
        }
        out
    }

    pub fn appearance_key(&self, id: &ContentId, stand_in: &str) -> u32 {
        if self.server_rider(id) { career::content_key(id) } else { ssx_formats::attrib::fnv1(stand_in.as_bytes()) }
    }

    pub fn session_rider(&self, key: u32) -> Option<(ContentId, String)> {
        let id = self.added(Kind::Character).into_iter().find(|id| career::content_key(id) == key)?;
        self.stand_in_of_server_rider(id)
    }

    pub fn session_stand_ins(&self) -> Vec<(ContentId, u32, u32)> {
        let riders = self.added(Kind::Character).into_iter().filter_map(|id| self.stand_in_of_server_rider(id));
        riders
            .map(|(id, stand_in)| {
                let key = career::content_key(&id);
                (id, key, ssx_formats::attrib::fnv1(stand_in.as_bytes()))
            })
            .collect()
    }

    fn stand_in_of_server_rider(&self, id: ContentId) -> Option<(ContentId, String)> {
        if !self.server_rider(&id) {
            return None;
        }
        let stand_in = self.character_def(&id).ok()?.value.stands_in_for?;
        Some((id, stand_in.path().to_string()))
    }

    fn server_rider(&self, id: &ContentId) -> bool {
        id.kind() == Kind::Character
            && id.namespace() != ssx_content::BASE_NAMESPACE
            && self.with_registry(|r| r.winner(id).is_some_and(|m| m.layer == Layer::Server))
    }

    pub fn added(&self, kind: Kind) -> Vec<ContentId> {
        self.with_registry(|r| {
            r.list_above(Layer::Base, kind)
                .into_iter()
                .filter(|id| id.namespace() != ssx_content::BASE_NAMESPACE)
                .collect()
        })
    }
}

fn failed(id: &ContentId, source: &str, problems: Vec<ssx_content::material::Problem>) -> LoadError {
    let lines: Vec<String> = problems
        .into_iter()
        .map(|(l, c, m)| if l == 0 { format!("{source}: {m}") } else { format!("{source}:{l}:{c}: {m}") })
        .collect();
    LoadError::Failed { id: id.clone(), message: lines.join("; ") }
}

#[cfg(test)]
mod session_tests {
    use ssx_content::PackageSource;
    use ssx_content::package::Origin;

    use super::*;

    fn package(dir: &std::path::Path, name: &str, rider: &str, stand_in: &str, origin: Origin) -> PackageSource {
        let root = dir.join(name);
        std::fs::create_dir_all(&root).unwrap();
        let manifest = format!(
            "[resource]\nname = \"{name}\"\nversion = \"1.0.0\"\napi = 1\ntype = \"mod\"\n\n[add]\n\"character/{rider}\" = \
             \"{rider}.toml\"\n"
        );
        std::fs::write(root.join("resource.toml"), manifest).unwrap();
        std::fs::write(root.join(format!("{rider}.toml")), format!("stands_in_for = \"ssx:character/{stand_in}\"\n"))
            .unwrap();
        PackageSource::open_as(&root, origin).unwrap()
    }

    #[test]
    fn a_servers_rider_is_named_by_its_id_and_a_client_mods_by_its_stand_in() {
        let dir = std::env::temp_dir().join(format!("ssx-net6-riders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let content = GameContent::of(&dir.join("absent-big"));
        {
            let mut r = content.registry.write().unwrap();
            r.mount(Layer::ClientMods, 0, Box::new(package(&dir, "mine", "kim", "zoe", Origin::Client)));
            r.mount(Layer::Server, 0, Box::new(package(&dir, "skins", "sam", "elise", Origin::Server)));
        }
        let (sam, kim): (ContentId, ContentId) =
            ("skins:character/sam".parse().unwrap(), "mine:character/kim".parse().unwrap());
        let key = career::content_key(&sam);
        assert_eq!(content.appearance_key(&sam, "elise"), key);
        assert_eq!(content.session_rider(key), Some((sam.clone(), "elise".to_string())));
        assert_eq!(content.session_stand_ins(), vec![(sam, key, ssx_formats::attrib::fnv1(b"elise"))]);
        let zoe = ssx_formats::attrib::fnv1(b"zoe");
        assert_eq!(content.appearance_key(&kim, "zoe"), zoe, "a client mod's rider: its stand-in");
        assert_eq!(content.session_rider(career::content_key(&kim)), None);
        assert_eq!(content.session_rider(zoe), None, "a disc rider's key is the caller's to resolve");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
