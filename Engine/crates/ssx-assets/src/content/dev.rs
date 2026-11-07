use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use ssx_content::limits::{MAX_FILE_BYTES, MAX_RECORD_BYTES};
use ssx_content::path::{MAX_PACKAGE_DEPTH, MAX_PACKAGE_ENTRIES};
use ssx_content::texture::settings::SETTINGS_FILE;
use ssx_content::{API_LEVEL, ContentHash, ContentId, Kind, PackageSource, Source, Version};

use super::store::GameContent;

pub const POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default)]
pub struct DevStatus {
    pub root: PathBuf,
    pub name: Option<String>,
    pub version: Option<Version>,
    pub reloads: u64,
    pub changed: usize,
    pub seconds: f64,
    pub at: Option<SystemTime>,
    pub broken: Vec<String>,
    pub scripts: u64,
}

pub struct DevWatch {
    status: Mutex<DevStatus>,
    stop: AtomicBool,
}

impl DevWatch {
    pub fn status(&self) -> DevStatus {
        self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

type Fingerprint = Vec<(String, u8, u64, Option<SystemTime>)>;

fn fingerprint(root: &Path) -> Fingerprint {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, rel, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            out.push((rel, 9, 0, None));
            continue;
        };
        for e in rd.flatten() {
            if out.len() > MAX_PACKAGE_ENTRIES {
                return out;
            }
            let name = format!("{rel}/{}", e.file_name().to_string_lossy());
            let Ok(meta) = e.path().symlink_metadata() else { continue };
            let kind = if meta.is_symlink() {
                2
            } else if meta.is_dir() {
                1
            } else {
                0
            };
            out.push((name.clone(), kind, meta.len(), meta.modified().ok()));
            if kind == 1 && depth < MAX_PACKAGE_DEPTH {
                stack.push((e.path(), name, depth + 1));
            }
        }
    }
    out.sort();
    out
}

fn signatures(p: &PackageSource) -> Result<BTreeMap<ContentId, ContentHash>, String> {
    let mut read = |tree: &ssx_content::path::PackageTree, rel: &str, max: u64| {
        tree.copy_to(rel, max, &mut std::io::sink()).map(|(_, h)| h).map_err(|e| e.to_string())
    };
    signatures_with(p, &mut read)
}

type FileHash<'a> = dyn FnMut(&ssx_content::path::PackageTree, &str, u64) -> Result<ContentHash, String> + 'a;

fn signatures_with(p: &PackageSource, read: &mut FileHash<'_>) -> Result<BTreeMap<ContentId, ContentHash>, String> {
    let tree = p.tree();
    let mut hashed: BTreeMap<String, ContentHash> = BTreeMap::new();
    let mut hash = |rel: &str, max: u64| -> Result<ContentHash, String> {
        if let Some(h) = hashed.get(rel) {
            return Ok(*h);
        }
        let h = match tree.kind(rel) {
            None => ContentHash::of(b"absent"),
            Some(_) => read(tree, rel, max)?,
        };
        hashed.insert(rel.to_string(), h);
        Ok(h)
    };
    let mut named: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let limits = ssx_content::model::Limits::default();
    let mut out = BTreeMap::new();
    for (id, entry) in p.manifest.add.iter().chain(&p.manifest.overrides) {
        let mut parts = format!("{entry:?}").into_bytes();
        for f in entry.files() {
            parts.extend(hash(f, MAX_FILE_BYTES)?.0);
            let rider_model = super::characters::RIDER_KINDS.contains(&id.kind())
                && ssx_content::model::FORMATS.iter().any(|e| f.to_ascii_lowercase().ends_with(&format!(".{e}")));
            if id.kind() == Kind::Model || rider_model {
                let files = named
                    .entry(f.to_string())
                    .or_insert_with(|| ssx_content::model::files_named(f, tree, &limits).unwrap_or_default());
                for n in files.iter() {
                    parts.extend(n.as_bytes());
                    parts.extend(hash(n, MAX_FILE_BYTES)?.0);
                }
            }
            if id.kind() == Kind::Texture {
                let dir = f.rsplit_once('/').map_or("", |(d, _)| d);
                let settings =
                    if dir.is_empty() { SETTINGS_FILE.to_string() } else { format!("{dir}/{SETTINGS_FILE}") };
                parts.extend(hash(&settings, MAX_RECORD_BYTES)?.0);
            }
            for n in record_files(tree, id, f) {
                parts.extend(n.as_bytes());
                parts.extend(hash(&n, MAX_FILE_BYTES)?.0);
            }
        }
        out.insert(id.clone(), ContentHash::of(&parts));
    }
    for (id, by_lang) in &ssx_content::package::PackageStrings::read(p).texts {
        let mut parts = b"string\0".to_vec();
        for (lang, text) in by_lang {
            parts.extend(lang.code().as_bytes());
            parts.push(0);
            parts.extend(text.as_bytes());
            parts.push(0);
        }
        out.insert(id.clone(), ContentHash::of(&parts));
    }
    for (id, f) in &p.manifest.patches {
        let mut parts = b"patch\0".to_vec();
        parts.extend(hash(f, MAX_RECORD_BYTES)?.0);
        if let Some(a) = out.get(id) {
            parts.extend(a.0);
        }
        out.insert(id.clone(), ContentHash::of(&parts));
    }
    Ok(out)
}

fn record_files(tree: &ssx_content::path::PackageTree, id: &ContentId, f: &str) -> Vec<String> {
    if f.ends_with(".toml") && id.kind() == Kind::Map {
        return map_files(tree, f);
    }
    if id.kind() == Kind::Anim {
        return super::anims::clip_files(tree, f);
    }
    if !f.ends_with(".toml") || !matches!(id.kind(), Kind::Sky | Kind::Material | Kind::Sound | Kind::Music) {
        return Vec::new();
    }
    let Ok(bytes) = tree.read(f, MAX_RECORD_BYTES) else { return Vec::new() };
    match id.kind() {
        Kind::Sound => ssx_content::audio::sounds::parse_toml(&bytes, ssx_content::audio::sounds::SoundClass::of(id))
            .map(|d| d.files)
            .unwrap_or_default(),
        Kind::Music => ssx_content::audio::music::parse_toml(&bytes, None)
            .or_else(|_| ssx_content::audio::music::parse_toml(&bytes, Some(id)))
            .map(|d| d.files().into_iter().map(str::to_string).collect())
            .unwrap_or_default(),
        Kind::Sky => ssx_content::sky::parse_toml(&bytes, f)
            .map(|d| d.files().into_iter().map(str::to_string).collect())
            .unwrap_or_default(),
        _ => ssx_content::material::parse_toml(&bytes, f)
            .ok()
            .and_then(|m| m.shader)
            .map(|s| vec![s.file])
            .unwrap_or_default(),
    }
}

fn map_files(tree: &ssx_content::path::PackageTree, f: &str) -> Vec<String> {
    let Ok(bytes) = tree.read(f, MAX_RECORD_BYTES) else { return Vec::new() };
    let Ok(t) = ssx_content::map::parse_toml(&bytes) else { return Vec::new() };
    let limits = ssx_content::map::scene_limits();
    let mut out = vec![t.scene.clone()];
    out.extend(ssx_content::model::files_named(&t.scene, tree, &limits).unwrap_or_default());
    out
}

fn m4_problem(source: &PackageSource, id: &ContentId) -> Option<String> {
    use ssx_content::particles::{SprayTable, is_spray, system_from_table};
    let asset = match source.load(id) {
        Ok(a) => a,
        Err(e) => return Some(e.to_string()),
    };
    let join = |ps: Vec<String>| Some(format!("{id}: {}", ps.join("; ")));
    match (id.kind(), asset.record()) {
        (Kind::Lighting, Some(t)) => ssx_content::lighting::LightingRecord::from_table(t).err().and_then(join),
        (Kind::Postfx, Some(t)) => ssx_content::postfx::PostFxRecord::from_table(t).err().and_then(join),
        (Kind::Particles, Some(t)) if is_spray(id) => SprayTable::from_table(t).err().and_then(join),
        (Kind::Particles, Some(t)) => system_from_table(t).err().and_then(join),
        (Kind::Material, _) => {
            let m = asset.value::<ssx_content::material::MaterialDef>()?;
            let hook = m.shader.as_ref()?;
            if let Some(p) = &hook.problem {
                return Some(format!("{id}: its shader hook: {p}"));
            }
            let (family, text) = (m.family?.hooks()?, hook.source.as_ref()?);
            let limits = ssx_content::shader::Limits::default();
            ssx_content::shader::check_hook(family, text.as_bytes(), &limits).err().map(|ps| {
                let lines: Vec<String> = ps.iter().map(|(l, c, m)| format!("{}:{l}:{c}: {m}", hook.file)).collect();
                lines.join("; ")
            })
        }
        _ => None,
    }
}

fn audio_problem(source: &PackageSource, id: &ContentId) -> Option<String> {
    use ssx_content::Source;
    let files: Vec<String> = match id.kind() {
        Kind::Sound => match source.sound_def(id) {
            Ok(d) => d.files,
            Err(e) => return Some(e.to_string()),
        },
        _ => match source.music_def(id) {
            Ok(d) => d.files().into_iter().map(str::to_string).collect(),
            Err(e) => return Some(e.to_string()),
        },
    };
    let limits = ssx_content::package::audio_limits(id.kind());
    for rel in files {
        let bytes = match source.audio_file(id, &rel)? {
            Ok(b) => b,
            Err(e) => return Some(e.to_string()),
        };
        let mut d = match ssx_content::audio::Decoder::open(&rel, bytes, &limits) {
            Ok(d) => d,
            Err(e) => return Some(format!("{id}: {e}")),
        };
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match d.next(&mut buf) {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => return Some(format!("{id}: {e}")),
            }
        }
    }
    None
}

fn rider_problem(source: &PackageSource, id: &ContentId) -> Option<String> {
    let entry = match source.rider_entry(id) {
        Ok(e) => e,
        Err(e) => return Some(e.to_string()),
    };
    if let Some((file, table)) = &entry.data {
        let lines = |ps: Vec<ssx_content::material::Problem>| {
            let ls: Vec<String> = ps.iter().map(|(l, c, m)| format!("{file}:{l}:{c}: {m}")).collect();
            Some(format!("{id}: {}", ls.join("; ")))
        };
        let read = match id.kind() {
            Kind::Character => ssx_content::rider::parse_character_table(table).err().and_then(lines),
            k => ssx_content::rider::parse_item_table(table, k == Kind::Board).err().and_then(lines),
        };
        if read.is_some() {
            return read;
        }
    }
    source.rider_model(id).err().map(|e| e.to_string())
}

fn changed(old: &BTreeMap<ContentId, ContentHash>, new: &BTreeMap<ContentId, ContentHash>) -> BTreeSet<ContentId> {
    let keys: BTreeSet<&ContentId> = old.keys().chain(new.keys()).collect();
    keys.into_iter().filter(|id| old.get(*id) != new.get(*id)).cloned().collect()
}

struct Good {
    name: String,
    signatures: BTreeMap<ContentId, ContentHash>,
    scripts: ContentHash,
}

fn scripts_signature(p: &PackageSource) -> Result<ContentHash, String> {
    let m = &p.manifest;
    let mut parts = format!(
        "{}\0{}\0{}\0{:?}\0{:?}\0{:?}",
        m.resource.name, m.resource.version, m.resource.api, m.scripts, m.files, m.settings
    )
    .into_bytes();
    let tree = p.tree();
    let mut luau: Vec<&str> = tree.files().filter(|f| f.ends_with(".luau")).collect();
    luau.sort_unstable();
    for f in luau {
        parts.extend(f.as_bytes());
        parts.extend(tree.copy_to(f, MAX_FILE_BYTES, &mut std::io::sink()).map_err(|e| e.to_string())?.1.0);
    }
    Ok(ContentHash::of(&parts))
}

fn reload(
    root: &Path,
    content: &GameContent,
    mounted: &[(String, Version)],
    good: &mut Option<Good>,
) -> Result<Option<Reloaded>, Vec<String>> {
    let source =
        PackageSource::open(root).map_err(|e| e.to_string().lines().map(str::to_string).collect::<Vec<_>>())?;
    let m = &source.manifest;
    let mut problems = Vec::new();
    if let Some(reason) = super::mods::package_rules(&source) {
        problems.push(reason);
    }
    if m.resource.api > API_LEVEL {
        problems.push(format!("needs API level {}; this game offers {API_LEVEL}", m.resource.api));
    }
    for dep in &m.dependencies.require {
        if !mounted.iter().any(|(name, v)| *name == dep.name && dep.req.matches(*v)) {
            problems.push(format!("requires {} {}, which is not mounted", dep.name, dep.req));
        }
    }
    let new = signatures(&source).map_err(|e| vec![e])?;
    let scripts = scripts_signature(&source).map_err(|e| vec![e])?;
    let old = good.as_ref().map(|g| g.signatures.clone()).unwrap_or_default();
    let renamed = good.as_ref().is_some_and(|g| g.name != m.resource.name);
    let scripts_changed = good.as_ref().is_none_or(|g| g.scripts != scripts);
    let ids = changed(&old, &new);
    if !problems.is_empty() {
        return Err(problems);
    }
    if ids.is_empty() && !renamed && !scripts_changed && good.is_some() {
        return Ok(None);
    }
    let loaded = [Kind::Texture, Kind::Model, Kind::Object];
    for id in ids.iter().filter(|id| loaded.contains(&id.kind()) && new.contains_key(*id)) {
        if let Err(e) = source.load(id) {
            problems.push(e.to_string());
        }
    }
    let mut maps = Vec::new();
    for id in new.keys().filter(|id| id.kind() == Kind::Map && source.manifest.entry(id).is_some()) {
        match source.load(id) {
            Ok(a) => maps.push((id.clone(), a)),
            Err(e) => problems.push(e.to_string()),
        }
    }
    let m4 = [Kind::Material, Kind::Sky, Kind::Lighting, Kind::Postfx, Kind::Particles];
    for id in ids.iter().filter(|id| m4.contains(&id.kind()) && source.manifest.entry(id).is_some()) {
        problems.extend(m4_problem(&source, id));
    }
    for id in ids
        .iter()
        .filter(|id| super::characters::RIDER_KINDS.contains(&id.kind()) && source.manifest.entry(id).is_some())
    {
        problems.extend(rider_problem(&source, id));
    }
    for id in ids.iter().filter(|id| id.kind() == Kind::Movie && source.manifest.entry(id).is_some()) {
        problems.extend(super::fmv::dev_problem(&source, id));
    }
    let mut anims = Vec::new();
    for id in new.keys().filter(|id| id.kind() == Kind::Anim && source.manifest.entry(id).is_some()) {
        match source.load(id) {
            Ok(a) => {
                if ids.contains(id) {
                    problems.extend(content.dev_anim_problem(&a, &m.resource.name, id));
                }
                anims.push((id.clone(), a));
            }
            Err(e) => problems.push(e.to_string()),
        }
    }
    let mut tricks = Vec::new();
    for id in new.keys().filter(|id| id.kind() == Kind::Trick && source.manifest.entry(id).is_some()) {
        match source.load(id) {
            Ok(a) => {
                problems.extend(content.dev_trick_problem(&a, &m.resource.name, id, &anims, &ids));
                tricks.push((id.clone(), a));
            }
            Err(e) => problems.push(e.to_string()),
        }
    }
    let mut gamemodes = Vec::new();
    for id in new.keys().filter(|id| id.kind() == Kind::Gamemode && source.manifest.entry(id).is_some()) {
        match source.load(id) {
            Ok(a) => {
                if let Some(g) = a.clone().into_value::<ssx_content::gamemode::GamemodeToml>() {
                    for e in g.maps.iter().filter(|e| e.map.namespace() == m.resource.name) {
                        if source.manifest.entry(&e.map).is_none() {
                            problems.push(format!("{id}: map.id {} is no entry of this package", e.map));
                        }
                    }
                }
                gamemodes.push((id.clone(), a));
            }
            Err(e) => problems.push(e.to_string()),
        }
    }
    for id in ids.iter().filter(|id| super::data::DATA_KINDS.contains(&id.kind())) {
        problems.extend(content.dev_data_problem(&source, id));
    }
    for id in
        ids.iter().filter(|id| matches!(id.kind(), Kind::Sound | Kind::Music) && source.manifest.entry(id).is_some())
    {
        problems.extend(audio_problem(&source, id));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let (name, version) = (m.resource.name.clone(), m.resource.version);
    let previous = good.as_ref().map(|g| g.name.clone());
    content.swap_dev_package(previous.as_deref(), source, ids.clone());
    content.hold_dev_maps(&name, maps);
    content.hold_dev_anims(&name, anims);
    content.hold_dev_assets(&name, tricks);
    content.hold_dev_assets(&name, gamemodes);
    *good = Some(Good { name: name.clone(), signatures: new, scripts });
    Ok(Some(Reloaded { ids: ids.len(), name, version, scripts: scripts_changed }))
}

struct Reloaded {
    ids: usize,
    name: String,
    version: Version,
    scripts: bool,
}

fn record(watch: &DevWatch, result: Result<Option<Reloaded>, Vec<String>>, start: Instant) {
    let mut s = watch.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match result {
        Ok(Some(r)) => {
            (s.name, s.version, s.changed) = (Some(r.name), Some(r.version), r.ids);
            s.scripts += u64::from(r.scripts);
            s.reloads += 1;
            (s.seconds, s.at) = (start.elapsed().as_secs_f64(), Some(SystemTime::now()));
            s.broken.clear();
        }
        Ok(None) => s.broken.clear(),
        Err(why) => {
            s.broken = why;
            s.at = Some(SystemTime::now());
        }
    }
}

pub fn watch(root: &Path, content: Arc<GameContent>, mounted: Vec<(String, Version)>) -> Arc<DevWatch> {
    let watch = Arc::new(DevWatch {
        status: Mutex::new(DevStatus { root: root.to_path_buf(), ..DevStatus::default() }),
        stop: AtomicBool::new(false),
    });
    let mut good = None;
    let mut last = fingerprint(root);
    let start = Instant::now();
    record(&watch, reload(root, &content, &mounted, &mut good), start);
    let (thread_watch, root) = (watch.clone(), root.to_path_buf());
    std::thread::spawn(move || {
        let mut pending: Option<Fingerprint> = None;
        while !thread_watch.stop.load(Ordering::Relaxed) {
            std::thread::sleep(POLL);
            let now = fingerprint(&root);
            match &pending {
                Some(p) if *p == now => {
                    pending = None;
                    last = now;
                    let start = Instant::now();
                    record(&thread_watch, reload(&root, &content, &mounted, &mut good), start);
                }
                _ if now != last => pending = Some(now),
                _ => pending = None,
            }
        }
    });
    watch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_naming_one_file_read_it_once() {
        let dir = std::env::temp_dir().join(format!("ssx-dev-signatures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("models")).unwrap();
        let mut manifest = String::from("[resource]\nname = \"many\"\nversion = \"1.0.0\"\napi = 1\n[add]\n");
        for i in 0..2000 {
            manifest.push_str(&format!("\"model/m{i}\" = \"models/m.gltf\"\n"));
        }
        std::fs::write(dir.join("resource.toml"), manifest).unwrap();
        std::fs::write(
            dir.join("models/m.gltf"),
            br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"m.bin","byteLength":4}]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("models/m.bin"), [0u8; 4]).unwrap();
        let p = PackageSource::open(&dir).unwrap();
        let mut reads = Vec::new();
        let mut read = |tree: &ssx_content::path::PackageTree, rel: &str, max: u64| {
            reads.push(rel.to_string());
            tree.copy_to(rel, max, &mut std::io::sink()).map(|(_, h)| h).map_err(|e| e.to_string())
        };
        let signed = signatures_with(&p, &mut read).unwrap();
        assert_eq!(reads, ["models/m.gltf", "models/m.bin"]);
        assert_eq!(signed.len(), 2000);
        assert!(signed.values().all(|h| *h == signed[&"many:model/m0".parse::<ContentId>().unwrap()]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_map_is_read_at_the_next_start_and_a_broken_one_keeps_the_last() {
        use ssx_content::map::template;
        let dir = std::env::temp_dir().join(format!("ssx-dev-map-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("maps")).unwrap();
        let manifest =
            "[resource]\nname = \"hill\"\nversion = \"1.0.0\"\napi = 1\n[add]\n\"map/course\" = \"map.toml\"\n";
        std::fs::write(dir.join("resource.toml"), manifest).unwrap();
        std::fs::write(dir.join("map.toml"), template::MAP_TOML).unwrap();
        let scene = template::gltf();
        std::fs::write(dir.join("maps/course.gltf"), &scene).unwrap();
        let content = GameContent::of(&dir.join("no-disc"));
        let id: ContentId = "hill:map/course".parse().unwrap();
        let spawn = |c: &GameContent| c.map(&id).expect("the map").value.definition.spawns[0].position;
        let mut good = None;
        assert!(reload(&dir, &content, &[], &mut good).expect("mounts").is_some());
        assert_eq!(spawn(&content), [0.0, -1.5, 5.0]);
        let at = "\"name\":\"spawn\",\"translation\":[0.0,-1.5,5.0]";
        assert!(scene.contains(at));
        let moved = scene.replace(at, "\"name\":\"spawn\",\"translation\":[2.0,-1.5,5.0]");
        std::fs::write(dir.join("maps/course.gltf"), &moved).unwrap();
        let r = reload(&dir, &content, &[], &mut good).expect("reloads").expect("a change");
        assert_eq!(r.ids, 1, "the map's id changed with its scene");
        assert_eq!(spawn(&content), [2.0, -1.5, 5.0]);
        std::fs::write(dir.join("maps/course.gltf"), moved.replace("\"ava_slot\":0", "\"ava_slot\":1")).unwrap();
        let Err(why) = reload(&dir, &content, &[], &mut good) else { panic!("a spawn without slot 0 is refused") };
        assert!(why.iter().any(|w| w.contains("ava_slot")), "{why:?}");
        assert_eq!(spawn(&content), [2.0, -1.5, 5.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_clip_is_read_at_the_next_start_and_a_broken_one_keeps_the_last() {
        use ssx_content::Layer;
        use ssx_content::anim::{Clip, template, write};
        let dir = std::env::temp_dir().join(format!("ssx-dev-anim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("anims")).unwrap();
        let manifest = format!(
            "[resource]\nname = \"sway\"\nversion = \"1.0.0\"\napi = 1\n[override]\n\"{}\" = \"anims/sway.toml\"\n",
            template::TARGET
        );
        std::fs::write(dir.join("resource.toml"), manifest).unwrap();
        std::fs::write(dir.join("anims/sway.toml"), template::ANIM_TOML.replace("carve_sway.glb", "sway.glb")).unwrap();
        let glb = |clip: &Clip| write::glb(&template::joints(), template::ANIMATION, clip).unwrap();
        let mut clip = template::clip();
        std::fs::write(dir.join("anims/sway.glb"), glb(&clip)).unwrap();
        let content = GameContent::of(&dir.join("no-disc"));
        let id: ContentId = template::TARGET.parse().unwrap();
        let held = |c: &GameContent| -> Arc<Clip> {
            let registry = c.registry.read().unwrap();
            let (info, source) = registry.providers(&id).find(|(i, _)| i.layer == Layer::ClientMods).expect("mounted");
            c.cached_load(&id, info.name, info.layer, source).expect("held").into_value::<Clip>().expect("a clip")
        };
        let mut good = None;
        assert!(reload(&dir, &content, &[], &mut good).expect("mounts").is_some());
        assert_eq!(*held(&content), clip);
        clip.joints[0].keys.values[4] = 0.25;
        std::fs::write(dir.join("anims/sway.glb"), glb(&clip)).unwrap();
        let r = reload(&dir, &content, &[], &mut good).expect("reloads").expect("a change");
        assert_eq!(r.ids, 1, "the clip's id changed with its glTF");
        assert_eq!(held(&content).joints[0].keys.values[4], 0.25);
        let mut broken = clip.clone();
        broken.joints[0].keys.times.swap(1, 2);
        std::fs::write(dir.join("anims/sway.glb"), glb(&broken)).unwrap();
        let Err(why) = reload(&dir, &content, &[], &mut good) else { panic!("keys out of order are refused") };
        assert!(why.iter().any(|w| w.contains("strictly increase")), "{why:?}");
        assert_eq!(*held(&content), clip);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
