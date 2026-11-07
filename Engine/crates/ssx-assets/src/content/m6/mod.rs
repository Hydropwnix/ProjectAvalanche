pub mod disc;
pub mod export;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use ssx_content::font::Converted;
use ssx_content::strings::{self, Lang};
use ssx_content::{Asset, ContentId, Kind, Layer, Registry};

use super::GameContent;
use super::store::{Shared, provides_kind};
pub use disc::{DiscFonts, DiscStrings, DiscUi, LibraryFont, PortStrings};

#[derive(Default)]
pub struct UiCache {
    ui: OnceLock<Option<Arc<DiscUi>>>,
    fonts: OnceLock<Option<Arc<DiscFonts>>>,
    strings: OnceLock<Option<Arc<DiscStrings>>>,
    port_strings: OnceLock<Option<Arc<PortStrings>>>,
    mounted: Mutex<[bool; 3]>,
    movies: Mutex<BTreeMap<(ContentId, u64), Option<PackageMovie>>>,
    refused: Mutex<BTreeMap<(ContentId, String), String>>,
    movie_strings: Mutex<BTreeMap<ContentId, Option<Arc<BTreeSet<String>>>>>,
}

#[derive(Clone, Debug)]
pub struct PackageMovie {
    pub source: String,
    pub bytes: Arc<[u8]>,
    pub info: ssx_content::ui::MovieInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageString {
    pub key: String,
    pub text: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct PackageFont {
    pub id: ContentId,
    pub source: String,
    pub font: Arc<Converted>,
    pub replaces: Option<LibraryFont>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl GameContent {
    pub(in crate::content) fn disc_ui(&self) -> Option<Arc<DiscUi>> {
        self.m6.ui.get_or_init(|| index(self, "movies", DiscUi::index(self.big_root()))).clone()
    }

    pub fn disc_fonts(&self) -> Option<Arc<DiscFonts>> {
        self.m6.fonts.get_or_init(|| index(self, "fonts", DiscFonts::index(self.big_root()))).clone()
    }

    pub fn disc_strings(&self) -> Option<Arc<DiscStrings>> {
        self.m6.strings.get_or_init(|| index(self, "strings", DiscStrings::index(self.big_root()))).clone()
    }

    pub(in crate::content) fn port_strings(&self) -> Option<Arc<PortStrings>> {
        self.m6.port_strings.get_or_init(|| index(self, "port strings", PortStrings::parse())).clone()
    }

    pub(super) fn mount_m6(&self, registry: &mut Registry) -> bool {
        let mut mounted = lock(&self.m6.mounted);
        self.mount_m6_into(registry, &mut mounted)
    }

    pub(super) fn mount_m6_preview(&self, registry: &mut Registry) -> bool {
        self.mount_m6_into(registry, &mut [false; 3])
    }

    fn mount_m6_into(&self, registry: &mut Registry, mounted: &mut [bool; 3]) -> bool {
        let mut any = false;
        for (i, kind) in [Kind::Ui, Kind::Font, Kind::String].into_iter().enumerate() {
            if mounted[i] || !provides_kind(registry, kind) {
                continue;
            }
            match kind {
                Kind::Ui => match self.disc_ui() {
                    Some(d) => registry.mount(Layer::Base, 0, Box::new(Shared(d))),
                    None => continue,
                },
                Kind::Font => match self.disc_fonts() {
                    Some(d) => registry.mount(Layer::Base, 0, Box::new(Shared(d))),
                    None => continue,
                },
                _ => {
                    let (Some(d), Some(p)) = (self.disc_strings(), self.port_strings()) else { continue };
                    registry.mount(Layer::Base, 0, Box::new(Shared(d)));
                    registry.mount(Layer::Port, 0, Box::new(Shared(p)));
                }
            }
            mounted[i] = true;
            any = true;
        }
        any
    }

    pub fn package_movie(&self, id: &ContentId) -> Option<PackageMovie> {
        if !self.packages_provide(Kind::Ui) {
            return None;
        }
        let generation = self.generation();
        let key = (id.clone(), generation);
        if let Some(hit) = lock(&self.m6.movies).get(&key) {
            return hit.clone();
        }
        let refused = lock(&self.m6.refused).clone();
        let taken = self.with_registry(|r| {
            for (m, src) in r.providers(id) {
                if m.layer <= Layer::Port {
                    return None;
                }
                if let Some(why) = refused.get(&(id.clone(), m.name.to_string())) {
                    self.passed_over(id, m.name, m.layer, why, Some("the next source"));
                    continue;
                }
                let bytes = match src.load(id) {
                    Ok(Asset::Bytes(b)) => b,
                    Ok(_) => continue,
                    Err(e) => {
                        self.passed_over(id, m.name, m.layer, &e.to_string(), Some("the next source"));
                        continue;
                    }
                };
                match ssx_content::ui::check_movie(&bytes, &ssx_formats::gfx::Limits::default()) {
                    Ok(info) => return Some(PackageMovie { source: m.name.to_string(), bytes, info }),
                    Err(e) => self.passed_over(id, m.name, m.layer, &e, Some("the next source")),
                }
            }
            None
        });
        let mut movies = lock(&self.m6.movies);
        movies.retain(|(_, g), _| *g == generation);
        movies.insert(key, taken.clone());
        taken
    }

    pub fn disc_movie_strings(&self, id: &ContentId) -> Option<Arc<BTreeSet<String>>> {
        if let Some(hit) = lock(&self.m6.movie_strings).get(id) {
            return hit.clone();
        }
        let strings = self.disc_ui().and_then(|d| d.files.get(id).cloned()).and_then(|f| {
            let bytes = std::fs::read(f).ok()?;
            let movie = ssx_formats::gfx::Movie::parse(&bytes).ok()?;
            let mut out = BTreeSet::new();
            let mut take = |actions: &[ssx_formats::gfx::avm1::Action]| {
                use ssx_formats::gfx::avm1::{ActionData, PushValue};
                for a in actions {
                    match &a.data {
                        ActionData::ConstantPool(p) => out.extend(p.iter().cloned()),
                        ActionData::Push(v) => out.extend(v.iter().filter_map(|x| match x {
                            PushValue::String(s) => Some(s.clone()),
                            _ => None,
                        })),
                        _ => {}
                    }
                }
            };
            for (_, t) in movie.all_tags() {
                match &t.tag {
                    ssx_formats::gfx::Tag::DoAction(a) => take(a),
                    ssx_formats::gfx::Tag::DoInitAction { actions, .. } => take(actions),
                    ssx_formats::gfx::Tag::PlaceObject(p) => {
                        for r in p.clip_actions.iter().flat_map(|c| &c.records) {
                            take(&r.actions);
                        }
                    }
                    _ => {}
                }
            }
            Some(Arc::new(out))
        });
        lock(&self.m6.movie_strings).insert(id.clone(), strings.clone());
        strings
    }

    pub fn refuse_movie(&self, id: &ContentId, source: &str, why: &str) {
        lock(&self.m6.refused).insert((id.clone(), source.to_string()), why.to_string());
        lock(&self.m6.movies).retain(|(i, _), _| i != id);
        self.notice(format!(
            "content: {id} from {source} stopped: {why}; its screen plays the disc's from its next load"
        ));
    }

    pub fn package_strings(&self, lang: Lang) -> Vec<PackageString> {
        if !self.packages_provide(Kind::String) {
            return Vec::new();
        }
        let disc = self.disc_strings();
        let port = self.port_strings();
        self.with_registry(|r| {
            let ids: BTreeSet<ContentId> = r.list_above(Layer::Port, Kind::String).into_iter().collect();
            let mut out = Vec::new();
            for id in ids {
                let (base_key, base) = match id.namespace() {
                    "ssx" => match disc.as_ref().and_then(|d| d.texts.get(&id)) {
                        Some((k, t)) => (Some(k.clone()), t.get(&lang).cloned()),
                        None => continue,
                    },
                    "ava" => match port.as_ref().and_then(|p| p.texts.get(&id)) {
                        Some((k, t)) => (Some(k.clone()), Some(t.clone())),
                        None => continue,
                    },
                    _ => (None, None),
                };
                let mut chosen = None;
                for want in [lang, Lang::En] {
                    for (m, src) in r.providers(&id).filter(|(m, _)| m.layer > Layer::Port) {
                        let Ok(Asset::Value(v)) = src.load(&id) else { continue };
                        let Some(texts) = v.downcast_ref::<BTreeMap<Lang, String>>() else { continue };
                        let Some(text) = texts.get(&want) else { continue };
                        let check =
                            strings::check_text(text).and_then(|()| strings::check_against(text, base.as_deref()));
                        match check {
                            Ok(()) => {
                                chosen = Some((text.clone(), m.name.to_string()));
                                break;
                            }
                            Err(why) => {
                                let why = format!("its {} text: {why}", want.code());
                                self.passed_over(&id, m.name, m.layer, &why, Some("the next source"));
                            }
                        }
                    }
                    if chosen.is_some() {
                        break;
                    }
                }
                if let Some((text, source)) = chosen {
                    let key = base_key.clone().unwrap_or_else(|| id.as_str().to_string());
                    out.push(PackageString { key, text, source });
                }
            }
            out
        })
    }

    pub fn package_fonts(&self) -> Vec<PackageFont> {
        if !self.packages_provide(Kind::Font) {
            return Vec::new();
        }
        let disc = self.disc_fonts();
        self.with_registry(|r| {
            let order: BTreeMap<(Layer, String), usize> =
                r.mounts().enumerate().map(|(i, m)| ((m.layer, m.name.to_string()), i)).collect();
            let mut out: Vec<(std::cmp::Reverse<usize>, PackageFont)> = Vec::new();
            for id in r.list_above(Layer::Port, Kind::Font) {
                let Some((m, src)) = r.providers(&id).find(|(m, _)| m.layer > Layer::Port) else { continue };
                let at = order.get(&(m.layer, m.name.to_string())).copied().unwrap_or(0);
                match src.load(&id).map(|a| a.into_value::<Converted>()) {
                    Ok(Some(font)) => {
                        let replaces = disc.as_ref().and_then(|d| d.fonts.get(&id).cloned());
                        let f = PackageFont { id: id.clone(), source: m.name.to_string(), font, replaces };
                        out.push((std::cmp::Reverse(at), f));
                    }
                    Ok(None) => {}
                    Err(e) => self.passed_over(&id, m.name, m.layer, &e.to_string(), None),
                }
            }
            out.sort_by(|a, b| (a.0, &a.1.id).cmp(&(b.0, &b.1.id)));
            out.into_iter().map(|(_, f)| f).collect()
        })
    }
}

fn index<T>(content: &GameContent, what: &str, r: anyhow::Result<T>) -> Option<Arc<T>> {
    match r {
        Ok(t) => Some(Arc::new(t)),
        Err(e) => {
            content.notice(format!("content: the disc's {what}: {e:#}"));
            None
        }
    }
}
