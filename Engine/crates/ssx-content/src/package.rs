use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::id::{ContentId, Kind};
use crate::limits::{MAX_FILE_BYTES, MAX_MANIFEST_BYTES, MAX_RECORD_BYTES, MAX_TOML_DEPTH, table_depth};
use crate::manifest::{Diagnostic, Entry, MANIFEST_FILE, Manifest, ManifestError, line_col};
use crate::path::PackageTree;
use crate::registry::{Asset, LoadError, Source};
use crate::texture::{self, FolderSettings, settings::SETTINGS_FILE};
use crate::{material, model, object, shader, sky};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Client,
    Server,
}

pub const SHADER_EXTENSIONS: [&str; 9] = ["wgsl", "wesl", "spv", "glsl", "vert", "frag", "hlsl", "metal", "fx"];

pub const MOVIE_EXTENSIONS: [&str; 2] = ["swf", "gfx"];

pub fn server_refusal(manifest: &Manifest, tree: &PackageTree) -> Option<String> {
    if manifest.gpu_code() {
        return Some(
            "it declares GPU code ([shaders]); shader code comes only from client mods the player installs".into(),
        );
    }
    let shader_file = tree.entries().find(|(p, k)| {
        *k == crate::path::EntryKind::File
            && p.rsplit_once('.').is_some_and(|(_, e)| SHADER_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    });
    if let Some((p, _)) = shader_file {
        return Some(format!("{p}: GPU code; shader code comes only from client mods the player installs"));
    }
    if manifest.actionscript() {
        return Some(
            "it declares movies with ActionScript ([actionscript]); a server's movies are not run until \
                     resource delivery decides them (multiplayer phase 6)"
                .into(),
        );
    }
    let movie = tree.entries().find(|(p, k)| {
        *k == crate::path::EntryKind::File
            && p.rsplit_once('.').is_some_and(|(_, e)| MOVIE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    });
    movie.map(|(p, _)| format!("{p}: a movie; a server's movies are not run until resource delivery decides them"))
}

pub struct PackageSource {
    pub root: PathBuf,
    pub manifest: Manifest,
    tree: PackageTree,
    texture_limits: texture::Limits,
    model_limits: model::Limits,
    texture_settings: Mutex<BTreeMap<String, Result<Arc<FolderSettings>, String>>>,
    models: Mutex<BTreeMap<String, Arc<ModelOnce>>>,
    costs: Mutex<BTreeMap<String, Result<model::ModelCost, String>>>,
    sky_limits: sky::Limits,
    shader_patterns: Vec<crate::glob::Pattern>,
    skinned: Mutex<BTreeMap<String, Arc<ModelOnce>>>,
    skinned_costs: Mutex<BTreeMap<String, Result<model::ModelCost, String>>>,
    music_defs: Mutex<BTreeMap<(String, audio::MusicClass), Arc<audio::MusicOnce>>>,
    audio_heads: Mutex<BTreeMap<(String, Kind), Result<crate::audio::AudioInfo, String>>>,
    actionscript_patterns: Vec<crate::glob::Pattern>,
    fonts: Mutex<BTreeMap<String, Arc<m6::FontOnce>>>,
    strings: OnceLock<Arc<m6::PackageStrings>>,
}

type ModelOnce = OnceLock<Result<Arc<model::Model>, String>>;

pub fn parse_record(bytes: &[u8], rel: &str) -> Result<toml::Table, String> {
    parse_record_at(bytes).map_err(|(line, col, m)| match line {
        0 => format!("{rel}: {m}"),
        _ => format!("{rel}:{line}:{col}: {m}"),
    })
}

pub fn parse_record_at(bytes: &[u8]) -> Result<toml::Table, (usize, usize, String)> {
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err((0, 0, format!("over {MAX_RECORD_BYTES} bytes")));
    }
    let text = std::str::from_utf8(bytes).map_err(|e| (0, 0, format!("not UTF-8: {e}")))?;
    let table: toml::Table = toml::from_str(text).map_err(|e| {
        let (line, col) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        (line, col, ssx_core::text::message(e.message()).into_string())
    })?;
    if table_depth(&table) > MAX_TOML_DEPTH {
        return Err((0, 0, format!("nested deeper than {MAX_TOML_DEPTH}")));
    }
    Ok(table)
}

impl PackageSource {
    pub fn open(root: &Path) -> Result<Self, ManifestError> {
        Self::open_as(root, Origin::Client)
    }

    pub fn open_as(root: &Path, origin: Origin) -> Result<Self, ManifestError> {
        let file = root.join(MANIFEST_FILE);
        let fail = |e: crate::path::ReadError| ManifestError {
            file: file.clone(),
            diagnostics: vec![Diagnostic { line: 0, column: 0, message: e.to_string().into() }],
        };
        let tree = PackageTree::open(root).map_err(fail)?;
        let bytes = tree.read(MANIFEST_FILE, MAX_MANIFEST_BYTES as u64).map_err(fail)?;
        let manifest = Manifest::parse_bytes(&bytes, &file)?;
        if origin == Origin::Server
            && let Some(why) = server_refusal(&manifest, &tree)
        {
            let message = format!("a server's package with GPU code or movies is refused: {why}");
            return Err(ManifestError {
                file,
                diagnostics: vec![Diagnostic { line: 0, column: 0, message: message.into() }],
            });
        }
        let shader_patterns =
            manifest.shaders.files.iter().filter_map(|p| crate::glob::Pattern::parse(p).ok()).collect();
        let actionscript_patterns =
            manifest.actionscript.files.iter().filter_map(|p| crate::glob::Pattern::parse(p).ok()).collect();
        Ok(PackageSource {
            root: root.to_path_buf(),
            manifest,
            tree,
            texture_limits: texture::Limits::default(),
            model_limits: model::Limits::default(),
            texture_settings: Mutex::new(BTreeMap::new()),
            models: Mutex::new(BTreeMap::new()),
            costs: Mutex::new(BTreeMap::new()),
            sky_limits: sky::Limits::default(),
            shader_patterns,
            skinned: Mutex::new(BTreeMap::new()),
            skinned_costs: Mutex::new(BTreeMap::new()),
            music_defs: Mutex::new(BTreeMap::new()),
            audio_heads: Mutex::new(BTreeMap::new()),
            actionscript_patterns,
            fonts: Mutex::new(BTreeMap::new()),
            strings: OnceLock::new(),
        })
    }

    pub fn tree(&self) -> &PackageTree {
        &self.tree
    }

    pub fn with_texture_limits(mut self, limits: texture::Limits) -> Self {
        self.texture_limits = limits;
        self
    }

    pub fn with_model_limits(mut self, limits: model::Limits) -> Self {
        self.model_limits = limits;
        self
    }

    fn load_model(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Entry::File(rel) = entry else {
            return Err(failed("a model is one file, not files by role".into()));
        };
        let cell = {
            let mut models = self.models.lock().map_err(|_| failed("a model reader panicked".into()))?;
            models.entry(rel.clone()).or_default().clone()
        };
        let decoded = cell.get_or_init(|| {
            let bytes = self.tree.read(rel, self.model_limits.max_file_bytes).map_err(|e| e.to_string())?;
            let m = model::decode_file(rel, &bytes, &self.tree, &self.model_limits).map_err(|e| e.to_string())?;
            Ok(Arc::new(m))
        });
        match decoded {
            Ok(m) => Ok(Asset::Value(m.clone())),
            Err(e) => Err(failed(e.clone())),
        }
    }

    fn load_record<T: Send + Sync + 'static>(
        &self,
        id: &ContentId,
        entry: &Entry,
        parse: fn(&[u8], &str) -> Result<T, Vec<material::Problem>>,
    ) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let rel = match entry {
            Entry::File(rel) if is_toml(rel) => rel,
            _ => return Err(failed(format!("a {} is one .toml file", id.kind().as_str()))),
        };
        let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
        let value = parse(&bytes, rel).map_err(|problems| {
            let lines: Vec<String> = problems
                .into_iter()
                .map(|(l, c, m)| if l == 0 { format!("{rel}: {m}") } else { format!("{rel}:{l}:{c}: {m}") })
                .collect();
            failed(lines.join("; "))
        })?;
        Ok(Asset::Value(Arc::new(value)))
    }

    fn load_material(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let asset = self.load_record(id, entry, material::parse_toml)?;
        let Some(def) = asset.value::<material::MaterialDef>() else { return Ok(asset) };
        let Some(hook) = &def.shader else { return Ok(asset) };
        let mut def = def.clone();
        let declared = self.declares_shader(&hook.file);
        let mut hook = hook.clone();
        match declared {
            Ok(()) => match self.tree.read(&hook.file, shader::Limits::default().max_bytes as u64) {
                Ok(bytes) => match std::str::from_utf8(&bytes) {
                    Ok(text) => {
                        hook.hash = Some(crate::hash::ContentHash::of(&bytes));
                        hook.source = Some(Arc::from(text));
                    }
                    Err(e) => hook.problem = Some(format!("{}: not UTF-8: {e}", hook.file)),
                },
                Err(e) => hook.problem = Some(e.to_string()),
            },
            Err(why) => hook.problem = Some(why),
        }
        def.shader = Some(hook);
        Ok(Asset::Value(Arc::new(def)))
    }

    pub fn declares_shader(&self, file: &str) -> Result<(), String> {
        let mut budget = crate::glob::Budget(1_000_000);
        let hit = self.shader_patterns.iter().any(|p| p.matches(file, &mut budget).unwrap_or(false));
        if hit { Ok(()) } else { Err(format!("{file}: GPU code the manifest does not declare in [shaders] files")) }
    }

    pub fn sky_def(&self, id: &ContentId) -> Result<sky::SkyDef, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let entry = self.manifest.entry(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        let Entry::File(rel) = entry else { return Err(failed("a sky is one file, not files by role".into())) };
        let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        let source = match ext.as_str() {
            "toml" => {
                let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
                return sky::parse_toml(&bytes, rel).map_err(|ps| failed(problems_text(rel, ps)));
            }
            "hdr" | "exr" => sky::CubeSource::Equirect(rel.clone()),
            "ktx2" | "dds" => sky::CubeSource::Cube(rel.clone()),
            _ => return Err(failed(format!("{rel}: six faces are named by a sky.toml (`faces = [...]`)"))),
        };
        let mut def = sky::SkyDef::scattering();
        def.name = rel.clone();
        def.kind = sky::SkyKind::Cubemap { source, color: sky::ColorEncoding::Srgb, face_size: None };
        def.clouds = false;
        Ok(def)
    }

    fn load_sky(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let def = self.sky_def(id)?;
        let cube = sky::decode(&def, &self.tree, &self.sky_limits)
            .map_err(|e| LoadError::Failed { id: id.clone(), message: e.to_string() })?;
        Ok(Asset::Value(Arc::new(sky::Sky { def, cube: cube.map(Arc::new) })))
    }

    fn folder_settings(&self, dir: &str) -> Result<Option<Arc<FolderSettings>>, String> {
        let rel = if dir.is_empty() { SETTINGS_FILE.to_string() } else { format!("{dir}/{SETTINGS_FILE}") };
        if self.tree.kind(&rel).is_none() {
            return Ok(None);
        }
        let mut cache = self.texture_settings.lock().map_err(|_| "a settings reader panicked".to_string())?;
        let parsed = cache.entry(dir.to_string()).or_insert_with(|| {
            let bytes = self.tree.read(&rel, MAX_RECORD_BYTES).map_err(|e| e.to_string())?;
            FolderSettings::parse(&bytes, &rel).map(Arc::new)
        });
        parsed.clone().map(Some)
    }

    fn load_texture(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Entry::File(rel) = entry else {
            return Err(failed("a texture is one file, not files by role".into()));
        };
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        let settings = self.folder_settings(dir).map_err(failed)?.map(|s| s.of(name)).unwrap_or_default();
        let bytes = self.read(id, rel, self.texture_limits.max_file_bytes)?;
        let tex =
            texture::decode_file(rel, &bytes, &settings, &self.texture_limits).map_err(|e| failed(e.to_string()))?;
        Ok(Asset::Value(Arc::new(tex)))
    }

    fn load_map(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let rel = match entry {
            Entry::File(rel) if is_toml(rel) => rel,
            _ => return Err(failed("a map is one map.toml file".into())),
        };
        let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
        let map = crate::map::read_map(&bytes, rel, &self.tree).map_err(|e| failed(e.to_string()))?;
        Ok(Asset::Value(Arc::new(map)))
    }

    fn load_anim(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let rel = match entry {
            Entry::File(rel) => rel,
            Entry::Files(_) => return Err(failed("a clip is one anim.toml or one .glb / .gltf file".into())),
        };
        let clip = if is_toml(rel) {
            let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
            crate::anim::read_anim(&bytes, rel, &self.tree)
                .map_err(|e| failed(e.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")))?
        } else {
            let toml =
                crate::anim::AnimToml { gltf: Some(rel.clone()), animation: None, additive: false, floats: Vec::new() };
            crate::anim::build(&toml, rel, &self.tree).map_err(|e| failed(e.to_string()))?
        };
        Ok(Asset::Value(Arc::new(clip)))
    }

    fn map_toml(&self, id: &ContentId, entry: &Entry) -> Result<crate::map::MapToml, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let rel = match entry {
            Entry::File(rel) if is_toml(rel) => rel,
            _ => return Err(failed("a map is one map.toml file".into())),
        };
        let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
        crate::map::parse_toml(&bytes).map_err(|p| failed(problems_text(rel, p)))
    }

    fn load_trick(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let rel = match entry {
            Entry::File(rel) if is_toml(rel) => rel,
            _ => return Err(failed("a trick is one trick.toml".into())),
        };
        let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
        let t = crate::trick::parse_toml(&bytes).map_err(|p| failed(problems_text(rel, p)))?;
        Ok(Asset::Value(Arc::new(t)))
    }

    fn load_gamemode(&self, id: &ContentId, entry: &Entry) -> Result<Asset, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        if self.manifest.resource.kind != crate::manifest::PackageType::Gamemode {
            return Err(failed("a gamemode comes in a package of type \"gamemode\"".into()));
        }
        let rel = match entry {
            Entry::File(rel) if is_toml(rel) => rel,
            _ => return Err(failed("a gamemode is one gamemode.toml".into())),
        };
        let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
        let g = crate::gamemode::parse_toml(&bytes).map_err(|p| failed(problems_text(rel, p)))?;
        Ok(Asset::Value(Arc::new(g)))
    }

    fn read(&self, id: &ContentId, rel: &str, max: u64) -> Result<Vec<u8>, LoadError> {
        self.tree.read(rel, max).map_err(|e| LoadError::Failed { id: id.clone(), message: e.to_string() })
    }

    fn read_toml(&self, id: &ContentId, rel: &str) -> Result<toml::Table, LoadError> {
        let bytes = self.read(id, rel, MAX_RECORD_BYTES)?;
        parse_record(&bytes, rel).map_err(|message| LoadError::Failed { id: id.clone(), message })
    }
}

fn problems_text(rel: &str, problems: Vec<material::Problem>) -> String {
    let lines: Vec<String> = problems
        .into_iter()
        .map(|(l, c, m)| if l == 0 { format!("{rel}: {m}") } else { format!("{rel}:{l}:{c}: {m}") })
        .collect();
    lines.join("; ")
}

fn is_toml(rel: &str) -> bool {
    Path::new(rel).extension().is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

impl Source for PackageSource {
    fn name(&self) -> &str {
        &self.manifest.resource.name
    }

    fn has(&self, id: &ContentId) -> bool {
        if id.kind() == Kind::String {
            return self.has_string(id);
        }
        self.manifest.entry(id).is_some()
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        if id.kind() == Kind::String {
            return self.load_string(id);
        }
        let entry = self.manifest.entry(id).ok_or_else(|| LoadError::Missing(id.clone()))?;
        match id.kind() {
            Kind::Texture => return self.load_texture(id, entry),
            Kind::Model => return self.load_model(id, entry),
            Kind::Object => return self.load_record(id, entry, object::parse_toml),
            Kind::Material => return self.load_material(id, entry),
            Kind::Sky => return self.load_sky(id),
            Kind::Sound => return self.load_sound(id),
            Kind::Music => return self.load_music(id),
            Kind::Ui => return self.load_movie(id, entry),
            Kind::Font => return self.load_font(id, entry),
            Kind::Movie => return self.load_movie_file(id, entry),
            Kind::Map => return self.load_map(id, entry),
            Kind::Anim => return self.load_anim(id, entry),
            Kind::Trick => return self.load_trick(id, entry),
            Kind::Gamemode => return self.load_gamemode(id, entry),
            k if rider::is_rider_kind(k) => return self.load_rider(id),
            _ => {}
        }
        match entry {
            Entry::File(f) if is_toml(f) => Ok(Asset::Record(self.read_toml(id, f)?)),
            Entry::File(f) => Ok(Asset::Bytes(Arc::from(self.read(id, f, MAX_FILE_BYTES)?))),
            Entry::Files(files) => {
                let mut out = BTreeMap::new();
                for (role, f) in files {
                    out.insert(role.clone(), Arc::from(self.read(id, f, MAX_FILE_BYTES)?));
                }
                Ok(Asset::Files(out))
            }
        }
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::String {
            return self.strings().texts.keys().cloned().collect();
        }
        self.manifest.ids_of(kind).cloned().collect()
    }

    fn map_record(&self, id: &ContentId) -> Option<Result<crate::map::MapToml, LoadError>> {
        if id.kind() != Kind::Map {
            return None;
        }
        let entry = self.manifest.entry(id)?;
        Some(self.map_toml(id, entry))
    }

    fn file_of(&self, id: &ContentId) -> Option<String> {
        match self.manifest.entry(id)? {
            Entry::File(rel) => Some(rel.clone()),
            Entry::Files(_) => None,
        }
    }

    fn model_cost(&self, id: &ContentId) -> Option<Result<model::ModelCost, LoadError>> {
        if id.kind() != Kind::Model {
            return None;
        }
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Some(rel) = self.file_of(id) else {
            return Some(Err(failed("a model is one file, not files by role".into())));
        };
        let mut costs = match self.costs.lock() {
            Ok(c) => c,
            Err(_) => return Some(Err(failed("a model reader panicked".into()))),
        };
        let cost = costs
            .entry(rel.clone())
            .or_insert_with(|| model::probe_file(&rel, &self.tree, &self.model_limits).map_err(|e| e.to_string()));
        Some(cost.clone().map_err(failed))
    }

    fn texture_probe(&self, id: &ContentId) -> Option<Result<crate::texture::TextureProbe, LoadError>> {
        if id.kind() != Kind::Texture {
            return None;
        }
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Some(rel) = self.file_of(id) else {
            return Some(Err(failed("a texture is one file, not files by role".into())));
        };
        let size = match self.tree.file_size(&rel) {
            Ok(s) => s,
            Err(e) => return Some(Err(failed(e.to_string()))),
        };
        let mut read = |len: u64| self.tree.read_range(&rel, 0, len).map_err(|e| e.to_string());
        Some(texture::probe_file(&rel, size, &mut read, &self.texture_limits).map_err(|e| failed(e.to_string())))
    }

    fn sky_cost(&self, id: &ContentId) -> Option<Result<(sky::SkyDef, sky::SkyCost), LoadError>> {
        if id.kind() != Kind::Sky {
            return None;
        }
        let def = match self.sky_def(id) {
            Ok(d) => d,
            Err(e) => return Some(Err(e)),
        };
        let cost = sky::probe(&def, &self.tree, &self.sky_limits)
            .map_err(|e| LoadError::Failed { id: id.clone(), message: e.to_string() });
        Some(cost.map(|c| (def, c)))
    }

    fn audio_probe(&self, id: &ContentId) -> Option<crate::registry::AudioProbe> {
        matches!(id.kind(), Kind::Sound | Kind::Music).then(|| self.probe_audio(id))
    }

    fn audio_file(&self, id: &ContentId, rel: &str) -> Option<Result<Arc<[u8]>, LoadError>> {
        if !matches!(id.kind(), Kind::Sound | Kind::Music) {
            return None;
        }
        Some(self.read_audio_file(id, rel, &audio::audio_limits(id.kind())))
    }

    fn rider_entry(&self, id: &ContentId) -> Option<Result<crate::rider::RiderEntry, LoadError>> {
        rider::is_rider_kind(id.kind()).then(|| PackageSource::rider_entry(self, id))
    }

    fn rider_model_cost(&self, id: &ContentId) -> Option<Result<Option<(String, model::ModelCost)>, LoadError>> {
        rider::is_rider_kind(id.kind()).then(|| PackageSource::rider_model_cost(self, id))
    }

    fn rider_model(&self, id: &ContentId) -> Option<Result<Option<Arc<model::Model>>, LoadError>> {
        rider::is_rider_kind(id.kind()).then(|| PackageSource::rider_model(self, id))
    }

    fn movie_file(&self, id: &ContentId) -> Option<Result<(std::fs::File, u64), LoadError>> {
        (id.kind() == Kind::Movie).then(|| self.open_movie(id))
    }

    fn patches(&self) -> Vec<ContentId> {
        self.manifest.patches.keys().cloned().collect()
    }

    fn patch(&self, id: &ContentId) -> Result<Option<toml::Table>, LoadError> {
        match self.manifest.patches.get(id) {
            Some(f) => self.read_toml(id, f).map(Some),
            None => Ok(None),
        }
    }
}

mod audio;
mod m6;
mod movie;
mod rider;
pub use audio::audio_limits;
pub use m6::PackageStrings;
pub use movie::PackageMovieHead;
pub use rider::{ROLES as RIDER_ROLES, is_rider_kind};
#[cfg(test)]
mod tests;
