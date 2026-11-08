use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use ssx_content::texture::cache::{Outcome, TextureCache, blocks_for};
use ssx_content::texture::{
    BlockChain, BlockFormat, Found, Sampling, Shape, Texture, TextureKind, Want, resolve, resolve_in,
};
use ssx_content::{Asset, ContentId, Kind, Layer, LoadError, PackageSource, Registry, Source};

use super::models::DiscModels;
use super::mods::{self, ModsReport};
use super::objects::DiscRecords;
use super::textures::DiscTextures;
use super::track_packages::TrackPackages;

pub const TEXTURE_CACHE: &str = "Cache/Textures";

mod warm;

pub struct GameContent {
    big_root: PathBuf,
    pub(super) registry: RwLock<Registry>,
    disc: Option<Arc<DiscTextures>>,
    models: Option<Arc<DiscModels>>,
    records: Option<Arc<DiscRecords>>,
    records_mounted: std::sync::atomic::AtomicBool,
    particles: OnceLock<Option<Arc<super::particles::DiscParticles>>>,
    particles_mounted: std::sync::atomic::AtomicBool,
    model_cache: Mutex<BTreeMap<ModelKey, Result<Asset, LoadError>>>,
    cache: RwLock<Option<TextureCache>>,
    pub(super) passed: Mutex<BTreeSet<(ContentId, String)>>,
    notices: Mutex<Vec<Notice>>,
    encoding: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    generation: AtomicU64,
    changes: Mutex<VecDeque<(u64, BTreeSet<ContentId>)>>,
    tracks: Mutex<BTreeMap<(String, u64, u8), Arc<TrackCell>>>,
    pub(super) presets: super::presets::PresetCache,
    pub(super) levels: super::level::LevelCache,
    pub(super) audio: super::audio::AudioCache,
    pub(super) riders: super::characters::RiderCache,
    pub(super) m6: super::m6::UiCache,
    pub(super) fmv: super::fmv::FmvCache,
    pub(super) data: super::data::DataCache,
    pub(super) anims: super::anims::AnimCache,
    pub(super) tricks: super::tricks::TrickCache,
}

pub(super) type TrackCell = OnceLock<Arc<TrackPackages>>;

type ModelKey = (ContentId, String, u64);

const MAX_CHANGE_LOG: usize = 64;

static STORES: OnceLock<Mutex<BTreeMap<PathBuf, Arc<GameContent>>>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub problem: bool,
    pub text: ssx_core::text::Text,
}

pub fn drain_notices() -> Vec<Notice> {
    let Some(stores) = STORES.get() else { return Vec::new() };
    let stores: Vec<Arc<GameContent>> =
        stores.lock().unwrap_or_else(std::sync::PoisonError::into_inner).values().cloned().collect();
    stores.iter().flat_map(|s| s.take_notices()).collect()
}

impl GameContent {
    pub fn of(big_root: &Path) -> Arc<GameContent> {
        let key = std::fs::canonicalize(big_root).unwrap_or_else(|_| big_root.to_path_buf());
        let stores = STORES.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut stores = stores.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        stores.entry(key.clone()).or_insert_with(|| Arc::new(GameContent::new(&key))).clone()
    }

    fn new(big_root: &Path) -> Self {
        let mut registry = Registry::new();
        let mut notices = Vec::new();
        let disc = match DiscTextures::index(big_root) {
            Ok(t) => {
                for (id, files) in &t.collisions {
                    let text = format!("content: {id} names {} files; left out", files.len());
                    notices.push(Notice { problem: true, text: text.into() });
                }
                let t = Arc::new(t);
                registry.mount(Layer::Base, 0, Box::new(Disc(t.clone())));
                Some(t)
            }
            Err(e) => {
                let text = format!("content: the disc's textures under {}: {e:#}", big_root.display());
                notices.push(Notice { problem: true, text: text.into() });
                None
            }
        };
        let models = match DiscModels::index(big_root) {
            Ok(m) => {
                for (id, files) in &m.collisions {
                    notices.push(Notice {
                        problem: true,
                        text: format!("content: {id} names {} files; left out", files.len()).into(),
                    });
                }
                let m = Arc::new(m);
                registry.mount(Layer::Base, 0, Box::new(Shared(m.clone())));
                Some(m)
            }
            Err(e) => {
                let text = format!("content: the disc's models under {}: {e:#}", big_root.display());
                notices.push(Notice { problem: true, text: text.into() });
                None
            }
        };
        let records = models.as_ref().map(|m| Arc::new(DiscRecords::new(big_root, m.clone())));
        registry.mount(Layer::Port, 0, Box::new(super::level::PortSkies));
        registry.mount(Layer::Port, 0, Box::new(super::markers::PortMarkers));
        GameContent {
            big_root: big_root.to_path_buf(),
            registry: RwLock::new(registry),
            disc,
            models,
            records,
            records_mounted: std::sync::atomic::AtomicBool::new(false),
            particles: OnceLock::new(),
            particles_mounted: std::sync::atomic::AtomicBool::new(false),
            model_cache: Mutex::new(BTreeMap::new()),
            cache: RwLock::new(None),
            passed: Mutex::new(BTreeSet::new()),
            encoding: Mutex::new(BTreeMap::new()),
            notices: Mutex::new(notices),
            generation: AtomicU64::new(0),
            changes: Mutex::new(VecDeque::new()),
            tracks: Mutex::new(BTreeMap::new()),
            presets: Default::default(),
            levels: Default::default(),
            audio: Default::default(),
            riders: Default::default(),
            m6: Default::default(),
            fmv: Default::default(),
            data: Default::default(),
            anims: Default::default(),
            tricks: Default::default(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn changes_since(&self, since: u64) -> Option<BTreeSet<ContentId>> {
        let log = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.generation() == since {
            return Some(BTreeSet::new());
        }
        let first = log.front().map_or(u64::MAX, |(g, _)| *g);
        if since + 1 < first {
            return None;
        }
        Some(log.iter().filter(|(g, _)| *g > since).flat_map(|(_, ids)| ids.iter().cloned()).collect())
    }

    pub fn preview_client_mods<R>(
        &self,
        app_dir: &Path,
        inspect: impl FnOnce(&ModsReport, &Registry) -> R,
    ) -> (ModsReport, R) {
        let mut registry = Registry::new();
        if let Some(disc) = &self.disc {
            registry.mount(Layer::Base, 0, Box::new(Disc(disc.clone())));
        }
        if let Some(m) = &self.models {
            registry.mount(Layer::Base, 0, Box::new(Shared(m.clone())));
        }
        registry.mount(Layer::Port, 0, Box::new(super::level::PortSkies));
        registry.mount(Layer::Port, 0, Box::new(super::markers::PortMarkers));
        let mut report = mods::mount_enabled(&mut registry, app_dir);
        if let Some(r) = &self.records
            && provides_records(&registry)
        {
            registry.mount(Layer::Base, 0, Box::new(Shared(r.clone())));
            report.registry = registry.report();
        }
        if provides_kind(&registry, Kind::Particles)
            && let Ok(p) = super::particles::DiscParticles::index(&self.big_root)
        {
            registry.mount(Layer::Base, 0, Box::new(Shared(Arc::new(p))));
            report.registry = registry.report();
        }
        let (sound, music) = super::audio::provides_audio(&registry);
        if sound && let Some(s) = self.disc_sounds() {
            registry.mount(Layer::Base, 0, Box::new(Shared(s)));
            report.registry = registry.report();
        }
        if music && let Some(m) = self.disc_music() {
            registry.mount(Layer::Base, 0, Box::new(Shared(m)));
            report.registry = registry.report();
        }
        if super::characters::RIDER_KINDS.iter().any(|&k| provides_kind(&registry, k))
            && let Some(d) = self.disc_riders()
        {
            registry.mount(Layer::Base, 0, Box::new(Shared(d)));
            report.registry = registry.report();
        }
        if self.mount_m6_preview(&mut registry) {
            report.registry = registry.report();
        }
        if self.mount_fmv_preview(&mut registry) {
            report.registry = registry.report();
        }
        if self.mount_data_preview(&mut registry) {
            report.registry = registry.report();
        }
        if self.mount_anims_preview(&mut registry) {
            report.registry = registry.report();
        }
        let seen = inspect(&report, &registry);
        (report, seen)
    }

    pub fn mount_client_mods_except(&self, app_dir: &Path, except: Option<&str>) -> ModsReport {
        let opened = mods::open_enabled_except(app_dir, except);
        self.warm_for(&opened.sources());
        let mut registry = self.registry.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.unmount(Layer::ClientMods);
        self.tracks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        let mut report = mods::mount_opened(&mut registry, opened);
        if self.mount_records(&mut registry) {
            report.registry = registry.report();
        }
        *self.cache.write().unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(TextureCache::new(&app_dir.join(TEXTURE_CACHE)));
        report
    }

    pub fn swap_dev_package(&self, previous: Option<&str>, source: PackageSource, changed: BTreeSet<ContentId>) {
        self.warm_for(&[&source]);
        {
            let mut registry = self.registry.write().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(name) = previous {
                registry.unmount_named(Layer::ClientMods, name);
            }
            registry.unmount_named(Layer::ClientMods, &source.manifest.resource.name);
            let priority = source.manifest.resource.priority;
            registry.mount(Layer::ClientMods, priority, Box::new(source));
            self.mount_records(&mut registry);
        }
        self.note_change(changed);
    }

    pub(super) fn note_change(&self, changed: BTreeSet<ContentId>) {
        self.passed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|(id, _)| !changed.contains(id));
        let mut log = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        log.push_back((generation, changed));
        while log.len() > MAX_CHANGE_LOG {
            log.pop_front();
        }
    }

    pub fn big_root(&self) -> &Path {
        &self.big_root
    }

    pub(super) fn track_decision(&self, track: &str, generation: u64, policy: u8) -> Arc<TrackCell> {
        let mut tracks = self.tracks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        tracks.retain(|(_, g, _), _| *g == generation);
        tracks.entry((track.to_string(), generation, policy)).or_default().clone()
    }

    pub(super) fn clear_track_decisions(&self) {
        self.tracks.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    }

    pub fn decisions(&self) -> Vec<Arc<TrackPackages>> {
        let generation = self.generation();
        let tracks = self.tracks.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let policy = self.sim_policy() as u8;
        tracks
            .iter()
            .filter(|((_, g, p), _)| *g == generation && *p == policy)
            .filter_map(|(_, cell)| cell.get().cloned())
            .collect()
    }

    pub fn priority_of(&self, source: &str, layer: Layer) -> i32 {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.mounts().find(|m| m.name == source && m.layer == layer).map_or(0, |m| m.priority)
    }

    pub fn has_base(&self, id: &ContentId) -> bool {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.providers(id).any(|(m, _)| m.layer == Layer::Base)
    }

    pub(super) fn mount_records(&self, registry: &mut Registry) -> bool {
        use std::sync::atomic::Ordering::AcqRel;
        let particles = self.mount_particles(registry)
            | self.mount_audio(registry)
            | self.mount_riders(registry)
            | self.mount_m6(registry)
            | self.mount_fmv(registry)
            | self.mount_tuning(registry)
            | self.mount_events(registry)
            | self.mount_anims(registry);
        let Some(r) = &self.records else { return particles };
        let maps = provides_kind(registry, Kind::Map);
        if !(provides_records(registry) || maps) || self.records_mounted.swap(true, AcqRel) {
            return particles;
        }
        registry.mount(Layer::Base, 0, Box::new(Shared(r.clone())));
        match r.names() {
            Ok(n) => self.info(format!(
                "content: the disc's objects, materials, lighting and post-FX presets named for the mounted packages \
                 in {:.2} s ({} ids)",
                n.seconds,
                n.by_id.len()
            )),
            Err(e) => self.notice(format!("content: the disc's objects and materials could not be named: {e}")),
        }
        true
    }

    fn mount_particles(&self, registry: &mut Registry) -> bool {
        use std::sync::atomic::Ordering::AcqRel;
        if !provides_kind(registry, Kind::Particles) || self.particles_mounted.load(Ordering::Acquire) {
            return false;
        }
        let Some(p) = self.particles_index() else { return false };
        if self.particles_mounted.swap(true, AcqRel) {
            return false;
        }
        registry.mount(Layer::Base, 0, Box::new(Shared(p)));
        true
    }

    fn particles_index(&self) -> Option<Arc<super::particles::DiscParticles>> {
        self.particles
            .get_or_init(|| match super::particles::DiscParticles::index(&self.big_root) {
                Ok(p) => Some(Arc::new(p)),
                Err(e) => {
                    self.notice(format!("content: the disc's particle systems: {e:#}"));
                    None
                }
            })
            .clone()
    }

    pub fn disc_particles(&self) -> Option<Arc<super::particles::DiscParticles>> {
        self.particles.get().cloned().flatten()
    }

    pub(super) fn records_for(&self, kind: Kind) -> Option<Arc<DiscRecords>> {
        let mounted = self.records_mounted.load(std::sync::atomic::Ordering::Acquire);
        (mounted && super::objects::is_record_kind(kind)).then(|| self.records.clone()).flatten()
    }

    pub(super) fn cached_load(
        &self,
        id: &ContentId,
        name: &str,
        layer: Layer,
        source: &dyn Source,
    ) -> Result<Asset, LoadError> {
        if matches!(id.kind(), Kind::Map | Kind::Anim | Kind::Trick | Kind::Gamemode)
            && let Some(hit) = self.model_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&(
                id.clone(),
                name.to_string(),
                self.generation(),
            ))
        {
            return hit.clone();
        }
        if layer == Layer::Base || id.kind() != Kind::Model {
            return source.load(id);
        }
        let generation = self.generation();
        let key = (id.clone(), name.to_string(), generation);
        if let Some(hit) = self.model_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return hit.clone();
        }
        let loaded = source.load(id);
        let mut cache = self.model_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|(_, _, g), _| *g == generation);
        cache.insert(key, loaded.clone());
        loaded
    }

    pub(super) fn hold_dev_maps(&self, name: &str, maps: Vec<(ContentId, Asset)>) {
        self.hold_dev_assets(name, maps);
    }

    pub(super) fn hold_dev_assets(&self, name: &str, assets: Vec<(ContentId, Asset)>) {
        let generation = self.generation();
        let mut cache = self.model_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|(_, _, g), _| *g == generation);
        for (id, asset) in assets {
            cache.insert((id, name.to_string(), generation), Ok(asset));
        }
    }

    pub(super) fn held_map(&self, id: &ContentId, name: &str) -> Option<Asset> {
        let cache = self.model_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.get(&(id.clone(), name.to_string(), self.generation())).and_then(|r| r.clone().ok())
    }

    pub fn disc_models(&self) -> Option<&Arc<DiscModels>> {
        self.models.as_ref()
    }

    pub fn disc_records(&self) -> Option<&Arc<DiscRecords>> {
        self.records.as_ref()
    }

    pub(super) fn notice(&self, line: String) {
        self.push(Notice { problem: true, text: line.into() });
    }

    pub(super) fn info(&self, line: String) {
        self.push(Notice { problem: false, text: line.into() });
    }

    fn push(&self, n: Notice) {
        let mut pending = self.notices.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !pending.iter().any(|p| p.problem == n.problem && p.text == n.text) {
            pending.push(n);
        }
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        std::mem::take(&mut *self.notices.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    pub fn mount_client_mods(&self, app_dir: &Path) -> ModsReport {
        self.mount_client_mods_except(app_dir, None)
    }

    pub fn with_registry<R>(&self, f: impl FnOnce(&Registry) -> R) -> R {
        f(&self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    pub fn texture(&self, id: &ContentId, want: &Want) -> Result<Found, LoadError> {
        let mut passed = Vec::new();
        let found = {
            let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            resolve(&registry, id, want, |f| passed.push(f))
        };
        let mut seen = self.passed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for f in passed {
            if seen.insert((f.id.clone(), f.source.clone())) {
                let next = match &found {
                    Ok(t) if t.source != f.source => format!("; {} ({}) is used instead", t.source, t.layer),
                    _ => String::new(),
                };
                self.notice(format!("content: {} from {} ({}) not used: {}{next}", f.id, f.source, f.layer, f.error));
            }
        }
        found
    }

    pub fn base_texture(&self, id: &ContentId, want: &Want) -> Result<Found, LoadError> {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        resolve_in(&registry, id, want, |l| l == Layer::Base, |_| {})
    }

    pub fn base_file(&self, id: &ContentId) -> Option<PathBuf> {
        self.disc.as_ref()?.file(id).map(Path::to_path_buf)
    }

    pub fn winner_layer(&self, id: &ContentId) -> Option<Layer> {
        self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner).winner(id).map(|m| m.layer)
    }

    pub fn has(&self, id: &ContentId) -> bool {
        self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner).has(id)
    }

    pub fn texture_for_gpu(&self, id: &ContentId, want: &Want, device_bc: bool) -> Result<Found, LoadError> {
        let found = self.texture(id, want)?;
        self.with_blocks(id, want, device_bc, found)
    }

    pub fn texture_for_gpu_from(
        &self,
        id: &ContentId,
        source: &str,
        layer: Layer,
        want: &Want,
        device_bc: bool,
    ) -> Result<Found, LoadError> {
        let found = {
            let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            let (_, s) = registry
                .providers(id)
                .find(|(m, _)| m.name == source && m.layer == layer)
                .ok_or_else(|| LoadError::Missing(id.clone()))?;
            let failed = |message: String| LoadError::Failed { id: id.clone(), message };
            let texture = match s.load(id)? {
                Asset::Value(v) => v
                    .downcast::<Texture>()
                    .map(|t| Arc::try_unwrap(t).unwrap_or_else(|shared| (*shared).clone()))
                    .map_err(|_| failed(format!("{source}: not a texture")))?,
                _ => return Err(failed(format!("{source}: not a texture"))),
            };
            let kind = ssx_content::texture::fit(texture.shape, texture.kind, want).map_err(failed)?;
            Found { texture, kind, layer, source: source.to_string() }
        };
        self.with_blocks(id, want, device_bc, found)
    }

    fn with_blocks(&self, id: &ContentId, want: &Want, device_bc: bool, mut found: Found) -> Result<Found, LoadError> {
        if found.layer == Layer::Base || !device_bc || found.texture.shape != Shape::Plane {
            return Ok(found);
        }
        let target = self.disc.as_ref().and_then(|d| d.block_format(id)).unwrap_or(match want.sampling {
            Sampling::NormalXy => BlockFormat::Bc5,
            Sampling::Texels if found.texture.has_alpha() => BlockFormat::Bc3,
            Sampling::Texels => BlockFormat::Bc1,
        });
        let fits = found
            .texture
            .blocks
            .as_ref()
            .is_some_and(|b| b.format == target && b.levels.len() == found.texture.levels.len());
        if !fits {
            match self.blocks(&found.texture, target, found.kind) {
                Ok(chain) => found.texture.blocks = Some(chain),
                Err(e) => {
                    self.notice(format!(
                        "content: {id} from {}: not block-encoded ({e}); uploaded as RGBA8",
                        found.source
                    ));
                    found.texture.blocks = None;
                }
            }
        }
        Ok(found)
    }

    pub fn blocks(&self, texture: &Texture, format: BlockFormat, kind: TextureKind) -> Result<BlockChain, String> {
        let cache = self.cache.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        let key = format!("{:?} {} {}", texture.source, format.name(), kind.as_str());
        let slot = self
            .encoding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(key.clone())
            .or_default()
            .clone();
        let guard = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let start = std::time::Instant::now();
        let encoded = blocks_for(cache.as_ref(), texture, format, kind);
        drop(guard);
        self.encoding.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&key);
        let (chain, outcome, store_error) = encoded?;
        if let Some(e) = store_error {
            self.notice(format!("content: the texture cache: {e}"));
        }
        if outcome != Outcome::Hit {
            let source = texture.source.map(|h| h.to_string()).unwrap_or_default();
            let stored = if outcome == Outcome::Stored { ", stored in the cache" } else { "" };
            self.info(format!(
                "content: encoded {} {}×{} ({} levels, file {source}) in {:.3} s{stored}",
                format.name(),
                texture.width(),
                texture.height(),
                chain.levels.len(),
                start.elapsed().as_secs_f64()
            ));
        }
        Ok(chain)
    }
}

struct Disc(Arc<DiscTextures>);

impl Source for Disc {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn has(&self, id: &ContentId) -> bool {
        self.0.has(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        self.0.load(id)
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        self.0.list(kind)
    }
}

pub(in crate::content) struct Shared<S: Source>(pub(in crate::content) Arc<S>);

impl<S: Source> Source for Shared<S> {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn has(&self, id: &ContentId) -> bool {
        self.0.has(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        self.0.load(id)
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        self.0.list(kind)
    }

    fn rider_entry(&self, id: &ContentId) -> Option<Result<ssx_content::rider::RiderEntry, LoadError>> {
        self.0.rider_entry(id)
    }
}

fn provides_records(registry: &Registry) -> bool {
    [Kind::Object, Kind::Material, Kind::Lighting, Kind::Postfx].into_iter().any(|k| provides_kind(registry, k))
}

pub(super) fn provides_kind(registry: &Registry, kind: Kind) -> bool {
    registry.mounts().any(|m| m.layer > Layer::Base)
        && (!registry.list_above(Layer::Base, kind).is_empty()
            || registry.patched_above(Layer::Base).iter().any(|id| id.kind() == kind))
}
