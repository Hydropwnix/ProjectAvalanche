pub mod compose;
pub mod headless;
pub mod runtime;
pub mod sets;

use bevy::prelude::*;
use bevy::shader::Shader;
use ssx_content::shader::{HookFamily, composed_source};

pub fn support_modules() -> Vec<(String, String)> {
    let object_types = "package::render::objects::hook_types";
    let sky_types = "package::render::sky::hook_types";
    vec![
        ("embedded://ssx_bevy/render/objects/hook_types.wesl".into(), ssx_content::shader::OBJECT_TYPES.into()),
        ("embedded://ssx_bevy/render/sky/hook_types.wesl".into(), ssx_content::shader::SKY_TYPES.into()),
        (
            "embedded://ssx_bevy/render/objects/hook.wesl".into(),
            composed_source(object_types, HookFamily::Object.identity()),
        ),
        (
            "embedded://ssx_bevy/render/lighting/hook.wesl".into(),
            composed_source(object_types, HookFamily::Object.identity()),
        ),
        ("embedded://ssx_bevy/render/sky/hook.wesl".into(), composed_source(sky_types, HookFamily::Sky.identity())),
    ]
}

#[derive(Resource)]
struct SupportModules {
    _handles: Vec<Handle<Shader>>,
}

pub struct HooksPlugin;

impl Plugin for HooksPlugin {
    fn build(&self, app: &mut App) {
        let Some(mut shaders) = app.world_mut().get_resource_mut::<Assets<Shader>>() else { return };
        let handles =
            support_modules().into_iter().map(|(path, source)| shaders.add(Shader::from_wesl(source, path))).collect();
        app.insert_resource(SupportModules { _handles: handles });
        app.init_resource::<runtime::HookQueue>().add_systems(Update, (runtime::start, runtime::finish).chain());
    }
}
