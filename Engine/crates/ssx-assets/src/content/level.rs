use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, OnceLock};

use ssx_content::material::{Family, MaterialDef, TextureRef};
use ssx_content::model::Model;
use ssx_content::sky::{Sky, SkyDef, SkyKind, scattering_id};
use ssx_content::texture::{Found as TextureFound, Want};
use ssx_content::{Asset, ContentId, Kind, Layer, LoadError, PORT_NAMESPACE, Source};

use super::presets::preset_texture_id;
use super::resolve::{Found, ModelUse};
use super::store::GameContent;
use super::track_packages::{
    MAX_HELD_BYTES, MAX_IMAGE_BYTES, MAX_JOINTS, MAX_TRIANGLES, MAX_VERTICES, Outcome, TextureChoice, Usage,
};
use crate::track_lighting::LightingPreset;

pub struct PortSkies;

impl Source for PortSkies {
    fn name(&self) -> &str {
        PORT_NAMESPACE
    }

    fn has(&self, id: &ContentId) -> bool {
        *id == scattering_id()
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        if *id != scattering_id() {
            return Err(LoadError::Missing(id.clone()));
        }
        Ok(Asset::Value(Arc::new(Sky { def: SkyDef::scattering(), cube: None })))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Sky { vec![scattering_id()] } else { Vec::new() }
    }
}

#[derive(Clone)]
pub enum LevelSky {
    Scattering,
    Cubemap { id: ContentId, source: String, sky: Arc<Sky> },
    Dome { id: ContentId, source: String, sky: Arc<Sky>, model: Found<Model>, material: Arc<MaterialDef> },
}

impl LevelSky {
    pub fn def(&self) -> Option<&SkyDef> {
        match self {
            LevelSky::Scattering => None,
            LevelSky::Cubemap { sky, .. } | LevelSky::Dome { sky, .. } => Some(&sky.def),
        }
    }
}

pub struct LevelPackages {
    pub track: String,
    pub preset: String,
    pub generation: u64,
    pub sky: LevelSky,
    pub textures: BTreeMap<ContentId, TextureChoice>,
    pub outcomes: Vec<Outcome>,
    pub used: Usage,
}

pub(super) fn budget() -> Usage {
    Usage {
        collision_triangles: 0,
        vertices: MAX_VERTICES,
        triangles: MAX_TRIANGLES,
        held_bytes: MAX_HELD_BYTES,
        image_bytes: MAX_IMAGE_BYTES,
        joints: MAX_JOINTS,
    }
}

type LevelKey = (String, String, u64);

type LevelSlot = Arc<OnceLock<Arc<LevelPackages>>>;

#[derive(Default)]
pub struct LevelCache(Mutex<BTreeMap<LevelKey, LevelSlot>>);

impl std::fmt::Debug for LevelPackages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.summary())
    }
}

impl LevelPackages {
    pub fn summary(&self) -> String {
        let sky = match &self.sky {
            LevelSky::Scattering => "the game's scattering sky".to_string(),
            LevelSky::Cubemap { id, source, sky } => {
                let face = sky.cube.as_ref().map_or(0, |c| c.size());
                format!("{id} from {source} (a cubemap, faces {face}²)")
            }
            LevelSky::Dome { id, source, .. } => format!("{id} from {source} (a dome)"),
        };
        let refused = self.outcomes.iter().filter(|o| o.refused.is_some()).count();
        format!(
            "{} / {}: sky {sky}; package textures {} taken, {refused} not; {} bytes of images, {} vertices",
            self.track,
            self.preset,
            self.textures.values().filter(|c| matches!(c, TextureChoice::Package { .. })).count(),
            self.used.image_bytes,
            self.used.vertices
        )
    }

    pub fn texture(&self, content: &GameContent, id: &ContentId, want: &Want) -> Result<TextureFound, LoadError> {
        match self.textures.get(id) {
            Some(TextureChoice::Package { source, layer, canonical }) => {
                content.texture_for_gpu_from(canonical, source, *layer, want, false)
            }
            Some(TextureChoice::Disc) => content.base_texture(id, want),
            Some(TextureChoice::Unavailable(why)) => Err(LoadError::Failed { id: id.clone(), message: why.clone() }),
            None => content.texture(id, want),
        }
    }
}

struct Decider<'a> {
    content: &'a GameContent,
    d: LevelPackages,
    files: BTreeMap<(String, String), ContentId>,
}

impl Decider<'_> {
    fn over(&self, add: &Usage) -> Option<String> {
        self.d
            .used
            .plus(add)
            .over(&budget())
            .map(|(what, amount, cap)| format!("the packages' {what} on this level would be {amount}; at most {cap}"))
    }

    fn report(&mut self, o: Outcome, instead: &str) {
        if let Some(why) = &o.refused {
            self.content.notice(format!(
                "content: on {} ({}), {} from {} is not used ({instead} is): {why}",
                self.d.track, self.d.preset, o.id, o.source
            ));
        }
        self.d.outcomes.push(o);
    }

    fn texture(&mut self, id: &ContentId) {
        if self.d.textures.contains_key(id) {
            return;
        }
        let providers = self.content.package_providers(id);
        let Some((source, layer, _)) = providers.first().cloned() else { return };
        let on_disc = self.content.has_base(id);
        let instead = if on_disc { "the disc's texture" } else { "no texture" };
        let fallback = |why: String| if on_disc { TextureChoice::Disc } else { TextureChoice::Unavailable(why) };
        let (probe, file) = match self.content.texture_probe_from(id, &source, layer) {
            Ok(p) => p,
            Err(e) => {
                let why = e.to_string();
                self.d.textures.insert(id.clone(), fallback(why.clone()));
                let o = Outcome { id: id.clone(), source, placements: 0, cost: Usage::default(), refused: Some(why) };
                return self.report(o, instead);
            }
        };
        let key = (source.clone(), file);
        let shared = self.files.get(&key).cloned();
        let add = Usage { image_bytes: if shared.is_some() { 0 } else { probe.decoded_bytes() }, ..Usage::default() };
        let mut o = Outcome { id: id.clone(), source: source.clone(), placements: 0, cost: add, refused: None };
        let choice = match self.over(&add) {
            Some(why) => {
                o.refused = Some(why.clone());
                fallback(why)
            }
            None => {
                self.d.used = self.d.used.plus(&add);
                let canonical = shared.unwrap_or_else(|| {
                    self.files.insert(key, id.clone());
                    id.clone()
                });
                TextureChoice::Package { source, layer, canonical }
            }
        };
        self.d.textures.insert(id.clone(), choice);
        self.report(o, instead);
    }

    fn sky(&mut self, named: Option<&ContentId>) {
        let id = named.cloned().unwrap_or_else(scattering_id);
        let providers = self.content.package_providers(&id);
        let Some((source, layer, _)) = providers.into_iter().find(|(_, l, _)| *l > Layer::Port) else {
            if named.is_some_and(|n| *n != scattering_id()) {
                self.content.notice(format!(
                    "content: on {} ({}), the preset's sky {id} is provided by no package; the scattering sky is drawn",
                    self.d.track, self.d.preset
                ));
            }
            return;
        };
        let refuse = |me: &mut Self, why: String, cost: Usage| {
            let o = Outcome { id: id.clone(), source: source.clone(), placements: 0, cost, refused: Some(why) };
            me.report(o, "the scattering sky");
        };
        let (def, cost) = match self.content.with_source(&id, &source, layer, |s| s.sky_cost(&id)).flatten() {
            Some(Ok(c)) => c,
            Some(Err(e)) => return refuse(self, e.to_string(), Usage::default()),
            None => return refuse(self, "its size cannot be told".into(), Usage::default()),
        };
        let reflection = match &def.kind {
            SkyKind::Cubemap { .. } if def.reflection => {
                cost.decoded_bytes.min(ssx_content::sky::cube_bytes(cost.face) / 2)
            }
            _ => 0,
        };
        let mut add = Usage { image_bytes: cost.decoded_bytes + reflection, ..Usage::default() };
        let mut dome = None;
        if let SkyKind::Dome { model, material } = &def.kind {
            match self.dome(model, material) {
                Ok((m, mat, m_add)) => {
                    add = add.plus(&m_add);
                    dome = Some((m, mat));
                }
                Err(why) => return refuse(self, why, add),
            }
        }
        if let Some(why) = self.over(&add) {
            return refuse(self, why, add);
        }
        let sky = match self.content.with_source(&id, &source, layer, |s| s.load(&id)) {
            Some(Ok(a)) => match a.into_value::<Sky>() {
                Some(s) => s,
                None => return refuse(self, "not a sky".into(), add),
            },
            Some(Err(e)) => return refuse(self, e.to_string(), add),
            None => return refuse(self, "its package is not mounted".into(), add),
        };
        self.d.used = self.d.used.plus(&add);
        let o = Outcome { id: id.clone(), source: source.clone(), placements: 0, cost: add, refused: None };
        self.d.outcomes.push(o);
        self.d.sky = match dome {
            Some((model, material)) => {
                if let Some(TextureRef::Id(t)) = material.textures.get("skyTexture") {
                    self.texture(t);
                }
                LevelSky::Dome { id, source, sky, model, material }
            }
            None => LevelSky::Cubemap { id, source, sky },
        };
    }

    fn dome(
        &mut self,
        model: &ContentId,
        material: &ContentId,
    ) -> Result<(Found<Model>, Arc<MaterialDef>, Usage), String> {
        let mat = self.content.material_by_id(material).map_err(|e| format!("its material {material}: {e}"))?;
        if mat.family != Some(Family::Sky) {
            return Err(format!("its material {material} is not of the sky family"));
        }
        let mut add = Usage::default();
        if let Some((source, layer, _)) = self.content.package_providers(model).first().cloned() {
            let (cost, _) =
                self.content.model_cost_from(model, &source, layer).map_err(|e| format!("its model: {e}"))?;
            add = Usage {
                vertices: cost.vertices,
                triangles: cost.triangles,
                held_bytes: cost.held_bytes,
                image_bytes: cost.drawn_image_bytes,
                collision_triangles: 0,
                joints: 0,
            };
            if let Some(why) = self.over(&add) {
                return Err(format!("its model: {why}"));
            }
            let m = self.content.model_from(model, &source, layer).map_err(|e| format!("its model: {e}"))?;
            return Ok((m, mat, add));
        }
        let m = self.content.model(model, ModelUse::Static).map_err(|e| format!("its model: {e}"))?;
        Ok((m, mat, add))
    }
}

impl GameContent {
    pub fn material_by_id(&self, id: &ContentId) -> Result<Arc<MaterialDef>, LoadError> {
        let got = self.with_registry(|r| r.load(id))?;
        got.asset
            .into_value::<MaterialDef>()
            .ok_or_else(|| LoadError::Failed { id: id.clone(), message: "not a material".into() })
    }

    pub fn level_packages(&self, track: &str, preset: &LightingPreset) -> Arc<LevelPackages> {
        let generation = self.generation();
        let cell = {
            let mut levels = self.levels.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            levels.retain(|(_, _, g), _| *g == generation);
            levels.entry((track.to_string(), preset.name.clone(), generation)).or_default().clone()
        };
        cell.get_or_init(|| Arc::new(self.decide_level(track, preset, generation))).clone()
    }

    fn decide_level(&self, track: &str, preset: &LightingPreset, generation: u64) -> LevelPackages {
        let d = LevelPackages {
            track: track.to_string(),
            preset: preset.name.clone(),
            generation,
            sky: LevelSky::Scattering,
            textures: BTreeMap::new(),
            outcomes: Vec::new(),
            used: Usage::default(),
        };
        let mut x = Decider { content: self, d, files: BTreeMap::new() };
        if !self.with_registry(|r| r.mounts().any(|m| m.layer > Layer::Port)) {
            return x.d;
        }
        x.sky(preset.sky.as_ref());
        let preset_textures: BTreeSet<ContentId> =
            preset.textures.values().filter_map(|t| preset_texture_id(t)).collect();
        for id in &preset_textures {
            x.texture(id);
        }
        for id in self.package_particle_textures() {
            x.texture(&id);
        }
        if !x.d.outcomes.is_empty() {
            self.info(format!("content: {}", x.d.summary()));
        }
        x.d
    }

    fn package_particle_textures(&self) -> BTreeSet<ContentId> {
        let ids: BTreeSet<ContentId> = self.with_registry(|r| {
            let mut ids: BTreeSet<ContentId> = r.list_above(Layer::Base, Kind::Particles).into_iter().collect();
            ids.extend(r.patched_above(Layer::Base).into_iter().filter(|i| i.kind() == Kind::Particles));
            ids
        });
        let mut out = BTreeSet::new();
        for id in ids.iter().filter(|i| !ssx_content::particles::is_spray(i)) {
            let Ok(got) = self.with_registry(|r| r.load(id)) else { continue };
            let Some(t) = got.asset.record() else { continue };
            let Ok(sys) = ssx_content::particles::system_from_table(t) else { continue };
            for e in &sys.effects {
                for a in e.groups.iter().flat_map(|g| &g.actions) {
                    for p in a.params.iter().chain(a.states.iter().flat_map(|s| &s.params)) {
                        if p.name == ssx_content::particles::TEXTURE_PARAMETER
                            && let ssx_formats::lynx::Value::String(s) = &p.value
                            && let Some(tid) = crate::particles::texture_id(s)
                        {
                            out.insert(tid);
                        }
                    }
                }
            }
        }
        out
    }
}
