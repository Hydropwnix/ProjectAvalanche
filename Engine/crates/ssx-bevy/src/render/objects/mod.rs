pub mod foreground;
mod load;
mod lods;
pub mod material;
mod part;
mod specs;

use std::collections::BTreeMap;

use bevy::camera::CameraUpdateSystems;
use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use ssx_assets::track_objects::Placement;

use load::ObjectLod;
pub use load::load;
pub use lods::{ObjectLods, placement_transform};
use lods::{follow_bodies, spawn_objects, update_object_lods};
pub use part::{LoadedPart, load_part, part_from_mesh};
pub use specs::MaterialHandle;
use specs::MaterialSpec;

const SHADOW_EFFECT: &str = "object_shadow.fx";
const FOREGROUND_EFFECT: &str = "terrain_foreground.fx";
const ALPHA_CUTOFF: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EffectKind {
    Opaque,
    Cutout,
    Foreground,
}

impl EffectKind {
    fn of(effect: &str) -> EffectKind {
        if effect.starts_with("object_foliage")
            || effect.starts_with("object_alpha")
            || effect == "player_hair.fx"
            || effect == "player_apparel.fx"
            || effect == "player_board.fx"
            || effect == "player_secondary.fx"
        {
            EffectKind::Cutout
        } else if effect == FOREGROUND_EFFECT {
            EffectKind::Foreground
        } else {
            EffectKind::Opaque
        }
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub struct ObjectStats {
    pub objects_drawn: usize,
    pub objects: usize,
    pub placements: usize,
    pub entities: usize,
    pub meshes: usize,
    pub materials: usize,
    pub textures: usize,
    pub load_secs: f32,
    pub effects: BTreeMap<String, usize>,
    pub shadow_proxies: usize,
    pub problems: Vec<String>,
}

impl ObjectStats {
    pub fn summary(&self) -> String {
        let effects: Vec<String> = self
            .effects
            .iter()
            .map(|(effect, n)| {
                let short = effect.trim_end_matches(".fx");
                match effect_note(effect) {
                    Some(note) => format!("{short} x{n} ({note})"),
                    None => format!("{short} x{n}"),
                }
            })
            .collect();
        let mut out = format!(
            "objects: {}/{} drawn | {} placements | {} entities | {} meshes | {} materials | {} textures ({:.2}s)\n\
             effects: {}",
            self.objects_drawn,
            self.objects,
            self.placements,
            self.entities,
            self.meshes,
            self.materials,
            self.textures,
            self.load_secs,
            effects.join(", "),
        );
        if self.shadow_proxies > 0 {
            out += &format!("\n  {SHADOW_EFFECT}: {} invisible shadow casters", self.shadow_proxies);
        }
        if !self.problems.is_empty() {
            out += &format!("\n  {} problems (missing files / unset parameters, see log)", self.problems.len());
        }
        out + "\n" + crate::render::objects::material::OBJECT_STATUS
    }
}

fn effect_note(effect: &str) -> Option<&'static str> {
    let anim = effect.contains("_anim");
    match EffectKind::of(effect) {
        EffectKind::Cutout if anim => Some("alpha-tested, no wind"),
        EffectKind::Cutout => Some("alpha-tested"),
        EffectKind::Foreground => Some("forward ramp + fog, listing port"),
        EffectKind::Opaque if effect == "object_texture_anim.fx" => Some("no UV animation"),
        EffectKind::Opaque if anim => Some("no wind"),
        EffectKind::Opaque => None,
    }
}

pub struct LoadedObjects {
    pub stats: ObjectStats,
    meshes: Vec<(String, Mesh)>,
    images: Vec<Image>,
    materials: Vec<MaterialSpec>,
    parts: Vec<Vec<Vec<(usize, usize)>>>,
    lods: Vec<ObjectLod>,
    placements: Vec<(usize, Placement)>,
}

#[derive(Resource)]
struct PendingObjects(Option<LoadedObjects>);

pub fn register(app: &mut App) {
    app.add_systems(crate::shell::level::LevelStartup, spawn_objects.run_if(resource_exists::<PendingObjects>))
        .add_systems(
            PostUpdate,
            update_object_lods
                .after(TransformSystems::Propagate)
                .after(CameraUpdateSystems)
                .before(VisibilitySystems::VisibilityPropagate),
        );
    app.add_systems(PostUpdate, follow_bodies.before(TransformSystems::Propagate));
    crate::shell::level::scoped::<PendingObjects>(app);
    crate::shell::level::scoped::<ObjectStats>(app);
    crate::shell::level::scoped::<ObjectLods>(app);
}

pub fn insert(world: &mut World, objects: LoadedObjects) {
    world.insert_resource(objects.stats.clone());
    world.insert_resource(PendingObjects(Some(objects)));
}
