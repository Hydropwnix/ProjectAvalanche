use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bevy::asset::io::AssetSourceId;
use bevy::asset::io::embedded::EmbeddedAssetRegistry;
use bevy::prelude::{App, AssetId, AssetServer, Assets, DefaultPlugins, Handle, PluginGroup, WindowPlugin, default};
use bevy::render::RenderPlugin;
use bevy::render::settings::{RenderCreation, WgpuSettings};
use bevy::shader::Shader;
use bevy::tasks::block_on;
use bevy::tasks::futures_lite::StreamExt;

use super::sets::CRATE;

pub fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

pub fn files_with(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> =
        std::fs::read_dir(dir).expect("source dir").map(|e| e.expect("dir entry").path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files_with(&path, exts, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.contains(&e)) {
            out.push(path);
        }
    }
}

pub fn below_src(path: &Path) -> String {
    path.strip_prefix(src_dir()).expect("below src").to_string_lossy().replace('\\', "/")
}

pub fn shader_files_on_disk() -> BTreeMap<String, PathBuf> {
    let mut files = Vec::new();
    files_with(&src_dir(), &["wesl", "wgsl"], &mut files);
    files.into_iter().map(|f| (format!("{CRATE}/{}", below_src(&f)), f)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Registration {
    Embedded,
    Included,
}

pub fn shader_files_registered() -> BTreeMap<String, Registration> {
    let mut sources = Vec::new();
    files_with(&src_dir(), &["rs"], &mut sources);
    let mut out = BTreeMap::new();
    for file in sources {
        let source = std::fs::read_to_string(&file).expect("rust source");
        let text: String = source
            .lines()
            .map(|l| if l.trim_start().starts_with("//") { "" } else { l })
            .collect::<Vec<_>>()
            .join("\n");
        let dir = below_src(file.parent().expect("parent"));
        for (mac, how) in [
            ("embedded_asset!(", Registration::Embedded),
            ("load_shader_library!(", Registration::Embedded),
            ("include_str!(", Registration::Included),
        ] {
            for (at, _) in text.match_indices(mac) {
                if text[..at].ends_with('"') {
                    continue;
                }
                let args = &text[at + mac.len()..];
                let Some(open) = args.find('"') else { continue };
                let Some(len) = args[open + 1..].find('"') else { continue };
                let name = &args[open + 1..open + 1 + len];
                if how == Registration::Included && !(name.ends_with(".wgsl") || name.ends_with(".wesl")) {
                    continue;
                }
                let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
                for part in name.split('/') {
                    match part {
                        ".." => drop(parts.pop()),
                        "." => {}
                        _ => parts.push(part),
                    }
                }
                let path = format!("{CRATE}/{}", parts.join("/"));
                if let Some(other) = out.insert(path.clone(), how) {
                    assert_eq!(other, how, "{path}: registered both ways");
                }
            }
        }
    }
    out
}

pub fn embedded_shader_paths(app: &App, dir: &Path, out: &mut Vec<String>) {
    let server = app.world().resource::<AssetServer>();
    let source = server.get_source(AssetSourceId::Name("embedded".into())).expect("the embedded asset source");
    let reader = source.reader();
    let mut stream = block_on(reader.read_directory(dir)).expect("an embedded directory");
    let mut entries = Vec::new();
    while let Some(path) = block_on(stream.next()) {
        entries.push(path);
    }
    entries.sort();
    for path in entries {
        if block_on(reader.is_directory(&path)).expect("an embedded path") {
            embedded_shader_paths(app, &path, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e == "wesl" || e == "wgsl") {
            out.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
}

pub fn loaded_shaders() -> Vec<(String, AssetId<Shader>, Shader)> {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::log::LogPlugin>()
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::audio::AudioPlugin>()
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings { backends: None, ..default() })),
                ..default()
            }),
    );
    let registrations = shader_files_registered();
    let mut included = Vec::new();
    {
        let registry = app.world().resource::<EmbeddedAssetRegistry>();
        for (path, file) in shader_files_on_disk() {
            let text = std::fs::read_to_string(&file).expect("a shader file");
            match registrations.get(&path) {
                Some(Registration::Embedded) => registry.insert_asset(file, Path::new(&path), text.into_bytes()),
                Some(Registration::Included) => included.push((path.clone(), text)),
                None => panic!("{path}: not registered"),
            }
        }
    }
    app.finish();
    app.cleanup();
    let mut paths = Vec::new();
    embedded_shader_paths(&app, Path::new(""), &mut paths);
    let handles: Vec<(String, Handle<Shader>)> = {
        let server = app.world().resource::<AssetServer>();
        paths.into_iter().map(|p| (p.clone(), server.load(format!("embedded://{p}")))).collect()
    };
    for frame in 0.. {
        app.update();
        let server = app.world().resource::<AssetServer>();
        let mut waiting = 0;
        for (path, handle) in &handles {
            match server.load_state(handle.id()) {
                bevy::asset::LoadState::Loaded => {}
                bevy::asset::LoadState::Failed(e) => panic!("{path}: {e}"),
                _ => waiting += 1,
            }
        }
        if waiting == 0 {
            break;
        }
        assert!(frame < 100_000, "{waiting} shaders still loading");
    }
    let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
    let mut out: Vec<(String, AssetId<Shader>, Shader)> = handles
        .into_iter()
        .map(|(path, handle)| {
            let shader = shaders.get(handle.id()).expect("a loaded shader").clone();
            (path, handle.id(), shader)
        })
        .collect();
    for (path, text) in included {
        let shader = Shader::from_wgsl(text, path.trim_start_matches("ssx_bevy/").to_string());
        let id = shaders.add(shader.clone()).id();
        out.push((path, id, shader));
    }
    for (path, text) in super::support_modules() {
        let shader = Shader::from_wesl(text, path.clone());
        let id = shaders.add(shader.clone()).id();
        out.push((path.trim_start_matches("embedded://").to_string(), id, shader));
    }
    out
}
