use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::pbr::wireframe::WireframeConfig;
use bevy::prelude::*;

use crate::camera::FlyCam;
use crate::render::lighting::GameLighting;
use crate::render::lighting::game::{GameLightAssets, LIGHTING_STATUS};
use crate::render::objects::ObjectStats;
use crate::render::terrain::material::TerrainFades;
use crate::render::terrain::{ShadingLabel, TrackStats};

const CONTROLS: &str = "WASD move | Q/E down/up | hold RMB look | Shift fast | wheel speed | F wireframe";

#[derive(Component)]
pub struct HudText;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(crate::shell::level::LevelStartup, spawn_hud)
            .add_systems(Update, update_hud.run_if(resource_exists::<TrackStats>));
    }
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        HudText,
        Text::default(),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
        Node {
            position_type: PositionType::Absolute,
            top: px(10),
            left: px(10),
            padding: UiRect::all(px(8)),
            ..default()
        },
    ));
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    stats: Res<TrackStats>,
    shading: Res<ShadingLabel>,
    fades: Option<Res<TerrainFades>>,
    lighting: Option<Res<GameLighting>>,
    post: Option<Res<crate::render::post::PostStatus>>,
    light_assets: Option<Res<GameLightAssets>>,
    objects: Option<Res<ObjectStats>>,
    wireframe: Res<WireframeConfig>,
    diagnostics: Res<DiagnosticsStore>,
    mut frames: Local<u32>,
    cameras: Query<Ref<FlyCam>>,
    mut text: Single<&mut Text, With<HudText>>,
) {
    let Some(cam) = cameras.iter().next() else {
        return;
    };
    *frames = frames.wrapping_add(1);
    let fades_changed = fades.as_ref().is_some_and(|f| f.is_changed());
    if !cam.is_changed() && !wireframe.is_changed() && !fades_changed && !(*frames).is_multiple_of(30) {
        return;
    }
    let frame_ms = diagnostics.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let objects = objects.map_or_else(|| "objects: off".to_string(), |o| o.summary());
    let fades = fades.map_or_else(String::new, |f| {
        let state = if f.0 { "on (game: snow detail to 125 m, rock to 480 m)" } else { "OFF (viewer)" };
        format!("\nterrain distance fades: {state} | G toggles")
    });
    let elevation = |d: Vec3| d.y.clamp(-1.0, 1.0).asin().to_degrees();
    let lighting = lighting.map_or_else(
        || "\nlighting: none (no drop found)".to_string(),
        |l| {
            format!(
                "\nlighting: {} | presets on this track: {}\n  sun {:.1} deg, key light (offset) {:.1} deg; {}",
                l.label(),
                l.available.join(", "),
                elevation(l.sun),
                elevation(l.sun_offset),
                light_assets.as_ref().map_or("", |a| a.summary.as_str()),
            )
        },
    );
    let post = post.map_or_else(String::new, |p| format!("\n{}", p.0));
    let lighting = format!("{lighting}\n{LIGHTING_STATUS}{post}");
    text.0 = format!(
        "{}\nsurface: {}\nshading: {}{fades}{lighting}\n{} files | {} meshes | {} vertices | {} triangles\n{objects}\n\
         speed {:.0} m/s | wireframe {} | {frame_ms:.1} ms/frame\n{CONTROLS}",
        stats.name,
        stats.surface,
        shading.0,
        stats.files,
        stats.meshes,
        stats.vertices,
        stats.triangles,
        cam.speed,
        if wireframe.global { "on" } else { "off" },
    );
}
