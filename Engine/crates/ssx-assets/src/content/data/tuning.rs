use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use ssx_content::data::ant_schema::{LINKED_FLOATS, Ranges, floats_differ, record_schema};
use ssx_content::data::{Class, Mode, RecordSchema, check_table_with};
use ssx_content::{Asset, ContentId, Kind, Layer, LoadError, Source};
use ssx_formats::ant::{Object, Stream};

use super::super::ant::{BUNDLE, asset_id};
use super::super::store::GameContent;

pub const PRESENTATION_CLASSES: &[&str] = &[];

pub fn class_of(class: &str) -> Class {
    if PRESENTATION_CLASSES.contains(&class) { Class::Presentation } else { Class::Simulation }
}

pub struct DiscTuning {
    pub stream: Stream,
    pub by_id: BTreeMap<ContentId, usize>,
    pub collisions: Vec<ContentId>,
    pub ranges: Ranges,
}

impl DiscTuning {
    pub fn load(big_root: &Path) -> Result<DiscTuning> {
        let path = big_root.join(BUNDLE);
        let data = ssx_rules::read::read_file(&path)?;
        let stream = Stream::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
        let mut by_id = BTreeMap::new();
        let mut collisions = Vec::new();
        for (i, a) in stream.assets.iter().enumerate() {
            let Some((kind, path)) = asset_id(&stream.schema, &a.object) else { continue };
            let Ok(id) = ContentId::new(ssx_content::BASE_NAMESPACE, kind, &path) else { continue };
            if by_id.insert(id.clone(), i).is_some() {
                collisions.push(id);
            }
        }
        for id in &collisions {
            by_id.remove(id);
        }
        let ranges = Ranges::of(&stream);
        Ok(DiscTuning { stream, by_id, collisions, ranges })
    }

    pub fn object(&self, id: &ContentId) -> Option<&Object> {
        self.by_id.get(id).and_then(|&i| self.stream.assets.get(i)).map(|a| &a.object)
    }

    pub fn schema_of(&self, id: &ContentId) -> Option<RecordSchema> {
        let obj = self.object(id)?;
        let class = class_of(&obj.type_desc(&self.stream.schema).name);
        Some(record_schema(&self.stream.schema, obj, &self.ranges, class))
    }

    pub fn guid_class(&self, g: &[u8; 16]) -> Option<String> {
        self.ranges.classes.get(&ssx_formats::ant::Guid(*g)).cloned()
    }

    pub fn record(&self, id: &ContentId) -> Result<toml::Table, String> {
        let obj = self.object(id).ok_or_else(|| format!("{id}: no such asset"))?;
        ssx_content::data::ant::to_table(&self.stream.schema, obj).map(|(t, _)| t)
    }

    pub fn applied(&self, id: &ContentId, table: &toml::Table) -> Result<Object, String> {
        let schema = self.schema_of(id).ok_or_else(|| format!("{id}: no such asset"))?;
        let spans = ssx_content::data::spans::Spans::default();
        let guids = |g: &[u8; 16]| self.guid_class(g);
        let problems = check_table_with(&schema, table, &spans, Mode::Whole, Some(&guids));
        if !problems.is_empty() {
            let shown: Vec<String> = problems.iter().take(8).map(|p| format!("{}: {}", p.path, p.message)).collect();
            return Err(shown.join("; "));
        }
        let mut obj = self.object(id).cloned().ok_or_else(|| format!("{id}: no such asset"))?;
        ssx_content::data::ant::apply(&self.stream.schema, &mut obj, table)?;
        Ok(obj)
    }
}

pub type BundleReader = fn(&Stream) -> Result<(), String>;

static READER: OnceLock<BundleReader> = OnceLock::new();

pub fn set_bundle_reader(reader: BundleReader) {
    let _ = READER.set(reader);
}

pub fn bundle_reader() -> Option<BundleReader> {
    READER.get().copied()
}

#[derive(Clone)]
pub struct Candidate {
    pub id: ContentId,
    pub index: usize,
    pub object: Object,
    pub table: toml::Table,
    pub source: String,
}

impl DiscTuning {
    pub fn linked(&self, mut records: Vec<Candidate>) -> (Vec<Candidate>, Vec<(ContentId, String)>) {
        let mut refused = Vec::new();
        for &(a, b, source) in LINKED_FLOATS {
            let (Ok(a), Ok(b)) = (a.parse::<ContentId>(), b.parse::<ContentId>()) else { continue };
            if !records.iter().any(|x| x.id == a || x.id == b) {
                continue;
            }
            let table = |id: &ContentId| {
                records.iter().find(|x| x.id == *id).map(|x| x.table.clone()).or_else(|| self.record(id).ok())
            };
            let (Some(ta), Some(tb)) = (table(&a), table(&b)) else { continue };
            if let Some(d) = floats_differ(&ta, &tb) {
                let why = format!("{a} and {b} differ ({d}): {source}");
                records.retain(|x| {
                    let linked = x.id == a || x.id == b;
                    if linked {
                        refused.push((x.id.clone(), why.clone()));
                    }
                    !linked
                });
            }
        }
        (records, refused)
    }

    pub fn readable(
        &self,
        records: Vec<Candidate>,
        reader: Option<BundleReader>,
    ) -> (Vec<Candidate>, Vec<(ContentId, String)>) {
        let mut refused = Vec::new();
        let Some(reader) = reader else { return (records, refused) };
        if records.is_empty() {
            return (records, refused);
        }
        let mut stream = self.stream.clone();
        for x in &records {
            if let Some(a) = stream.assets.get_mut(x.index) {
                a.object = x.object.clone();
            }
        }
        if reader(&stream).is_ok() || reader(&self.stream).is_err() {
            return (records, refused);
        }
        let mut stream = self.stream.clone();
        let set = |s: &mut Stream, x: &Candidate| {
            s.assets.get_mut(x.index).map(|a| std::mem::replace(&mut a.object, x.object.clone()))
        };
        let restore = |s: &mut Stream, x: &Candidate, before: Object| {
            if let Some(a) = s.assets.get_mut(x.index) {
                a.object = before;
            }
        };
        let (kept, no) = keep_each(&mut stream, records, set, restore, reader);
        let shown = |e: &str| ssx_core::text::shown_max(e, 240).to_string();
        refused.extend(no.into_iter().map(|(x, e)| (x.id, format!("the game's reader: {}", shown(&e)))));
        (kept, refused)
    }
}

pub(super) fn keep_each<S, T, U>(
    state: &mut S,
    items: Vec<T>,
    set: impl Fn(&mut S, &T) -> Option<U>,
    restore: impl Fn(&mut S, &T, U),
    reads: impl Fn(&S) -> Result<(), String>,
) -> (Vec<T>, Vec<(T, String)>) {
    let (mut kept, mut refused) = (Vec::new(), Vec::new());
    for x in items {
        let Some(before) = set(state, &x) else {
            refused.push((x, "the bundle has no such asset".to_string()));
            continue;
        };
        match reads(state) {
            Ok(()) => kept.push(x),
            Err(e) => {
                restore(state, &x, before);
                refused.push((x, e));
            }
        }
    }
    (kept, refused)
}

impl Source for DiscTuning {
    fn name(&self) -> &str {
        ssx_content::BASE_NAMESPACE
    }

    fn has(&self, id: &ContentId) -> bool {
        self.by_id.contains_key(id)
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        self.record(id).map(Asset::Record).map_err(|message| LoadError::Failed { id: id.clone(), message })
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        self.by_id.keys().filter(|id| id.kind() == kind).cloned().collect()
    }
}

#[derive(Default)]
pub struct TuningCache {
    pub(in crate::content) disc: OnceLock<Option<Arc<DiscTuning>>>,
    pub(in crate::content) mounted: std::sync::atomic::AtomicBool,
    candidates: Mutex<Option<(super::events::Generation, Arc<Vec<Candidate>>)>>,
    applied: Mutex<Option<(super::events::Generation, bool, Arc<Applied>)>>,
}

pub type Applied = Vec<(usize, Object)>;

pub const KINDS: [Kind; 2] = [Kind::Tuning, Kind::Scoring];

pub fn anim_all(big_root: &Path) -> Result<Stream> {
    let path = big_root.join(BUNDLE);
    let data = ssx_rules::read::read_file(&path)?;
    let mut stream = Stream::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    let content = GameContent::of(big_root);
    if KINDS.iter().any(|&k| content.packages_provide(k)) {
        for (i, obj) in content.tuning_records().iter() {
            if let Some(a) = stream.assets.get_mut(*i) {
                a.object = obj.clone();
            }
        }
    }
    Ok(stream)
}

pub fn routed_bundle(path: &Path) -> Option<&Path> {
    let mut root = path;
    for _ in 0..BUNDLE.split('/').count() {
        root = root.parent()?;
    }
    (root.join(BUNDLE) == path).then_some(root)
}

impl GameContent {
    pub fn disc_tuning(&self) -> Option<Arc<DiscTuning>> {
        self.data
            .tuning
            .disc
            .get_or_init(|| match DiscTuning::load(self.big_root()) {
                Ok(t) => {
                    for id in &t.collisions {
                        self.notice(format!("content: {id} names two assets of the bundle; left out"));
                    }
                    Some(Arc::new(t))
                }
                Err(e) => {
                    self.notice(format!("content: the disc's tuning: {e:#}"));
                    None
                }
            })
            .clone()
    }

    pub(in crate::content) fn mount_tuning(&self, registry: &mut ssx_content::Registry) -> bool {
        use std::sync::atomic::Ordering::AcqRel;
        let wanted = KINDS.iter().any(|&k| super::super::store::provides_kind(registry, k));
        if !wanted || self.data.tuning.mounted.load(std::sync::atomic::Ordering::Acquire) {
            return false;
        }
        let Some(d) = self.disc_tuning() else { return false };
        if self.data.tuning.mounted.swap(true, AcqRel) {
            return false;
        }
        registry.mount(Layer::Base, 0, Box::new(super::super::store::Shared(d)));
        true
    }

    pub fn tuning_records(&self) -> Arc<Applied> {
        let generation = (self.generation(), self.sim_policy() as u8);
        let reader = bundle_reader();
        let mut slot = self.data.tuning.applied.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((g, read, v)) = slot.as_ref()
            && *g == generation
            && *read == reader.is_some()
        {
            return v.clone();
        }
        let candidates = self.tuning_candidates();
        let out: Applied = match self.disc_tuning() {
            Some(disc) => {
                let (kept, refused) = disc.readable(candidates.to_vec(), reader);
                for (id, why) in refused {
                    self.notice(format!("content: {id} not used ({why}); the disc's asset is"));
                }
                kept.into_iter()
                    .map(|c| {
                        self.info(format!("content: {}: from {}", c.id, c.source));
                        (c.index, c.object)
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        let out = Arc::new(out);
        *slot = Some((generation, reader.is_some(), out.clone()));
        out
    }

    pub fn tuning_candidates(&self) -> Arc<Vec<Candidate>> {
        let generation = (self.generation(), self.sim_policy() as u8);
        let mut slot = self.data.tuning.candidates.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((g, v)) = slot.as_ref()
            && *g == generation
        {
            return v.clone();
        }
        let out = Arc::new(self.apply_tuning());
        *slot = Some((generation, out.clone()));
        out
    }

    fn apply_tuning(&self) -> Vec<Candidate> {
        let Some(disc) = self.disc_tuning() else { return Vec::new() };
        let mut out = Vec::new();
        for id in self.changed_ids(&KINDS) {
            let Some(&index) = disc.by_id.get(&id) else {
                self.notice(format!("content: {id}: the bundle has no such asset (tuning is changed, not added)"));
                continue;
            };
            let class =
                disc.object(&id).map_or(Class::Simulation, |o| class_of(&o.type_desc(&disc.stream.schema).name));
            let routed = disc.record(&id).and_then(|t| self.record_for(&id, t, class));
            match routed.and_then(|(table, source, layer)| disc.applied(&id, &table).map(|o| (o, table, source, layer)))
            {
                Ok((_, _, source, Layer::Base)) if source == ssx_content::BASE_NAMESPACE => {}
                Ok((object, table, source, _)) => out.push(Candidate { id, index, object, table, source }),
                Err(why) => self.notice(format!("content: {id} not used ({why}); the disc's asset is")),
            }
        }
        let (kept, refused) = disc.linked(out);
        for (id, why) in refused {
            self.notice(format!("content: {id} not used ({why}); the disc's asset is"));
        }
        kept
    }
}
