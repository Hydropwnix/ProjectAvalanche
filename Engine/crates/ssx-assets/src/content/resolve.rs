use std::sync::Arc;

use ssx_content::material::{Family, MaterialDef};
use ssx_content::model::Model;
use ssx_content::object::ObjectDef;
use ssx_content::texture::{self, BlockFormat, Sampling, TextureKind, TextureSettings, Want};
use ssx_content::{Asset, ContentId, Kind, Layer, LoadError, Source};

use super::objects::material_def;
use super::store::GameContent;
use crate::track_objects::CLASS_GAMEOBJECT;
use crate::vault_db::{CLASS_MATERIAL, Material as VaultMaterial};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelUse {
    Static,
    Rider,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialUse {
    Objects,
    Riders,
}

#[derive(Clone, Debug)]
pub struct Found<T> {
    pub id: ContentId,
    pub value: Arc<T>,
    pub source: String,
    pub layer: Layer,
}

fn model_misfit(use_: ModelUse) -> Option<&'static str> {
    match use_ {
        ModelUse::Static => None,
        ModelUse::Rider => Some(
            "a rider's part is not replaced by a model id: a character's, gear's or board's entry gives its look \
             (skinned to the riders' skeleton, content.md §17)",
        ),
    }
}

fn material_misfit(def: &MaterialDef, use_: MaterialUse) -> Option<String> {
    match (use_, def.family) {
        (MaterialUse::Objects, Some(Family::Object | Family::Foreground)) => None,
        (MaterialUse::Objects, _) => Some(format!("the effect {:?} is not one the track objects draw", def.effect)),
        (MaterialUse::Riders, Some(Family::Character)) => None,
        (MaterialUse::Riders, _) => {
            Some(format!("the effect {:?} is not one of the riders' (the character family)", def.effect))
        }
    }
}

struct Loaded {
    source: String,
    layer: Layer,
    next: Option<String>,
    asset: Result<Asset, LoadError>,
}

impl GameContent {
    pub(super) fn passed_over(&self, id: &ContentId, source: &str, layer: Layer, why: &str, next: Option<&str>) {
        let mut seen = self.passed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if seen.insert((id.clone(), source.to_string())) {
            let next = next.map_or(String::new(), |n| format!("; {n} is used instead"));
            self.notice(format!("content: {id} from {source} ({layer}) not used: {why}{next}"));
        }
    }

    fn provider(
        &self,
        id: &ContentId,
        i: usize,
        skip: impl Fn(Layer) -> Option<String>,
    ) -> Option<(Loaded, Option<String>)> {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut providers = registry.providers(id).skip(i);
        let (info, source) = providers.next()?;
        let next = providers.next().map(|(n, _)| n.name.to_string());
        let skipped = skip(info.layer);
        let asset = match &skipped {
            Some(_) => Err(LoadError::Missing(id.clone())),
            None => self.cached_load(id, info.name, info.layer, source),
        };
        Some((Loaded { source: info.name.to_string(), layer: info.layer, next, asset }, skipped))
    }

    pub fn model(&self, id: &ContentId, use_: ModelUse) -> Result<Found<Model>, LoadError> {
        let mut last = LoadError::Missing(id.clone());
        let skip = |layer: Layer| (layer != Layer::Base).then(|| model_misfit(use_)).flatten().map(str::to_string);
        for i in 0.. {
            let Some((p, skipped)) = self.provider(id, i, skip) else { break };
            if let Some(why) = skipped {
                self.passed_over(id, &p.source, p.layer, &why, p.next.as_deref());
                continue;
            }
            match p.asset.map(|a| a.into_value::<Model>()) {
                Ok(Some(value)) => return Ok(Found { id: id.clone(), value, source: p.source, layer: p.layer }),
                Ok(None) => self.passed_over(id, &p.source, p.layer, "not a decoded model", p.next.as_deref()),
                Err(e) => {
                    self.passed_over(id, &p.source, p.layer, &e.to_string(), p.next.as_deref());
                    last = e;
                }
            }
        }
        Err(last)
    }

    fn strongest_above_base<T: Send + Sync + 'static>(
        &self,
        id: &ContentId,
        fits: impl Fn(&T) -> Option<String>,
    ) -> Option<Found<T>> {
        for i in 0.. {
            let (p, _) = self.provider(id, i, |l| (l == Layer::Base).then(|| "the base".to_string()))?;
            if p.layer == Layer::Base {
                return None;
            }
            let value = p
                .asset
                .map_err(|e| e.to_string())
                .and_then(|a| a.into_value::<T>().ok_or_else(|| format!("not a decoded {}", id.kind().as_str())));
            match value.and_then(|v| fits(&v).map_or(Ok(v), Err)) {
                Ok(value) => return Some(Found { id: id.clone(), value, source: p.source, layer: p.layer }),
                Err(why) => self.passed_over(id, &p.source, p.layer, &why, p.next.as_deref()),
            }
        }
        None
    }

    fn record_override<T: Send + Sync + 'static>(
        &self,
        kind: Kind,
        class: u32,
        key: u32,
        fits: impl Fn(&T) -> Option<String>,
    ) -> Option<Found<T>> {
        let records = self.records_for(kind)?;
        let names = match records.names() {
            Ok(n) => n,
            Err(e) => {
                self.notice(format!("content: the disc's {} names could not be worked out: {e}", kind.as_str()));
                return None;
            }
        };
        let id = names.by_key.get(&(class, key))?;
        self.strongest_above_base(id, fits)
    }

    pub fn material_for(&self, m: &VaultMaterial, use_: MaterialUse) -> MaterialDef {
        let fits = |d: &MaterialDef| material_misfit(d, use_);
        let found = (m.key != 0).then(|| self.record_override(Kind::Material, CLASS_MATERIAL, m.key, fits)).flatten();
        found.map_or_else(|| material_def(m), |f| (*f.value).clone())
    }

    pub fn material(&self, id: &ContentId, use_: MaterialUse) -> Result<Found<MaterialDef>, LoadError> {
        let mut last = LoadError::Missing(id.clone());
        for i in 0.. {
            let Some((p, _)) = self.provider(id, i, |_| None) else { break };
            let value = p.asset.and_then(|a| {
                a.into_value::<MaterialDef>()
                    .ok_or_else(|| LoadError::Failed { id: id.clone(), message: "not a decoded material".into() })
            });
            match value {
                Ok(v) => match material_misfit(&v, use_) {
                    None => return Ok(Found { id: id.clone(), value: v, source: p.source, layer: p.layer }),
                    Some(why) => {
                        self.passed_over(id, &p.source, p.layer, &why, p.next.as_deref());
                        last = LoadError::Failed { id: id.clone(), message: why };
                    }
                },
                Err(e) => {
                    self.passed_over(id, &p.source, p.layer, &e.to_string(), p.next.as_deref());
                    last = e;
                }
            }
        }
        Err(last)
    }

    pub fn package_providers(&self, id: &ContentId) -> Vec<(String, Layer, i32)> {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        registry
            .providers(id)
            .filter(|(m, _)| m.layer != Layer::Base)
            .map(|(m, _)| (m.name.to_string(), m.layer, m.priority))
            .collect()
    }

    pub(super) fn with_source<R>(
        &self,
        id: &ContentId,
        source: &str,
        layer: Layer,
        f: impl FnOnce(&dyn Source) -> R,
    ) -> Option<R> {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        registry.providers(id).find(|(m, _)| m.name == source && m.layer == layer).map(|(_, s)| f(s))
    }

    pub fn model_from(&self, id: &ContentId, source: &str, layer: Layer) -> Result<Found<Model>, LoadError> {
        let asset = self
            .with_source(id, source, layer, |s| self.cached_load(id, source, layer, s))
            .unwrap_or_else(|| Err(LoadError::Missing(id.clone())))?;
        let value = asset
            .into_value::<Model>()
            .ok_or_else(|| LoadError::Failed { id: id.clone(), message: format!("{source}: not a decoded model") })?;
        Ok(Found { id: id.clone(), value, source: source.to_string(), layer })
    }

    pub fn base_model(&self, id: &ContentId) -> Result<Found<Model>, LoadError> {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (m, source) = registry
            .providers(id)
            .find(|(m, _)| m.layer == Layer::Base)
            .ok_or_else(|| LoadError::Missing(id.clone()))?;
        let name = m.name.to_string();
        let value = source
            .load(id)?
            .into_value::<Model>()
            .ok_or_else(|| LoadError::Failed { id: id.clone(), message: "the disc's asset is not a model".into() })?;
        Ok(Found { id: id.clone(), value, source: name, layer: Layer::Base })
    }

    pub fn model_cost_from(
        &self,
        id: &ContentId,
        source: &str,
        layer: Layer,
    ) -> Result<(ssx_content::model::ModelCost, String), LoadError> {
        let missing = || LoadError::Failed { id: id.clone(), message: format!("{source}: its cost cannot be told") };
        self.with_source(id, source, layer, |s| {
            let file = s.file_of(id).ok_or_else(missing)?;
            let cost = s.model_cost(id).ok_or_else(missing)??;
            Ok((cost, file))
        })
        .unwrap_or_else(|| Err(LoadError::Missing(id.clone())))
    }

    pub fn texture_probe_from(
        &self,
        id: &ContentId,
        source: &str,
        layer: Layer,
    ) -> Result<(ssx_content::texture::TextureProbe, String), LoadError> {
        let missing = || LoadError::Failed { id: id.clone(), message: format!("{source}: its size cannot be told") };
        self.with_source(id, source, layer, |s| {
            let file = s.file_of(id).ok_or_else(missing)?;
            let probe = s.texture_probe(id).ok_or_else(missing)??;
            Ok((probe, file))
        })
        .unwrap_or_else(|| Err(LoadError::Missing(id.clone())))
    }

    pub fn object_candidates(&self, key: u32) -> Option<(ContentId, Vec<Found<ObjectDef>>)> {
        let records = self.records_for(Kind::Object)?;
        let names = match records.names() {
            Ok(n) => n,
            Err(e) => {
                self.notice(format!("content: the disc's object names could not be worked out: {e}"));
                return None;
            }
        };
        let id = names.by_key.get(&(CLASS_GAMEOBJECT, key))?.clone();
        let mut found = Vec::new();
        for (source, layer, _) in self.package_providers(&id) {
            let value = self
                .with_source(&id, &source, layer, |s| s.load(&id))
                .unwrap_or_else(|| Err(LoadError::Missing(id.clone())))
                .map_err(|e| e.to_string())
                .and_then(|a| a.into_value::<ObjectDef>().ok_or_else(|| "not a decoded game object".to_string()));
            match value {
                Ok(value) => found.push(Found { id: id.clone(), value, source, layer }),
                Err(why) => self.passed_over(&id, &source, layer, &why, None),
            }
        }
        (!found.is_empty()).then_some((id, found))
    }

    pub fn model_image_for_gpu(
        &self,
        model: &ContentId,
        image: usize,
        kind: TextureKind,
        want: &Want,
        device_bc: bool,
    ) -> Result<texture::Found, LoadError> {
        let failed = |message: String| LoadError::Failed { id: model.clone(), message };
        let m = if super::characters::RIDER_KINDS.contains(&model.kind()) {
            let (value, at) = self.rider_look_model(model)?;
            Found { id: model.clone(), value, source: at.source, layer: at.layer }
        } else if model.kind() == Kind::Map {
            let map = self.map(model)?;
            let props = map.value.props.as_ref().ok_or_else(|| failed("its props have no own mesh".into()))?;
            Found { id: model.clone(), value: props.model.clone(), source: map.source, layer: map.layer }
        } else {
            self.model(model, ModelUse::Static)?
        };
        let img = m.value.images.get(image).ok_or_else(|| failed(format!("no image {image}")))?;
        let settings = TextureSettings { kind: Some(kind) };
        let mut tex = texture::decode_file(&img.file_name, &img.bytes, &settings, &texture::Limits::default())
            .map_err(|e| failed(e.to_string()))?;
        let kind = texture::fit(tex.shape, Some(kind), want).map_err(|e| failed(format!("{}: {e}", img.file_name)))?;
        if device_bc {
            let target = match want.sampling {
                Sampling::NormalXy => BlockFormat::Bc5,
                Sampling::Texels if tex.has_alpha() => BlockFormat::Bc3,
                Sampling::Texels => BlockFormat::Bc1,
            };
            match self.blocks(&tex, target, kind) {
                Ok(chain) => tex.blocks = Some(chain),
                Err(e) => {
                    self.notice(format!("content: {model} image {image}: not block-encoded ({e}); uploaded as RGBA8"));
                    tex.blocks = None;
                }
            }
        }
        Ok(texture::Found { texture: tex, kind, layer: m.layer, source: m.source })
    }
}
