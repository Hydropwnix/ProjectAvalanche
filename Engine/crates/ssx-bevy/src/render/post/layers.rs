use bevy::prelude::{App, Resource, World};
use ssx_assets::postfx::LevelPresets;
use ssx_assets::postfx_state::{FIELDS, PostFxScene, PostFxState};

#[derive(Resource)]
pub struct PostFxLayers {
    pub scene: PostFxScene,
}

pub(super) fn build(app: &mut App) {
    crate::shell::level::scoped::<PostFxLayers>(app);
}

pub struct LayersLoad {
    scene: PostFxScene,
    refused: Vec<String>,
    failed: Vec<String>,
}

impl LayersLoad {
    pub fn scene(&self) -> &PostFxScene {
        &self.scene
    }
}

pub fn load(presets: &LevelPresets) -> LayersLoad {
    let refused = std::iter::once(("default", &presets.base))
        .chain(presets.named.iter().map(|(n, p)| (n.as_str(), p)))
        .filter_map(|(n, p)| {
            let r = PostFxState::refused(p);
            (!r.is_empty()).then(|| format!("{n}: {}", r.join(", ")))
        })
        .collect();
    LayersLoad { scene: PostFxScene::of(presets), refused, failed: presets.failed.clone() }
}

fn fields_of(s: &PostFxState) -> String {
    let names: Vec<String> = FIELDS
        .iter()
        .zip(&s.fields)
        .filter(|(_, v)| v.is_some())
        .map(|(f, _)| f.name.map_or_else(|| format!("{:#010x}", f.key), str::to_string))
        .collect();
    let cubes =
        if s.colour_cubes.is_empty() { String::new() } else { format!(" + {} colour cubes", s.colour_cubes.len()) };
    format!("{}{cubes}", names.join(" "))
}

pub fn insert(world: &mut World, load: LayersLoad) -> (String, Option<String>) {
    let LayersLoad { scene, refused, failed } = load;
    let presets: Vec<String> =
        scene.presets.iter().map(|(n, (p, s))| format!("{n} (layer priority {p:?}: {})", fields_of(s))).collect();
    let line = format!("post-FX layers: {}; not loaded (their colour cubes): {failed:?}", presets.join(", "));
    let warning = (!refused.is_empty()).then(|| format!("post-FX presets: not in the state: {}", refused.join("; ")));
    world.insert_resource(PostFxLayers { scene });
    (line, warning)
}
